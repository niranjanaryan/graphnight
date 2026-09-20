use async_graphql::Request;
use graphnight_core::models::*;
use graphnight_core::security::SessionPolicy;
use graphnight_graphql::context::GraphQLContext;
use graphnight_graphql::{build_schema, schema::QueryInput};
use graphnight_sql::dialects::get_dialect;
use graphnight_sql::executor::{ConnectionManager, QueryExecutor};
use graphnight_sql::SqlEngine;
use graphnight_storage::StorageBackend;
use graphnight_storage::YamlStorage;
use pretty_assertions::assert_eq;
use serde_json::json;
use std::sync::Arc;
use tempfile::tempdir;

async fn setup_test_env() -> (Arc<SqlEngine>, Arc<dyn StorageBackend>, tempfile::TempDir) {
    let dir = tempdir().unwrap();
    let path = dir.path().to_path_buf();
    let storage = Arc::new(YamlStorage::new(&path).unwrap());

    // Create datasource
    let ds = DataSource {
        name: "postgres".to_string(),
        driver: "postgres".to_string(),
        connection_string: "postgresql://localhost/test".to_string(),
        description: None,
        models: vec!["orders".to_string()],
        pool_size: Some(5),
        meta: Default::default(),
    };
    storage.create_datasource(ds).await.unwrap();

    // Create model
    let model = Model {
        name: "orders".to_string(),
        datasource: "postgres".to_string(),
        description: Some("Test orders model".to_string()),
        measures: vec![
            Measure::simple("revenue", AggregationType::Sum),
            Measure::simple("order_count", AggregationType::Count),
        ],
        dimensions: vec![Dimension::new("status"), Dimension::new("store_id")],
        time_dimensions: vec![TimeDimension::new("created_at", TimeGranularity::Day)],
        joins: vec![],
        sql: None,
        meta: Default::default(),
    };
    let model_for_sql = model.clone();
    storage.create_model(model).await.unwrap();

    let dialect = get_dialect("postgres");
    let conn_manager = Arc::new(ConnectionManager::new());
    let executor = Arc::new(QueryExecutor::new(conn_manager));
    let sql_engine = Arc::new(
        SqlEngine::new(dialect, executor)
            .unwrap()
            .with_models(vec![model_for_sql]),
    );

    (sql_engine, storage, dir)
}

#[tokio::test]
async fn test_query_resolver_dry_run() {
    let (sql_engine, storage, _dir) = setup_test_env().await;
    let schema = build_schema(sql_engine, storage);

    let query = r#"
        query {
            query(dryRun: true, input: {
                name: "orders"
                measures: [{ formula: "revenue:sum", aggregation: SUM }]
                dimensions: [{ name: "status" }]
                filters: [{ field: "status", operator: EQ, value: "completed" }]
                order: [{ field: "revenue:sum", descending: true }]
                limit: 10
            }) {
                sql
                data
            }
        }
    "#;

    let response = schema.execute(Request::new(query)).await;
    assert!(response.is_ok());
}

#[tokio::test]
async fn test_query_denied_by_policy() {
    let (sql_engine, storage, _dir) = setup_test_env().await;
    let schema = build_schema(sql_engine.clone(), storage.clone());
    let ctx = GraphQLContext::new(sql_engine, storage)
        .with_policy(SessionPolicy::new().with_denied_models(vec!["orders".into()]));

    let query = r#"
        query {
            query(dryRun: true, input: {
                name: "orders"
                measures: [{ formula: "revenue", aggregation: SUM }]
            }) { sql }
        }
    "#;

    let response = schema.execute(Request::new(query).data(ctx)).await;
    assert!(!response.errors.is_empty());
    assert!(response.errors[0].message.contains("denied"));
}

#[tokio::test]
async fn test_auth_required_rejects_anonymous() {
    let (sql_engine, storage, _dir) = setup_test_env().await;
    let schema = build_schema(sql_engine.clone(), storage.clone());
    let ctx = GraphQLContext::new(sql_engine, storage).with_auth_required(true);

    let query = r#"
        query {
            query(dryRun: true, input: {
                name: "orders"
                measures: [{ formula: "revenue", aggregation: SUM }]
            }) { sql }
        }
    "#;

    let response = schema.execute(Request::new(query).data(ctx)).await;
    assert!(!response.errors.is_empty());
    assert!(response.errors[0]
        .message
        .contains("Authentication required"));
}

