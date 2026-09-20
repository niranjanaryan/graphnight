use crate::auth::{
    enforce_policy, reject_if_auth_required, require_admin_if_auth, require_user_if_auth,
};
use crate::context::GraphQLContext;
use crate::multistage::{MultiStageExecutor, StageInput};
use crate::schema::JsonValue;
use crate::schema::*;
use async_graphql::*;
use graphnight_core::models::{DataSource, Model, Query as CoreQuery};
use graphnight_core::security::AuditEntry;
use graphnight_core::validate_connection_string_input;
use graphnight_sql::{IntrospectionConfig, SchemaIntrospector, SqlEngine, infer_model_from_table};
use graphnight_storage::StorageBackend;
use std::sync::Arc;
use tracing::{info, warn};

/// GraphQL Query resolvers
pub struct QueryRoot {
    sql_engine: Arc<SqlEngine>,
    storage: Arc<dyn StorageBackend>,
}

impl QueryRoot {
    pub fn new(sql_engine: Arc<SqlEngine>, storage: Arc<dyn StorageBackend>) -> Self {
        Self {
            sql_engine,
            storage,
        }
    }

    async fn execute_query(
        &self,
        ctx: &Context<'_>,
        input: QueryInput,
        dry_run: Option<bool>,
        start: std::time::Instant,
        model_name_out: &mut Option<String>,
    ) -> Result<QueryResponse> {
        reject_if_auth_required(ctx)?;
        let mut query: CoreQuery = input.into();

        let model_name = query
            .name
            .as_ref()
            .or_else(|| query.source_model.as_ref().map(|s| &s.model))
            .ok_or_else(|| Error::new("Query must have a name or source_model"))?
            .clone();
        *model_name_out = Some(model_name.clone());

        let model = self
            .storage
            .get_model(&model_name, None)
            .await?
            .ok_or_else(|| Error::new(format!("Model not found: {}", model_name)))?;

        let gql_ctx = ctx.data::<GraphQLContext>().ok();
        let policy = gql_ctx.and_then(|g| g.session_policy.as_ref().cloned());

        enforce_policy(ctx, &mut query, &model_name, &model.datasource)?;

        let query_timeout = policy
            .as_ref()
            .and_then(|p| p.query_timeout_secs)
            .unwrap_or(300);

        if dry_run.unwrap_or(false) {
            let sql = self.sql_engine.generate_sql(&query)?;
            return Ok(QueryResponse {
                data: vec![],
                columns: vec![],
                sql: Some(sql),
                attributes: None,
                population: None,
                population_inferred: false,
                execution_time_ms: start.elapsed().as_millis() as f64,
            });
        }

        let datasource = self
            .storage
            .get_datasource(&model.datasource)
            .await?
            .ok_or_else(|| Error::new(format!("Datasource not found: {}", model.datasource)))?;

        let sql = self.sql_engine.generate_sql(&query)?;

        let execute_future = self.sql_engine.execute_sqlx(&datasource, &sql);
        let results = tokio::time::timeout(
            std::time::Duration::from_secs(query_timeout),
            execute_future,
        )
        .await
        .map_err(|_| anyhow::anyhow!("Query timeout exceeded ({}s)", query_timeout))??;

        let columns = if !results.is_empty() {
            results[0].keys().cloned().collect()
        } else {
            vec![]
        };
        let row_count = results.len();

        let mut data: Vec<JsonValue> = results
            .into_iter()
            .map(|m| serde_json::to_value(m).unwrap())
            .collect();

        // Apply column masks from policy
        if let Some(policy) = gql_ctx.and_then(|g| g.session_policy.as_ref()) {
            if !policy.column_masks.is_empty() {
                for row in &mut data {
                    if let JsonValue::Object(map) = row {
                        for (col, mask_fn) in &policy.column_masks {
                            if let Some(JsonValue::String(val)) = map.get(col) {
                                let masked = mask_fn(val);
                                map.insert(col.clone(), JsonValue::String(masked));
                            }
                        }
                    }
                }
            }
        }

        info!(
            model = %model_name,
            rows = row_count,
            duration_ms = start.elapsed().as_millis() as u64,
            "query executed"
        );

        Ok(QueryResponse {
            data,
            columns,
            sql: Some(sql),
            attributes: None,
            population: None,
            population_inferred: false,
            execution_time_ms: start.elapsed().as_millis() as f64,
        })
    }
}

