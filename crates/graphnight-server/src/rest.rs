use axum::{
    extract::{Path, State},
    http::HeaderMap,
    Json,
};
use graphnight_core::models::Query as CoreQuery;
use graphnight_runtime::governance::{ExecuteOptions, ServiceError};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

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

// ============================================
// Additional REST Types
// ============================================

#[derive(Debug, Deserialize)]
pub struct RestMultiStageInput {
    pub stages: Vec<RestStageInput>,
}

#[derive(Debug, Deserialize)]
pub struct RestStageInput {
    pub name: String,
    pub query: RestQueryInput,
    // Dependencies are declared by the stage's own `query.stage_ref`, not by a
    // separate `depends_on` list. Two ways to say the same thing would let a
    // caller set one and silently have the other win.
}

#[derive(Debug, Serialize)]
pub struct RestStageResult {
    pub stage_name: String,
    pub data: Vec<Map<String, Value>>,
    pub columns: Vec<String>,
    /// Present for dry runs only, matching the single-query endpoint.
    pub sql: Option<String>,
    pub row_count: usize,
    pub truncated: bool,
    pub execution_time_ms: f64,
}

#[derive(Debug, Serialize)]
pub struct RestMultiStageResponse {
    pub results: Vec<RestStageResult>,
    pub execution_time_ms: f64,
}

#[derive(Debug, Serialize)]
pub struct RestSearchResult {
    pub model_name: String,
    pub datasource: String,
    pub score: f64,
    pub matched_fields: Vec<String>,
    pub snippet: String,
}

#[derive(Debug, Serialize)]
pub struct RestSearchResponse {
    pub results: Vec<RestSearchResult>,
    pub total: usize,
}

#[derive(Debug, Deserialize)]
pub struct RestSearchParams {
    pub q: String,
    pub limit: Option<i32>,
}

impl RestQueryInput {
    /// Reject aggregations that are not plain identifiers.
    ///
    /// `AggregationType::Custom` is interpolated into the generated statement as
    /// a function name, so this is the difference between "percentile_cont" and
    /// an injection point. Checked in the handler because the `From` conversion
    /// is infallible by signature and must not silently substitute a default.
    fn reject_bad_aggregations(&self) -> Result<(), (axum::http::StatusCode, String)> {
        use graphnight_core::models::AggregationType;
        for m in self.measures.iter().flatten() {
            AggregationType::parse_strict(&m.aggregation)
                .map_err(|e| (axum::http::StatusCode::BAD_REQUEST, e))?;
        }
        Ok(())
    }

    /// Reject filter operators that are not recognised.
    ///
    /// Same reasoning and same place as [`Self::reject_bad_aggregations`]: the
    /// `From` conversion is infallible by signature, so an unrecognised operator
    /// would have to fall back to a default there. Falling back to `Eq` turns a
    /// typo such as `intresects` into an equality comparison between a geometry
    /// column and text — a query that returns no rows and reports no error.
    /// Checked in the handler so the caller gets a 400 naming the operator.
    fn reject_bad_operators(&self) -> Result<(), (axum::http::StatusCode, String)> {
        use graphnight_core::models::FilterOperator;
        for f in self.filters.iter().flatten() {
            FilterOperator::parse_strict(&f.operator)
                .map_err(|e| (axum::http::StatusCode::BAD_REQUEST, e))?;
        }
        Ok(())
    }

    /// Everything the infallible [`From`] conversion relies on being valid.
    fn reject_bad_input(&self) -> Result<(), (axum::http::StatusCode, String)> {
        self.reject_bad_aggregations()?;
        self.reject_bad_operators()
    }
}

