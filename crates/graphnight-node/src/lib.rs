use napi::bindgen_prelude::*;
use napi_derive::napi;
use std::collections::HashMap;
use std::sync::Arc;

use graphnight_core::models::{
    AggregationType, DataSource, Dimension, Filter, FilterOperator, Formula, Measure, Model,
    OrderBy, Query as CoreQuery, SourceSpec, TimeDimension, TimeGranularity,
};
use graphnight_runtime::governance::{CallContext, ExecuteOptions, QueryService};
use graphnight_sql::SqlEngine;
use graphnight_storage::{Memory, MemoryFilter, StorageBackend, YamlStorage};

/// The call context for an embedded (in-process) client.
///
/// There is no server and no credential channel, so the host process is the
/// identity: it is trusted and unrestricted. What still applies is the
/// structural governance the service always enforces — model and datasource
/// resolution, the row cap, column masks and the audit trail.
fn embedded_context() -> CallContext {
    CallContext {
        user_id: Some("embedded".to_string()),
        tenant_id: None,
        is_admin: true,
        auth_required: false,
        policy: None,
        principal_label: Some("node".to_string()),
    }
}

/// Helper to convert errors to napi::Error
fn to_napi_error<E: std::fmt::Display>(e: E) -> Error {
    Error::new(Status::GenericFailure, e.to_string())
}

/// NAPI-compatible wrapper for GraphNight errors
#[napi(object)]
pub struct GraphNightError {
    pub message: String,
    pub code: String,
}

impl From<anyhow::Error> for GraphNightError {
    fn from(e: anyhow::Error) -> Self {
        Self {
            message: e.to_string(),
            code: "INTERNAL_ERROR".to_string(),
        }
    }
}

impl From<graphnight_core::errors::StorageError> for GraphNightError {
    fn from(e: graphnight_core::errors::StorageError) -> Self {
        Self {
            message: e.to_string(),
            code: "STORAGE_ERROR".to_string(),
        }
    }
}

/// Query result structure - uses JSON strings for flexible data
#[napi(object)]
pub struct QueryResult {
    pub data: String, // JSON string of array of objects
    pub columns: Vec<String>,
    pub sql: String,
    pub execution_time_ms: f64,
    pub row_count: u32,
}

/// Model summary for listing
#[napi(object)]
pub struct ModelSummary {
    pub name: String,
    pub datasource: String,
    pub description: String,
    pub measures: Vec<String>,
    pub dimensions: Vec<String>,
    pub time_dimensions: Vec<String>,
}

/// DataSource summary for listing
#[napi(object)]
pub struct DataSourceSummary {
    pub name: String,
    pub driver: String,
    pub description: String,
    pub models: Vec<String>,
    pub pool_size: Option<u32>,
}

/// Search result
#[napi(object)]
pub struct SearchResult {
    pub model_name: String,
    pub datasource: String,
    pub score: f64,
    pub matched_fields: Vec<String>,
    pub snippet: String,
}

