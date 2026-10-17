# GraphNight docs

Practical guides for GraphNight **2.0.0**. Start with getting started, then pick a surface (CLI, agent/MCP, GraphQL, Python, Node.js) or ops topic (auth, deploy).

## Index

| Doc | Contents |
|-----|----------|
| [getting-started.md](getting-started.md) | Build or install, scaffold, dry-run, server, Docker, pip |
| [concepts.md](concepts.md) | Models, measures, joins, auth modes, storage, caching |
| [usage-cli.md](usage-cli.md) | `init`, `model list`, dry-run, search, multi-stage, `serve` tip |
| [usage-agent.md](usage-agent.md) | MCP server, built-in agent, tool catalog, governance |
| [migration-cube-dbt.md](migration-cube-dbt.md) | `graphnight import cube` / `import dbt` |
| [usage-elixir.md](usage-elixir.md) | `mix deps.get`, `iex`, `GraphNight.query/2`, Ecto integration |
| [usage-graphql.md](usage-graphql.md) | curl, `dryRun`, auth headers, GraphiQL |
| [usage-python.md](usage-python.md) | `pip install graphnight` and client examples |
| [usage-native.md](usage-native.md) | `npm install @graphnight/native` — Native Rust bindings |
| [usage-client.md](usage-client.md) | `npm install @graphnight/client` — HTTP GraphQL client |
| [auth.md](auth.md) | API keys, OIDC JWT, PolicyEnforcer, tenant header, `DEV_OPEN` |
| [deploy.md](deploy.md) | Docker, compose HA profile, reverse-proxy TLS, env cheat sheet |
| [spatial.md](spatial.md) | Geometry filters and measures, dialect support, what is not supported yet |

## SDKs

| Language | Package | Install | Repo |
|----------|---------|---------|------|
| **Python** | `graphnight` | `pip install graphnight` | [PyPI](https://pypi.org/project/graphnight/) • [Source](../crates/graphnight-python) |
| **Node.js (Native)** | `@graphnight/native` | `npm install @graphnight/native` | [npm](https://www.npmjs.com/package/@graphnight/native) • [Source](../crates/graphnight-node) |
| **Node.js (HTTP)** | `@graphnight/client` | `npm install @graphnight/client` | [npm](https://www.npmjs.com/package/@graphnight/client) • [Source](../node-client) |
| **Rust** | `graphnight-*` crates | `cargo add graphnight-core` | [crates.io](https://crates.io/search?q=graphnight) • [Source](../crates) |
| **Elixir** | `:graphnight` | `mix deps.add :graphnight` | [Hex](https://hex.pm/packages/graphnight) • [Source](../crates/graphnight-elixir) |

## Examples in-repo

| Path | Purpose |
|------|---------|
| [`examples/graphnight.toml`](../examples/graphnight.toml) | YAML metadata server config |
| [`examples/graphnight.postgres.toml`](../examples/graphnight.postgres.toml) | Postgres metadata (HA) |
| [`examples/data/`](../examples/data/) | Sample `models.yaml` / `datasources.yaml` |
| [`examples/query.json`](../examples/query.json) | CLI query payload |
| [`examples/query.graphql`](../examples/query.graphql) | GraphQL dry-run + list models |
| [`node-client/`](../node-client/) | Node.js HTTP client source & tests |
| [`node-native/`](../node-native/) | Node.js native SDK source & tests |

## Related root docs

- [README.md](../README.md) — features and quick start
- [SECURITY.md](../SECURITY.md) — auth, CORS, secrets, reporting
- [CHANGELOG.md](../CHANGELOG.md) — 2.0.0 notes and remaining gaps
- [PLANS.md](../PLANS.md) — in-flight work, open decisions, spatial roadmap
- [LAUNCH.md](../LAUNCH.md) — OSS vs production checklist
- [ARCHITECTURE.md](../ARCHITECTURE.md) — longer-term blueprint (many items vision-only)

## Incomplete features (honest status)

| Feature | Status in 1.0 |
|---------|----------------|
| Vault / cloud secret managers | Not integrated — use `env:VARNAME` refs |
| MCP / REST agent APIs | Implemented — `/mcp`, `graphnight mcp --stdio`, `/api/v1/query/multi-stage`, `/api/v1/search` |
| `multiStageQuery` DAG | Implemented — topological sort + cycle detection |
| `ingestModels` (DB introspection) | Implemented — PostgreSQL, MySQL, SQLite |
| Column masks on responses | Implemented — `SessionPolicy.column_masks` |
| Query timeout end-to-end | Implemented — `SessionPolicy.query_timeout_secs` |
| In-process TLS | Not implemented — terminate at a reverse proxy |
| OIDC browser login UI | JWT bearer validation only |
| Export back to Cube / dbt | Not implemented — importers are one-way; keep the converted YAML |
| DuckDB as an execution target | Implemented — behind the `duckdb` feature (bundled); server and CLI enable it |
| DuckDB schema introspection | Not implemented — write the model explicitly; see [spatial.md](spatial.md) |
| GeoParquet | Not implemented — no reader or loader; see [spatial.md](spatial.md) |
| Geometry values in results | Implemented — a geometry column is returned as GeoJSON; see [spatial.md](spatial.md) |
| Geometry column auto-detection | Implemented — introspection reads `udt_name`; see [spatial.md](spatial.md) |
