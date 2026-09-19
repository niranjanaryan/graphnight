# GraphNight

**Status: alpha (not production-ready)**

GraphNight is an embeddable semantic layer for AI agents and humans. Define metrics once, query them through GraphQL or a CLI, and generate dialect-specific SQL for Postgres, MySQL, SQLite, and DuckDB.

This repository is an early public preview. Core query planning and SQL generation work; authentication, enforced RLS, caching, and production ops are **not** ready for customer data. See [LAUNCH.md](LAUNCH.md) for the OSS and production checklists.

## Features (today)

- Semantic models (measures, dimensions, time dimensions, joins)
- Formula helpers (`sum`, `avg`, `count`, `time_shift`, `ratio`, …)
- SQL generation for Postgres / MySQL / SQLite / DuckDB
- GraphQL API on **Axum** + CLI (`graphnight init`, query dry-run, model list)
- YAML / SQLite metadata storage and Tantivy search
- Session policy / RLS types in-library (**not yet enforced on the live request path**)

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

Longer walkthrough: [docs/getting-started.md](docs/getting-started.md). Concepts: [docs/concepts.md](docs/concepts.md).

### Run the server

```bash
./target/release/graphnight-server \
  --config examples/graphnight.toml \
  --storage-path examples/data \
  --host 127.0.0.1 \
  --port 8080
```

Health check: `curl http://127.0.0.1:8080/health`  
GraphQL: `POST http://127.0.0.1:8080/graphql`

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
| `examples/graphnight.toml` | Server config |
| `examples/data/models.yaml` | Demo semantic models |
| `examples/data/datasources.yaml` | Demo datasource |
| `examples/query.json` | CLI query payload |
| `examples/query.graphql` | GraphQL examples |

## Repository layout

```text
Cargo.toml                 # workspace root
crates/
  graphnight-core/         # models, formulas, joins, security types
  graphnight-sql/          # SQL generator + sqlx executor
  graphnight-storage/      # YAML / SQLite / Tantivy
  graphnight-graphql/      # async-graphql schema
  graphnight-server/       # Axum GraphQL server binary
  graphnight-cli/          # CLI binary
  graphnight-python/       # PyO3 bindings (early)
examples/                  # sample config + models
docs/                      # getting started + concepts
```

See [ARCHITECTURE.md](ARCHITECTURE.md) for the longer-term blueprint (REST, MCP, Flight SQL, importers, caching). Many items there are vision-only.

## Security

By default (no API keys), GraphQL remains open for local demos and logs a warning.

To require auth:

```bash
export GRAPHNIGHT_API_KEYS='alice:secret1'
export GRAPHNIGHT_ADMIN_KEYS='admin:adminsecret'
cargo run -p graphnight-server -- --host 127.0.0.1 --storage-path ./examples/data
# curl -H 'Authorization: Bearer secret1' ...
```

When keys are set: anonymous requests fail; queries use `PolicyEnforcer`; datasource/model writes need an admin key. See [SECURITY.md](SECURITY.md).

Still do **not** expose this to the internet with production warehouse credentials (no OIDC/SSO, audit is not durable yet).

## Python

Experimental PyO3 bindings live under `crates/graphnight-python/`. They are **not** published to PyPI in this alpha. The root `pyproject.toml` is a workspace marker only.

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

1. **v0.1.0-alpha** — semantic core demo (formulas, joins, CLI init, honest stubs) — shipped
2. **Debt train** — sqlx 0.8 + Axum server — done on `main`
3. **v0.2** — API-key auth + `PolicyEnforcer` on the live path — done on `main`
4. **v0.3-beta** — plan/result cache, streaming executor, `/metrics`, Criterion benches — on `main`
5. **v1.0** — OIDC/SSO, durable audit, production ops when LAUNCH section B is green
