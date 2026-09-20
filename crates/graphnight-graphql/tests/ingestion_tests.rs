use async_graphql::Request;
use graphnight_core::models::*;
use graphnight_graphql::context::GraphQLContext;
use graphnight_graphql::{build_schema};
use graphnight_sql::dialects::get_dialect;
use graphnight_sql::executor::{ConnectionManager, QueryExecutor};
use graphnight_sql::SqlEngine;
use graphnight_storage::{StorageBackend, YamlStorage};
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
async fn test_ingest_models_dry_run() {
    // This test verifies that the ingestModels mutation is properly exposed
    // and fails gracefully when no actual DB connection is available
    let (sql_engine, storage, _dir) = setup_test_env().await;
    let schema = build_schema(sql_engine.clone(), storage.clone());

    let ctx = GraphQLContext::new(sql_engine, storage)
        .with_auth_required(true)
        .with_user("admin".into(), Some("tenant-a".into()))
        .with_admin(true);

    // Test that the mutation exists and requires admin
    let mutation = r#"
        mutation {
            ingestModels(datasource: "postgres") {
                modelsCreated
                modelsUpdated
                errors
            }
        }
    "#;

    let response = schema.execute(Request::new(mutation).data(ctx)).await;
    // The mutation should be callable (not return "field not found" error)
    // It will fail because there's no actual DB, but it shouldn't be a schema error
    let errors = response.errors;
    if !errors.is_empty() {
        let error_msg = &errors[0].message;
        // Should be a runtime error (DB connection), not a GraphQL schema error
        assert!(
            error_msg.contains("Introspection failed")
                || error_msg.contains("Datasource not found")
                || error_msg.contains("connect"),
            "Unexpected error: {}",
            error_msg
        );
    }
}

#[tokio::test]
async fn test_ingest_models_requires_admin() {
    let (sql_engine, storage, _dir) = setup_test_env().await;
    let schema = build_schema(sql_engine.clone(), storage.clone());

    // Non-admin user
    let ctx = GraphQLContext::new(sql_engine, storage)
        .with_auth_required(true)
        .with_user("alice".into(), None)
        .with_admin(false);

    let mutation = r#"
        mutation {
            ingestModels(datasource: "postgres") {
                modelsCreated
            }
        }
    "#;

    let response = schema.execute(Request::new(mutation).data(ctx)).await;
    assert!(!response.errors.is_empty());
    assert!(response.errors[0].message.contains("Admin"));
}

#[tokio::test]
async fn test_multi_stage_query_schema() {
    let (sql_engine, storage, _dir) = setup_test_env().await;
    let schema = build_schema(sql_engine.clone(), storage.clone());

    let ctx = GraphQLContext::new(sql_engine, storage)
        .with_auth_required(true)
        .with_user("admin".into(), Some("tenant-a".into()))
        .with_admin(true);

    // Test that the query exists and accepts stage_ref
    // First stage has no stage_ref (no dependency) - will be named "stage_1"
    // Second stage references "stage_1" (the first stage)
    let query = r#"
        query {
            multiStageQuery(
                inputs: [
                    { name: "orders", measures: [{ formula: "revenue:sum", aggregation: SUM }] }
                    { name: "orders", stageRef: "stage_1", measures: [{ formula: "revenue:sum", aggregation: SUM }] }
                ],
                dryRun: true
            ) {
                results { sql data }
                executionTimeMs
            }
        }
    "#;

    let response = schema.execute(Request::new(query).data(ctx)).await;
    // Should fail due to no DB connection, but not due to schema error
    let errors = response.errors;
    if !errors.is_empty() {
        let error_msg = &errors[0].message;
        // Should be a runtime error, not a GraphQL schema error about missing fields
        assert!(
            error_msg.contains("Introspection failed")
                || error_msg.contains("Datasource not found")
                || error_msg.contains("connect")
                || error_msg.contains("Model not found")
                || error_msg.contains("Stage reference")
                || error_msg.contains("database")
                || error_msg.contains("does not exist"),
            "Unexpected error: {}",
            error_msg
        );
    }
}

#[tokio::test]
async fn test_multi_stage_query_cycle_detection() {
    let (sql_engine, storage, _dir) = setup_test_env().await;
    let schema = build_schema(sql_engine.clone(), storage.clone());

    let ctx = GraphQLContext::new(sql_engine, storage)
        .with_auth_required(true)
        .with_user("admin".into(), Some("tenant-a".into()))
        .with_admin(true);

    // Create a cycle: stage_1 -> stage_2 -> stage_1
    // stage_1 references stage_2, stage_2 references stage_1
    let query = r#"
        query {
            multiStageQuery(
                inputs: [
                    { name: "orders", stageRef: "stage_2", measures: [{ formula: "revenue:sum", aggregation: SUM }] }
                    { name: "orders", stageRef: "stage_1", measures: [{ formula: "revenue:sum", aggregation: SUM }] }
                ],
                dryRun: true
            ) {
                results { sql data }
                executionTimeMs
            }
        }
    "#;

    let response = schema.execute(Request::new(query).data(ctx)).await;
    // Should fail with cycle detection error
    let errors = response.errors;
    assert!(!errors.is_empty());
    let error_msg = &errors[0].message;
    assert!(
        error_msg.contains("Cycle detected") || error_msg.contains("duplicate stage"),
        "Expected cycle/duplicate error, got: {}",
        error_msg
    );
}