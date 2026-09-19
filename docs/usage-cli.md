# CLI usage

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

## model list / get

```bash
graphnight --storage-path ./examples/data model list
graphnight --storage-path ./examples/data model list --datasource demo
graphnight --storage-path ./examples/data model get orders
```

Create / delete from YAML files:

```bash
graphnight --storage-path ./graphnight_data model create --file ./model.yaml
graphnight --storage-path ./graphnight_data model delete orders
```

## query dry-run

Generate SQL only (no warehouse):

```bash
graphnight --storage-path ./examples/data \
  query dry-run --file ./examples/query.json
```

Equivalent shortcut:

```bash
graphnight --storage-path ./examples/data sql --file ./examples/query.json
```

## query run

Execute against the model's datasource (credentials must work):

```bash
graphnight --storage-path ./graphnight_data \
  query run --file ./query.json --format table

graphnight --storage-path ./graphnight_data \
  query run --file ./query.json --format json
```

## datasources

```bash
graphnight --storage-path ./examples/data datasource list
graphnight --storage-path ./examples/data datasource get demo
```

## search

Full-text style model search (Tantivy on YAML/SQLite backends; Postgres metadata search is `ILIKE` on the server):

```bash
graphnight --storage-path ./examples/data search "revenue" --limit 10
```

## memory

Agent memory helpers (`list` / `get` / `save` / `delete`) against the same storage backend. Useful for local experiments; not a substitute for durable product memory stores.

## serve tip

```bash
graphnight serve --host 0.0.0.0 --port 8080
```

prints a placeholder and tells you to use **`graphnight-server`**. The CLI `serve` subcommand does **not** start the Axum GraphQL process.

Production / local API server:

```bash
graphnight-server \
  --config ./graphnight.toml \
  --storage-path ./graphnight_data \
  --host 127.0.0.1 \
  --port 8080
```

See [getting-started.md](getting-started.md) and [usage-graphql.md](usage-graphql.md).
