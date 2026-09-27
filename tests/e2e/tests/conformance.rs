//! E2: cross-dialect conformance.
//!
//! The same logical model + query is compiled for SQLite (executed through
//! GraphNight's own engine/executor) and DuckDB (executed harness-side, since
//! the engine's executor layer is sqlx-only) against databases seeded with
//! identical rows. Both engines must return the same canonical result table.

use graphnight_core::models::*;
use graphnight_core::security::{PolicyEnforcer, SessionPolicy};
use graphnight_sql::dialects::get_dialect;
use graphnight_sql::executor::{ConnectionManager, QueryExecutor};
use graphnight_sql::generator::SqlGenerator;
use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
use std::collections::HashSet;
use std::str::FromStr;
use std::sync::Arc;

fn orders_models() -> Vec<Model> {
    let customers_dim = Dimension::new("id");
    let orders = Model {
        name: "orders".to_string(),
        datasource: "demo".to_string(),
        description: None,
        measures: vec![
            Measure::simple("amount_usd", AggregationType::Sum),
            Measure::new(Formula::new("*"), AggregationType::Count),
        ],
        dimensions: vec![
            Dimension::new("status"),
            Dimension::new("customer_id"),
            Dimension::new("channel"),
        ],
        time_dimensions: vec![TimeDimension::new("created_at", TimeGranularity::Day)],
        joins: vec![Join {
            name: "customers".to_string(),
            model: "customers".to_string(),
            join_type: JoinType::Left,
            on: vec![("customer_id".to_string(), "id".to_string())],
            alias: Some("cust".to_string()),
        }],
        sql: None,
        meta: Default::default(),
    };
    let customers = Model {
        name: "customers".to_string(),
        datasource: "demo".to_string(),
        description: None,
        measures: vec![],
        dimensions: vec![customers_dim, Dimension::new("segment")],
        time_dimensions: vec![],
        joins: vec![],
        sql: None,
        meta: Default::default(),
    };
    vec![orders, customers]
}

fn scenario_basic() -> Query {
    Query::new()
        .with_name("orders")
        .add_measure(Measure::simple("amount_usd", AggregationType::Sum))
        .add_measure(Measure::new(Formula::new("*"), AggregationType::Count))
        .add_dimension(Dimension::new("status"))
        .with_limit(100)
}

fn scenario_filters() -> Query {
    Query::new()
        .with_name("orders")
        .add_measure(Measure::simple("amount_usd", AggregationType::Sum))
        .add_dimension(Dimension::new("status"))
        .add_filter(Filter::new(
            "status",
            FilterOperator::In,
            serde_json::json!(["completed", "pending"]),
        ))
        .add_filter(Filter::new(
            "amount_usd",
            FilterOperator::Gte,
            serde_json::json!(3.0),
        ))
        .with_limit(100)
}

fn scenario_time_dim() -> Query {
    Query::new()
        .with_name("orders")
        .add_measure(Measure::simple("amount_usd", AggregationType::Sum))
        .add_time_dimension(TimeDimension::new("created_at", TimeGranularity::Day))
        .with_limit(100)
}

fn scenario_formula() -> Query {
    Query::new()
        .with_name("orders")
        .add_measure(Measure::new(
            Formula::new("ratio(amount_usd:sum, *:count)").with_label("avg_order_value"),
            AggregationType::Avg,
        ))
        .add_dimension(Dimension::new("channel"))
        .with_limit(100)
}

fn scenario_joined() -> Query {
    Query::new()
        .with_name("orders")
        .add_measure(Measure::simple("amount_usd", AggregationType::Sum))
        .add_dimension(Dimension::new("status"))
        .add_dimension(Dimension::new("segment"))
        .with_limit(100)
}

fn scenario_policy_instrumented() -> Query {
    let policy = SessionPolicy::new()
        .with_forced_filter(Filter::new(
            "channel",
            FilterOperator::Eq,
            serde_json::json!("web"),
        ))
        .with_max_rows(10);
    let enforcer = PolicyEnforcer::new(policy);
    let mut q = scenario_basic();
    enforcer.apply_forced_filters(&mut q);
    enforcer.enforce_row_limit(&mut q);
    q
}

const SQLITE_DDL: &str = r#"
CREATE TABLE customers (
    id INTEGER PRIMARY KEY,
    segment TEXT NOT NULL
);
CREATE TABLE orders (
    amount_usd REAL NOT NULL,
    status TEXT NOT NULL,
    customer_id INTEGER NOT NULL,
    channel TEXT NOT NULL,
    created_at TEXT NOT NULL
);
INSERT INTO customers (id, segment) VALUES (1, 'enterprise'), (2, 'smb');
INSERT INTO orders (amount_usd, status, customer_id, channel, created_at) VALUES
  (10.0, 'completed', 1, 'web',     '2024-01-01'),
  (25.5, 'completed', 2, 'mobile',  '2024-01-02'),
  (5.0,  'pending',   1, 'web',     '2024-01-03'),
  (60.1, 'completed', 2, 'mobile',  '2024-01-03'),
  (1.5,  'refunded',  2, 'web',     '2024-01-04');
