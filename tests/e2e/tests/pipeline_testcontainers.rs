//! End-to-end against real Postgres (and optional MySQL) via testcontainers.
//!
//! Runs in CI when Docker is available (GitHub-hosted runners). Locally without
//! Docker, set `GRAPHNIGHT_SKIP_TESTCONTAINERS=1`, or the tests soft-skip when
//! the Docker daemon is unreachable.

use graphnight_core::models::{DataSource, Query};
use graphnight_sql::dialects::{MySqlDialect, PostgresDialect};
use graphnight_sql::executor::{ConnectionManager, QueryExecutor};
use graphnight_sql::generator::SqlGenerator;
use graphnight_storage::{StorageBackend, YamlStorage};
use std::collections::HashMap;
use std::path::PathBuf;
use std::process::Command;
use std::sync::Arc;
use testcontainers::runners::AsyncRunner;
use testcontainers::ImageExt;
use testcontainers_modules::mysql::Mysql;
use testcontainers_modules::postgres::Postgres;

fn repo_examples_data() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../examples/data")
}

fn examples_query_json() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../examples/query.json")
}

fn skip_testcontainers(label: &str) -> bool {
    match std::env::var("GRAPHNIGHT_SKIP_TESTCONTAINERS") {
        Ok(v) if matches!(v.as_str(), "1" | "true" | "TRUE" | "yes" | "YES") => {
            eprintln!("{label}: skipped (GRAPHNIGHT_SKIP_TESTCONTAINERS={v})");
            return true;
        }
        _ => {}
    }

    let docker_ok = Command::new("docker")
        .arg("info")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false);

    if !docker_ok {
        eprintln!(
            "{label}: skipped (Docker daemon unavailable; set GRAPHNIGHT_SKIP_TESTCONTAINERS=1 to silence)"
        );
        return true;
    }
    false
}

fn num(v: &serde_json::Value) -> Option<f64> {
    v.as_f64()
        .or_else(|| v.as_i64().map(|i| i as f64))
        .or_else(|| v.as_str()?.parse().ok())
}

