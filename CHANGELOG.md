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

### Added

- Root `CHANGELOG.md`
- **API key auth** (`GRAPHNIGHT_API_KEYS` / `GRAPHNIGHT_ADMIN_KEYS`) with request-scoped GraphQL context
- **`PolicyEnforcer` on the live query path** (model/datasource allowlists, forced filters, RLS, max rows)
- Admin gates on datasource/model mutations when auth is required
- Tenant hint via `X-Tenant-Id` → forced `tenant_id` filter
- **Plan cache** (LRU) and **result cache** (LRU + TTL) in `SqlEngine`
- **Row streaming** executor (`sqlx::fetch`) with Postgres/MySQL statement timeouts
- Prometheus text metrics at `GET /metrics`
- GraphQL WebSocket subscriptions at `/graphql/ws` (`liveQuery` polling, `modelChanges`)
- Criterion benches: `cargo bench -p graphnight-sql`

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