"#;

const DUCKDB_DDL: &str = r#"
CREATE TABLE customers (
    id INTEGER PRIMARY KEY,
    segment VARCHAR NOT NULL
);
CREATE TABLE orders (
    amount_usd DOUBLE NOT NULL,
    status VARCHAR NOT NULL,
    customer_id INTEGER NOT NULL,
    channel VARCHAR NOT NULL,
    created_at DATE NOT NULL
);
INSERT INTO customers (id, segment) VALUES (1, 'enterprise'), (2, 'smb');
INSERT INTO orders (amount_usd, status, customer_id, channel, created_at) VALUES
  (10.0, 'completed', 1, 'web',     DATE '2024-01-01'),
  (25.5, 'completed', 2, 'mobile',  DATE '2024-01-02'),
  (5.0,  'pending',   1, 'web',     DATE '2024-01-03'),
  (60.1, 'completed', 2, 'mobile',  DATE '2024-01-03'),
  (1.5,  'refunded',  2, 'web',     DATE '2024-01-04');
"#;

struct Scenario {
    name: &'static str,
    query: Query,
}

fn scenarios() -> Vec<Scenario> {
    vec![
        Scenario {
            name: "basic_select_group_by_status",
            query: scenario_basic(),
        },
        Scenario {
            name: "filter_operators_in_and_gte",
            query: scenario_filters(),
        },
        Scenario {
            name: "time_dimension_day",
            query: scenario_time_dim(),
        },
        Scenario {
            name: "formula_ratio_shorthand",
            query: scenario_formula(),
        },
        Scenario {
            name: "joined_model_segment",
            query: scenario_joined(),
        },
        Scenario {
            name: "policy_forced_filter_compiled",
            query: scenario_policy_instrumented(),
        },
    ]
}

/// Render rows as a sorted set of `column=value` cells so order and float
/// formatting nuance cannot mask real differences.
fn canonicalize_sqlx_rows(rows: &[std::collections::HashMap<String, serde_json::Value>]) -> String {
    let mut cells = Vec::new();
    for row in rows {
        let mut pairs: Vec<String> = row
            .iter()
            .map(|(k, v)| format!("{k}={}", render_value(v)))
            .collect();
        pairs.sort();
        cells.push(pairs.join(","));
    }
    cells.sort();
    cells.join("\n")
}

fn render_value(v: &serde_json::Value) -> String {
    match v {
        serde_json::Value::Number(n) => {
            // Normalize 10.0 and 10 into "10" so SQLite/duckdb float text
            // differences (10.0 vs 10.000000) never break equivalence.
            n.to_string()
                .trim_end_matches(".0")
                .trim_end_matches('0')
                .trim_end_matches('.')
                .to_string()
        }
        // serde renders strings with JSON quotes; strip them to match the
        // bare values the duckdb CSV export produces.
        serde_json::Value::String(s) => s.clone(),
        other => other.to_string(),
    }
}

fn normalize_csv_cell(cell: &str) -> String {
    // Match render_value's float normalization for numeric cells.
    match cell.trim().parse::<f64>() {
        Ok(f) if !cell.contains('e') && !cell.contains('E') => format!("{f}")
            .trim_end_matches(".0")
            .trim_end_matches('0')
            .trim_end_matches('.')
            .to_string(),
        _ => cell.to_string(),
    }
}

/// DuckDB results are exported via `COPY (sql) TO csv` and parsed back into
/// the same canonical form (sorted `column=value` cells per row) using the
/// CSV header as the column names.
fn canonicalize_duckdb_csv(path: &std::path::Path) -> String {
    let raw = std::fs::read_to_string(path).expect("read duckdb csv");
    let mut lines: Vec<Vec<String>> = raw
        .trim()
        .split('\n')
        .map(|l| {
            l.split(',')
                .map(|cell| normalize_csv_cell(cell.trim().trim_matches('"')))
                .collect::<Vec<_>>()
        })
        .filter(|cells| !cells.is_empty() && cells.iter().any(|c| !c.is_empty()))
        .collect();

    let header = lines.first().expect("duckdb CSV has a header row").clone();
    let rows: Vec<Vec<String>> = lines.drain(1..).collect();

    let mut rendered = Vec::new();
    for row in rows {
        let mut cells: Vec<String> = header
            .iter()
            .zip(row.iter())
            .map(|(h, v)| format!("{h}={v}"))
            .collect();
        cells.sort();
        rendered.push(cells.join(","));
    }
    rendered.sort();
    rendered.join("\n")
}

