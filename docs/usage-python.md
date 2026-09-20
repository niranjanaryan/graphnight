# Python Usage

Package: [`graphnight`](https://pypi.org/project/graphnight/) **1.0.0** — PyO3 bindings over local YAML storage and the SQL engine.

The Python client does **not** talk to a remote GraphNight GraphQL server. Server features (OIDC, HA Postgres metadata, Docker) live in the Rust binaries. For HTTP GraphQL from Python, call `/graphql` with any HTTP library and the headers from [usage-graphql.md](usage-graphql.md).

## Install

```bash
pip install graphnight==1.0.0
pip install 'graphnight[pandas]'   # optional
```

Multi-platform wheels are published from CI (Linux manylinux/musllinux, macOS, Windows).

## Scaffold Data

Use the CLI from this repo (or any built `graphnight` binary):

```bash
cargo run -p graphnight-cli -- init ./my-project
# then point the client at ./my-project/graphnight_data
```

Or use the checked-in `examples/data` directory.

## Client Examples

### Construct and List Models

```python
from graphnight import GraphNightClient

client = GraphNightClient(storage_path="./examples/data")
models = client.list_models()
print(models)
```

`url` is accepted for API compatibility but ignored today; storage is always local YAML under `storage_path` (default `./graphnight_data`).

### Generate SQL (dry-run)

```python
sql = client.generate_sql({
    "name": "orders",
    "measures": [
        {"formula": {"expression": "amount_usd", "label": "Revenue"}, "aggregation": "sum"},
        {"formula": {"expression": "*", "label": "Orders"}, "aggregation": "count"},
    ],
    "dimensions": [{"name": "status"}],
    "limit": 100,
})
print(sql)
```

### Execute a Query

Requires a reachable datasource (see `datasources.yaml` connection string):

```python
result = client.query({
    "name": "orders",
    "measures": [
        {"formula": {"expression": "amount_usd", "label": "Revenue"}, "aggregation": "sum"},
    ],
    "dimensions": [{"name": "status"}],
    "limit": 10,
})
# result: dict with data, columns, sql, execution_time_ms
print(result["sql"])
print(result["data"])
```

### pandas Helper

```python
# Requires pandas; currently returns the same structure as query()
# (DataFrame conversion may still be thin — check package version notes)
df_or_dict = client.query_df({
    "name": "orders",
    "measures": [
        {"formula": {"expression": "*", "label": "Orders"}, "aggregation": "count"},
    ],
    "dimensions": [{"name": "status"}],
})
```

## Advanced Usage

### Advanced Formulas

```python
# Window functions
result = client.query({
    "name": "orders",
    "measures": [
        {"formula": {"expression": "running_total(amount_usd:sum)", "label": "Running Revenue"}, "aggregation": "sum"},
        {"formula": {"expression": "pct_change(amount_usd:sum)", "label": "Revenue Pct Change"}, "aggregation": "sum"},
    ],
    "dimensions": [{"name": "created_at"}],
    "time_dimensions": [{"dimension": "created_at", "granularity": "MONTH"}],
    "filters": [{"field": "status", "operator": "eq", "value": "completed"}],
    "order": [{"field": "created_at", "descending": True}],
})

# Time shift (compare to previous period)
result = client.query({
    "name": "orders",
    "measures": [
        {"formula": {"expression": "time_shift(amount_usd:sum, 1, 'MONTH')", "label": "Prev Month Revenue"}, "aggregation": "sum"},
        {"formula": {"expression": "ratio(amount_usd:sum, time_shift(amount_usd:sum, 1, 'MONTH'))", "label": "Month-over-Month Ratio"}, "aggregation": "sum"},
    ],
    "time_dimensions": [{"dimension": "created_at", "granularity": "MONTH"}],
})

# Custom CASE expressions
result = client.query({
    "name": "orders",
    "measures": [
        {"formula": {"expression": "CASE WHEN tier = 'premium' THEN amount_usd * 1.1 ELSE amount_usd END", "label": "Adjusted Revenue"}, "aggregation": "sum"},
    ],
    "dimensions": [{"name": "tier"}],
})
```

### Time Dimensions & Granularities

```python
result = client.query({
    "name": "orders",
    "measures": [
        {"formula": {"expression": "amount_usd", "label": "Revenue"}, "aggregation": "sum"},
    ],
    "time_dimensions": [
        {"dimension": "created_at", "granularity": "HOUR"},
        {"dimension": "created_at", "granularity": "DAY"},
        {"dimension": "created_at", "granularity": "WEEK"},
        {"dimension": "created_at", "granularity": "MONTH"},
        {"dimension": "created_at", "granularity": "QUARTER"},
        {"dimension": "created_at", "granularity": "YEAR"},
    ],
    "filters": [
        {"field": "created_at", "operator": "gte", "value": "2024-01-01"},
        {"field": "created_at", "operator": "lte", "value": "2024-12-31"},
    ],
})
```

### Joins

```python
# Define joins in model YAML, then query uses them automatically
result = client.query({
    "name": "orders",
    "measures": [
        {"formula": {"expression": "amount_usd", "label": "Revenue"}, "aggregation": "sum"},
        {"formula": {"expression": "customers.lifetime_value", "label": "Customer LTV"}, "aggregation": "sum"},
    ],
    "dimensions": [
        {"name": "status"},
        {"name": "customers.tier"},
    ],
})
```

### Advanced Filters

```python
result = client.query({
    "name": "orders",
    "measures": [{"formula": {"expression": "amount_usd", "label": "Revenue"}, "aggregation": "sum"}],
    "dimensions": [{"name": "status"}],
    "filters": [
        {"field": "status", "operator": "eq", "value": "completed"},
        {"field": "store_id", "operator": "in", "value": [1, 2, 3]},
        {"field": "amount_usd", "operator": "gte", "value": 100},
        {"field": "created_at", "operator": "between", "value": ["2024-01-01", "2024-12-31"]},
        {"field": "customer_email", "operator": "like", "value": "%@company.com"},
        # OR conditions
        {"field": "status", "operator": "eq", "value": "pending", "or_condition": True},
    ],
    "order": [
        {"field": "amount_usd:sum", "descending": True},
        {"field": "status", "descending": False},
    ],
    "limit": 100,
    "offset": 0,
})
```

### Multi-Stage DAG Queries

```python
# Stage 1: Top 10 customers by revenue
stage1 = {
    "name": "orders",
    "measures": [{"formula": {"expression": "amount_usd", "label": "Revenue"}, "aggregation": "sum"}],
    "dimensions": [{"name": "customer_id"}],
    "order": [{"field": "amount_usd:sum", "descending": True}],
    "limit": 10,
}

# Stage 2: Drill down into top customers' orders (references stage1)
stage2 = {
    "name": "orders",
    "measures": [{"formula": {"expression": "amount_usd", "label": "Revenue"}, "aggregation": "sum"}],
    "dimensions": [{"name": "product_id"}],
    "filters": [{"field": "customer_id", "operator": "eq", "value": "{{stage1.customer_id}}"}],
    "stage_ref": "stage1",
}

result = client.multi_stage_query([stage1, stage2], dry_run=False)
# result.results[0] - top 10 customers
# result.results[1] - products for each top customer
```

### Dry Run (SQL Generation Only)

```python
sql = client.generate_sql({
    "name": "orders",
    "measures": [{"formula": {"expression": "amount_usd", "label": "Revenue"}, "aggregation": "sum"}],
    "dimensions": [{"name": "status"}],
    "filters": [{"field": "status", "operator": "eq", "value": "completed"}],
})
print(sql)
# SELECT status, SUM(amount_usd) AS "Revenue" FROM orders WHERE status = 'completed' GROUP BY status
```

### Model Management

```python
# Create a model
model = {
    "name": "orders",
    "datasource": "analytics",
    "description": "Order transactions",
    "measures": [
        {"formula": {"expression": "amount_usd", "label": "Revenue"}, "aggregation": "sum"},
        {"formula": {"expression": "*", "label": "Orders"}, "aggregation": "count"},
    ],
    "dimensions": [
        {"name": "status", "label": "Order Status"},
        {"name": "store_id", "label": "Store"},
    ],
    "time_dimensions": [
        {"dimension": "created_at", "granularity": "DAY"},
    ],
    "joins": [],
}
client.create_model(model)

# Update a model
updated_model = {**model, "description": "Updated description"}
client.update_model("orders", updated_model)

# Delete a model
client.delete_model("orders")
```

### Datasource Management

```python
# List datasources
datasources = client.list_datasources()

# Create a datasource
ds = {
    "name": "analytics",
    "driver": "postgres",
    "connection_string": "postgresql://user:pass@localhost/db",
    "description": "Analytics database",
    "models": ["orders"],
    "pool_size": 10,
}
client.create_datasource(ds)

# Get datasource
ds = client.get_datasource("analytics")
```

### DataFrame Support (pandas)

```python
# Requires: pip install 'graphnight[pandas]'
df = client.query_df({
    "name": "orders",
    "measures": [{"formula": {"expression": "amount_usd", "label": "Revenue"}, "aggregation": "sum"}],
    "dimensions": [{"name": "status"}],
})

# df is a pandas DataFrame
print(df.head())
print(df.dtypes)

# Export to CSV
df.to_csv("orders_by_status.csv", index=False)

# Export to Parquet
df.to_parquet("orders_by_status.parquet")
```

### Error Handling

```python
from graphnight import GraphNightClient, GraphNightError

client = GraphNightClient(storage_path="./graphnight_data")

try:
    result = client.query({
        "name": "orders",
        "measures": [{"formula": {"expression": "amount_usd", "label": "Revenue"}, "aggregation": "sum"}],
    })
except GraphNightError as e:
    if "Model not found" in str(e):
        print("Model doesn't exist")
    elif "Datasource not found" in str(e):
        print("Datasource not configured")
    elif "Query timeout" in str(e):
        print("Query took too long")
    else:
        print(f"Error: {e}")
```

### Async Usage (Future)

```python
# Async support is planned for future releases
# client = await GraphNightClient.create(storage_path="./graphnight_data")
# result = await client.query_async(query)
```

## Limits

- Local YAML only — no Postgres metadata backend in the Python client
- Unit tests for the extension are disabled in-tree (`unit_tests.rs.disabled`); treat the API as stable for list/generate_sql/query but verify against your environment
- Auth / PolicyEnforcer apply on the **server** GraphQL path, not inside `GraphNightClient`

## See Also

- `crates/graphnight-python/README.md`
- [CLI Usage](usage-cli.md)
- [GraphQL Usage](usage-graphql.md)
- [Elixir Usage](usage-elixir.md)