#[Object]
impl QueryRoot {
    /// Execute a semantic query
    async fn query(
        &self,
        ctx: &Context<'_>,
        input: QueryInput,
        dry_run: Option<bool>,
        _explain: Option<bool>,
    ) -> Result<QueryResponse> {
        let start = std::time::Instant::now();
        let gql = ctx.data::<GraphQLContext>().ok();
        let user_id = gql.and_then(|g| g.user_id.clone());
        let tenant_id = gql.and_then(|g| g.tenant_id.clone());
        let audit_sink = gql.and_then(|g| g.audit_sink.clone());

        let mut model_name: Option<String> = None;
        let result = self
            .execute_query(ctx, input, dry_run, start, &mut model_name)
            .await;

        if let Some(sink) = audit_sink {
            let duration_ms = start.elapsed().as_millis() as u64;
            let (success, row_count, error) = match &result {
                Ok(resp) => (true, Some(resp.data.len()), None),
                Err(e) => (false, None, Some(e.message.clone())),
            };
            let entry = AuditEntry {
                timestamp: chrono::Utc::now(),
                user_id,
                tenant_id,
                action: "query".to_string(),
                model: model_name,
                query_hash: None,
                row_count,
                duration_ms,
                success,
                error,
            };
            if let Err(e) = sink.write(&entry) {
                warn!(error = %e, "failed to write durable audit entry");
            }
        }

        result
    }

    /// Execute multiple queries as a DAG.
    ///
    /// Each query can reference a previous stage via `stage_ref` to filter
    /// results based on prior stage output.
    async fn multi_stage_query(
        &self,
        ctx: &Context<'_>,
        inputs: Vec<QueryInput>,
        dry_run: Option<bool>,
    ) -> Result<MultiStageResponse> {
        reject_if_auth_required(ctx)?;
        let start = std::time::Instant::now();

        let is_dry_run = dry_run.unwrap_or(false);

        let mut stages = Vec::new();
        let mut stage_names = std::collections::HashSet::new();

        for (idx, input) in inputs.into_iter().enumerate() {
            // Always auto-generate stage name; stage_ref is only for dependency
            let stage_name = format!("stage_{}", idx + 1);

            if !stage_names.insert(stage_name.clone()) {
                return Err(Error::new(format!(
                    "Duplicate stage name: {}",
                    stage_name
                )));
            }

            let query: CoreQuery = input.into();
            let depends_on = query
                .stage_ref
                .as_ref()
                .map(|s| vec![s.clone()])
                .unwrap_or_default();

            stages.push(StageInput {
                query,
                stage_name: stage_name.clone(),
                depends_on,
            });
        }

        let gql_ctx = ctx.data::<GraphQLContext>().ok();

        let executor = MultiStageExecutor::new(self.sql_engine.clone(), self.storage.clone())
            .with_query_timeout(std::time::Duration::from_secs(
                gql_ctx
                    .and_then(|g| g.session_policy.as_ref())
                    .and_then(|p| p.query_timeout_secs)
                    .unwrap_or(300),
            ));
        let stage_results = executor.execute_dag(stages).await.map_err(|e| Error::new(e.to_string()))?;

        let mut results = Vec::new();
        for stage_result in stage_results {
            let columns = if !stage_result.data.is_empty() {
                stage_result.data[0].keys().cloned().collect()
            } else {
                vec![]
            };

            let mut data: Vec<JsonValue> = stage_result
                .data
                .into_iter()
                .map(|m| serde_json::to_value(m).unwrap())
                .collect();

            // Apply column masks from policy
            if let Some(policy) = gql_ctx.and_then(|g| g.session_policy.as_ref()) {
                if !policy.column_masks.is_empty() {
                    for row in &mut data {
                        if let JsonValue::Object(map) = row {
                            for (col, mask_fn) in &policy.column_masks {
                                if let Some(JsonValue::String(val)) = map.get(col) {
                                    let masked = mask_fn(val);
                                    map.insert(col.clone(), JsonValue::String(masked));
                                }
                            }
                        }
                    }
                }
            }

            let sql = if is_dry_run {
                Some(stage_result.sql)
            } else {
                None
            };

            results.push(QueryResponse {
                data,
                columns,
                sql,
                attributes: None,
                population: None,
                population_inferred: false,
                execution_time_ms: start.elapsed().as_millis() as f64,
            });
        }

        Ok(MultiStageResponse {
            results,
            execution_time_ms: start.elapsed().as_millis() as f64,
        })
    }

