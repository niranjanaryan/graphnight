use axum::{
    extract::{Path, Query, State},
    http::HeaderMap,
    Json,
};
use graphnight_core::models::{DataSource, Model, Query as CoreQuery};
use graphnight_core::security::SessionPolicy;
use graphnight_graphql::context::GraphQLContext;
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Deserialize)]
pub struct RestQueryInput {
    pub name: Option<String>,
    pub source_model: Option<RestSourceSpec>,
    pub measures: Option<Vec<RestMeasureInput>>,
    pub dimensions: Option<Vec<RestDimensionInput>>,
    pub time_dimensions: Option<Vec<RestTimeDimensionInput>>,
    pub filters: Option<Vec<RestFilterInput>>,
    pub order: Option<Vec<RestOrderByInput>>,
    pub limit: Option<i32>,
    pub offset: Option<i32>,
    pub whole_periods_only: Option<bool>,
    pub distinct_dimension_values: Option<bool>,
    pub stage_ref: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct RestSourceSpec {
    pub model: String,
    pub datasource: Option<String>,
    pub alias: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct RestMeasureInput {
    pub formula: String,
    pub label: Option<String>,
    pub format: Option<String>,
    pub aggregation: String,
}

#[derive(Debug, Deserialize)]
pub struct RestDimensionInput {
    pub name: String,
    pub label: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct RestTimeDimensionInput {
    pub dimension: String,
    pub granularity: String,
    pub label: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct RestFilterInput {
    pub field: String,
    pub operator: String,
    pub value: Value,
    pub or_condition: Option<bool>,
}

#[derive(Debug, Deserialize)]
pub struct RestOrderByInput {
    pub field: String,
    pub descending: Option<bool>,
}

#[derive(Debug, Serialize)]
pub struct RestQueryResponse {
    pub data: Vec<Value>,
    pub columns: Vec<String>,
    pub sql: Option<String>,
    pub execution_time_ms: f64,
}

#[derive(Debug, Serialize)]
pub struct RestModelInfo {
    pub name: String,
    pub datasource: String,
    pub description: Option<String>,
    pub measures: Vec<String>,
    pub dimensions: Vec<String>,
    pub time_dimensions: Vec<String>,
}

#[derive(Debug, Serialize)]
pub struct RestDatasourceInfo {
    pub name: String,
    pub driver: String,
    pub description: Option<String>,
    pub models: Vec<String>,
}

#[derive(Debug, Serialize)]
pub struct RestModelsResponse {
    pub models: Vec<RestModelInfo>,
}

#[derive(Debug, Serialize)]
pub struct RestDatasourcesResponse {
    pub datasources: Vec<RestDatasourceInfo>,
}

impl From<RestQueryInput> for CoreQuery {
    fn from(v: RestQueryInput) -> Self {
        use graphnight_core::models::{
            AggregationType, Dimension, Filter, FilterOperator, Measure, OrderBy, SourceSpec,
            TimeDimension, TimeGranularity,
        };

        let parse_aggregation = |s: &str| match s.to_uppercase().as_str() {
            "SUM" => AggregationType::Sum,
            "AVG" => AggregationType::Avg,
            "COUNT" => AggregationType::Count,
            "MIN" => AggregationType::Min,
            "MAX" => AggregationType::Max,
            "COUNT_DISTINCT" => AggregationType::CountDistinct,
            _ => AggregationType::Custom(s.to_string()),
        };

        let parse_granularity = |s: &str| match s.to_uppercase().as_str() {
            "SECOND" => TimeGranularity::Second,
            "MINUTE" => TimeGranularity::Minute,
            "HOUR" => TimeGranularity::Hour,
            "DAY" => TimeGranularity::Day,
            "WEEK" => TimeGranularity::Week,
            "MONTH" => TimeGranularity::Month,
            "QUARTER" => TimeGranularity::Quarter,
            "YEAR" => TimeGranularity::Year,
            _ => TimeGranularity::Day,
        };

        let parse_operator = |s: &str| match s.to_uppercase().as_str() {
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
        };

        CoreQuery {
            name: v.name,
            source_model: v.source_model.map(|s| SourceSpec {
                model: s.model,
                datasource: s.datasource,
                alias: s.alias,
            }),
            measures: v
                .measures
                .unwrap_or_default()
                .into_iter()
                .map(|m| Measure {
                    formula: graphnight_core::models::Formula::new(m.formula)
                        .with_label(m.label.unwrap_or_default())
                        .with_format(m.format.unwrap_or_default()),
                    aggregation: parse_aggregation(&m.aggregation),
                })
                .collect(),
            dimensions: v
                .dimensions
                .unwrap_or_default()
                .into_iter()
                .map(|d| Dimension::new(d.name).with_label(d.label.unwrap_or_default()))
                .collect(),
            time_dimensions: v
                .time_dimensions
                .unwrap_or_default()
                .into_iter()
                .map(|t| {
                    TimeDimension::new(t.dimension, parse_granularity(&t.granularity))
                        .with_label(t.label.unwrap_or_default())
                })
                .collect(),
            filters: v
                .filters
                .unwrap_or_default()
                .into_iter()
                .map(|f| {
                    let mut filter = Filter::new(f.field, parse_operator(&f.operator), f.value);
                    if f.or_condition.unwrap_or(false) {
                        filter = filter.or();
                    }
                    filter
                })
                .collect(),
            order: v
                .order
                .unwrap_or_default()
                .into_iter()
                .map(|o| OrderBy::new(o.field, o.descending.unwrap_or(false)))
                .collect(),
            limit: v.limit.map(|l| l as usize),
            offset: v.offset.map(|o| o as usize),
            whole_periods_only: v.whole_periods_only,
            distinct_dimension_values: v.distinct_dimension_values,
            stage_ref: v.stage_ref,
        }
    }
}

pub async fn rest_list_models(
    State(state): State<crate::AppState>,
    headers: HeaderMap,
) -> Result<Json<RestModelsResponse>, (axum::http::StatusCode, String)> {
    let api_key = crate::auth_config::extract_api_key(&headers);
    let tenant_id = crate::auth_config::extract_tenant(&headers);
    let identity = state.auth.resolve_identity(api_key.as_deref(), tenant_id).await;

    let gql_ctx = GraphQLContext::new(state.sql_engine.clone(), state.storage.clone())
        .with_auth_required(state.auth.auth_required)
        .with_policy(state.auth.default_policy(identity.as_ref()));

    let models = gql_ctx
        .storage
        .list_models(None)
        .await
        .map_err(|e| (axum::http::StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    let model_infos: Vec<RestModelInfo> = models
        .into_iter()
        .map(|m| RestModelInfo {
            name: m.name,
            datasource: m.datasource,
            description: m.description,
            measures: m.measures.iter().map(|m| m.formula.expression.clone()).collect(),
            dimensions: m.dimensions.iter().map(|d| d.name.clone()).collect(),
            time_dimensions: m.time_dimensions.iter().map(|t| t.dimension.clone()).collect(),
        })
        .collect();

    Ok(Json(RestModelsResponse { models: model_infos }))
}

pub async fn rest_get_model(
    State(state): State<crate::AppState>,
    headers: HeaderMap,
    Path(name): Path<String>,
) -> Result<Json<RestModelInfo>, (axum::http::StatusCode, String)> {
    let api_key = crate::auth_config::extract_api_key(&headers);
    let tenant_id = crate::auth_config::extract_tenant(&headers);
    let identity = state.auth.resolve_identity(api_key.as_deref(), tenant_id).await;

    let gql_ctx = GraphQLContext::new(state.sql_engine.clone(), state.storage.clone())
        .with_auth_required(state.auth.auth_required)
        .with_policy(state.auth.default_policy(identity.as_ref()));

    let model = gql_ctx
        .storage
        .get_model(&name, None)
        .await
        .map_err(|e| (axum::http::StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
        .ok_or_else(|| {
            (
                axum::http::StatusCode::NOT_FOUND,
                format!("Model not found: {}", name),
            )
        })?;

    Ok(Json(RestModelInfo {
        name: model.name,
        datasource: model.datasource,
        description: model.description,
        measures: model.measures.iter().map(|m| m.formula.expression.clone()).collect(),
        dimensions: model.dimensions.iter().map(|d| d.name.clone()).collect(),
        time_dimensions: model.time_dimensions.iter().map(|t| t.dimension.clone()).collect(),
    }))
}

pub async fn rest_list_datasources(
    State(state): State<crate::AppState>,
    headers: HeaderMap,
) -> Result<Json<RestDatasourcesResponse>, (axum::http::StatusCode, String)> {
    let api_key = crate::auth_config::extract_api_key(&headers);
    let tenant_id = crate::auth_config::extract_tenant(&headers);
    let identity = state.auth.resolve_identity(api_key.as_deref(), tenant_id).await;

    let gql_ctx = GraphQLContext::new(state.sql_engine.clone(), state.storage.clone())
        .with_auth_required(state.auth.auth_required)
        .with_policy(state.auth.default_policy(identity.as_ref()));

    let datasources = gql_ctx
        .storage
        .list_datasources()
        .await
        .map_err(|e| (axum::http::StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    let ds_infos: Vec<RestDatasourceInfo> = datasources
        .into_iter()
        .map(|d| RestDatasourceInfo {
            name: d.name,
            driver: d.driver,
            description: d.description,
            models: d.models,
        })
        .collect();

    Ok(Json(RestDatasourcesResponse {
        datasources: ds_infos,
    }))
}

pub async fn rest_query(
    State(state): State<crate::AppState>,
    headers: HeaderMap,
    Json(input): Json<RestQueryInput>,
) -> Result<Json<RestQueryResponse>, (axum::http::StatusCode, String)> {
    let start = std::time::Instant::now();
    let api_key = crate::auth_config::extract_api_key(&headers);
    let tenant_id = crate::auth_config::extract_tenant(&headers);
    let identity = state.auth.resolve_identity(api_key.as_deref(), tenant_id).await;

    let policy = state.auth.default_policy(identity.as_ref());

    let gql_ctx = GraphQLContext::new(state.sql_engine.clone(), state.storage.clone())
        .with_auth_required(state.auth.auth_required)
        .with_policy(policy.clone())
        .with_audit_sink(state.audit_sink.clone());

    let storage = gql_ctx.storage.clone();

    if let Some(id) = identity {
        gql_ctx
            .with_user(id.user_id, id.tenant_id)
            .with_admin(id.is_admin);
    }

    let query: CoreQuery = input.into();
    let model_name = query
        .name
        .as_ref()
        .or_else(|| query.source_model.as_ref().map(|s| &s.model))
        .ok_or_else(|| {
            (
                axum::http::StatusCode::BAD_REQUEST,
                "Query must have a name or source_model".to_string(),
            )
        })?
        .clone();

    let model = storage
        .get_model(&model_name, None)
        .await
        .map_err(|e| (axum::http::StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
        .ok_or_else(|| {
            (
                axum::http::StatusCode::NOT_FOUND,
                format!("Model not found: {}", model_name),
            )
        })?;

    let datasource = storage
        .get_datasource(&model.datasource)
        .await
        .map_err(|e| (axum::http::StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
        .ok_or_else(|| {
            (
                axum::http::StatusCode::NOT_FOUND,
                format!("Datasource not found: {}", model.datasource),
            )
        })?;

    let sql = state
        .sql_engine
        .generate_sql(&query)
        .map_err(|e| (axum::http::StatusCode::BAD_REQUEST, e.to_string()))?;

    let results = state
        .sql_engine
        .execute_sqlx(&datasource, &sql)
        .await
        .map_err(|e| (axum::http::StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    let columns = if !results.is_empty() {
        results[0].keys().cloned().collect()
    } else {
        vec![]
    };

    let mut data: Vec<Value> = results
        .into_iter()
        .map(|m| serde_json::to_value(m).unwrap())
        .collect();

    // Apply column masks from policy
    if !policy.column_masks.is_empty() {
        for row in &mut data {
            if let Value::Object(map) = row {
                for (col, mask_fn) in &policy.column_masks {
                    if let Some(Value::String(val)) = map.get(col) {
                        let masked = mask_fn(val);
                        map.insert(col.clone(), Value::String(masked));
                    }
                }
            }
        }
    }

    Ok(Json(RestQueryResponse {
        data,
        columns,
        sql: Some(sql),
        execution_time_ms: start.elapsed().as_millis() as f64,
    }))
}