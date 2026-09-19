# Python usage

Package: [`graphnight`](https://pypi.org/project/graphnight/) **1.0.0** — PyO3 bindings over local YAML storage and the SQL engine.

The Python client does **not** talk to a remote GraphNight GraphQL server. Server features (OIDC, HA Postgres metadata, Docker) live in the Rust binaries. For HTTP GraphQL from Python, call `/graphql` with any HTTP library and the headers from [usage-graphql.md](usage-graphql.md).

## Install

```bash
pip install graphnight==1.0.0
pip install 'graphnight[pandas]'   # optional
```

Multi-platform wheels are published from CI (Linux manylinux/musllinux, macOS, Windows).

## Scaffold data

Use the CLI from this repo (or any built `graphnight` binary):

```bash
cargo run -p graphnight-cli -- init ./my-project
# then point the client at ./my-project/graphnight_data
```

Or use the checked-in `examples/data` directory.

## Client examples

### Construct and list models

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

### Execute a query

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

### pandas helper

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

## Limits

- Local YAML only — no Postgres metadata backend in the Python client
- Unit tests for the extension are disabled in-tree (`unit_tests.rs.disabled`); treat the API as stable for list/generate_sql/query but verify against your environment
- Auth / PolicyEnforcer apply on the **server** GraphQL path, not inside `GraphNightClient`

See also `crates/graphnight-python/README.md`.