    /// List all models
    async fn models(
        &self,
        _ctx: &Context<'_>,
        datasource: Option<String>,
    ) -> Result<Vec<ModelInfo>> {
        let models = self.storage.list_models(datasource.as_deref()).await?;
        Ok(models
            .into_iter()
            .map(|m| ModelInfo {
                name: m.name,
                datasource: m.datasource,
                description: m.description,
                measures: m
                    .measures
                    .iter()
                    .map(|m| m.formula.expression.clone())
                    .collect(),
                dimensions: m.dimensions.iter().map(|d| d.name.clone()).collect(),
                time_dimensions: m
                    .time_dimensions
                    .iter()
                    .map(|t| t.dimension.clone())
                    .collect(),
            })
            .collect())
    }

    /// Get a specific model
    async fn model(
        &self,
        _ctx: &Context<'_>,
        name: String,
        datasource: Option<String>,
    ) -> Result<Option<ModelInfo>> {
        let model = self.storage.get_model(&name, datasource.as_deref()).await?;
        Ok(model.map(|m| ModelInfo {
            name: m.name,
            datasource: m.datasource,
            description: m.description,
            measures: m
                .measures
                .iter()
                .map(|m| m.formula.expression.clone())
                .collect(),
            dimensions: m.dimensions.iter().map(|d| d.name.clone()).collect(),
            time_dimensions: m
                .time_dimensions
                .iter()
                .map(|t| t.dimension.clone())
                .collect(),
        }))
    }

    /// List all datasources
    async fn datasources(&self, _ctx: &Context<'_>) -> Result<Vec<DatasourceInfo>> {
        let datasources = self.storage.list_datasources().await?;
        Ok(datasources
            .into_iter()
            .map(|d| DatasourceInfo {
                name: d.name,
                driver: d.driver,
                description: d.description,
                models: d.models,
            })
            .collect())
    }

    /// Search models
    async fn search(
        &self,
        _ctx: &Context<'_>,
        q: String,
        limit: Option<i32>,
    ) -> Result<Vec<SearchResult>> {
        let results = self
            .storage
            .search(&q, limit.unwrap_or(10) as usize)
            .await?;
        Ok(results
            .into_iter()
            .map(|r| SearchResult {
                model_name: r.model_name,
                datasource: r.datasource,
                score: r.score,
                matched_fields: r.matched_fields,
                snippet: r.snippet,
            })
            .collect())
    }

    /// Inspect a model
    async fn inspect(
        &self,
        ctx: &Context<'_>,
        model: String,
        datasource: String,
    ) -> Result<Option<ModelInfo>> {
        self.model(ctx, model, Some(datasource)).await
    }

    /// List memories
    async fn memories(&self, _ctx: &Context<'_>, filter: MemoryFilter) -> Result<Vec<Memory>> {
        let filter_model = graphnight_storage::MemoryFilter {
            query: filter.query,
            entity: filter.entity,
            limit: filter.limit.map(|l| l as usize),
            offset: filter.offset.map(|o| o as usize),
        };
        let memories = self.storage.list_memories(filter_model).await?;
        Ok(memories
            .into_iter()
            .map(|m| Memory {
                id: m.id,
                learning: m.learning,
                linked_entities: m.linked_entities,
                description: m.description,
                created_at: m.created_at.to_rfc3339(),
                updated_at: m.updated_at.to_rfc3339(),
            })
            .collect())
    }

