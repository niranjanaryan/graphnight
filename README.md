# GraphNight

**Status: alpha (not production-ready)**

GraphNight is an embeddable semantic layer for AI agents and humans. Define metrics once, query them through GraphQL or a CLI, and generate dialect-specific SQL for Postgres, MySQL, SQLite, and DuckDB.

This repository is an early public preview. Core query planning and SQL generation work; authentication, enforced RLS, caching, and production ops are **not** ready for customer data. See [LAUNCH.md](LAUNCH.md) for the OSS and production checklists.

## Features (today)

- Semantic models (measures, dimensions, time dimensions, joins)
- Formula helpers (`sum`, `avg`, `count`, `time_shift`, `ratio`, …)
- SQL generation for Postgres / MySQL / SQLite / DuckDB
- GraphQL API + CLI
- YAML / SQLite metadata storage and Tantivy search
- Session policy / RLS types in-library (**not yet enforced on the live request path**)

## Quick start

### Requirements

- Rust 1.75+ (stable)
- Optional: a SQL database for non-`dryRun` queries

### Build

```bash
cd rust-engine
cargo build --release -p graphnight-cli -p graphnight-server
```

### Run the server

```bash
# from repo root
./rust-engine/target/release/graphnight-server \
  --config examples/graphnight.toml \
  --storage-path examples/data \
  --host 127.0.0.1 \
  --port 8080
```

Health check:

```bash
curl http://127.0.0.1:8080/health
```

GraphQL endpoint: `POST http://127.0.0.1:8080/graphql`

### Dry-run a query (no database required)

```bash
curl http://127.0.0.1:8080/graphql \
  -H 'content-type: application/json' \
  -d @- <<'EOF'
{
  "query": "query { query(input: { name: \"orders\", measures: [{ formula: \"amount_usd\", aggregation: SUM, label: \"Revenue\" }], dimensions: [{ name: \"status\" }], limit: 10 }, dryRun: true) { sql columns executionTimeMs } }"
}
EOF
```

Sample documents live under [`examples/`](examples/):

| Path | Purpose |
|------|---------|
| `examples/graphnight.toml` | Server config |
| `examples/data/models.yaml` | Demo semantic models |
| `examples/data/datasources.yaml` | Demo datasource |
| `examples/query.json` | CLI query payload |
| `examples/query.graphql` | GraphQL examples |

### CLI

```bash
cd rust-engine
cargo run -p graphnight-cli -- \
  --storage-path ../examples/data \
  model list

cargo run -p graphnight-cli -- \
  --storage-path ../examples/data \
  query dry-run --file ../examples/query.json
```

## Workspace layout

```text
rust-engine/
  graphnight-core/       # models, formulas, joins, security types
  graphnight-sql/        # SQL generator + sqlx executor
  graphnight-storage/    # YAML / SQLite / Tantivy
  graphnight-graphql/    # async-graphql schema
  graphnight-server/     # Tide GraphQL server binary
  graphnight-cli/        # CLI binary
  graphnight-python/     # PyO3 bindings (early)
```

See [ARCHITECTURE.md](ARCHITECTURE.md) for the longer-term blueprint (REST, MCP, Flight SQL, importers, caching).

## Security warning

Alpha servers:

- Bind openly unless you pass `--host 127.0.0.1`
- Expose GraphQL **without authentication**
- Accept datasource `connection_string` values via mutations
- Do **not** apply row-level security on the live query path yet

Do not expose this process to the internet or attach production credentials.

## Development

```bash
cd rust-engine
cargo test --workspace --exclude graphnight-python
cargo fmt --all -- --check
cargo clippy --workspace --exclude graphnight-python -- -D warnings
```

CI runs the same test command on pushes and pull requests (see `.github/workflows/ci.yml`).

## License

Apache License 2.0. See [LICENSE](LICENSE).

## Roadmap

Tracked in [LAUNCH.md](LAUNCH.md):

1. Ship a usable OSS v0.1 (docs, CI, examples, alpha labeling) — in progress
2. Enforce auth + `PolicyEnforcer` on every query
3. Metrics, audit log, Docker, integration tests
4. Caching, multi-stage DAG, MCP/REST — then production launch