/// Memory item
#[napi(object)]
pub struct MemoryItem {
    pub id: String,
    pub learning: String,
    pub linked_entities: Vec<String>,
    pub description: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

/// Input structures for queries
#[napi(object)]
pub struct FormulaInput {
    pub expression: String,
    pub label: Option<String>,
    pub format: Option<String>,
}

#[napi(object)]
pub struct MeasureInput {
    pub formula: FormulaInput,
    pub aggregation: String,
}

#[napi(object)]
pub struct DimensionInput {
    pub name: String,
    pub label: Option<String>,
}

#[napi(object)]
pub struct TimeDimensionInput {
    pub dimension: String,
    pub granularity: String,
    pub label: Option<String>,
}

#[napi(object)]
pub struct FilterInput {
    pub field: String,
    pub operator: String,
    pub value: Option<String>,       // JSON string
    pub values: Option<Vec<String>>, // JSON strings
    pub or_condition: Option<bool>,
}

#[napi(object)]
pub struct OrderByInput {
    pub field: String,
    pub descending: Option<bool>,
}

#[napi(object)]
pub struct QueryInput {
    pub name: Option<String>,
    pub source_model: Option<SourceSpecInput>,
    pub measures: Vec<MeasureInput>,
    pub dimensions: Vec<DimensionInput>,
    pub time_dimensions: Vec<TimeDimensionInput>,
    pub filters: Vec<FilterInput>,
    pub order: Vec<OrderByInput>,
    pub limit: Option<u32>,
    pub offset: Option<u32>,
    pub whole_periods_only: Option<bool>,
    pub distinct_dimension_values: Option<bool>,
}

#[napi(object)]
pub struct SourceSpecInput {
    pub model: String,
    pub datasource: Option<String>,
    pub alias: Option<String>,
}

/// Model input for creation
#[napi(object)]
pub struct ModelInput {
    pub name: String,
    pub datasource: String,
    pub description: Option<String>,
    pub measures: Vec<MeasureInput>,
    pub dimensions: Vec<DimensionInput>,
    pub time_dimensions: Vec<TimeDimensionInput>,
    pub joins: Vec<JoinInput>,
}

#[napi(object)]
pub struct JoinInput {
    pub name: String,
    pub model: String,
    pub join_type: String,
    pub on: Vec<JoinOnInput>,
    pub alias: Option<String>,
}

#[napi(object)]
pub struct JoinOnInput {
    pub left: String,
    pub right: String,
}

/// DataSource input for creation
#[napi(object)]
pub struct DataSourceInput {
    pub name: String,
    pub driver: String,
    pub connection_string: String,
    pub description: Option<String>,
    pub models: Vec<String>,
    pub pool_size: Option<u32>,
}

/// Main GraphNight client
#[napi]
pub struct GraphNightClient {
    /// All query execution goes through the service, so an embedded client gets
    /// the same row cap, column masks and audit trail as the server.
    service: Arc<QueryService>,
    storage: Arc<dyn StorageBackend + Send + Sync>,
    runtime: tokio::runtime::Runtime,
}

#[napi]
impl GraphNightClient {
    /// Create a new GraphNight client with local YAML storage
    ///
    /// @param storage_path - Path to YAML storage directory (default: "./graphnight_data")
    /// @returns GraphNightClient instance
    #[napi(constructor)]
    pub fn new(storage_path: Option<String>) -> Result<Self> {
        let storage_path = storage_path.unwrap_or_else(|| "./graphnight_data".to_string());

        let storage = Arc::new(YamlStorage::new(&storage_path).map_err(to_napi_error)?);

        let runtime = tokio::runtime::Runtime::new().map_err(to_napi_error)?;

        runtime.block_on(storage.load()).map_err(to_napi_error)?;

        let models = runtime
            .block_on(storage.list_models(None))
            .map_err(to_napi_error)?;

        let datasources = runtime
            .block_on(storage.list_datasources())
            .map_err(to_napi_error)?;

        // Default dialect only covers models pointing at an unregistered
        // datasource; with_datasources resolves each model to its own driver.
        let default_driver = datasources
            .first()
            .map(|d| d.driver.clone())
            .unwrap_or_else(|| "postgres".to_string());
        let dialect = graphnight_sql::dialects::get_dialect(&default_driver);
        let conn_manager = Arc::new(graphnight_sql::executor::ConnectionManager::new());
        let executor = Arc::new(graphnight_sql::executor::QueryExecutor::new(conn_manager));
        let sql_engine = SqlEngine::new(dialect, executor)
            .map_err(to_napi_error)?
            .with_datasources(datasources)
            .with_models(models);

        Ok(Self {
            service: Arc::new(QueryService::new(Arc::new(sql_engine), storage.clone())),
            storage,
            runtime,
        })
    }

    /// Execute a query and return data + SQL
    #[napi]
    pub fn query(&self, query: QueryInput) -> Result<QueryResult> {
        let core_query = self.convert_query(query)?;
        let start = std::time::Instant::now();
        let outcome = self
            .runtime
            .block_on(self.service.execute(
                &embedded_context(),
                core_query,
                ExecuteOptions::executing(),
            ))
            .map_err(to_napi_error)?;

        let data: Vec<HashMap<String, serde_json::Value>> = outcome
            .data
            .iter()
            .map(|v| match v {
                serde_json::Value::Object(m) => {
                    m.iter().map(|(k, v)| (k.clone(), v.clone())).collect()
                }
                other => {
                    let mut m = HashMap::new();
                    m.insert("value".to_string(), other.clone());
                    m
                }
            })
            .collect();

        let data_json = serde_json::to_string(&data).map_err(|e| {
            Error::new(
                Status::GenericFailure,
                format!("Failed to serialize data: {}", e),
            )
        })?;

        Ok(QueryResult {
            data: data_json,
            columns: outcome.columns.clone(),
            sql: outcome.sql.unwrap_or_default(),
            execution_time_ms: start.elapsed().as_millis() as f64,
            row_count: data.len() as u32,
        })
    }