#[tokio::test]
async fn test_admin_required_for_datasource_create() {
    let (sql_engine, storage, _dir) = setup_test_env().await;
    let schema = build_schema(sql_engine.clone(), storage.clone());
    let ctx = GraphQLContext::new(sql_engine, storage)
        .with_auth_required(true)
        .with_user("alice".into(), None)
        .with_admin(false);

    let mutation = r#"
        mutation {
            createDatasource(input: {
                name: "x"
                driver: "sqlite"
                connectionString: "sqlite::memory:"
            }) { name }
        }
    "#;

    let response = schema.execute(Request::new(mutation).data(ctx)).await;
    assert!(!response.errors.is_empty());
    assert!(response.errors[0].message.contains("Admin"));
}

/// Admin + auth-required succeeds on a read-only dry-run (no mutations).
#[tokio::test]
async fn test_admin_auth_succeeds_dry_run_query() {
    let (sql_engine, storage, _dir) = setup_test_env().await;
    let schema = build_schema(sql_engine.clone(), storage.clone());
    let ctx = GraphQLContext::new(sql_engine, storage)
        .with_auth_required(true)
        .with_user("admin".into(), Some("tenant-a".into()))
        .with_admin(true);

    let query = r#"
        query {
            query(dryRun: true, input: {
                name: "orders"
                measures: [{ formula: "revenue", aggregation: SUM }]
                dimensions: [{ name: "status" }]
                limit: 5
            }) {
                sql
                data
            }
        }
    "#;

    let response = schema.execute(Request::new(query).data(ctx)).await;
    assert!(
        response.errors.is_empty(),
        "admin dry-run should succeed: {:?}",
        response.errors
    );
    let data = response.data.into_json().unwrap();
    let sql = data["query"]["sql"].as_str().expect("sql field");
    assert!(
        sql.to_uppercase().contains("SELECT") && sql.to_uppercase().contains("SUM"),
        "unexpected sql: {sql}"
    );
    assert!(data["query"]["data"].is_array());
}

#[tokio::test]
async fn test_models_resolver() {
    let (sql_engine, storage, _dir) = setup_test_env().await;
    let schema = build_schema(sql_engine, storage);

    let query = r#"
        query {
            models {
                name
                datasource
                measures
            }
        }
    "#;

    let response = schema.execute(Request::new(query)).await;
    assert!(response.is_ok());
}

#[tokio::test]
async fn test_model_resolver() {
    let (sql_engine, storage, _dir) = setup_test_env().await;
    let schema = build_schema(sql_engine, storage);

    let query = r#"
        query {
            model(name: "orders") {
                name
                measures
            }
        }
    "#;

    let response = schema.execute(Request::new(query)).await;
    assert!(response.is_ok());
}

#[tokio::test]
async fn test_datasources_resolver() {
    let (sql_engine, storage, _dir) = setup_test_env().await;
    let schema = build_schema(sql_engine, storage);

    let query = r#"
        query {
            datasources {
                name
                driver
            }
        }
    "#;

    let response = schema.execute(Request::new(query)).await;
    assert!(response.is_ok());
}

#[tokio::test]
async fn test_search_resolver() {
    let (sql_engine, storage, _dir) = setup_test_env().await;
    let schema = build_schema(sql_engine, storage);

    let query = r#"
        query {
            search(q: "revenue", limit: 10) {
                modelName
                score
            }
        }
    "#;

    let response = schema.execute(Request::new(query)).await;
    assert!(response.is_ok());
}

#[tokio::test]
async fn test_memories_resolver() {
    let (sql_engine, storage, _dir) = setup_test_env().await;

    // Save a memory first
    use graphnight_storage::Memory;
    let memory = Memory {
        id: "mem_1".to_string(),
        learning: "Test memory".to_string(),
        linked_entities: vec!["revenue:sum".to_string()],
        description: Some("Test".to_string()),
        created_at: chrono::Utc::now(),
        updated_at: chrono::Utc::now(),
        meta: Default::default(),
    };
    storage.save_memory(memory).await.unwrap();

    let schema = build_schema(sql_engine, storage);

    let query = r#"
        query {
            memories(filter: {}) {
                id
                learning
            }
        }
    "#;

    let response = schema.execute(Request::new(query)).await;
    assert!(response.is_ok());
}