    /// Get a specific memory
    async fn memory(&self, _ctx: &Context<'_>, id: String) -> Result<Option<Memory>> {
        let memory = self.storage.get_memory(&id).await?;
        Ok(memory.map(|m| Memory {
            id: m.id,
            learning: m.learning,
            linked_entities: m.linked_entities,
            description: m.description,
            created_at: m.created_at.to_rfc3339(),
            updated_at: m.updated_at.to_rfc3339(),
        }))
    }
}

/// GraphQL Mutation resolvers
pub struct MutationRoot {
    sql_engine: Arc<SqlEngine>,
    storage: Arc<dyn StorageBackend>,
}

impl MutationRoot {
    pub fn new(sql_engine: Arc<SqlEngine>, storage: Arc<dyn StorageBackend>) -> Self {
        Self {
            sql_engine,
            storage,
        }
    }
}

#[Object]
impl MutationRoot {
    /// Create a new model
    async fn create_model(&self, ctx: &Context<'_>, input: CreateModelInput) -> Result<ModelInfo> {
        require_admin_if_auth(ctx)?;
        let model = Model {
            name: input.name.clone(),
            datasource: input.datasource.clone(),
            description: input.description,
            measures: input.measures.into_iter().map(|m| m.into()).collect(),
            dimensions: input.dimensions.into_iter().map(|d| d.into()).collect(),
            time_dimensions: input
                .time_dimensions
                .into_iter()
                .map(|t| t.into())
                .collect(),
            joins: input
                .joins
                .unwrap_or_default()
                .into_iter()
                .map(|j| j.into())
                .collect(),
            sql: None,
            meta: std::collections::HashMap::new(),
        };

        let created = self.storage.create_model(model).await?;
        self.sql_engine.invalidate_result_cache();

        Ok(ModelInfo {
            name: created.name,
            datasource: created.datasource,
            description: created.description,
            measures: created
                .measures
                .iter()
                .map(|m| m.formula.expression.clone())
                .collect(),
            dimensions: created.dimensions.iter().map(|d| d.name.clone()).collect(),
            time_dimensions: created
                .time_dimensions
                .iter()
                .map(|t| t.dimension.clone())
                .collect(),
        })
    }

    /// Update a model
    async fn update_model(
        &self,
        ctx: &Context<'_>,
        name: String,
        input: UpdateModelInput,
    ) -> Result<ModelInfo> {
        require_admin_if_auth(ctx)?;
        let mut model = self
            .storage
            .get_model(&name, None)
            .await?
            .ok_or_else(|| Error::new(format!("Model not found: {}", name)))?;

        if let Some(desc) = input.description {
            model.description = Some(desc);
        }
        if let Some(measures) = input.measures {
            model.measures = measures.into_iter().map(|m| m.into()).collect();
        }
        if let Some(dimensions) = input.dimensions {
            model.dimensions = dimensions.into_iter().map(|d| d.into()).collect();
        }
        if let Some(time_dimensions) = input.time_dimensions {
            model.time_dimensions = time_dimensions.into_iter().map(|t| t.into()).collect();
        }
        if let Some(joins) = input.joins {
            model.joins = joins.into_iter().map(|j| j.into()).collect();
        }

        let updated = self.storage.update_model(&name, model).await?;
        self.sql_engine.invalidate_result_cache();

        Ok(ModelInfo {
            name: updated.name,
            datasource: updated.datasource,
            description: updated.description,
            measures: updated
                .measures
                .iter()
                .map(|m| m.formula.expression.clone())
                .collect(),
            dimensions: updated.dimensions.iter().map(|d| d.name.clone()).collect(),
            time_dimensions: updated
                .time_dimensions
                .iter()
                .map(|t| t.dimension.clone())
                .collect(),
        })
    }

    /// Delete a model
    async fn delete_model(
        &self,
        ctx: &Context<'_>,
        name: String,
        datasource: Option<String>,
    ) -> Result<bool> {
        require_admin_if_auth(ctx)?;
        let deleted = self
            .storage
            .delete_model(&name, datasource.as_deref())
            .await
            .map_err(|e| Error::new(e.to_string()))?;
        self.sql_engine.invalidate_result_cache();
        Ok(deleted)
    }

