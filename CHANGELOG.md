# Changelog

All notable changes to GraphNight are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [2.0.0] - 2026-10-16

### Added
- **DuckDB execution**: the executor now has a DuckDB driver, so a generated DuckDB statement actually runs instead of failing with "unsupported driver". It is behind a `duckdb` feature (enabled by the server and CLI) that bundles libduckdb, so a user installs nothing; the cost is a large one-time C++ build. One connection per datasource behind a mutex, because a DuckDB connection is not `Sync`. Decoding is by the column's own type, not a chain of widening attempts — DuckDB will happily decode a `DOUBLE` as `i64` and round it, so `1.5` would have become `2`
- **Geometry values in results**: a geometry column in a result set is returned as a GeoJSON object, converted in SQL with `ST_AsGeoJSON(column)` so the driver only ever sees text. GeoJSON was chosen over WKT and WKB because it round-trips back into the spatial filter operators and is what mapping clients consume
- **Geometry column auto-detection**: introspection now reads `udt_name` alongside `data_type`. PostGIS reports `USER-DEFINED` in `data_type` and the real name (`geometry`, `geography`) in `udt_name`, which marks the column spatial on the inferred model and is what drives the GeoJSON conversion. Detection does not change generated SQL on its own — models stay explicit about which columns are spatial
- **Spatial filters** (`intersects`, `within`, `contains`): a filter can now test a geometry column against a geometry value, rendered as `ST_Intersects(geom, ST_GeomFromGeoJSON(CAST($1 AS VARCHAR)))`. The value is a GeoJSON object or a WKT string and is still bound, so it cannot alter the statement's shape — the constructor is what turns bound text into a geometry. Supported on PostgreSQL (PostGIS) and DuckDB spatial; MySQL and SQLite refuse with a reason rather than emitting SQL that would fail at the driver. Available through the SQL engine, REST, GraphQL, and the Node, Python and Elixir bindings. Exposed as filter operators rather than function calls because no function-call path can carry a geometry value: literals are refused there by design, which is what makes that path safe
- **Tests for the embedding environment wiring.** Which combinations of `GRAPHNIGHT_EMBEDDING_*` settings enable hybrid retrieval, and which fall back to lexical, was untested. The resolution logic is now separated from the env read so it can be tested without mutating process environment, which is racy under a parallel runner. Covers the deliberate choice that a local OpenAI-compatible endpoint needs no API key
- **Allowlisted functions in measure expressions**: a measure may now call a function from a fixed list, which is what makes spatial measures expressible — `ST_Distance(a, b)` and `ST_Intersects(a, b)` previously became quoted column names and failed at the database. Covers the spatial predicates and constructors in PostGIS, DuckDB spatial and GDAL that share the `ST_*` names, plus common string, numeric and date functions. The call is rebuilt from validated parts, so a name or argument cannot carry SQL with it. Single-level calls with column or numeric arguments only; nesting, string literals, operators and subqueries are refused with a specific message rather than passed through
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
- **Spatial filter operators are not infix.** `FilterOperator::sql_operator` now returns `Option`, so a spatial operator cannot be rendered as a comparison between two values; the generator is forced to handle it as a function call. Each predicate also gets a distinct result-cache key, since the same column and geometry under a different predicate is a different question
- **A join to an unregistered model is now refused.** Join edges come from each registered model's `joins` list, so a model could name a target that was never registered; traversal accepted the declared edge and generated a `JOIN` against a table outside the model registry. Since models are what policy is applied to, that reached a table no policy decision covered. Now refused at generation with a message naming the target and where it was reached from
- **Qualified dimension names resolve.** `customers.name` was quoted whole into a three-part identifier (`orders."customers.name"`) that no dialect accepts, so the obvious spelling of a joined field produced SQL that failed at the database. A dotted name whose prefix is a known model or join alias now resolves to the two-part form. Bare names still resolve across joins as before
- **MySQL string-literal escaping**: backslashes are now escaped before quotes. MySQL treats a backslash as an escape character inside string literals, so a filter value of `\'` previously emitted a literal whose quote was escaped rather than closed, letting the remainder of the value be parsed as SQL. This was fixed before filter values became bound parameters, and remains necessary for the inline rendering used by dry-run output, error messages and audit records — which is never executed. Escaping is now correct on all four dialects
- **`SqlEngine::execute_stream` is no longer public.** It ran caller-supplied
  SQL with no policy check, no row cap and no metrics, which contradicted the
  claim in `SECURITY.md` that every query goes through the same policy. It had
  no callers and is now removed; use `QueryService` for anything a caller can
  reach
- The `sql.plan` span recorded generated SQL, which contains literal filter
  values. It now records a digest, matching what `sql.execute` already did