    /// Generate SQL for a query without executing it
    #[napi]
    pub fn generate_sql(&self, query: QueryInput) -> Result<String> {
        let core_query = self.convert_query(query)?;
        let outcome = self
            .runtime
            .block_on(self.service.execute(
                &embedded_context(),
                core_query,
                ExecuteOptions::dry_run(),
            ))
            .map_err(to_napi_error)?;
        Ok(outcome.sql.unwrap_or_default())
    }

    /// Dry-run a query: return generated SQL without hitting a DB
    #[napi]
    pub fn dry_run(&self, query: QueryInput) -> Result<QueryResult> {
        let sql = self.generate_sql(query)?;
        Ok(QueryResult {
            data: "[]".to_string(),
            columns: vec![],
            sql,
            execution_time_ms: 0.0,
            row_count: 0,
        })
    }

    /// List models, optionally filtered by datasource
    #[napi]
    pub fn list_models(&self, datasource: Option<String>) -> Result<Vec<ModelSummary>> {
        let storage = self.storage.clone();
        let models = self
            .runtime
            .block_on(async move { storage.list_models(datasource.as_deref()).await })
            .map_err(to_napi_error)?;

        Ok(models
            .iter()
            .map(|m| ModelSummary {
                name: m.name.clone(),
                datasource: m.datasource.clone(),
                description: m.description.clone().unwrap_or_default(),
                measures: m
                    .measures
                    .iter()
                    .map(|meas| meas.formula.expression.clone())
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

    /// Get a single model by name
    #[napi]
    pub fn get_model(
        &self,
        name: String,
        datasource: Option<String>,
    ) -> Result<Option<ModelSummary>> {
        let storage = self.storage.clone();
        let model = self
            .runtime
            .block_on(async move { storage.get_model(&name, datasource.as_deref()).await })
            .map_err(to_napi_error)?;

        Ok(model.map(|m| ModelSummary {
            name: m.name.clone(),
            datasource: m.datasource.clone(),
            description: m.description.clone().unwrap_or_default(),
            measures: m
                .measures
                .iter()
                .map(|meas| meas.formula.expression.clone())
                .collect(),
            dimensions: m.dimensions.iter().map(|d| d.name.clone()).collect(),
            time_dimensions: m
                .time_dimensions
                .iter()
                .map(|t| t.dimension.clone())
                .collect(),
        }))
    }

    /// Create a new model
    #[napi]
    pub fn create_model(&self, model: ModelInput) -> Result<ModelSummary> {
        let core_model = self.convert_model(model)?;

        let storage = self.storage.clone();
        let created = self
            .runtime
            .block_on(async move { storage.create_model(core_model).await })
            .map_err(to_napi_error)?;

        // Register in the SQL engine too, else the new model would be listed
        // and searchable but fail to generate SQL.
        self.service
            .register_model(&embedded_context(), created.clone())
            .map_err(to_napi_error)?;

        Ok(ModelSummary {
            name: created.name,
            datasource: created.datasource,
            description: created.description.unwrap_or_default(),
            measures: created
                .measures
                .iter()
                .map(|meas| meas.formula.expression.clone())
                .collect(),
            dimensions: created.dimensions.iter().map(|d| d.name.clone()).collect(),
            time_dimensions: created
                .time_dimensions
                .iter()
                .map(|t| t.dimension.clone())
                .collect(),
        })
    }

    /// List datasources
    #[napi]
    pub fn list_datasources(&self) -> Result<Vec<DataSourceSummary>> {
        let storage = self.storage.clone();
        let datasources = self
            .runtime
            .block_on(async move { storage.list_datasources().await })
            .map_err(to_napi_error)?;

        Ok(datasources
            .into_iter()
            .map(|d| DataSourceSummary {
                name: d.name,
                driver: d.driver,
                description: d.description.unwrap_or_default(),
                models: d.models,
                pool_size: d.pool_size,
            })
            .collect())
    }

    /// Create a new datasource
    #[napi]
    pub fn create_datasource(&self, datasource: DataSourceInput) -> Result<DataSourceSummary> {
        let core_ds = DataSource {
            name: datasource.name,
            driver: datasource.driver,
            connection_string: datasource.connection_string,
            description: datasource.description,
            models: datasource.models,
            pool_size: datasource.pool_size,
            meta: HashMap::new(),
        };

        let storage = self.storage.clone();
        let created = self
            .runtime
            .block_on(async move { storage.create_datasource(core_ds).await })
            .map_err(to_napi_error)?;

        Ok(DataSourceSummary {
            name: created.name,
            driver: created.driver,
            description: created.description.unwrap_or_default(),
            models: created.models,
            pool_size: created.pool_size,
        })
    }

    /// Search models by query string
    #[napi]
    pub fn search(&self, query: String, limit: Option<u32>) -> Result<Vec<SearchResult>> {
        let storage = self.storage.clone();
        let results = self
            .runtime
            .block_on(async move { storage.search(&query, limit.unwrap_or(10) as usize).await })
            .map_err(to_napi_error)?;

        Ok(results
            .into_iter()
            .map(|r| SearchResult {
                model_name: r.model_name,
                datasource: r.datasource,
                score: r.score as f64,
                matched_fields: r.matched_fields,
                snippet: r.snippet,
            })
            .collect())
    }

    /// Save a memory item
    #[napi]
    pub fn save_memory(
        &self,
        learning: String,
        linked_entities: Vec<String>,
        id: Option<String>,
        description: Option<String>,
    ) -> Result<MemoryItem> {
        let storage = self.storage.clone();
        let memory = Memory {
            id: id.unwrap_or_else(|| uuid::Uuid::new_v4().to_string()),
            learning,
            linked_entities,
            description,
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
            meta: HashMap::new(),
        };

        let saved = self
            .runtime
            .block_on(async move { storage.save_memory(memory).await })
            .map_err(to_napi_error)?;

        Ok(MemoryItem {
            id: saved.id,
            learning: saved.learning,
            linked_entities: saved.linked_entities,
            description: saved.description,
            created_at: saved.created_at.to_rfc3339(),
            updated_at: saved.updated_at.to_rfc3339(),
        })
    }

    /// List memories with optional filters
    #[napi]
    pub fn list_memories(
        &self,
        query: Option<String>,
        entity: Option<String>,
        limit: Option<u32>,
        offset: Option<u32>,
    ) -> Result<Vec<MemoryItem>> {
        let storage = self.storage.clone();
        let filter = MemoryFilter {
            query,
            entity,
            limit: limit.map(|l| l as usize),
            offset: offset.map(|o| o as usize),
        };

        let memories = self
            .runtime
            .block_on(async move { storage.list_memories(filter).await })
            .map_err(to_napi_error)?;

        Ok(memories
            .into_iter()
            .map(|m| MemoryItem {
                id: m.id,
                learning: m.learning,
                linked_entities: m.linked_entities,
                description: m.description,
                created_at: m.created_at.to_rfc3339(),
                updated_at: m.updated_at.to_rfc3339(),
            })
            .collect())
    }

    /// Delete a memory by ID
    #[napi]
    pub fn delete_memory(&self, id: String) -> Result<bool> {
        let storage = self.storage.clone();
        let success = self
            .runtime
            .block_on(async move { storage.delete_memory(&id).await })
            .map_err(to_napi_error)?;
        Ok(success)
    }
}

impl GraphNightClient {
    fn convert_query(&self, input: QueryInput) -> Result<CoreQuery> {
        let mut query = CoreQuery::new();

        query.name = input.name;

        if let Some(source_model) = input.source_model {
            query.source_model = Some(SourceSpec {
                model: source_model.model,
                datasource: source_model.datasource,
                alias: source_model.alias,
            });
        }

        query.measures = input
            .measures
            .into_iter()
            .map(|m| Measure {
                formula: Formula {
                    expression: m.formula.expression,
                    label: m.formula.label,
                    format: m.formula.format,
                },
                aggregation: parse_aggregation(&m.aggregation),
            })
            .collect();

        query.dimensions = input
            .dimensions
            .into_iter()
            .map(|d| Dimension {
                name: d.name,
                label: d.label,
            })
            .collect();

        query.time_dimensions = input
            .time_dimensions
            .into_iter()
            .map(|t| TimeDimension {
                dimension: t.dimension,
                granularity: parse_granularity(&t.granularity),
                label: t.label,
            })
            .collect();

        query.filters = input
            .filters
            .into_iter()
            .map(|f| {
                let value = f
                    .value
                    .and_then(|v| serde_json::from_str(&v).ok())
                    .unwrap_or(serde_json::Value::Null);

                // A bad operator is an error here, not a silent `Eq`: a typo
                // would otherwise become `field = 'text'`, which matches nothing
                // and reports no failure.
                let operator = parse_filter_operator(&f.operator)?;
                Ok(Filter {
                    field: f.field,
                    operator,
                    value,
                    or_condition: f.or_condition.unwrap_or(false),
                })
            })
            .collect::<Result<Vec<_>>>()?;

        query.order = input
            .order
            .into_iter()
            .map(|o| OrderBy {
                field: o.field,
                descending: o.descending.unwrap_or(false),
            })
            .collect();

        query.limit = input.limit.map(|l| l as usize);
        query.offset = input.offset.map(|o| o as usize);
        query.whole_periods_only = input.whole_periods_only;
        query.distinct_dimension_values = input.distinct_dimension_values;

        Ok(query)
    }

    fn convert_model(&self, input: ModelInput) -> Result<Model> {
        let measures = input
            .measures
            .into_iter()
            .map(|m| Measure {
                formula: Formula {
                    expression: m.formula.expression,
                    label: m.formula.label,
                    format: m.formula.format,
                },
                aggregation: parse_aggregation(&m.aggregation),
            })
            .collect();

        let dimensions = input
            .dimensions
            .into_iter()
            .map(|d| Dimension {
                name: d.name,
                label: d.label,
            })
            .collect();

        let time_dimensions = input
            .time_dimensions
            .into_iter()
            .map(|t| TimeDimension {
                dimension: t.dimension,
                granularity: parse_granularity(&t.granularity),
                label: t.label,
            })
            .collect();

        let joins = input
            .joins
            .into_iter()
            .map(|j| graphnight_core::models::Join {
                name: j.name,
                model: j.model,
                join_type: parse_join_type(&j.join_type),
                on: j.on.into_iter().map(|o| (o.left, o.right)).collect(),
                alias: j.alias,
            })
            .collect();

        Ok(Model {
            name: input.name,
            datasource: input.datasource,
            description: input.description,
            measures,
            dimensions,
            time_dimensions,
            joins,
            sql: None,
            meta: HashMap::new(),
        })
    }
}

fn parse_aggregation(s: &str) -> AggregationType {
    match s.to_lowercase().as_str() {
        "sum" => AggregationType::Sum,
        "avg" => AggregationType::Avg,
        "count" => AggregationType::Count,
        "min" => AggregationType::Min,
        "max" => AggregationType::Max,
        "count_distinct" => AggregationType::CountDistinct,
        _ => AggregationType::Sum,
    }
}

fn parse_granularity(s: &str) -> TimeGranularity {
    match s.to_lowercase().as_str() {
        "second" => TimeGranularity::Second,
        "minute" => TimeGranularity::Minute,
        "hour" => TimeGranularity::Hour,
        "day" => TimeGranularity::Day,
        "week" => TimeGranularity::Week,
        "month" => TimeGranularity::Month,
        "quarter" => TimeGranularity::Quarter,
        "year" => TimeGranularity::Year,
        _ => TimeGranularity::Day,
    }
}

fn parse_filter_operator(s: &str) -> Result<FilterOperator> {
    // Strict: an unrecognised operator is an error rather than a silent `Eq`,
    // so a typo reports itself instead of returning no rows. The accepted set
    // lives in `FilterOperator::parse_strict`, shared with every other surface.
    FilterOperator::parse_strict(s).map_err(|e| Error::new(Status::GenericFailure, e))
}

fn parse_join_type(s: &str) -> graphnight_core::models::JoinType {
    match s.to_lowercase().as_str() {
        "inner" => graphnight_core::models::JoinType::Inner,
        "left" => graphnight_core::models::JoinType::Left,
        "right" => graphnight_core::models::JoinType::Right,
        "full" => graphnight_core::models::JoinType::Full,
        _ => graphnight_core::models::JoinType::Inner,
    }
}