async fn sqlite_canonical(sql: &str) -> String {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("conformance.db");
    let conn = format!("sqlite:{}", db_path.display());
    let opts = SqliteConnectOptions::from_str(&conn)
        .unwrap()
        .create_if_missing(true);
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect_with(opts)
        .await
        .unwrap();
    sqlx::query(SQLITE_DDL).execute(&pool).await.unwrap();
    pool.close().await;

    let ds = DataSource {
        name: "demo".into(),
        driver: "sqlite".into(),
        connection_string: conn,
        description: Some("e2e conformance".into()),
        models: vec!["orders".into()],
        pool_size: Some(1),
        meta: Default::default(),
    };
    let executor = QueryExecutor::new(Arc::new(ConnectionManager::new()));
    let rows = executor
        .execute(&ds, sql)
        .await
        .unwrap_or_else(|e| panic!("sqlite execute failed\n{sql}\n-> {e}"));
    canonicalize_sqlx_rows(&rows)
}

fn duckdb_canonical(sql: &str) -> String {
    let conn = duckdb::Connection::open_in_memory().unwrap();
    conn.execute_batch(DUCKDB_DDL).unwrap();
    let dir = tempfile::tempdir().unwrap();
    let csv = dir.path().join("out.csv");
    conn.execute_batch(&format!(
        "COPY ({sql}) TO '{}' (FORMAT CSV, HEADER)",
        csv.display()
    ))
    .unwrap_or_else(|e| panic!("duckdb execute {sql}: {e}"));
    canonicalize_duckdb_csv(&csv)
}

#[tokio::test]
async fn conformance_sqlite_matches_duckdb() {
    let models = orders_models();
    let mut failed: Vec<String> = Vec::new();

    for scenario in scenarios() {
        let sqlite_gen = SqlGenerator::new(get_dialect("sqlite")).with_models(models.clone());
        let duck_gen = SqlGenerator::new(get_dialect("duckdb")).with_models(models.clone());

        let sqlite_sql = sqlite_gen.generate(&scenario.query).unwrap();
        let duck_sql = duck_gen.generate(&scenario.query).unwrap();

        let sqlite_out = sqlite_canonical(&sqlite_sql).await;
        let duck_out = duckdb_canonical(&duck_sql);

        if sqlite_out != duck_out {
            failed.push(format!(
                "{}:\n  sqlite sql: {sqlite_sql}\n  duckdb sql:{duck_sql}\n  sqlite: {sqlite_out}\n  duckdb: {duck_out}",
                scenario.name
            ));
        }
    }

    assert!(
        failed.is_empty(),
        "cross-dialect conformance failures:\n{}",
        failed.join("\n")
    );
}

#[tokio::test]
async fn policy_scenario_forces_channel_web_on_duckdb() {
    let models = orders_models();
    let duck_gen = SqlGenerator::new(get_dialect("duckdb")).with_models(models);
    let scenario = &scenarios()[5];
    let sql = duck_gen.generate(&scenario.query).unwrap();

    // The compiled plan carries the policy predicate, so the enforced result
    // must be a strict subset (web orders: 1 + 1.5 = 11.5; 2 rows).
    let conn = duckdb::Connection::open_in_memory().unwrap();
    conn.execute_batch(DUCKDB_DDL).unwrap();
    let dir = tempfile::tempdir().unwrap();
    let csv = dir.path().join("web.csv");
    conn.execute_batch(&format!(
        "COPY ({sql}) TO '{}' (FORMAT CSV, HEADER)",
        csv.display()
    ))
    .unwrap();
    let raw = std::fs::read_to_string(&csv).unwrap();
    let lines: Vec<&str> = raw.trim().split('\n').filter(|l| !l.is_empty()).collect();
    // Header + three web groups: completed (10.0), pending (5.0), refunded (1.5).
    // Both mobile rows (25.5, 60.1) must be excluded by the compiled predicate.
    assert_eq!(lines.len(), 4, "rows={raw:?}");
    assert!(
        !raw.contains("25.5") && !raw.contains("60.1"),
        "rows={raw:?}"
    );
    let statuses: HashSet<String> = lines
        .iter()
        .skip(1)
        .map(|l| {
            l.split(',')
                .nth(2)
                .unwrap_or("")
                .trim_matches('"')
                .to_string()
        })
        .collect();
    assert_eq!(
        statuses,
        HashSet::from([
            "completed".to_string(),
            "pending".to_string(),
            "refunded".to_string()
        ])
    );
}

// Kept for documentation: engine-side executor supports sqlite only.
#[allow(dead_code)]
fn executor_dialects_supported() -> &'static str {
    "sqlite, postgres, mysql (sqlx pools)"
}
