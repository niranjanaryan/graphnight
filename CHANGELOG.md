# Changelog

All notable changes to GraphNight are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added
- **OpenTelemetry tracing** for the plan → SQL → execute path: `sql.plan` and `sql.execute` spans (dialect, datasource, row count, cache hit, statement digest) exported over OTLP/HTTP when `GRAPHNIGHT_OTEL_EXPORTER_OTLP_ENDPOINT` is set. Standard `OTEL_*` variables are honoured too. Off by default — with no endpoint configured no exporter thread starts
- **Cube and dbt importers** (`graphnight import cube|dbt`): convert an existing semantic model to GraphNight models, print for review, and store with `--apply`. Cube converts close to 1:1 (measures, dimensions, time dimensions, joins, `sql_table`); dbt converts columns and requires measures to be declared in `meta.graphnight_measures`, because dbt does not model metrics and guessing an aggregation would silently answer the wrong question. See [docs/migration-cube-dbt.md](docs/migration-cube-dbt.md)
- **`.env` loading** in the server and CLI binaries, so the committed `.env.example` is actionable. A value already exported into the environment wins
- **SECURITY.md**: private vulnerability reporting, severity and response expectations, supported versions, deployment guidance, and a plain list of known limitations
- **MCP server** (`/mcp` over HTTP, `graphnight mcp --stdio` for local use): JSON-RPC tool interface over the same governed service as GraphQL and REST
- **CLI agent** (`graphnight agent`): bounded tool-calling loop with Anthropic and OpenAI-compatible providers, step/turn budgets, and a trace
- **CLI tool access** (`graphnight tools list|show|call`): invoke any tool directly, `--all` to include mutating tools
- **Semantic-layer mutation tools**: `create_model`, `update_model`, `delete_model`, `create_datasource`, `update_datasource`, `delete_datasource`; admin-only, hidden unless mutations are enabled, and refused for a datasource that is in use
- **Memory tools**: `remember` and `recall_memories` with user/global scoping, so a caller only sees memories its scope allows
- **REST** `POST /api/v1/query/multi-stage` and `GET /api/v1/search`
- **Strict aggregation validation**: `AggregationType::parse_strict` rejects anything that is not a plain (optionally schema-qualified) identifier, closing a SQL-injection path in the REST query body
- **Runtime model registration**: `QueryService::register_model` / `unregister_model`, so a model created or updated while the process is running becomes queryable immediately
- **Model Ingestion from DB Schema** (`ingestModels` mutation): Auto-generates semantic models (measures, dimensions, time_dimensions, joins) from PostgreSQL, MySQL, and SQLite databases via `information_schema` / `sqlite_master` introspection
- **Multi-Stage DAG Queries** (`multiStageQuery` query): Execute multiple queries as a directed acyclic graph with topological sorting and cycle detection; stages can reference previous stage results via `stage_ref` for filter chaining
- **Schema Introspection Module** (`graphnight-sql::introspection`): Public API for table/column/foreign key discovery and automatic model inference
- GitHub Actions CI: Separate check (fmt/clippy), Linux/macOS test matrices with Postgres/MySQL services, binary build, security audit, SBOM generation
- GitHub Actions Wheels: Cross-platform Python wheel builds (Linux/macOS/Windows) + PyPI publish on tag push
- Dependabot config for automated dependency updates (Cargo, GitHub Actions, pip)

### Security
- **MySQL string-literal escaping**: backslashes are now escaped before quotes. MySQL treats a backslash as an escape character inside string literals, so a filter value of `\'` previously emitted a literal whose quote was escaped rather than closed, letting the remainder of the value be parsed as SQL. Values are escaped and interpolated rather than sent as bound parameters; the escaping is what makes that safe, and it is now correct on all four dialects
- `SECURITY.md` documents that filter values are escaped and interpolated rather than bound as parameters, instead of claiming otherwise

### Fixed
- **CLI `query run` short-option collision**: `--file` and `--format` both claimed `-f`, which panics under `clap`'s debug assertions. `--format` is now long-only

### Changed
- **One governed query path**: GraphQL, REST, MCP, the agent, the CLI and both language SDKs now execute through a single `QueryService`, which owns policy, row caps, column masks, timeouts, audit and error mapping. No interface has its own SQL execution path
- **Search is policy-filtered**: `search` filters the shared model index through the service, so a caller cannot discover a model their policy denies
- **WebSocket subscriptions** pass `Authorization` / `X-API-Key` and tenant in `connection_init`; every tick is governed and errors are delivered on the stream instead of terminating it
- The SQL engine's model registry is now interior-mutable, fixing a bug where models created through the admin API were listed and searchable but failed to generate SQL
- Metadata endpoints (`models`, `model`, `datasources`, `datasource`) are policy-filtered on both GraphQL and REST
- `graphnight sql <file>` is documented as the older spelling of `graphnight query dry-run`; both are now governed
- `query` JSON files may omit empty `measures` / `dimensions` / `filters` / `order` / `time_dimensions` arrays
- `multiStageQuery` moved from Mutation to Query (read-only DAG execution)
- `QueryInput` now includes optional `stage_ref` field for multi-stage query dependencies
- Improved clippy cleanliness across workspace (fixed `option_as_deref`, `unnecessary_map_or`, `unwrap_or_default`); the workspace is now warning-free under `cargo clippy --all-targets`
- Removed the parallel GraphQL policy-enforcement helper, which duplicated `PolicyEnforcer`; its test coverage moved to the runtime, where enforcement actually happens
- Removed the Elixir bindings, which were unmaintained relative to the rest of the workspace

## [1.2.0] - 2026-09-27

### Performance
- **FNV-1a hashing for cache keys**: replaced SipHash (`DefaultHasher`) with FNV-1a 64-bit hashing on plan-cache and result-cache keys. FNV-1a is ~2-3× faster to calculate and is adequate for cache keys over an uncontrolled domain; it is documented as not a MAC. Cold compile improved ~10% headline and ~3% across query families
- **Result-cache shape reuse**: the result cache now stores the final `serde_json::Map` shape instead of raw `HashMap`s, so a cache hit is a single clone with no per-row conversion. The executor returns `HashMap`s and every consumer wants `Map`s, so the conversion is done once on miss and skipped entirely on hit

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

[Unreleased]: https://github.com/niranjanaryan/graphnight/compare/v1.2.0...HEAD
[1.2.0]: https://github.com/niranjanaryan/graphnight/releases/tag/v1.2.0
[1.0.1]: https://github.com/niranjanaryan/graphnight/releases/tag/v1.0.1
[1.0.0]: https://github.com/niranjanaryan/graphnight/releases/tag/v1.0.0
[0.4.0]: https://pypi.org/project/graphnight/0.4.0/
[0.1.0-alpha]: https://github.com/niranjanaryan/graphnight/releases/tag/v0.1.0-alpha
