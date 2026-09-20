use rustler::{Encoder, Env, Error, NifResult, Term, ResourceArc, Decoder};
use graphnight_core::models::{DataSource, Model, Query as CoreQuery, Measure, Dimension, TimeDimension, AggregationType, TimeGranularity, Filter, FilterOperator, OrderBy, SourceSpec, Formula, Join, JoinType};
use graphnight_sql::{dialects::get_dialect, executor::{ConnectionManager, QueryExecutor}, SqlEngine};
use graphnight_storage::{StorageBackend, YamlStorage};
use std::sync::Arc;
use std::collections::HashMap;
use std::path::PathBuf;

/// Wrapper for serde_json::Value that implements Encoder/Decoder via JSON string
#[derive(Debug, Clone, Default, PartialEq)]
pub struct JsonValue(pub serde_json::Value);

impl<'a> Decoder<'a> for JsonValue {
    fn decode(term: Term<'a>) -> NifResult<Self> {
        let binary: rustler::Binary<'a> = term.decode()?;
        let s = std::str::from_utf8(binary.as_slice())
            .map_err(|_| Error::Term(Box::new("Invalid UTF-8")))?;
        let value: serde_json::Value = serde_json::from_str(s)
            .map_err(|e| Error::Term(Box::new(format!("JSON parse error: {}", e))))?;
        Ok(JsonValue(value))
    }
}

impl Encoder for JsonValue {
    fn encode<'a>(&self, env: Env<'a>) -> Term<'a> {
        let json_str = serde_json::to_string(&self.0).unwrap_or_else(|_| "null".to_string());
        json_str.encode(env)
    }
}

impl From<serde_json::Value> for JsonValue {
    fn from(v: serde_json::Value) -> Self {
        JsonValue(v)
    }
}

impl From<JsonValue> for serde_json::Value {
    fn from(v: JsonValue) -> Self {
        v.0
    }
}

mod atoms {
    rustler::atoms! {
        ok,
        error,
        not_found,
        model,
        datasource,
        measure,
        dimension,
        time_dimension,
        filter,
        order_by,
        query,
        sql,
        data,
        columns,
        execution_time_ms,
        name,
        label,
        format,
        aggregation,
        granularity,
        field,
        operator,
        value,
        or_condition,
        descending,
        limit,
        offset,
        whole_periods_only,
        distinct_dimension_values,
        stage_ref,
        sum,
        avg,
        count,
        min,
        max,
        count_distinct,
        second,
        minute,
        hour,
        day,
        week,
        month,
        quarter,
        year,
        eq,
        neq,
        gt,
        gte,
        lt,
        lte,
        like,
        ilike,
        in_,
        not_in,
        is_null,
        is_not_null,
        between,
        not_between,
        inner,
        left,
        right,
        full,
    }
}

#[derive(Debug, rustler::NifStruct)]
#[module = "GraphNight.Model"]
pub struct ElixirModel {
    pub name: String,
    pub datasource: String,
    pub description: Option<String>,
    pub measures: Vec<ElixirMeasure>,
    pub dimensions: Vec<ElixirDimension>,
    pub time_dimensions: Vec<ElixirTimeDimension>,
    pub joins: Vec<ElixirJoin>,
}

#[derive(Debug, rustler::NifStruct)]
#[module = "GraphNight.Measure"]
pub struct ElixirMeasure {
    pub formula: String,
    pub label: Option<String>,
    pub format: Option<String>,
    pub aggregation: String,
}

#[derive(Debug, rustler::NifStruct)]
#[module = "GraphNight.Dimension"]
pub struct ElixirDimension {
    pub name: String,
    pub label: Option<String>,
}

#[derive(Debug, rustler::NifStruct)]
#[module = "GraphNight.TimeDimension"]
pub struct ElixirTimeDimension {
    pub dimension: String,
    pub granularity: String,
    pub label: Option<String>,
}

#[derive(Debug, rustler::NifStruct)]
#[module = "GraphNight.Join"]
pub struct ElixirJoin {
    pub name: String,
    pub model: String,
    pub join_type: String,
    pub on: Vec<(String, String)>,
    pub alias: Option<String>,
}