fn assert_completed_revenue(rows: &[HashMap<String, serde_json::Value>], sql: &str) {
    assert!(
        !rows.is_empty(),
        "expected aggregated rows from demo orders; sql={sql}"
    );
    assert_eq!(rows.len(), 2, "rows={rows:?} sql={sql}");

    let mut revenue_by_status = HashMap::new();
    for row in rows {
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

async fn load_models_and_query() -> (Vec<graphnight_core::models::Model>, Query) {
    let data_dir = repo_examples_data();
    let storage = YamlStorage::new(&data_dir).unwrap();
    storage.load().await.unwrap();
    let models = storage.list_models(None).await.unwrap();
    let query: Query =
        serde_json::from_str(&std::fs::read_to_string(examples_query_json()).unwrap()).unwrap();
    (models, query)
}

const PG_SEED_SQL: &str = r#"
CREATE TABLE customers (
    id INTEGER PRIMARY KEY,
    segment TEXT NOT NULL
);
CREATE TABLE orders (
    amount_usd DOUBLE PRECISION NOT NULL,
    status TEXT NOT NULL,
    customer_id INTEGER NOT NULL,
    created_at TIMESTAMP NOT NULL
);
INSERT INTO customers (id, segment) VALUES (1, 'enterprise'), (2, 'smb');
INSERT INTO orders (amount_usd, status, customer_id, created_at) VALUES
  (10.0, 'completed', 1, '2024-01-01'),
  (25.5, 'completed', 2, '2024-01-02'),
  (5.0,  'pending',   1, '2024-01-03');
"#;

const MYSQL_SEED_SQL: &str = r#"
CREATE TABLE customers (
    id INT PRIMARY KEY,
    segment VARCHAR(64) NOT NULL
);
CREATE TABLE orders (
    amount_usd DOUBLE NOT NULL,
    status VARCHAR(64) NOT NULL,
    customer_id INT NOT NULL,
    created_at DATETIME NOT NULL
);
INSERT INTO customers (id, segment) VALUES (1, 'enterprise'), (2, 'smb');
INSERT INTO orders (amount_usd, status, customer_id, created_at) VALUES
  (10.0, 'completed', 1, '2024-01-01 00:00:00'),
  (25.5, 'completed', 2, '2024-01-02 00:00:00'),
  (5.0,  'pending',   1, '2024-01-03 00:00:00');
"#;

#[tokio::test]
async fn examples_query_executes_against_postgres_testcontainer() {
    if skip_testcontainers("postgres_testcontainers") {
        return;
    }

    let (models, query) = load_models_and_query().await;
    let generator = SqlGenerator::new(Box::new(PostgresDialect)).with_models(models);
    let sql = generator.generate(&query).unwrap();
    assert!(sql.contains("SUM"), "sql={sql}");
    assert!(sql.contains("GROUP BY"), "sql={sql}");

    // Prefer alpine tag for smaller pulls (module default is 11-alpine).
    let container = Postgres::default()
        .with_tag("16-alpine")
        .start()
        .await
        .expect("start Postgres testcontainer");
    let host = container.get_host().await.expect("postgres host");
    let port = container
        .get_host_port_ipv4(5432)
        .await
        .expect("postgres mapped port");
    let conn = format!("postgres://postgres:postgres@{host}:{port}/postgres");

    let pool = sqlx::postgres::PgPoolOptions::new()
        .max_connections(2)
        .connect(&conn)
        .await
        .expect("connect to testcontainer Postgres");
    for stmt in PG_SEED_SQL.split(';').map(str::trim).filter(|s| !s.is_empty()) {
        sqlx::query(stmt)
            .execute(&pool)
            .await
            .unwrap_or_else(|e| panic!("seed failed for `{stmt}`: {e}"));
    }
    pool.close().await;

    let ds = DataSource {
        name: "demo-pg".into(),
        driver: "postgres".into(),
        connection_string: conn,
        description: Some("e2e postgres testcontainer".into()),
        models: vec!["orders".into(), "customers".into()],
        pool_size: Some(2),
        meta: Default::default(),
    };

    let executor = QueryExecutor::new(Arc::new(ConnectionManager::new()));
    let rows = executor.execute(&ds, &sql).await.unwrap();
    assert_completed_revenue(&rows, &sql);
}

/// MySQL coverage mirrors Postgres. Optional: larger image; enable with `--ignored`.
/// Soft-skips without Docker when not ignored.
#[tokio::test]
#[ignore = "optional MySQL testcontainers; run with --ignored when Docker has space"]
async fn examples_query_executes_against_mysql_testcontainer() {
    if skip_testcontainers("mysql_testcontainers") {
        return;
    }

    let (models, query) = load_models_and_query().await;
    let generator = SqlGenerator::new(Box::new(MySqlDialect)).with_models(models);
    let sql = generator.generate(&query).unwrap();
    assert!(sql.contains("SUM"), "sql={sql}");
    assert!(sql.contains("GROUP BY"), "sql={sql}");

    let container = Mysql::default()
        .with_tag("8.0")
        .start()
        .await
        .expect("start MySQL testcontainer");
    let host = container.get_host().await.expect("mysql host");
    let port = container
        .get_host_port_ipv4(3306)
        .await
        .expect("mysql mapped port");
    // Default image: root with empty password, database `test`.
    let conn = format!("mysql://root:@{host}:{port}/test");

    let pool = sqlx::mysql::MySqlPoolOptions::new()
        .max_connections(2)
        .connect(&conn)
        .await
        .expect("connect to testcontainer MySQL");
    for stmt in MYSQL_SEED_SQL
        .split(';')
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        sqlx::query(stmt)
            .execute(&pool)
            .await
            .unwrap_or_else(|e| panic!("seed failed for `{stmt}`: {e}"));
    }
    pool.close().await;

    let ds = DataSource {
        name: "demo-mysql".into(),
        driver: "mysql".into(),
        connection_string: conn,
        description: Some("e2e mysql testcontainer".into()),
        models: vec!["orders".into(), "customers".into()],
        pool_size: Some(2),
        meta: Default::default(),
    };

    let executor = QueryExecutor::new(Arc::new(ConnectionManager::new()));
    let rows = executor.execute(&ds, &sql).await.unwrap();
    assert_completed_revenue(&rows, &sql);
}
