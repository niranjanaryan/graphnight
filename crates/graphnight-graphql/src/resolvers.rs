use crate::auth::{
    enforce_policy, reject_if_auth_required, require_admin_if_auth, require_user_if_auth,
};
use crate::schema::JsonValue;
use crate::schema::*;
use async_graphql::*;
use graphnight_core::models::{DataSource, Model, Query as CoreQuery};
use graphnight_sql::SqlEngine;
use graphnight_storage::StorageBackend;
use std::sync::Arc;
use tracing::info;

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
        reject_if_auth_required(ctx)?;
        let start = std::time::Instant::now();
        let mut query: CoreQuery = input.into();

        let model_name = query
            .name
            .as_ref()
            .or_else(|| query.source_model.as_ref().map(|s| &s.model))
            .ok_or_else(|| Error::new("Query must have a name or source_model"))?
            .clone();

        let model = self
            .storage
            .get_model(&model_name, None)
            .await?
            .ok_or_else(|| Error::new(format!("Model not found: {}", model_name)))?;

        enforce_policy(ctx, &mut query, &model_name, &model.datasource)?;

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
        let results = self.sql_engine.execute_sqlx(&datasource, &sql).await?;

        let columns = if !results.is_empty() {
            results[0].keys().cloned().collect()
        } else {
            vec![]
        };
        let row_count = results.len();

        let data: Vec<JsonValue> = results
            .into_iter()
            .map(|m| serde_json::to_value(m).unwrap())
            .collect();

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

    /// Execute multiple queries as a DAG.
    ///
    /// Alpha: true DAG / `stage_ref` execution is not implemented. Fails loudly
    /// so clients do not treat a sequential stub as supported.
    async fn multi_stage_query(
        &self,
        _ctx: &Context<'_>,
        _inputs: Vec<QueryInput>,
        _dry_run: Option<bool>,
    ) -> Result<MultiStageResponse> {
        Err(Error::new(
            "multiStageQuery is not supported in this alpha build (no DAG / stage_ref execution yet)",
        ))
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
    storage: Arc<dyn StorageBackend>,
}

impl MutationRoot {
    pub fn new(_sql_engine: Arc<SqlEngine>, storage: Arc<dyn StorageBackend>) -> Self {
        Self { storage }
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
        self.storage
            .delete_model(&name, datasource.as_deref())
            .await
            .map_err(|e| Error::new(e.to_string()))
    }

    /// Create a new datasource
    async fn create_datasource(
        &self,
        ctx: &Context<'_>,
        input: CreateDatasourceInput,
    ) -> Result<DatasourceInfo> {
        require_admin_if_auth(ctx)?;
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
    ///
    /// Alpha: unsupported. Define models via YAML or `createModel`.
    async fn ingest_models(
        &self,
        ctx: &Context<'_>,
        _datasource: String,
    ) -> Result<IngestionReport> {
        require_admin_if_auth(ctx)?;
        Err(Error::new(
            "ingestModels is not supported in this alpha build; define models via YAML or createModel",
        ))
    }
}

/// GraphQL Subscription resolvers
pub struct SubscriptionRoot;

#[Subscription]
impl SubscriptionRoot {
    /// Subscribe to live query results
    async fn live_query(
        &self,
        _ctx: &Context<'_>,
        _input: QueryInput,
        _interval_ms: i32,
    ) -> impl futures::Stream<Item = QueryResponse> {
        // Would implement polling query execution
        futures::stream::empty()
    }

    /// Subscribe to model changes
    async fn model_changes(
        &self,
        _ctx: &Context<'_>,
        _datasource: String,
    ) -> impl futures::Stream<Item = ModelChangeEvent> {
        // Would listen to model change events
        futures::stream::empty()
    }
}