#[derive(Debug, rustler::NifStruct)]
#[module = "GraphNight.DataSource"]
pub struct ElixirDataSource {
    pub name: String,
    pub driver: String,
    pub connection_string: String,
    pub description: Option<String>,
    pub models: Vec<String>,
    pub pool_size: Option<u32>,
}

#[derive(Debug, rustler::NifStruct)]
#[module = "GraphNight.Query"]
pub struct ElixirQuery {
    pub name: Option<String>,
    pub source_model: Option<ElixirSourceSpec>,
    pub measures: Vec<ElixirMeasure>,
    pub dimensions: Vec<ElixirDimension>,
    pub time_dimensions: Vec<ElixirTimeDimension>,
    pub filters: Vec<ElixirFilter>,
    pub order: Vec<ElixirOrderBy>,
    pub limit: Option<usize>,
    pub offset: Option<usize>,
    pub whole_periods_only: Option<bool>,
    pub distinct_dimension_values: Option<bool>,
    pub stage_ref: Option<String>,
}

#[derive(Debug, rustler::NifStruct)]
#[module = "GraphNight.SourceSpec"]
pub struct ElixirSourceSpec {
    pub model: String,
    pub datasource: Option<String>,
    pub alias: Option<String>,
}

#[derive(Debug, rustler::NifStruct)]
#[module = "GraphNight.Filter"]
pub struct ElixirFilter {
    pub field: String,
    pub operator: String,
    pub value: JsonValue,
    pub or_condition: bool,
}

#[derive(Debug, rustler::NifStruct)]
#[module = "GraphNight.OrderBy"]
pub struct ElixirOrderBy {
    pub field: String,
    pub descending: bool,
}

#[derive(Debug, rustler::NifStruct)]
#[module = "GraphNight.QueryResult"]
pub struct ElixirQueryResult {
    pub data: Vec<HashMap<String, JsonValue>>,
    pub columns: Vec<String>,
    pub sql: Option<String>,
    pub execution_time_ms: f64,
}

struct EngineResource {
    sql_engine: Arc<SqlEngine>,
    storage: Arc<dyn StorageBackend>,
    runtime: tokio::runtime::Runtime,
}

fn parse_aggregation(s: &str) -> AggregationType {
    match s.to_uppercase().as_str() {
        "SUM" => AggregationType::Sum,
        "AVG" => AggregationType::Avg,
        "COUNT" => AggregationType::Count,
        "MIN" => AggregationType::Min,
        "MAX" => AggregationType::Max,
        "COUNT_DISTINCT" => AggregationType::CountDistinct,
        _ => AggregationType::Custom(s.to_string()),
    }
}

fn parse_granularity(s: &str) -> TimeGranularity {
    match s.to_uppercase().as_str() {
        "SECOND" => TimeGranularity::Second,
        "MINUTE" => TimeGranularity::Minute,
        "HOUR" => TimeGranularity::Hour,
        "DAY" => TimeGranularity::Day,
        "WEEK" => TimeGranularity::Week,
        "MONTH" => TimeGranularity::Month,
        "QUARTER" => TimeGranularity::Quarter,
        "YEAR" => TimeGranularity::Year,
        _ => TimeGranularity::Day,
    }
}

fn parse_operator(s: &str) -> FilterOperator {
    match s.to_uppercase().as_str() {
        "EQ" => FilterOperator::Eq,
        "NEQ" => FilterOperator::Neq,
        "GT" => FilterOperator::Gt,
        "GTE" => FilterOperator::Gte,
        "LT" => FilterOperator::Lt,
        "LTE" => FilterOperator::Lte,
        "LIKE" => FilterOperator::Like,
        "ILIKE" => FilterOperator::ILike,
        "IN" => FilterOperator::In,
        "NOT_IN" => FilterOperator::NotIn,
        "IS_NULL" => FilterOperator::IsNull,
        "IS_NOT_NULL" => FilterOperator::IsNotNull,
        "BETWEEN" => FilterOperator::Between,
        "NOT_BETWEEN" => FilterOperator::NotBetween,
        _ => FilterOperator::Eq,
    }
}

fn parse_join_type(s: &str) -> JoinType {
    match s.to_uppercase().as_str() {
        "INNER" => JoinType::Inner,
        "LEFT" => JoinType::Left,
        "RIGHT" => JoinType::Right,
        "FULL" => JoinType::Full,
        _ => JoinType::Left,
    }
}