#[tokio::test]
async fn test_schema_builds() {
    let (sql_engine, storage, _dir) = setup_test_env().await;
    let schema = build_schema(sql_engine, storage);

    // Verify schema builds successfully
    assert!(schema.sdl().contains("type Query"));
    assert!(schema.sdl().contains("type Mutation"));
    assert!(schema.sdl().contains("type Subscription"));
}

#[test]
fn test_query_input_conversion() {
    let input = QueryInput {
        name: Some("orders".to_string()),
        source_model: None,
        measures: Some(vec![graphnight_graphql::schema::MeasureInput {
            formula: "revenue:sum".to_string(),
            label: Some("Revenue".to_string()),
            format: Some("currency".to_string()),
            aggregation: graphnight_graphql::schema::AggregationType::Sum,
        }]),
        dimensions: Some(vec![graphnight_graphql::schema::DimensionInput {
            name: "status".to_string(),
            label: Some("Status".to_string()),
        }]),
        time_dimensions: Some(vec![graphnight_graphql::schema::TimeDimensionInput {
            dimension: "created_at".to_string(),
            granularity: graphnight_graphql::schema::TimeGranularity::Month,
            label: Some("Month".to_string()),
        }]),
        filters: Some(vec![graphnight_graphql::schema::FilterInput {
            field: "status".to_string(),
            operator: graphnight_graphql::schema::FilterOperator::Eq,
            value: json!("completed"),
            or_condition: Some(false),
        }]),
        order: Some(vec![graphnight_graphql::schema::OrderByInput {
            field: "revenue:sum".to_string(),
            descending: Some(true),
        }]),
        limit: Some(100),
        offset: Some(0),
        whole_periods_only: Some(true),
        distinct_dimension_values: Some(false),
        stage_ref: None,
    };

    let query: graphnight_core::models::Query = input.into();

    assert_eq!(query.name, Some("orders".to_string()));
    assert_eq!(query.measures.len(), 1);
    assert_eq!(query.measures[0].formula.expression, "revenue:sum");
    assert_eq!(query.measures[0].aggregation, AggregationType::Sum);
    assert_eq!(query.dimensions.len(), 1);
    assert_eq!(query.dimensions[0].name, "status");
    assert_eq!(query.time_dimensions.len(), 1);
    assert_eq!(query.time_dimensions[0].granularity, TimeGranularity::Month);
    assert_eq!(query.filters.len(), 1);
    assert_eq!(query.filters[0].field, "status");
    assert_eq!(query.filters[0].operator, FilterOperator::Eq);
    assert_eq!(query.order.len(), 1);
    assert_eq!(query.order[0].field, "revenue:sum");
    assert!(query.order[0].descending);
    assert_eq!(query.limit, Some(100));
    assert_eq!(query.offset, Some(0));
    assert_eq!(query.whole_periods_only, Some(true));
    assert_eq!(query.distinct_dimension_values, Some(false));
}

#[test]
fn test_aggregation_type_conversion() {
    use graphnight_graphql::schema::AggregationType;

    let gql = AggregationType::Sum;
    let core: AggregationType = gql.into();
    assert_eq!(core, AggregationType::Sum);

    let gql = AggregationType::CountDistinct;
    let core: AggregationType = gql.into();
    assert_eq!(core, AggregationType::CountDistinct);
}

#[test]
fn test_time_granularity_conversion() {
    use graphnight_graphql::schema::TimeGranularity;

    let gql = TimeGranularity::Month;
    let core: TimeGranularity = gql.into();
    assert_eq!(core, TimeGranularity::Month);

    let gql = TimeGranularity::Year;
    let core: TimeGranularity = gql.into();
    assert_eq!(core, TimeGranularity::Year);
}

#[test]
fn test_filter_operator_conversion() {
    use graphnight_graphql::schema::FilterOperator;

    let gql = FilterOperator::Eq;
    let core: FilterOperator = gql.into();
    assert_eq!(core, FilterOperator::Eq);

    let gql = FilterOperator::Between;
    let core: FilterOperator = gql.into();
    assert_eq!(core, FilterOperator::Between);
}

#[test]
fn test_join_type_conversion() {
    use graphnight_graphql::schema::JoinType;

    let gql = JoinType::Left;
    let core: JoinType = gql.into();
    assert_eq!(core, JoinType::Left);
}