    /// Create a new datasource
    async fn create_datasource(
        &self,
        ctx: &Context<'_>,
        input: CreateDatasourceInput,
    ) -> Result<DatasourceInfo> {
        require_admin_if_auth(ctx)?;
        validate_connection_string_input(&input.connection_string).map_err(Error::new)?;
        let ds = DataSource {
            name: input.name.clone(),
            driver: input.driver,
            connection_string: input.connection_string,
            description: input.description,
            models: vec![],
            pool_size: input.pool_size.map(|p| p as u32),
            meta: std::collections::HashMap::new(),
        };

        let created = self.storage.create_datasource(ds).await?;

        Ok(DatasourceInfo {
            name: created.name,
            driver: created.driver,
            description: created.description,
            models: created.models,
        })
    }

    /// Update a datasource
    async fn update_datasource(
        &self,
        ctx: &Context<'_>,
        name: String,
        input: UpdateDatasourceInput,
    ) -> Result<DatasourceInfo> {
        require_admin_if_auth(ctx)?;
        let mut ds = self
            .storage
            .get_datasource(&name)
            .await?
            .ok_or_else(|| Error::new(format!("Datasource not found: {}", name)))?;

        if let Some(desc) = input.description {
            ds.description = Some(desc);
        }
        if let Some(conn_str) = input.connection_string {
            validate_connection_string_input(&conn_str).map_err(Error::new)?;
            ds.connection_string = conn_str;
        }
        if let Some(pool_size) = input.pool_size {
            ds.pool_size = Some(pool_size as u32);
        }

        let updated = self.storage.update_datasource(&name, ds).await?;

        Ok(DatasourceInfo {
            name: updated.name,
            driver: updated.driver,
            description: updated.description,
            models: updated.models,
        })
    }

    /// Save a memory
    async fn save_memory(&self, ctx: &Context<'_>, input: SaveMemoryInput) -> Result<Memory> {
        require_user_if_auth(ctx)?;
        let memory = graphnight_storage::Memory {
            id: input.id.unwrap_or_else(|| uuid::Uuid::new_v4().to_string()),
            learning: input.learning,
            linked_entities: input.linked_entities,
            description: input.description,
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
            meta: std::collections::HashMap::new(),
        };

        let saved = self.storage.save_memory(memory).await?;

        Ok(Memory {
            id: saved.id,
            learning: saved.learning,
            linked_entities: saved.linked_entities,
            description: saved.description,
            created_at: saved.created_at.to_rfc3339(),
            updated_at: saved.updated_at.to_rfc3339(),
        })
    }

    /// Delete a memory
    async fn forget_memory(&self, ctx: &Context<'_>, id: String) -> Result<ForgetMemoryResponse> {
        require_user_if_auth(ctx)?;
        let success = self
            .storage
            .delete_memory(&id)
            .await
            .map_err(|e| Error::new(e.to_string()))?;
        Ok(ForgetMemoryResponse { success, id })
    }

    /// Ingest models from a datasource via warehouse introspection.
    async fn ingest_models(
        &self,
        ctx: &Context<'_>,
        datasource: String,
    ) -> Result<IngestionReport> {
        require_admin_if_auth(ctx)?;

        let ds = self
            .storage
            .get_datasource(&datasource)
            .await?
            .ok_or_else(|| Error::new(format!("Datasource not found: {}", datasource)))?;

        let executor = self.sql_engine.executor().clone();
        let introspector = SchemaIntrospector::new(executor);

        let tables = introspector
            .introspect(&ds, IntrospectionConfig::default())
            .await
            .map_err(|e| Error::new(format!("Introspection failed: {}", e)))?;

        let mut models_created = 0;
        let mut models_updated = 0;
        let mut errors = Vec::new();

        for table in &tables {
            let model = infer_model_from_table(table, &datasource, &tables);
            let model_name = model.name.clone();
            match self.storage.get_model(&model_name, None).await {
                Ok(Some(_)) => {
                    if let Err(e) = self.storage.update_model(&model_name, model).await {
                        errors.push(format!("Failed to update model {}: {}", model_name, e));
                    } else {
                        models_updated += 1;
                    }
                }
                Ok(None) => {
                    if let Err(e) = self.storage.create_model(model).await {
                        errors.push(format!("Failed to create model: {}", e));
                    } else {
                        models_created += 1;
                    }
                }
                Err(e) => {
                    errors.push(format!("Storage error: {}", e));
                }
            }
        }

        self.sql_engine.invalidate_result_cache();

        Ok(IngestionReport {
            models_created,
            models_updated,
            errors,
        })
    }
}

