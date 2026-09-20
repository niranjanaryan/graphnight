# Changelog

All notable changes to GraphNight are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added
- **Model Ingestion from DB Schema** (`ingestModels` mutation): Auto-generates semantic models (measures, dimensions, time_dimensions, joins) from PostgreSQL, MySQL, and SQLite databases via `information_schema` / `sqlite_master` introspection
- **Multi-Stage DAG Queries** (`multiStageQuery` query): Execute multiple queries as a directed acyclic graph with topological sorting and cycle detection; stages can reference previous stage results via `stage_ref` for filter chaining
- **Schema Introspection Module** (`graphnight-sql::introspection`): Public API for table/column/foreign key discovery and automatic model inference
- GitHub Actions CI: Separate check (fmt/clippy), Linux/macOS test matrices with Postgres/MySQL services, binary build, security audit, SBOM generation
- GitHub Actions Wheels: Cross-platform Python wheel builds (Linux/macOS/Windows) + PyPI publish on tag push
- Dependabot config for automated dependency updates (Cargo, GitHub Actions, pip)

### Changed
- `multiStageQuery` moved from Mutation to Query (read-only DAG execution)
- `QueryInput` now includes optional `stage_ref` field for multi-stage query dependencies
- Improved clippy cleanliness across workspace (fixed `option_as_deref`, `unnecessary_map_or`, `unwrap_or_default`)

## [1.0.1] - 2026-09-20

Production polish: docs/examples, Python client tests, request IDs, rate limiting, runbook, SBOM CI.

### Added

- Request IDs: middleware generates or propagates `x-request-id`, echoes it on responses, and records `request_id` on HTTP tracing spans
- In-process rate limiting via `GRAPHNIGHT_RATE_LIMIT_RPS` (per API key or client IP; `0`/unset disables; `429` + `Retry-After`; `/health` and `/metrics` exempt)
- Ops runbook: `docs/runbook.md` (deploy, rollback, key/OIDC rotation, audit log, health/metrics, incidents)
- CI job **SBOM / vulnerability scan**: `cargo audit` (`continue-on-error`) + Anchore SPDX SBOM artifact
- Expanded docs: CLI/GraphQL/Python/auth/deploy guides under `docs/`
- Richer `examples/` (auth curls, GraphQL ops, HA, query JSON, Python scripts)
- Python client: sync `GraphNightClient` API with `dry_run_query`, richer
  `create_model` / query formula parsing, pytest suite under
  `crates/graphnight-python/tests/`, and `examples/python/` scripts

### Changed

- `.env.example` documents audit log, rate limit, auth/OIDC/CORS, metadata URL, and secret-ref vars

## [1.0.0] - 2026-09-20

First stable release. API keys + OIDC JWT, PolicyEnforcer on the live path, caches, Docker, HA Postgres metadata, and multi-platform PyPI wheels.

### Changed

- Upgrade sqlx from 0.7 to **0.8.6** (`runtime-tokio`, `tls-none`)
- Migrate GraphQL HTTP server from Tide to **Axum 0.8** with GraphiQL at `/graphql`
- **CORS** defaults to localhost allowlist (`GRAPHNIGHT_CORS_ORIGINS`; `*` warns)
- Workspace / Python package version **1.0.0**

### Added

- Semantic core: models, formula parser, join walker, SQL generation (Postgres/MySQL/SQLite/DuckDB)
- GraphQL API + CLI (`graphnight init`, query dry-run, model/datasource/memory/search)
- YAML / SQLite / **Postgres** metadata storage (HA shared store; Postgres search is `ILIKE`-only)
- **API key auth** + **OIDC JWT bearer** (JWKS discovery); admin/tenant claims; PolicyEnforcer on queries
- Durable JSONL audit log (`GRAPHNIGHT_AUDIT_LOG`)
- Plan/result caches, streaming executor, statement timeouts, Prometheus `/metrics`
- GraphQL WebSocket subscriptions (`/graphql/ws`)
- Dockerfile + docker-compose (incl. `ha` profile for metadata Postgres)
- Deep `GET /health` (503 when storage fails)
- `env:VARNAME` connection-string refs + `GRAPHNIGHT_REQUIRE_SECRET_REFS`
- E2E: SQLite pipeline + **Postgres testcontainers** (skip with `GRAPHNIGHT_SKIP_TESTCONTAINERS=1`)
- Criterion benches; multi-platform wheels CI; PyPI package `graphnight`

### Security notes

- Prefer API keys or OIDC in any shared deployment (`GRAPHNIGHT_DEV_OPEN=1` is local-only)
- Terminate TLS at a reverse proxy
- Vault/cloud secret managers are not integrated yet — use `env:` refs
- Column masks and end-to-end query timeout enforcement remain incomplete

## [0.4.0] - 2026-09-20

PyPI `graphnight` 0.4.0 (superseded by 1.0.0). See git tag `v0.4.0-beta` for the Docker/audit/e2e/CORS train.

## [0.1.0-alpha] - 2026-09-20

Initial public alpha (semantic core + `crates/` layout). See git tags `v0.1.0-alpha` … `v0.3.0-beta` for intermediate trains.

[Unreleased]: https://github.com/niranjanaryan/graphnight/compare/v1.0.1...HEAD
[1.0.1]: https://github.com/niranjanaryan/graphnight/releases/tag/v1.0.1
[1.0.0]: https://github.com/niranjanaryan/graphnight/releases/tag/v1.0.0
[0.4.0]: https://pypi.org/project/graphnight/0.4.0/
[0.1.0-alpha]: https://github.com/niranjanaryan/graphnight/releases/tag/v0.1.0-alpha
