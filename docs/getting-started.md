# Getting started (v1.0)

GraphNight 1.0: define semantic models in YAML (or Postgres metadata), generate dialect SQL, and query via CLI, GraphQL, or Python.

## Requirements

- Rust 1.75+ (stable) to build from source
- Optional: a SQL warehouse for non-`dryRun` queries
- Optional: Docker for the packaged server
- Optional: Python 3.9+ for `pip install graphnight`

## 1. Install / build

### From source

```bash
cargo build --release -p graphnight-cli -p graphnight-server
```

Binaries land in `target/release/graphnight` and `target/release/graphnight-server`.

### Python client (PyPI)

```bash
pip install graphnight==1.0.0
# optional pandas helper surface
pip install 'graphnight[pandas]'
```

Bindings use local YAML storage; they do not replace the GraphQL server. See [usage-python.md](usage-python.md).

### Docker image

```bash
docker compose up --build
# http://127.0.0.1:8080/health
```

See [deploy.md](deploy.md).

## 2. Scaffold a project

```bash
cargo run -p graphnight-cli -- init ./my-project
# or, after install: graphnight init ./my-project
cd my-project
```

Writes `graphnight.toml`, `graphnight_data/models.yaml`, `graphnight_data/datasources.yaml`, and `query.json`.

Use `--force` to overwrite existing files.

## 3. Dry-run a query (no database required)

```bash
cargo run -p graphnight-cli -- \
  --storage-path ./graphnight_data \
  query dry-run --file ./query.json
```

Or with the release binary from the project directory:

```bash
../target/release/graphnight --storage-path ./graphnight_data query dry-run --file ./query.json
```

You should see dialect SQL (`SUM` / `COUNT` / `GROUP BY`, plus joins when models declare them).

Checked-in examples:

```bash
cargo run -p graphnight-cli -- \
  --storage-path ./examples/data \
  query dry-run --file ./examples/query.json
```

## 4. List models

```bash
cargo run -p graphnight-cli -- --storage-path ./graphnight_data model list
```

More CLI commands: [usage-cli.md](usage-cli.md).

## 5. Start the GraphQL server

```bash
./target/release/graphnight-server \
  --config ./graphnight.toml \
  --storage-path ./graphnight_data \
  --host 127.0.0.1 \
  --port 8080
```

- Health: `GET http://127.0.0.1:8080/health` (JSON; `503` if metadata storage is unreachable)
- GraphQL / GraphiQL: `http://127.0.0.1:8080/graphql`
- Metrics: `GET http://127.0.0.1:8080/metrics`

Auth is **open** until you set API keys or OIDC (a warning is logged). For local demos you can set `GRAPHNIGHT_DEV_OPEN=1` to silence it. For shared deployments, configure auth — see [auth.md](auth.md).

Minimal authenticated start:

```bash
export GRAPHNIGHT_API_KEYS='alice:secret1'
export GRAPHNIGHT_ADMIN_KEYS='admin:adminsecret'
./target/release/graphnight-server \
  --config ./graphnight.toml \
  --storage-path ./graphnight_data \
  --host 127.0.0.1 \
  --port 8080
```

curl / GraphiQL examples: [usage-graphql.md](usage-graphql.md).

### Shared Postgres metadata (HA)

Default storage is YAML on disk. For multi-replica servers:

```toml
[storage]
type = "postgres"
path = "env:GRAPHNIGHT_METADATA_DATABASE_URL"
```

```bash
export GRAPHNIGHT_METADATA_DATABASE_URL='postgresql://graphnight:graphnight@127.0.0.1:5433/graphnight_meta'
docker compose --profile ha up -d metadata-db
cargo run -p graphnight-server -- --config examples/graphnight.postgres.toml --host 127.0.0.1
```

Tables are created automatically (`CREATE TABLE IF NOT EXISTS`). Model search on this backend uses SQL `ILIKE` (not Tantivy). Compose profile details: [deploy.md](deploy.md).

## Security (short)

- Prefer `GRAPHNIGHT_API_KEYS` / `GRAPHNIGHT_ADMIN_KEYS` or `GRAPHNIGHT_OIDC_ISSUER` before any shared network bind
- Prefer `env:VARNAME` for warehouse connection strings; set `GRAPHNIGHT_REQUIRE_SECRET_REFS=1` to enforce on GraphQL writes
- Terminate TLS at a reverse proxy (in-process TLS is not implemented)
- Full detail: [SECURITY.md](../SECURITY.md) and [auth.md](auth.md)

## Next

- [concepts.md](concepts.md) — models, measures, joins, auth modes, storage, caching
- [usage-cli.md](usage-cli.md) · [usage-graphql.md](usage-graphql.md) · [usage-python.md](usage-python.md)
- [deploy.md](deploy.md) · [auth.md](auth.md)
- [../LAUNCH.md](../LAUNCH.md) — remaining production checklist items