### Fixed
- **`TelemetryGuard::flush` no longer appears to work while blocking the runtime.** The method blocks on a channel until the SDK's exporter acknowledges, with a hardcoded 5s timeout. The end-to-end telemetry test called it by awaiting it on a `current_thread` runtime that also served the stub collector, so the collector could not accept the export the flush was waiting for: the test deadlocked until the timeout expired, then passed. It was asserting nothing about latency, and its 5.2s runtime was the symptom. Now run via `spawn_blocking` on a multi-thread runtime, and it finishes in ~1ms. `flush` is documented as blocking, since `Drop` calls it and a guard dropped on a single-threaded runtime stalls it
- **A Cube join to a cube outside the imported file is reported.** Join targets are often declared in another file, so the import warns rather than dropping the join (dropping it would silently change the fact grain). The warning reuses the generation-time wording, so an operator meets the same sentence in the import report and in the query error
- **`QueryExecutor::execute_literal` is now crate-private.** It runs SQL with no bound values and was a public entry point for ungenerated statements, which contradicts the property that every statement reaching a database comes from `SqlGenerator`. Only schema introspection called it
- **Bound parameters are now verified against a live Postgres.** The tests asserting `$1`/`$2` numbering only inspected generated SQL, and the tests that executed bound parameters ran on SQLite, where the placeholder is `?` and binding is positional by construction — so arguments bound to the wrong Postgres slot would have passed every test. A containerised test now runs generate → bind → execute → extract, covering slot ordering, values that match nothing, `%` as data rather than a LIKE wildcard, NULL comparison semantics, and a bound value that is itself valid SQL
- **Silent result loss on common Postgres types.** `int2`, `int4`, `float4`, `numeric` and `SUM(numeric)` decoded to JSON `null` with no error, because three hand-written copies of the extraction chain had drifted and each listed only the types its author had a model for. The copies are now one macro, and a type that cannot be decoded is an error rather than a null
- **Postgres `int4`, `int2`, `float4` and `numeric` results came back as `null`.** The three hand-written row-extraction chains (one per driver) had drifted, each listing only the types its author had a model for. sqlx will not widen `int4` to `i64` or narrow `numeric` to `f64`, so those values matched no attempt and were returned as `null` with no error. `SUM` over a `numeric` column — the shape used by the README revenue example — returned null for every row. All three chains now come from one macro, every integer and float width a driver can return is handled, `numeric` is decoded as an arbitrary-precision decimal rather than rounded through `f64`, and Postgres `timestamp`/`date` are covered. Regression test runs against a real database via testcontainers
- **A column type that cannot be decoded is now an error instead of `null`**, so dropped data can never be mistaken for a genuine SQL NULL
- **`graphnight query run --format` accepted `csv` and `parquet`, which the command cannot produce.** An unrecognised value ran the query and then printed a message while exiting 0. The accepted values are now constrained by the argument parser, so a typo is rejected before the query executes
- **`Model.sql` was ignored entirely.** A model declaring a custom SQL source
  still generated `FROM "<model name>"`, so a Cube cube reading a
  schema-qualified or differently named table silently read the wrong table —
  or none. The generator now inlines the override as a subquery, and the Cube
  importer emits `SELECT * FROM <table>` when a cube's table differs from its
  name. A model's SQL also never had its trailing semicolon stripped
- **Filter values and order direction are no longer mandatory in JSON.** The
  `or_condition` and `descending` fields were required on the wire despite
  defaulting to `false` in code, so a hand-written REST/CLI payload was rejected
  for omitting a field that has an obvious default
- **Trace filtering no longer follows the log level.** The log `EnvFilter` was
  applied to the whole registry, so `RUST_LOG=warn` exported no spans at all.
  Logs and spans are filtered independently now;
  `GRAPHNIGHT_OTEL_TRACES_FILTER` controls spans and defaults to `info`
- **CLI `query run` short-option collision**: `--file` and `--format` both claimed `-f`, which panics under `clap`'s debug assertions. `--format` is now long-only

### Changed
- **An unrecognized filter operator is now an error, not `=`.** Node, Python, Elixir and REST each ended their operator mapping with a lenient default, so a typo such as `"intresects"` silently became an equality comparison and returned no rows. All four now reject an unknown operator with an error. This is a breaking change for any client relying on the lenient default, made deliberately
- **Filter values are now bound parameters, not interpolated text.** The generated statement carries placeholders (`$1`, `$2` on PostgreSQL, `?` elsewhere) and values travel over the driver, so a filter value is data to the database parser rather than SQL to it. This closes the last interpolation path and makes the injection posture a property of the driver rather than of escaping. A value is still rendered inline for dry-run output, error messages, and audit records, and that rendering is never executed. `SqlEngine::generate_sql` now returns a `GeneratedSql` carrying the executable statement and its parameters; anything that used it for display purposes formats it explicitly
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
- Removed the unused `arrow` dependency (and the commented-out `parquet`) from `graphnight-sql`; both were placeholders for GeoParquet and will be added back when that work starts

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

[Unreleased]: https://github.com/niranjanaryan/graphnight/compare/v2.0.0...HEAD
[2.0.0]: https://github.com/niranjanaryan/graphnight/releases/tag/v2.0.0
[1.2.0]: https://github.com/niranjanaryan/graphnight/releases/tag/v1.2.0
[1.0.1]: https://github.com/niranjanaryan/graphnight/releases/tag/v1.0.1
[1.0.0]: https://github.com/niranjanaryan/graphnight/releases/tag/v1.0.0
[0.4.0]: https://pypi.org/project/graphnight/0.4.0/
[0.1.0-alpha]: https://github.com/niranjanaryan/graphnight/releases/tag/v0.1.0-alpha