fn elixir_model_to_core(m: ElixirModel) -> Model {
    Model {
        name: m.name,
        datasource: m.datasource,
        description: m.description,
        measures: m.measures.into_iter().map(|mm| Measure {
            formula: Formula::new(mm.formula).with_label(mm.label.unwrap_or_default()).with_format(mm.format.unwrap_or_default()),
            aggregation: parse_aggregation(&mm.aggregation),
        }).collect(),
        dimensions: m.dimensions.into_iter().map(|d| Dimension::new(d.name).with_label(d.label.unwrap_or_default())).collect(),
        time_dimensions: m.time_dimensions.into_iter().map(|t| TimeDimension::new(t.dimension, parse_granularity(&t.granularity)).with_label(t.label.unwrap_or_default())).collect(),
        joins: m.joins.into_iter().map(|j| Join {
            name: j.name,
            model: j.model,
            join_type: parse_join_type(&j.join_type),
            on: j.on,
            alias: j.alias,
        }).collect(),
        sql: None,
        meta: HashMap::new(),
    }
}

fn elixir_ds_to_core(ds: ElixirDataSource) -> DataSource {
    DataSource {
        name: ds.name,
        driver: ds.driver,
        connection_string: ds.connection_string,
        description: ds.description,
        models: ds.models,
        pool_size: ds.pool_size,
        meta: HashMap::new(),
    }
}

fn elixir_query_to_core(q: ElixirQuery) -> CoreQuery {
    CoreQuery {
        name: q.name,
        source_model: q.source_model.map(|s| SourceSpec {
            model: s.model,
            datasource: s.datasource,
            alias: s.alias,
        }),
        measures: q.measures.into_iter().map(|mm| Measure {
            formula: Formula::new(mm.formula).with_label(mm.label.unwrap_or_default()).with_format(mm.format.unwrap_or_default()),
            aggregation: parse_aggregation(&mm.aggregation),
        }).collect(),
        dimensions: q.dimensions.into_iter().map(|d| Dimension::new(d.name).with_label(d.label.unwrap_or_default())).collect(),
        time_dimensions: q.time_dimensions.into_iter().map(|t| TimeDimension::new(t.dimension, parse_granularity(&t.granularity)).with_label(t.label.unwrap_or_default())).collect(),
        filters: q.filters.into_iter().map(|f| {
            let mut filter = Filter::new(f.field, parse_operator(&f.operator), serde_json::Value::from(f.value));
            if f.or_condition {
                filter = filter.or();
            }
            filter
        }).collect(),
        order: q.order.into_iter().map(|o| OrderBy::new(o.field, o.descending)).collect(),
        limit: q.limit,
        offset: q.offset,
        whole_periods_only: q.whole_periods_only,
        distinct_dimension_values: q.distinct_dimension_values,
        stage_ref: q.stage_ref,
    }
}

fn core_model_to_elixir(m: Model) -> ElixirModel {
    ElixirModel {
        name: m.name,
        datasource: m.datasource,
        description: m.description,
        measures: m.measures.into_iter().map(|mm| ElixirMeasure {
            formula: mm.formula.expression,
            label: mm.formula.label,
            format: mm.formula.format,
            aggregation: mm.aggregation.to_string(),
        }).collect(),
        dimensions: m.dimensions.into_iter().map(|d| ElixirDimension {
            name: d.name,
            label: d.label,
        }).collect(),
        time_dimensions: m.time_dimensions.into_iter().map(|t| ElixirTimeDimension {
            dimension: t.dimension,
            granularity: granularity_to_string(t.granularity),
            label: t.label,
        }).collect(),
        joins: m.joins.into_iter().map(|j| ElixirJoin {
            name: j.name,
            model: j.model,
            join_type: join_type_to_string(j.join_type),
            on: j.on,
            alias: j.alias,
        }).collect(),
    }
}

fn granularity_to_string(g: TimeGranularity) -> String {
    match g {
        TimeGranularity::Second => "SECOND",
        TimeGranularity::Minute => "MINUTE",
        TimeGranularity::Hour => "HOUR",
        TimeGranularity::Day => "DAY",
        TimeGranularity::Week => "WEEK",
        TimeGranularity::Month => "MONTH",
        TimeGranularity::Quarter => "QUARTER",
        TimeGranularity::Year => "YEAR",
    }.to_string()
}

