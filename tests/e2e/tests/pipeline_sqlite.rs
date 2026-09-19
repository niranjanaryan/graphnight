//! End-to-end: examples YAML → SQL dry-run → optional SQLite execute.
//!
//! Postgres/MySQL live DB coverage lives in `pipeline_testcontainers.rs`.

use graphnight_core::models::{DataSource, Query};
use graphnight_sql::dialects::{PostgresDialect, SqliteDialect};
use graphnight_sql::executor::{ConnectionManager, QueryExecutor};
use graphnight_sql::generator::SqlGenerator;
use graphnight_storage::{StorageBackend, YamlStorage};
use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
use std::path::PathBuf;
use std::str::FromStr;
use std::sync::Arc;

fn repo_examples_data() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../examples/data")
}

fn examples_query_json() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../examples/query.json")
}

#[tokio::test]
async fn examples_storage_boots_and_dry_run_sql() {
    let data_dir = repo_examples_data();
    assert!(
        data_dir.join("models.yaml").exists(),
        "expected examples at {:?}",
        data_dir
    );

    let storage = YamlStorage::new(&data_dir).unwrap();
    storage.load().await.unwrap();

    let models = storage.list_models(None).await.unwrap();
    assert!(models.iter().any(|m| m.name == "orders"));
    assert!(models.iter().any(|m| m.name == "customers"));

    let datasources = storage.list_datasources().await.unwrap();
    assert!(datasources.iter().any(|d| d.name == "demo"));
    assert!(datasources.iter().any(|d| d.driver == "sqlite"));

    let query: Query =
        serde_json::from_str(&std::fs::read_to_string(examples_query_json()).unwrap()).unwrap();

    let pg = SqlGenerator::new(Box::new(PostgresDialect)).with_models(models.clone());
    let pg_sql = pg.generate(&query).unwrap();
    assert!(pg_sql.contains("SUM"));
    assert!(pg_sql.contains("COUNT"));
    assert!(pg_sql.contains("GROUP BY"));
    assert!(pg_sql.contains("LIMIT 100"));

    let sqlite = SqlGenerator::new(Box::new(SqliteDialect)).with_models(models);
    let sqlite_sql = sqlite.generate(&query).unwrap();
    assert!(sqlite_sql.contains("SUM"));
    assert!(sqlite_sql.contains("\"orders\""));
    assert!(sqlite_sql.contains("GROUP BY"));
}

#[tokio::test]
async fn examples_query_executes_against_temp_sqlite() {
    let data_dir = repo_examples_data();
    let storage = YamlStorage::new(&data_dir).unwrap();
    storage.load().await.unwrap();
    let models = storage.list_models(None).await.unwrap();

    let query: Query =
        serde_json::from_str(&std::fs::read_to_string(examples_query_json()).unwrap()).unwrap();

    let generator = SqlGenerator::new(Box::new(SqliteDialect)).with_models(models);
    let sql = generator.generate(&query).unwrap();

    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("orders_demo.db");
    let conn = format!("sqlite:{}", db_path.display());

    let opts = SqliteConnectOptions::from_str(&conn)
        .unwrap()
        .create_if_missing(true);
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect_with(opts)
        .await
        .unwrap();

    // Generator emits model joins from examples YAML (orders → customers).
    sqlx::query(
        r#"
        CREATE TABLE customers (
            id INTEGER PRIMARY KEY,
            segment TEXT NOT NULL
        );
        CREATE TABLE orders (
            amount_usd REAL NOT NULL,
            status TEXT NOT NULL,
            customer_id INTEGER NOT NULL,
            created_at TEXT NOT NULL
        );
        "#,
    )
    .execute(&pool)
    .await
    .unwrap();

    sqlx::query(
        r#"
        INSERT INTO customers (id, segment) VALUES (1, 'enterprise'), (2, 'smb');
        INSERT INTO orders (amount_usd, status, customer_id, created_at) VALUES
          (10.0, 'completed', 1, '2024-01-01'),
          (25.5, 'completed', 2, '2024-01-02'),
          (5.0,  'pending',   1, '2024-01-03');
        "#,
    )
    .execute(&pool)
    .await
    .unwrap();
    pool.close().await;

    let ds = DataSource {
        name: "demo".into(),
        driver: "sqlite".into(),
        connection_string: conn,
        description: Some("e2e temp sqlite".into()),
        models: vec!["orders".into()],
        pool_size: Some(1),
        meta: Default::default(),
    };

    let executor = QueryExecutor::new(Arc::new(ConnectionManager::new()));
    let rows = executor.execute(&ds, &sql).await.unwrap();

    assert!(
        !rows.is_empty(),
        "expected aggregated rows from demo orders; sql={sql}"
    );

    // Two status groups: completed (2 rows, sum 35.5) and pending (1 row, sum 5.0).
    assert_eq!(rows.len(), 2, "rows={rows:?} sql={sql}");

    fn num(v: &serde_json::Value) -> Option<f64> {
        v.as_f64()
            .or_else(|| v.as_i64().map(|i| i as f64))
            .or_else(|| v.as_str()?.parse().ok())
    }

    let mut revenue_by_status = std::collections::HashMap::new();
    for row in &rows {
        let status = row
            .get("Order Status")
            .or_else(|| row.get("status"))
            .and_then(|v| v.as_str())
            .unwrap_or("?");
        let revenue = row
            .get("Revenue (USD)")
            .or_else(|| row.get("amount_usd"))
            .and_then(num)
            .or_else(|| row.values().find_map(num))
            .expect("numeric revenue present");
        revenue_by_status.insert(status.to_string(), revenue);
    }

    let completed = revenue_by_status.get("completed").copied().unwrap_or(0.0);
    assert!(
        (completed - 35.5).abs() < 1e-6,
        "expected completed revenue 35.5, got {completed}; rows={rows:?}"
    );
}
