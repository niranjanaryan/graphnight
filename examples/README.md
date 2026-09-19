# GraphNight examples

Sample config, models, CLI query payloads, GraphQL documents, auth curls, and HA notes.

## Index

| Path | Description |
|------|-------------|
| [`graphnight.toml`](graphnight.toml) | Default YAML metadata server config |
| [`graphnight.postgres.toml`](graphnight.postgres.toml) | Shared Postgres metadata (HA) |
| [`data/models.yaml`](data/models.yaml) | Demo `orders` / `customers` models |
| [`data/datasources.yaml`](data/datasources.yaml) | Demo SQLite datasource |
| [`query.json`](query.json) | Basic CLI dry-run payload |
| [`query.graphql`](query.graphql) | Short GraphQL dry-run + list models |
| [`queries/`](queries/) | Extra CLI dry-run JSON samples |
| [`graphql/`](graphql/) | GraphQL ops + curl one-liners |
| [`auth/`](auth/) | API key / OIDC curls + policy notes |
| [`ha/`](ha/) | Postgres metadata + Compose `--profile ha` |

## CLI dry-run

```bash
cargo run -p graphnight-cli -- \
  --storage-path ./examples/data \
  model list

cargo run -p graphnight-cli -- \
  --storage-path ./examples/data \
  query dry-run --file ./examples/query.json

cargo run -p graphnight-cli -- \
  --storage-path ./examples/data \
  query dry-run --file ./examples/queries/revenue_by_status.json

cargo run -p graphnight-cli -- \
  --storage-path ./examples/data \
  query dry-run --file ./examples/queries/ratio_formula.json

cargo run -p graphnight-cli -- \
  --storage-path ./examples/data \
  query dry-run --file ./examples/queries/with_join.json
```

`with_join.json` uses the `orders` model, which declares a left join to `customers` — generated SQL includes `LEFT JOIN "customers"`.

## Server

```bash
cargo run -p graphnight-server -- \
  --config examples/graphnight.toml \
  --storage-path examples/data \
  --host 127.0.0.1 --port 8080
```

Then see [`graphql/README.md`](graphql/README.md) and [`auth/README.md`](auth/README.md).