fn join_type_to_string(j: JoinType) -> String {
    match j {
        JoinType::Inner => "INNER",
        JoinType::Left => "LEFT",
        JoinType::Right => "RIGHT",
        JoinType::Full => "FULL",
    }.to_string()
}

fn core_ds_to_elixir(ds: DataSource) -> ElixirDataSource {
    ElixirDataSource {
        name: ds.name,
        driver: ds.driver,
        connection_string: ds.connection_string,
        description: ds.description,
        models: ds.models,
        pool_size: ds.pool_size,
    }
}

fn query_result_to_elixir(data: Vec<serde_json::Map<String, serde_json::Value>>, columns: Vec<String>, sql: Option<String>, execution_time_ms: f64) -> ElixirQueryResult {
    ElixirQueryResult {
        data: data.into_iter()
            .map(|row| row.into_iter().map(|(k, v)| (k, JsonValue(v))).collect())
            .collect(),
        columns,
        sql,
        execution_time_ms,
    }
}

fn init_resource_type(env: Env) -> bool {
    rustler::resource!(EngineResource, env);
    true
}

#[rustler::nif]
fn init_engine(env: Env, storage_path: String) -> NifResult<ResourceArc<EngineResource>> {
    // Initialize the resource type (returns bool)
    let _success = init_resource_type(env);
    
    let runtime = tokio::runtime::Runtime::new()
        .map_err(|e| Error::Term(Box::new(format!("Runtime error: {}", e))))?;

    let path = PathBuf::from(storage_path);
    let storage = runtime.block_on(async {
        YamlStorage::new(&path).map_err(|e| Error::Term(Box::new(e.to_string())))
    })?;

    let storage = Arc::new(storage);

    let models = runtime.block_on(async {
        storage.list_models(None).await.map_err(|e| Error::Term(Box::new(e.to_string())))
    })?;

    let dialect = get_dialect("postgres");
    let conn_manager = Arc::new(ConnectionManager::new());
    let executor = Arc::new(QueryExecutor::new(conn_manager));
    let sql_engine = Arc::new(
        SqlEngine::new(dialect, executor)
            .map_err(|e| Error::Term(Box::new(e.to_string())))?
            .with_models(models)
    );

    let resource = EngineResource {
        sql_engine,
        storage,
        runtime,
    };

    Ok(ResourceArc::new(resource))
}

#[rustler::nif]
fn create_datasource(engine: ResourceArc<EngineResource>, ds: ElixirDataSource) -> NifResult<ElixirDataSource> {
    let core_ds = elixir_ds_to_core(ds);
    let created = engine.runtime.block_on(async {
        engine.storage.create_datasource(core_ds).await.map_err(|e| Error::Term(Box::new(e.to_string())))
    })?;
    Ok(core_ds_to_elixir(created))
}

#[rustler::nif]
fn list_datasources(engine: ResourceArc<EngineResource>) -> NifResult<Vec<ElixirDataSource>> {
    let datasources = engine.runtime.block_on(async {
        engine.storage.list_datasources().await.map_err(|e| Error::Term(Box::new(e.to_string())))
    })?;
    Ok(datasources.into_iter().map(core_ds_to_elixir).collect())
}

#[rustler::nif]
fn get_datasource(engine: ResourceArc<EngineResource>, name: String) -> NifResult<Option<ElixirDataSource>> {
    let ds = engine.runtime.block_on(async {
        engine.storage.get_datasource(&name).await.map_err(|e| Error::Term(Box::new(e.to_string())))
    })?;
    Ok(ds.map(core_ds_to_elixir))
}

#[rustler::nif]
fn create_model(engine: ResourceArc<EngineResource>, model: ElixirModel) -> NifResult<ElixirModel> {
    let core_model = elixir_model_to_core(model);
    let created = engine.runtime.block_on(async {
        engine.storage.create_model(core_model).await.map_err(|e| Error::Term(Box::new(e.to_string())))
    })?;
    engine.sql_engine.invalidate_result_cache();
    Ok(core_model_to_elixir(created))
}

