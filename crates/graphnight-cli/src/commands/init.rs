use anyhow::{bail, Context, Result};
use std::path::{Path, PathBuf};

const GRAPHNIGHT_TOML: &str = r#"[server]
host = "127.0.0.1"
port = 8080

[storage]
type = "yaml"
path = "./graphnight_data"

# WARNING: Alpha builds expose GraphQL with no authentication.
# Do not point this at production databases.
[datasources.demo]
driver = "sqlite"
connection_string = "sqlite:./graphnight_data/demo.db"
description = "Local demo SQLite database"
pool_size = 1
"#;

const MODELS_YAML: &str = r#"- name: orders
  datasource: demo
  description: Order facts for the demo warehouse
  measures:
    - formula:
        expression: amount_usd
        label: Revenue (USD)
        format: currency_usd
      aggregation: sum
    - formula:
        expression: "*"
        label: Orders
        format: null
      aggregation: count
  dimensions:
    - name: status
      label: Order Status
    - name: customer_id
      label: Customer
  time_dimensions:
    - dimension: created_at
      granularity: day
  joins:
    - name: customers
      model: customers
      join_type: left
      on:
        - [customer_id, id]
      alias: cust
  sql: null
  meta: {}

- name: customers
  datasource: demo
  description: Customer dimension
  measures:
    - formula:
        expression: "*"
        label: Customers
        format: null
      aggregation: count
  dimensions:
    - name: id
      label: Customer ID
    - name: segment
      label: Segment
  time_dimensions: []
  joins: []
  sql: null
  meta: {}
"#;

const DATASOURCES_YAML: &str = r#"- name: demo
  driver: sqlite
  connection_string: sqlite:./graphnight_data/demo.db
  description: Local demo SQLite database
  models:
    - orders
    - customers
  pool_size: 1
  meta: {}
"#;

const QUERY_JSON: &str = r#"{
  "name": "orders",
  "measures": [
    {
      "formula": {
        "expression": "amount_usd",
        "label": "Revenue (USD)"
      },
      "aggregation": "sum"
    },
    {
      "formula": {
        "expression": "*",
        "label": "Orders"
      },
      "aggregation": "count"
    }
  ],
  "dimensions": [
    {
      "name": "status",
      "label": "Order Status"
    }
  ],
  "time_dimensions": [],
  "filters": [],
  "order": [],
  "limit": 100
}
"#;

/// Scaffold a starter GraphNight project in `dir`.
pub fn init_project(dir: impl AsRef<Path>, force: bool) -> Result<PathBuf> {
    let dir = dir.as_ref();
    std::fs::create_dir_all(dir)
        .with_context(|| format!("failed to create directory {}", dir.display()))?;

    let data_dir = dir.join("graphnight_data");
    std::fs::create_dir_all(&data_dir)?;

    write_file(dir.join("graphnight.toml"), GRAPHNIGHT_TOML, force)?;
    write_file(data_dir.join("models.yaml"), MODELS_YAML, force)?;
    write_file(data_dir.join("datasources.yaml"), DATASOURCES_YAML, force)?;
    write_file(dir.join("query.json"), QUERY_JSON, force)?;

    Ok(dir.canonicalize().unwrap_or_else(|_| dir.to_path_buf()))
}

fn write_file(path: PathBuf, contents: &str, force: bool) -> Result<()> {
    if path.exists() && !force {
        bail!(
            "{} already exists (pass --force to overwrite)",
            path.display()
        );
    }
    std::fs::write(&path, contents)
        .with_context(|| format!("failed to write {}", path.display()))?;
    Ok(())
}
