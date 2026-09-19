# Changelog

All notable changes to GraphNight are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html)
with `-alpha` / `-beta` pre-release tags while the API is unstable.

## [Unreleased]

### Changed

- Upgrade sqlx from 0.7 to **0.8.6** (`runtime-tokio`, `tls-none`)
- Migrate GraphQL HTTP server from Tide to **Axum 0.8** with GraphiQL at `/graphql`
- Drop unused `tide` / `async-graphql-tide` dependencies from the GraphQL crate
- **CORS** no longer uses `CorsLayer::permissive()`; default allowlist is localhost only

### Added

- **Postgres metadata storage** (`PostgresMetadataStorage`): `storage.type = "postgres"` with JSONB tables, `CREATE TABLE IF NOT EXISTS` on connect, connection via `storage.path` / `--storage-path` or `GRAPHNIGHT_METADATA_DATABASE_URL` (`env:VAR` refs supported). Search is `ILIKE`-only (not Tantivy). Compose profile `ha` adds optional `metadata-db` + `server-postgres`; see `examples/graphnight.postgres.toml`
- Root `CHANGELOG.md`
- **OIDC/SSO JWT bearer auth** (`GRAPHNIGHT_OIDC_ISSUER` + optional audience/client/admin/tenant claims): discovery + JWKS validation; JWT Bearer tried before API-key fallback; `auth_required` when OIDC is configured
- **API key auth** (`GRAPHNIGHT_API_KEYS` / `GRAPHNIGHT_ADMIN_KEYS`) with request-scoped GraphQL context
- **`PolicyEnforcer` on the live query path** (model/datasource allowlists, forced filters, RLS, max rows)
- Admin gates on datasource/model mutations when auth is required
- Tenant hint via `X-Tenant-Id` → forced `tenant_id` filter
- **Durable JSONL audit log** (`GRAPHNIGHT_AUDIT_LOG`, default `./graphnight_data/audit.jsonl`) on GraphQL `query` success/failure
- **Plan cache** (LRU) and **result cache** (LRU + TTL) in `SqlEngine`
- **Row streaming** executor (`sqlx::fetch`) with Postgres/MySQL statement timeouts
- Prometheus text metrics at `GET /metrics`
- GraphQL WebSocket subscriptions at `/graphql/ws` (`liveQuery` polling, `modelChanges`)
- Criterion benches: `cargo bench -p graphnight-sql`
- **E2E crate** `tests/e2e` (`graphnight-e2e`): examples YAML load → SQL dry-run (Postgres + SQLite dialects) → execute against temp SQLite `orders` table; Postgres testcontainers stub `#[ignore]`d
- GraphQL auth e2e: admin + `auth_required` succeeds on non-destructive dry-run query
- CI step runs `cargo test -p graphnight-e2e`
- **Dockerfile** + **docker-compose.yml** (multi-stage release build of `graphnight-server` + `graphnight` CLI; data volume; API key env)
- Deep **`GET /health`** JSON (`storage` via `list_models`, open pool counts); returns **503** when storage fails
- **`GRAPHNIGHT_CORS_ORIGINS`** (comma allowlist, `*` for open with warning; empty → localhost)
- **`env:VARNAME` connection-string refs** resolved at pool connect; config startup validates refs
- **`GRAPHNIGHT_REQUIRE_SECRET_REFS`** rejects raw connection strings on GraphQL datasource mutations
- TLS guidance in `SECURITY.md` / `.env.example` (terminate at reverse proxy; no in-process TLS yet)
- **PyPI** package [`graphnight` 0.4.0](https://pypi.org/project/graphnight/) (macOS arm64 wheel + sdist; CI builds more platforms)
- GitHub Releases for `v0.1.0-alpha` … `v0.4.0-beta`
- **Wheels CI** (`.github/workflows/wheels.yml`) for Linux/macOS/Windows via maturin-action

## [0.1.0-alpha] - 2026-09-20

### Added

- Rust workspace at repository root with crates under `crates/`
- Semantic core: models, formula parser, join walker, SQL generation (Postgres/MySQL/SQLite/DuckDB)
- GraphQL API (query/dry-run, model/datasource/memory CRUD surfaces)
- CLI: `graphnight init`, model/datasource/memory/search, query dry-run
- YAML/SQLite storage + Tantivy search
- Examples, getting-started docs, CI (fmt/clippy/test + example dry-run)
- `LAUNCH.md`, `CONTRIBUTING.md`, `SECURITY.md`
- Apache-2.0 license

### Security

- **Alpha:** GraphQL is unauthenticated; RLS/session policies exist in-library but are **not** enforced on the live request path. Do not expose to the internet or attach production credentials.

### Known limitations

- `ingestModels` / `multiStageQuery` unsupported (explicit errors)
- Subscriptions are stubs; no result/plan cache
- Python bindings experimental (not on PyPI)
- Auth / RLS still not enforced on the live path (planned for v0.2)

[Unreleased]: https://github.com/niranjanaryan/graphnight/compare/v0.1.0-alpha...HEAD
[0.1.0-alpha]: https://github.com/niranjanaryan/graphnight/releases/tag/v0.1.0-alpha