#[rustler::nif]
fn list_models(engine: ResourceArc<EngineResource>, datasource: Option<String>) -> NifResult<Vec<ElixirModel>> {
    let models = engine.runtime.block_on(async {
        engine.storage.list_models(datasource.as_deref()).await.map_err(|e| Error::Term(Box::new(e.to_string())))
    })?;
    Ok(models.into_iter().map(core_model_to_elixir).collect())
}

#[rustler::nif]
fn get_model(engine: ResourceArc<EngineResource>, name: String, datasource: Option<String>) -> NifResult<Option<ElixirModel>> {
    let model = engine.runtime.block_on(async {
        engine.storage.get_model(&name, datasource.as_deref()).await.map_err(|e| Error::Term(Box::new(e.to_string())))
    })?;
    Ok(model.map(core_model_to_elixir))
}

#[rustler::nif]
fn update_model(engine: ResourceArc<EngineResource>, name: String, model: ElixirModel) -> NifResult<ElixirModel> {
    let core_model = elixir_model_to_core(model);
    let updated = engine.runtime.block_on(async {
        engine.storage.update_model(&name, core_model).await.map_err(|e| Error::Term(Box::new(e.to_string())))
    })?;
    engine.sql_engine.invalidate_result_cache();
    Ok(core_model_to_elixir(updated))
}

#[rustler::nif]
fn delete_model(engine: ResourceArc<EngineResource>, name: String, datasource: Option<String>) -> NifResult<bool> {
    let deleted = engine.runtime.block_on(async {
        engine.storage.delete_model(&name, datasource.as_deref()).await.map_err(|e| Error::Term(Box::new(e.to_string())))
    })?;
    engine.sql_engine.invalidate_result_cache();
    Ok(deleted)
}

#[rustler::nif]
fn nif_execute_query(engine: ResourceArc<EngineResource>, query: ElixirQuery, dry_run: bool) -> NifResult<ElixirQueryResult> {
    nif_execute_query_impl(engine, query, dry_run)
}

fn nif_execute_query_impl(engine: ResourceArc<EngineResource>, query: ElixirQuery, dry_run: bool) -> NifResult<ElixirQueryResult> {
    let core_query = elixir_query_to_core(query);
    let start = std::time::Instant::now();

    let model_name = core_query
        .name
        .as_ref()
        .or_else(|| core_query.source_model.as_ref().map(|s| &s.model))
        .ok_or_else(|| Error::Term(Box::new("Query must have a name or source_model".to_string())))?
        .clone();

    let model = engine.runtime.block_on(async {
        engine.storage.get_model(&model_name, None).await.map_err(|e| Error::Term(Box::new(e.to_string())))
    })?.ok_or_else(|| Error::Term(Box::new(format!("Model not found: {}", model_name))))?;

    let datasource = engine.runtime.block_on(async {
        engine.storage.get_datasource(&model.datasource).await.map_err(|e| Error::Term(Box::new(e.to_string())))
    })?.ok_or_else(|| Error::Term(Box::new(format!("Datasource not found: {}", model.datasource))))?;

    let sql = engine.sql_engine.generate_sql(&core_query)
        .map_err(|e| Error::Term(Box::new(e.to_string())))?;

    if dry_run {
        return Ok(ElixirQueryResult {
            data: vec![],
            columns: vec![],
            sql: Some(sql),
            execution_time_ms: start.elapsed().as_millis() as f64,
        });
    }

    let results = engine.runtime.block_on(async {
        engine.sql_engine.execute_sqlx(&datasource, &sql).await.map_err(|e| Error::Term(Box::new(e.to_string())))
    })?;

    let columns = if !results.is_empty() {
        results[0].keys().cloned().collect()
    } else {
        vec![]
    };

    let execution_time_ms = start.elapsed().as_millis() as f64;

    Ok(query_result_to_elixir(results, columns, Some(sql), execution_time_ms))
}

#[rustler::nif]
fn nif_dry_run_query(engine: ResourceArc<EngineResource>, query: ElixirQuery) -> NifResult<ElixirQueryResult> {
    // Call the internal function directly to avoid macro hygiene issues
    nif_execute_query_impl(engine, query, true)
}

rustler::init!("Elixir.GraphNight.Native", [
    init_engine,
    create_datasource,
    list_datasources,
    get_datasource,
    create_model,
    list_models,
    get_model,
    update_model,
    delete_model,
    nif_execute_query,
    nif_dry_run_query,
]);