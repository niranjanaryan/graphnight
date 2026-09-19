# GraphNight

**Status: 1.0.0**

GraphNight is an embeddable semantic layer for AI agents and humans. Define metrics once, query them through GraphQL or a CLI, and generate dialect-specific SQL for Postgres, MySQL, SQLite, and DuckDB.

See [LAUNCH.md](LAUNCH.md) for the full checklist. Remaining gaps (vault secret managers, some observability polish) are documented under Security notes in [CHANGELOG.md](CHANGELOG.md).

## Features (today)

- Semantic models (measures, dimensions, time dimensions, joins)
- Formula helpers (`sum`, `avg`, `count`, `time_shift`, `ratio`, …)
- SQL generation for Postgres / MySQL / SQLite / DuckDB
- GraphQL API on **Axum** + CLI (`graphnight init`, query dry-run, model list)
- YAML / SQLite / **Postgres** metadata storage (HA shared store) and Tantivy search (Postgres search is ILIKE-only)
- API keys + **OIDC JWT** auth; `PolicyEnforcer` on the live query path (allow/deny, forced filters, RLS, max rows)
- Plan/result caches, streaming executor, Docker, durable JSONL audit
## Quick start

### Requirements

- Rust 1.75+ (stable)
- Optional: a SQL database for non-`dryRun` queries

### Build

```bash
cargo build --release -p graphnight-cli -p graphnight-server
```

### Scaffold + dry-run (recommended)

```bash
cargo run -p graphnight-cli -- init ./my-project
cargo run -p graphnight-cli -- \
  --storage-path ./my-project/graphnight_data \
  query dry-run --file ./my-project/query.json
```

Or use the checked-in examples:

```bash
cargo run -p graphnight-cli -- \
  --storage-path ./examples/data \
  model list

cargo run -p graphnight-cli -- \
  --storage-path ./examples/data \
  query dry-run --file ./examples/query.json
```

More samples (auth curls, GraphQL docs, HA, ratio/join dry-runs): **[examples/README.md](examples/README.md)**.

Longer walkthrough: [docs/getting-started.md](docs/getting-started.md). Full docs index: [docs/README.md](docs/README.md).
### Run the server

```bash
./target/release/graphnight-server \
  --config examples/graphnight.toml \
  --storage-path examples/data \
  --host 127.0.0.1 \
  --port 8080
```

Health check (JSON; `503` if metadata storage is unreachable):  
`curl -s http://127.0.0.1:8080/health`  
GraphQL: `POST http://127.0.0.1:8080/graphql`

### Docker

```bash
docker compose up --build
# http://127.0.0.1:8080/health
# optional auth: GRAPHNIGHT_API_KEYS=alice:secret1 docker compose up --build
```

Data persists in the `graphnight-data` volume (`/data` in the container). Image includes `graphnight-server` and the `graphnight` CLI.

HA metadata (optional Postgres):

```bash
docker compose --profile ha up --build metadata-db server-postgres
# metadata on :5433, server on :8081
# GRAPHNIGHT_METADATA_DATABASE_URL=postgresql://graphnight:graphnight@127.0.0.1:5433/graphnight_meta
```

### Postgres metadata storage

For multi-replica deployments, set shared metadata in `graphnight.toml`:

```toml
[storage]
type = "postgres"
path = "env:GRAPHNIGHT_METADATA_DATABASE_URL"
```

Or omit `path` and export `GRAPHNIGHT_METADATA_DATABASE_URL` directly. `path` / `--storage-path` may also be a raw `postgresql://…` URL. Schema tables are created on connect (`CREATE TABLE IF NOT EXISTS`). Model search on this backend is basic `ILIKE` (not Tantivy). See `examples/graphnight.postgres.toml`.

```bash
curl http://127.0.0.1:8080/graphql \
  -H 'content-type: application/json' \
  -d @- <<'EOF'
{
  "query": "query { query(input: { name: \"orders\", measures: [{ formula: \"amount_usd\", aggregation: SUM, label: \"Revenue\" }], dimensions: [{ name: \"status\" }], limit: 10 }, dryRun: true) { sql columns executionTimeMs } }"
}
EOF
```

| Path | Purpose |
|------|---------|
| [`examples/README.md`](examples/README.md) | Index of all examples |
| `examples/graphnight.toml` | Server config (YAML metadata) |
| `examples/graphnight.postgres.toml` | HA Postgres metadata config |
| `examples/data/models.yaml` | Demo semantic models |
| `examples/data/datasources.yaml` | Demo datasource |
| `examples/query.json` | CLI query payload |
| `examples/query.graphql` | GraphQL examples |
| `examples/queries/` | Extra CLI dry-run JSON |
| `examples/graphql/` | GraphQL ops + curl one-liners |
| `examples/auth/` | API key / OIDC curls + policy notes |
| `examples/ha/` | Compose `--profile ha` notes |

