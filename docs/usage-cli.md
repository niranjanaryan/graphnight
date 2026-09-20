# CLI Usage

Binary: `graphnight` (`crates/graphnight-cli`). Global flags:

```text
--config <path>          default: graphnight.toml
--storage-path <path>    default: ./graphnight_data
--log-level <level>      default: info
```

The CLI uses **YAML** storage under `--storage-path`. Postgres HA metadata is a server concern; for day-to-day local work, point `--storage-path` at a data directory.

## init

Scaffold config, sample models, datasources, and a query file:

```bash
graphnight init ./my-project
# overwrite existing files:
graphnight init ./my-project --force
```

Creates:

- `graphnight.toml`
- `graphnight_data/models.yaml`
- `graphnight_data/datasources.yaml`
- `query.json`

## Model Commands

### List Models

```bash
graphnight --storage-path ./examples/data model list
graphnight --storage-path ./examples/data model list --datasource demo
graphnight --storage-path ./examples/data model get orders
```

### Create / Delete from YAML Files

```bash
graphnight --storage-path ./graphnight_data model create --file ./model.yaml
graphnight --storage-path ./graphnight_data model delete orders
```

### Model YAML Example

```yaml
name: orders
datasource: analytics
description: "Order transactions"
measures:
  - formula:
      expression: amount_usd
      label: Revenue
    aggregation: sum
  - formula:
      expression: "*"
      label: Orders
    aggregation: count
dimensions:
  - name: status
    label: Order Status
  - name: store_id
    label: Store
time_dimensions:
  - dimension: created_at
    granularity: DAY
joins: []
```

## Query Commands

### Dry Run (SQL Generation Only)

Generate SQL only (no warehouse):

```bash
graphnight --storage-path ./examples/data \
  query dry-run --file ./examples/query.json
```

Equivalent shortcut:

```bash
graphnight --storage-path ./examples/data sql --file ./examples/query.json
```

### Query Run

Execute against the model's datasource (credentials must work):

```bash
graphnight --storage-path ./graphnight_data \
  query run --file ./query.json --format table

graphnight --storage-path ./graphnight_data \
  query run --file ./query.json --format json
```

### Query JSON Example

```json
{
  "name": "orders",
  "measures": [
    {"formula": {"expression": "amount_usd", "label": "Revenue"}, "aggregation": "sum"},
    {"formula": {"expression": "*", "label": "Orders"}, "aggregation": "count"}
  ],
  "dimensions": [{"name": "status"}],
  "time_dimensions": [{"dimension": "created_at", "granularity": "MONTH"}],
  "filters": [
    {"field": "status", "operator": "eq", "value": "completed"},
    {"field": "created_at", "operator": "gte", "value": "2024-01-01"}
  ],
  "order": [{"field": "amount_usd:sum", "descending": true}],
  "limit": 100
}
```

### Multi-Stage DAG Queries

```bash
# Create a multi-stage query file
cat > multi_stage.json << 'EOF'
{
  "stages": [
    {
      "name": "orders",
      "measures": [{"formula": {"expression": "amount_usd", "label": "Revenue"}, "aggregation": "sum"}],
      "dimensions": [{"name": "customer_id"}],
      "order": [{"field": "amount_usd:sum", "descending": true}],
      "limit": 10
    },
    {
      "name": "orders",
      "measures": [{"formula": {"expression": "amount_usd", "label": "Revenue"}, "aggregation": "sum"}],
      "dimensions": [{"name": "product_id"}],
      "filters": [{"field": "customer_id", "operator": "eq", "value": "{{stage1.customer_id}}"}],
      "stage_ref": "stage1"
    }
  ]
}
EOF

graphnight --storage-path ./graphnight_data query multi-stage --file multi_stage.json
```

## Datasource Commands

```bash
graphnight --storage-path ./examples/data datasource list
graphnight --storage-path ./examples/data datasource get demo
```

### Datasource YAML Example

```yaml
name: analytics
driver: postgres
connection_string: "postgresql://user:pass@localhost/db"
description: "Analytics database"
models:
  - orders
  - customers
pool_size: 10
```

### Create / Delete Datasources

```bash
graphnight --storage-path ./graphnight_data datasource create --file ./datasource.yaml
graphnight --storage-path ./graphnight_data datasource delete analytics
```

## Search

Full-text style model search (Tantivy on YAML/SQLite backends; Postgres metadata search is `ILIKE` on the server):