/// GraphQL Subscription resolvers
pub struct SubscriptionRoot {
    sql_engine: Arc<SqlEngine>,
    storage: Arc<dyn StorageBackend>,
}

impl SubscriptionRoot {
    pub fn new(sql_engine: Arc<SqlEngine>, storage: Arc<dyn StorageBackend>) -> Self {
        Self {
            sql_engine,
            storage,
        }
    }
}

#[Subscription]
impl SubscriptionRoot {
    /// Poll a semantic query on an interval (WebSocket). GraphQL still buffers
    /// each tick's payload; the SQL layer streams rows into that buffer.
    async fn live_query(
        &self,
        _ctx: &Context<'_>,
        input: QueryInput,
        interval_ms: i32,
    ) -> impl futures::Stream<Item = QueryResponse> {
        let sql_engine = self.sql_engine.clone();
        let storage = self.storage.clone();
        let interval = std::time::Duration::from_millis(interval_ms.max(250) as u64);

        async_stream::stream! {
            let mut ticker = tokio::time::interval(interval);
            loop {
                ticker.tick().await;
                let start = std::time::Instant::now();
                let query: CoreQuery = input.clone().into();
                let Some(model_name) = query
                    .name
                    .clone()
                    .or_else(|| query.source_model.as_ref().map(|s| s.model.clone()))
                else {
                    continue;
                };

                let Ok(Some(model)) = storage.get_model(&model_name, None).await else {
                    continue;
                };
                let Ok(Some(datasource)) = storage.get_datasource(&model.datasource).await else {
                    continue;
                };
                let Ok(sql) = sql_engine.generate_sql(&query) else {
                    continue;
                };
                match sql_engine.execute_sqlx(&datasource, &sql).await {
                    Ok(results) => {
                        let columns = if !results.is_empty() {
                            results[0].keys().cloned().collect()
                        } else {
                            vec![]
                        };
                        let data: Vec<JsonValue> = results
                            .into_iter()
                            .map(|m| serde_json::to_value(m).unwrap())
                            .collect();
                        yield QueryResponse {
                            data,
                            columns,
                            sql: Some(sql),
                            attributes: None,
                            population: None,
                            population_inferred: false,
                            execution_time_ms: start.elapsed().as_millis() as f64,
                        };
                    }
                    Err(_) => continue,
                }
            }
        }
    }

    /// Subscribe to model changes (best-effort polling of model list fingerprints).
    async fn model_changes(
        &self,
        _ctx: &Context<'_>,
        datasource: String,
    ) -> impl futures::Stream<Item = ModelChangeEvent> {
        let storage = self.storage.clone();
        async_stream::stream! {
            let mut last = String::new();
            let mut ticker = tokio::time::interval(std::time::Duration::from_secs(2));
            loop {
                ticker.tick().await;
                let Ok(models) = storage.list_models(Some(&datasource)).await else {
                    continue;
                };
                let fingerprint = models
                    .iter()
                    .map(|m| format!("{}:{}", m.name, m.description.clone().unwrap_or_default()))
                    .collect::<Vec<_>>()
                    .join("|");
                if fingerprint != last {
                    if !last.is_empty() {
                        for m in &models {
                            yield ModelChangeEvent {
                                event_type: "updated".to_string(),
                                model_name: m.name.clone(),
                                datasource: m.datasource.clone(),
                                timestamp: chrono::Utc::now().to_rfc3339(),
                            };
                        }
                    }
                    last = fingerprint;
                }
            }
        }
    }
}