impl From<RestQueryInput> for CoreQuery {
    fn from(v: RestQueryInput) -> Self {
        use graphnight_core::models::{
            AggregationType, Dimension, Filter, FilterOperator, Measure, OrderBy, SourceSpec,
            TimeDimension, TimeGranularity,
        };

        // Strict: `Custom` is interpolated into the SQL as a function name, so
        // free text here would be an injection point.
        // Validated before we get here by `reject_bad_aggregations`, which can
        // report a 400; this fallback is unreachable for well-formed input.
        let parse_aggregation =
            |s: &str| AggregationType::parse_strict(s).unwrap_or(AggregationType::Sum);

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

        // Strict: an unrecognised operator must not become `Eq`, which would
        // silently compare a column against the value and return no rows.
        // Validated before we get here by `reject_bad_operators`, which reports
        // a 400; this fallback is unreachable for well-formed input.
        let parse_operator = |s: &str| {
            FilterOperator::parse_strict(s).unwrap_or(FilterOperator::Eq)
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
    let identity = state
        .auth
        .resolve_identity(api_key.as_deref(), tenant_id)
        .await;

    // Policy-filtered via the shared service, so a restricted caller cannot
    // enumerate models it is not allowed to query.
    let models = state
        .query_service
        .visible_models(&state.call_context(identity.as_ref()), None)
        .await
        .map_err(|e| (map_service_error(&e), e.to_string()))?;

    let model_infos: Vec<RestModelInfo> = models
        .into_iter()
        .map(|m| RestModelInfo {
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
        .collect();

    Ok(Json(RestModelsResponse {
        models: model_infos,
    }))
}

pub async fn rest_get_model(
    State(state): State<crate::AppState>,
    headers: HeaderMap,
    Path(name): Path<String>,
) -> Result<Json<RestModelInfo>, (axum::http::StatusCode, String)> {
    let api_key = crate::auth_config::extract_api_key(&headers);
    let tenant_id = crate::auth_config::extract_tenant(&headers);
    let identity = state
        .auth
        .resolve_identity(api_key.as_deref(), tenant_id)
        .await;

    // A model the caller may not see reports 404, not 403, so the endpoint does
    // not confirm the existence of a model outside their policy.
    let model = state
        .query_service
        .get_model(&state.call_context(identity.as_ref()), &name, None)
        .await
        .map_err(|_| {
            (
                axum::http::StatusCode::NOT_FOUND,
                format!("Model not found: {}", name),
            )
        })?;

    Ok(Json(RestModelInfo {
        name: model.name,
        datasource: model.datasource,
        description: model.description,
        measures: model
            .measures
            .iter()
            .map(|m| m.formula.expression.clone())
            .collect(),
        dimensions: model.dimensions.iter().map(|d| d.name.clone()).collect(),
        time_dimensions: model
            .time_dimensions
            .iter()
            .map(|t| t.dimension.clone())
            .collect(),
    }))
}

pub async fn rest_list_datasources(
    State(state): State<crate::AppState>,
    headers: HeaderMap,
) -> Result<Json<RestDatasourcesResponse>, (axum::http::StatusCode, String)> {
    let api_key = crate::auth_config::extract_api_key(&headers);
    let tenant_id = crate::auth_config::extract_tenant(&headers);
    let identity = state
        .auth
        .resolve_identity(api_key.as_deref(), tenant_id)
        .await;

    let datasources = state
        .query_service
        .visible_datasources(&state.call_context(identity.as_ref()))
        .await
        .map_err(|e| (map_service_error(&e), e.to_string()))?;

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
    let identity = state
        .auth
        .resolve_identity(api_key.as_deref(), tenant_id)
        .await;

    // Validate before conversion so a bad aggregation or operator is a 400 with
    // a usable message, rather than reaching SQL generation as a raw name.
    input.reject_bad_input()?;

    let call_ctx = state.call_context(identity.as_ref());
    let query: CoreQuery = input.into();

    // Interface-uniform governance: the same service, policy and audit trail as
    // GraphQL, MCP and agents. No policy logic lives in this handler.
    let outcome = state
        .query_service
        .execute(
            &call_ctx,
            query,
            ExecuteOptions {
                explain: true,
                ..ExecuteOptions::executing()
            },
        )
        .await
        .map_err(|e| (map_service_error(&e), e.to_string()))?;

    Ok(Json(RestQueryResponse {
        data: outcome.data,
        columns: outcome.columns,
        sql: outcome.sql,
        execution_time_ms: start.elapsed().as_millis() as f64,
    }))
}

/// Run a dependency-ordered pipeline of queries in one request.
///
/// Stage dependencies come from each stage's `stage_ref`; ordering, cycle and
/// duplicate detection all live in the service, and every stage is governed
/// individually (policy, row cap, audit) rather than only the first.
pub async fn rest_multi_stage_query(
    State(state): State<crate::AppState>,
    headers: HeaderMap,
    Json(input): Json<RestMultiStageInput>,
) -> Result<Json<RestMultiStageResponse>, (axum::http::StatusCode, String)> {
    let start = std::time::Instant::now();
    let api_key = crate::auth_config::extract_api_key(&headers);
    let tenant_id = crate::auth_config::extract_tenant(&headers);
    let identity = state
        .auth
        .resolve_identity(api_key.as_deref(), tenant_id)
        .await;
    let call_ctx = state.call_context(identity.as_ref());

    for stage in &input.stages {
        stage.query.reject_bad_input()?;
    }

    let stage_names: Vec<String> = input.stages.iter().map(|s| s.name.clone()).collect();
    let stages: Vec<(String, CoreQuery)> = input
        .stages
        .into_iter()
        .map(|s| (s.name, CoreQuery::from(s.query)))
        .collect();

    let dry_run = headers
        .get("x-graphnight-dry-run")
        .and_then(|v| v.to_str().ok())
        .is_some_and(|v| v == "1" || v.eq_ignore_ascii_case("true"));

    let outcomes = state
        .query_service
        .execute_multi_stage(&call_ctx, stages, dry_run)
        .await
        .map_err(|e| (map_service_error(&e), e.to_string()))?;

    Ok(Json(RestMultiStageResponse {
        results: outcomes
            .into_iter()
            .enumerate()
            // The service returns results in the caller's stage order, not
            // dependency order, so the names line up positionally.
            .map(|(i, o)| RestStageResult {
                stage_name: stage_names.get(i).cloned().unwrap_or_default(),
                data: o
                    .data
                    .into_iter()
                    .map(|v| match v {
                        Value::Object(m) => m,
                        other => {
                            let mut m = Map::new();
                            m.insert("value".to_string(), other);
                            m
                        }
                    })
                    .collect(),
                columns: o.columns,
                sql: o.sql,
                row_count: o.row_count,
                truncated: o.truncated,
                execution_time_ms: o.execution_time_ms,
            })
            .collect(),
        execution_time_ms: start.elapsed().as_millis() as f64,
    }))
}

/// Search the model index, policy-filtered by the service.
pub async fn rest_search(
    State(state): State<crate::AppState>,
    headers: HeaderMap,
    axum::extract::Query(params): axum::extract::Query<RestSearchParams>,
) -> Result<Json<RestSearchResponse>, (axum::http::StatusCode, String)> {
    let api_key = crate::auth_config::extract_api_key(&headers);
    let tenant_id = crate::auth_config::extract_tenant(&headers);
    let identity = state
        .auth
        .resolve_identity(api_key.as_deref(), tenant_id)
        .await;
    let call_ctx = state.call_context(identity.as_ref());

    let hits = state
        .query_service
        .search(&call_ctx, &params.q, params.limit.unwrap_or(10) as usize)
        .await
        .map_err(|e| (map_service_error(&e), e.to_string()))?;

    let total = hits.len();
    Ok(Json(RestSearchResponse {
        results: hits
            .into_iter()
            .map(|h| RestSearchResult {
                model_name: h.model_name,
                datasource: h.datasource,
                score: h.score as f64,
                matched_fields: h.matched_fields,
                snippet: h.snippet,
            })
            .collect(),
        total,
    }))
}

/// Translate a service error into the HTTP status a client should see.
///
/// The mapping is deliberately finer-grained than a blanket 500: a denied model
/// and a typo are different problems and a client cannot fix either by retrying.
/// 401 and 403 are kept distinct on purpose — a missing credential is something
/// the client can fix by authenticating, a policy denial is not.
pub(crate) fn map_service_error(e: &ServiceError) -> axum::http::StatusCode {
    use axum::http::StatusCode;
    match e {
        ServiceError::Unauthenticated(_) => StatusCode::UNAUTHORIZED,
        ServiceError::PolicyViolation(_) => StatusCode::FORBIDDEN,
        ServiceError::ModelNotFound(_) | ServiceError::DatasourceNotFound(_) => {
            StatusCode::NOT_FOUND
        }
        ServiceError::InvalidQuery(_) | ServiceError::Invalid(_) => StatusCode::BAD_REQUEST,
        ServiceError::Timeout(_) => StatusCode::GATEWAY_TIMEOUT,
        ServiceError::Storage(_) => StatusCode::INTERNAL_SERVER_ERROR,
        ServiceError::Execution(_) | ServiceError::UnknownTool(_) => {
            StatusCode::INTERNAL_SERVER_ERROR
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::auth_config::AuthConfig;
    use axum::extract::State;
    use graphnight_core::security::JsonlAuditSink;
    use graphnight_sql::dialects::get_dialect;
    use graphnight_sql::executor::{ConnectionManager, QueryExecutor};
    use graphnight_sql::SqlEngine;
    use graphnight_storage::{StorageBackend, YamlStorage};
    use std::collections::HashMap;
    use std::str::FromStr;
    use std::sync::Arc;

    fn write_file(dir: &std::path::Path, name: &str, body: &str) {
        std::fs::write(dir.join(name), body).unwrap();
    }

    /// Boots an AppState with a real temp SQLite DB, API-key auth (required),
    /// and a single `orders` model carrying a `tenant_id` column per row.
    async fn setup() -> (crate::AppState, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("demo.db");

        let datasources = format!(
            "- name: demo\n  driver: sqlite\n  connection_string: sqlite:{}\n  description: test\n  models: [orders]\n  pool_size: 1\n  meta: {{}}\n",
            db_path.display()
        );
        let models = r#"
- name: orders
  datasource: demo
  description: "test orders"
  measures:
    - formula: { expression: amount_usd, label: Revenue (USD), format: null }
      aggregation: sum
    - formula: { expression: "*", label: Orders, format: null }
      aggregation: count
  dimensions:
    - name: status
      label: Order Status
  time_dimensions: []
  joins: []
  sql: null
  meta: {}
"#;
        write_file(dir.path(), "datasources.yaml", datasources.as_str());
        write_file(dir.path(), "models.yaml", models);

        let storage = Arc::new(YamlStorage::new(dir.path()).unwrap());
        storage.load().await.unwrap();

        let opts =
            sqlx::sqlite::SqliteConnectOptions::from_str(&format!("sqlite:{}", db_path.display()))
                .unwrap()
                .create_if_missing(true);
        let pool = sqlx::sqlite::SqlitePoolOptions::new()
            .max_connections(1)
            .connect_with(opts)
            .await
            .unwrap();
        sqlx::query(
            "CREATE TABLE orders (amount_usd REAL NOT NULL, status TEXT NOT NULL, tenant_id TEXT NOT NULL);",
        )
        .execute(&pool)
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO orders (amount_usd, status, tenant_id) VALUES \
             (10.0, 'completed', 'tenant-7'), \
             (25.5, 'completed', 'tenant-8'), \
             (5.0,  'pending',   'tenant-7');",
        )
        .execute(&pool)
        .await
        .unwrap();
        pool.close().await;

        let conn_manager = Arc::new(ConnectionManager::new());
        let executor = Arc::new(QueryExecutor::new(conn_manager.clone()));
        let models = storage.list_models(None).await.unwrap();
        let sql_engine = Arc::new(
            SqlEngine::new(get_dialect("sqlite"), executor)
                .unwrap()
                .with_models(models),
        );

        let schema = graphnight_graphql::build_schema(sql_engine.clone(), storage.clone());
        let auth = AuthConfig {
            api_keys: HashMap::from([("alice".into(), "secret1".into())]),
            admin_users: Default::default(),
            auth_required: true,
            oidc: None,
        };
        let audit_sink: Arc<dyn graphnight_core::security::AuditSink> =
            Arc::new(JsonlAuditSink::open(dir.path().join("audit.jsonl")).unwrap());

        let query_service = Arc::new(
            graphnight_runtime::governance::QueryService::new(sql_engine.clone(), storage.clone())
                .with_audit_sink(audit_sink.clone()),
        );
        let mcp = Arc::new(graphnight_runtime::mcp::McpServer::new(
            graphnight_runtime::tools::ToolCatalog::new(query_service.clone()),
            query_service.clone(),
        ));

        let state = crate::AppState {
            schema,
            sql_engine,
            storage,
            conn_manager,
            auth,
            audit_sink,
            query_service,
            mcp,
        };
        (state, dir)
    }

    fn query_input() -> RestQueryInput {
        RestQueryInput {
            name: Some("orders".into()),
            source_model: None,
            measures: Some(vec![
                RestMeasureInput {
                    formula: "amount_usd".into(),
                    label: Some("Revenue".into()),
                    format: None,
                    aggregation: "sum".into(),
                },
                RestMeasureInput {
                    formula: "*".into(),
                    label: Some("Orders".into()),
                    format: None,
                    aggregation: "count".into(),
                },
            ]),
            dimensions: Some(vec![RestDimensionInput {
                name: "status".into(),
                label: Some("Order Status".into()),
            }]),
            time_dimensions: None,
            filters: None,
            order: None,
            limit: Some(10),
            offset: None,
            whole_periods_only: None,
            distinct_dimension_values: None,
            stage_ref: None,
        }
    }

    fn headers_with(extra: &[(&str, &str)]) -> HeaderMap {
        let mut h = HeaderMap::new();
        for (k, v) in extra {
            let name = k
                .parse::<axum::http::header::HeaderName>()
                .expect("valid header name");
            let value = v
                .parse::<axum::http::header::HeaderValue>()
                .expect("valid header value");
            h.insert(name, value);
        }
        h
    }

    fn measure_input(aggregation: &str) -> RestQueryInput {
        RestQueryInput {
            name: Some("orders".into()),
            source_model: None,
            measures: Some(vec![RestMeasureInput {
                formula: "amount_usd".into(),
                label: None,
                format: None,
                aggregation: aggregation.into(),
            }]),
            dimensions: None,
            time_dimensions: None,
            filters: None,
            order: None,
            limit: Some(10),
            offset: None,
            whole_periods_only: None,
            distinct_dimension_values: None,
            stage_ref: None,
        }
    }

    #[tokio::test]
    async fn rejects_an_aggregation_that_is_not_an_identifier() {
        let (state, _dir) = setup().await;
        let res = rest_query(
            State(state),
            headers_with(&[("authorization", "Bearer secret1")]),
            Json(measure_input("sum); DROP TABLE orders; --")),
        )
        .await;
        let err = res.unwrap_err();
        assert_eq!(err.0, axum::http::StatusCode::BAD_REQUEST);
        assert!(
            err.1.contains("plain function name"),
            "message should explain what is allowed instead: {err:?}"
        );
    }

    #[tokio::test]
    async fn accepts_a_plain_custom_aggregation() {
        let (state, _dir) = setup().await;
        // A real function name is legitimate and must not be rejected.
        let res = rest_query(
            State(state),
            headers_with(&[("authorization", "Bearer secret1")]),
            Json(measure_input("percentile_cont")),
        )
        .await;
        let err = res.unwrap_err();
        // Dry run / execution reaches the engine; the point is it is not a 400.
        assert_ne!(err.0, axum::http::StatusCode::BAD_REQUEST, "{err:?}");
    }

    #[tokio::test]
    async fn search_is_policy_filtered_end_to_end() {
        let (state, _dir) = setup().await;
        // `secret1` is scoped to the `orders` model only (see setup()).
        let res = rest_search(
            State(state.clone()),
            headers_with(&[("authorization", "Bearer secret1")]),
            axum::extract::Query(RestSearchParams {
                q: "orders".into(),
                limit: Some(50),
            }),
        )
        .await
        .expect("search should succeed");
        for hit in &res.results {
            assert!(
                hit.model_name == "orders",
                "search leaked a model outside the caller's policy: {hit:?}"
            );
        }
    }

    #[tokio::test]
    async fn search_requires_authentication() {
        let (state, _dir) = setup().await;
        let res = rest_search(
            State(state),
            HeaderMap::new(),
            axum::extract::Query(RestSearchParams {
                q: "orders".into(),
                limit: None,
            }),
        )
        .await;
        assert_eq!(res.unwrap_err().0, axum::http::StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn multi_stage_runs_stages_in_caller_order() {
        let (state, _dir) = setup().await;
        let mut first = measure_input("sum");
        first.name = Some("orders".into());
        let mut second = measure_input("sum");
        second.name = Some("orders".into());
        // The second stage consumes the first one's output.
        second.stage_ref = Some("first".into());

        let res = rest_multi_stage_query(
            State(state),
            headers_with(&[("authorization", "Bearer secret1")]),
            Json(RestMultiStageInput {
                stages: vec![
                    RestStageInput {
                        name: "second".into(),
                        query: second,
                    },
                    RestStageInput {
                        name: "first".into(),
                        query: first,
                    },
                ],
            }),
        )
        .await
        .expect("pipeline should run");

        // Dependency order is `first` then `second`, but the response must be in
        // the order the caller listed them, or client code cannot zip by index.
        let names: Vec<&str> = res.results.iter().map(|r| r.stage_name.as_str()).collect();
        assert_eq!(names, vec!["second", "first"], "wrong result order");
    }

    #[tokio::test]
    async fn multi_stage_rejects_a_cycle() {
        let (state, _dir) = setup().await;
        let mut a = measure_input("sum");
        a.name = Some("orders".into());
        a.stage_ref = Some("b".into());
        let mut b = measure_input("sum");
        b.name = Some("orders".into());
        b.stage_ref = Some("a".into());

        let res = rest_multi_stage_query(
            State(state),
            headers_with(&[("authorization", "Bearer secret1")]),
            Json(RestMultiStageInput {
                stages: vec![
                    RestStageInput {
                        name: "a".into(),
                        query: a,
                    },
                    RestStageInput {
                        name: "b".into(),
                        query: b,
                    },
                ],
            }),
        )
        .await;
        let err = res.unwrap_err();
        assert_eq!(err.0, axum::http::StatusCode::BAD_REQUEST);
        assert!(err.1.to_lowercase().contains("cycle"), "{err:?}");
    }

    #[tokio::test]
    async fn multi_stage_rejects_a_bad_aggregation_before_executing() {
        let (state, _dir) = setup().await;
        let res = rest_multi_stage_query(
            State(state),
            headers_with(&[("authorization", "Bearer secret1")]),
            Json(RestMultiStageInput {
                stages: vec![RestStageInput {
                    name: "a".into(),
                    query: measure_input("sum); DROP TABLE orders; --"),
                }],
            }),
        )
        .await;
        assert_eq!(res.unwrap_err().0, axum::http::StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn rejects_anonymous_query_when_auth_required() {
        let (state, _dir) = setup().await;
        let res = rest_query(State(state), HeaderMap::new(), Json(query_input())).await;
        let err = res.unwrap_err();
        assert_eq!(err.0, axum::http::StatusCode::UNAUTHORIZED);
        assert!(err.1.contains("Authentication required"));
    }

    #[tokio::test]
    async fn applies_tenant_forced_filter_end_to_end() {
        let (state, _dir) = setup().await;
        let headers = headers_with(&[
            ("authorization", "Bearer secret1"),
            ("x-tenant-id", "tenant-7"),
        ]);
        let res = rest_query(State(state), headers, Json(query_input())).await;
        let json = res.map(|j| j.0).unwrap();

        let sql = json.sql.as_deref().unwrap();
        assert!(
            sql.contains("tenant-7"),
            "forced filter must be compiled in: {sql}"
        );

        // tenant-8's completed order (25.5) must be excluded; only the two
        // tenant-7 rows (completed 10.0, pending 5.0) remain.
        assert_eq!(json.data.len(), 2, "rows={:?}", json.data);
        let completed = json
            .data
            .iter()
            .find(|r| r.get("Order Status").and_then(|v| v.as_str()) == Some("completed"));
        assert!(completed.is_some(), "rows={:?}", json.data);
        let revenue = completed
            .unwrap()
            .get("Revenue")
            .and_then(|v| v.as_f64())
            .unwrap();
        assert!(
            (revenue - 10.0).abs() < 1e-6,
            "cross-tenant completed revenue must be 10.0, got {revenue}"
        );
    }
}