## Repository layout

```text
Cargo.toml                 # workspace root
crates/
  graphnight-core/         # models, formulas, joins, security types
  graphnight-sql/          # SQL generator + sqlx executor
  graphnight-storage/      # YAML / SQLite / Postgres / Tantivy
  graphnight-graphql/      # async-graphql schema
  graphnight-server/       # Axum GraphQL server binary
  graphnight-cli/          # CLI binary
  graphnight-python/       # PyO3 bindings (early)
examples/                  # sample config, models, auth/GraphQL/HA usage
docs/                      # practical guides (see Docs below)
```

See [ARCHITECTURE.md](ARCHITECTURE.md) for the longer-term blueprint (REST, MCP, Flight SQL, importers). Many items there are vision-only.

## Docs

| Doc | Contents |
|-----|----------|
| [docs/README.md](docs/README.md) | Docs index |
| [docs/getting-started.md](docs/getting-started.md) | Build, init, dry-run, server, Docker, pip |
| [docs/concepts.md](docs/concepts.md) | Models, auth modes, storage backends, caching |
| [docs/usage-cli.md](docs/usage-cli.md) | CLI: init, model list, dry-run, serve tip |
| [docs/usage-graphql.md](docs/usage-graphql.md) | curl, dryRun, auth headers, GraphiQL |
| [docs/usage-python.md](docs/usage-python.md) | `pip install graphnight` + client examples |
| [docs/auth.md](docs/auth.md) | API keys, OIDC JWT, PolicyEnforcer, tenant, `DEV_OPEN` |
| [docs/deploy.md](docs/deploy.md) | Docker, compose HA, reverse-proxy TLS, env cheat sheet |

## Security

By default (no API keys / OIDC), GraphQL remains open for local demos and logs a warning.

To require auth:

```bash
export GRAPHNIGHT_API_KEYS='alice:secret1'
export GRAPHNIGHT_ADMIN_KEYS='admin:adminsecret'
# optional SSO: export GRAPHNIGHT_OIDC_ISSUER='https://login.example.com/realms/app'
cargo run -p graphnight-server -- --host 127.0.0.1 --storage-path ./examples/data
# curl -H 'Authorization: Bearer secret1' ...
```

When keys or OIDC are set: anonymous requests fail; queries use `PolicyEnforcer`; datasource/model writes need an admin identity. See [docs/auth.md](docs/auth.md) and [SECURITY.md](SECURITY.md) for CORS (`GRAPHNIGHT_CORS_ORIGINS`), TLS (terminate at a reverse proxy), and `env:VARNAME` datasource secret refs.

Still do **not** expose this to the internet with production warehouse credentials. Vault integrations and some policy edges (column masks, full query timeout) remain incomplete — see [CHANGELOG.md](CHANGELOG.md).

## Python

```bash
pip install graphnight
```

PyPI: https://pypi.org/project/graphnight/ (`1.0.0`)

Bindings live under `crates/graphnight-python/` (Rust extension). Multi-platform wheels are built by `.github/workflows/wheels.yml` (Linux manylinux/musllinux, macOS, Windows) and published on version tags / manual dispatch. Optional: `pip install 'graphnight[pandas]'`.

Local develop + tests (not in default CI):

```bash
cd crates/graphnight-python && maturin develop && pytest
```

Runnable scripts: [`examples/python/`](examples/python/).

## Development

```bash
cargo test --workspace --exclude graphnight-python
cargo fmt --all -- --check
cargo clippy --workspace --exclude graphnight-python --all-targets
```

CI runs unit/integration tests plus an example CLI dry-run (see `.github/workflows/ci.yml`).

## Contributing

See [CONTRIBUTING.md](CONTRIBUTING.md). Security reports: [SECURITY.md](SECURITY.md).

## License

Apache License 2.0. See [LICENSE](LICENSE).

## Roadmap

Tracked in [LAUNCH.md](LAUNCH.md):

1. **v0.1–v0.4** — alpha/beta trains (core → auth → cache → Docker/e2e)
2. **v1.0.0** — OIDC JWT, HA Postgres metadata, Postgres testcontainers, multi-platform PyPI
3. **Next** — vault/cloud secret managers, OpenTelemetry, MCP/REST, multi-stage DAG
