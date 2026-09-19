# graphnight

Python bindings for [GraphNight](https://github.com/niranjanaryan/graphnight) — an embeddable semantic layer for AI agents and humans.

**Status: 1.0.0.** Rust-backed local YAML client (`GraphNightClient`). Server features (OIDC, HA metadata, Docker) live in the main repo binaries.

## Install

From PyPI (wheels):

```bash
pip install graphnight==1.0.0
```

From a source checkout (develop / contribute):

```bash
cd crates/graphnight-python
python3.11 -m venv .venv && source .venv/bin/activate
pip install maturin pytest
# Reuse an existing Cargo target to save disk:
#   export CARGO_TARGET_DIR=../../target
maturin develop
pip install -e '.[dev]'   # optional: pytest, ruff, mypy
```

## Quick start

```python
from graphnight import GraphNightClient

client = GraphNightClient(storage_path="./graphnight_data")

# Datasource + model
client.create_datasource({
    "name": "demo",
    "driver": "postgres",
    "connection_string": "postgresql://localhost/demo",
})
client.create_model({
    "name": "orders",
    "datasource": "demo",
    "description": "Order facts",
    "measures": [
        {"formula": {"expression": "amount_usd", "label": "Revenue"}, "aggregation": "sum"},
        {"formula": "*", "aggregation": "count"},
    ],
    "dimensions": ["status", {"name": "customer_id", "label": "Customer"}],
    "time_dimensions": [{"dimension": "created_at", "granularity": "day"}],
})

print(client.list_models())
print(client.list_datasources())
print(client.get_model("orders"))
```

Scaffold data with the CLI instead:

```bash
cargo install --path crates/graphnight-cli
graphnight init ./my-project
```

## Query helpers

All client methods are **synchronous** (they drive an internal Tokio runtime).

```python
query = {
    "name": "orders",
    "measures": [
        {"formula": {"expression": "amount_usd"}, "aggregation": "sum"},
    ],
    "dimensions": [{"name": "status"}],
    "filters": [{"field": "status", "operator": "eq", "value": "completed"}],
    "limit": 100,
}

# SQL only — no database connection needed
sql = client.generate_sql(query)
dry = client.dry_run_query(query)  # {"sql": "...", "name": "orders"}

# Execute against the datasource connection_string (needs a live DB)
# result = client.query(query)  # {"data", "columns", "sql", "execution_time_ms"}
```

### API surface

| Method | Purpose |
|--------|---------|
| `GraphNightClient(url=None, storage_path=None)` | Local YAML client (`url` reserved / ignored) |
| `list_models(datasource=None)` | List model summaries |
| `get_model(name, datasource=None)` | Fetch one model or `None` |
| `create_model(dict)` | Persist a model |
| `list_datasources()` | List datasources |
| `create_datasource(dict)` | Persist a datasource |
| `generate_sql(query)` | Return SQL string |
| `dry_run_query(query)` | Return `{"sql", "name?"}` without executing |
| `query(query)` / `query_df(query)` | Execute SQL via sqlx (DataFrame TBD) |
| `search(q, limit=None)` | Search models |
| `save_memory(...)` / `list_memories(...)` / `forget_memory(id)` | Agent learnings |

Formula fields accept a string (`"amount_usd"`) or a dict
`{"expression", "label?", "format?"}`. Dimensions accept a string or
`{"name", "label?"}`.

## Tests

```bash
cd crates/graphnight-python
maturin develop
pytest
```

Tests use tempfile storage and do not need a warehouse. See also
`examples/python/` for runnable scripts.

## CI note

Main CI excludes this crate (`cargo test --workspace --exclude graphnight-python`)
because PyO3 needs a Python interpreter and maturin. Run the commands above locally
or in a dedicated job when changing bindings. Multi-platform wheels are built by
`.github/workflows/wheels.yml`.

## Links

- Examples: [`examples/python/`](../../examples/python/)
- Docs: https://github.com/niranjanaryan/graphnight/tree/main/docs
- Changelog: https://github.com/niranjanaryan/graphnight/blob/main/CHANGELOG.md
- Security: https://github.com/niranjanaryan/graphnight/blob/main/SECURITY.md

## License

Apache-2.0