```bash
graphnight --storage-path ./examples/data search "revenue" --limit 10
graphnight --storage-path ./examples/data search "orders" --fields "name,description,measures"
```

## Memory

Agent memory helpers (`list` / `get` / `save` / `delete`) against the same storage backend. Useful for local experiments; not a substitute for durable product memory stores.

```bash
graphnight --storage-path ./graphnight_data memory save --key "my-key" --value "my-value"
graphnight --storage-path ./graphnight_data memory get --key "my-key"
graphnight --storage-path ./graphnight_data memory list
graphnight --storage-path ./graphnight_data memory delete --key "my-key"
```

## Serve

```bash
graphnight serve --host 0.0.0.0 --port 8080
```

Prints a placeholder and tells you to use **`graphnight-server`**. The CLI `serve` subcommand does **not** start the Axum GraphQL process.

Production / local API server:

```bash
graphnight-server \
  --config ./graphnight.toml \
  --storage-path ./graphnight_data \
  --host 127.0.0.1 \
  --port 8080
```

## Advanced Examples

### Complex Query with All Features

```bash
cat > complex_query.json << 'EOF'
{
  "name": "orders",
  "measures": [
    {"formula": {"expression": "amount_usd", "label": "Revenue"}, "aggregation": "sum"},
    {"formula": {"expression": "running_total(amount_usd:sum)", "label": "Running Revenue"}, "aggregation": "sum"},
    {"formula": {"expression": "pct_change(amount_usd:sum)", "label": "Pct Change"}, "aggregation": "sum"},
    {"formula": {"expression": "time_shift(amount_usd:sum, 1, 'MONTH')", "label": "Prev Month"}, "aggregation": "sum"}
  ],
  "dimensions": [
    {"name": "status"},
    {"name": "store_id"}
  ],
  "time_dimensions": [
    {"dimension": "created_at", "granularity": "MONTH"}
  ],
  "filters": [
    {"field": "status", "operator": "in", "value": ["completed", "shipped"]},
    {"field": "amount_usd", "operator": "gte", "value": 50},
    {"field": "created_at", "operator": "between", "value": ["2024-01-01", "2024-12-31"]}
  ],
  "order": [
    {"field": "created_at", "descending": false},
    {"field": "amount_usd:sum", "descending": true}
  ],
  "limit": 100,
  "offset": 0
}
EOF

# Dry run to see SQL
graphnight --storage-path ./graphnight_data sql --file complex_query.json

# Execute
graphnight --storage-path ./graphnight_data query run --file complex_query.json --format json | jq '.data[]'
```

### Using with Different Storage Backends

```bash
# YAML (default)
graphnight --storage-path ./graphnight_data model list

# SQLite
graphnight --storage-path ./graphnight_data --storage-type sqlite model list

# With Tantivy search enabled
graphnight --storage-path ./graphnight_data --storage-type tantivy search "revenue"
```

### Batch Operations

```bash
# Create multiple models from a directory
for f in ./models/*.yaml; do
  graphnight --storage-path ./graphnight_data model create --file "$f"
done

# Export all models
graphnight --storage-path ./graphnight_data model list --format json > all_models.json

# Export all datasources
graphnight --storage-path ./graphnight_data datasource list --format json > all_datasources.json
```

### Query with Different Output Formats

```bash
# Table (default, human-readable)
graphnight query run --file query.json --format table

# JSON (for scripting)
graphnight query run --file query.json --format json

# CSV (for spreadsheets)
graphnight query run --file query.json --format csv > results.csv

# Parquet (for data science)
graphnight query run --file query.json --format parquet > results.parquet
```

### Using Environment Variables

```bash
# Set storage path via env var
export GRAPHNIGHT_STORAGE_PATH=./graphnight_data
graphnight model list

# Set config path
export GRAPHNIGHT_CONFIG=./production.toml
graphnight query run --file query.json
```

### CI/CD Integration

```bash
# In CI pipeline - validate models
graphnight --storage-path ./graphnight_data model list --format json | jq '.[] | select(.measures | length == 0)' 
# Should return empty - validates all models have measures

# Validate SQL generation
graphnight --storage-path ./graphnight_data sql --file ./queries/*.json --validate-only
```

## See Also

- [Getting Started](getting-started.md)
- [GraphQL Usage](usage-graphql.md)
- [Python Usage](usage-python.md)
- [Elixir Usage](usage-elixir.md)