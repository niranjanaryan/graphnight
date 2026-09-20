# GraphNight docs

Practical guides for GraphNight **1.0.0**. Start with getting started, then pick a surface (CLI, GraphQL, Python, Elixir, Node.js) or ops topic (auth, deploy).

## Index

| Doc | Contents |
|-----|----------|
| [getting-started.md](getting-started.md) | Build or install, scaffold, dry-run, server, Docker, pip |
| [concepts.md](concepts.md) | Models, measures, joins, auth modes, storage, caching |
| [usage-cli.md](usage-cli.md) | `init`, `model list`, dry-run, search, multi-stage, `serve` tip |
| [usage-graphql.md](usage-graphql.md) | curl, `dryRun`, auth headers, GraphiQL |
| [usage-python.md](usage-python.md) | `pip install graphnight` and client examples |
| [usage-elixir.md](usage-elixir.md) | Elixir bindings, Phoenix integration, Oban jobs, Ecto |
| [usage-native.md](usage-native.md) | `npm install @graphnight/native` — Native Rust bindings |
| [usage-client.md](usage-client.md) | `npm install @graphnight/client` — HTTP GraphQL client |
| [auth.md](auth.md) | API keys, OIDC JWT, PolicyEnforcer, tenant header, `DEV_OPEN` |
| [deploy.md](deploy.md) | Docker, compose HA profile, reverse-proxy TLS, env cheat sheet |

## SDKs

| Language | Package | Install | Repo |
|----------|---------|---------|------|
| **Python** | `graphnight` | `pip install graphnight` | [PyPI](https://pypi.org/project/graphnight/) • [Source](../crates/graphnight-python) |
| **Elixir** | `graphnight` | `mix deps.get` (from Git) | [Hex.pm](https://hex.pm/packages/graphnight) • [Source](https://github.com/niranjanaryan/graphnight-elixir) |
| **Node.js (Native)** | `@graphnight/native` | `npm install @graphnight/native` | [npm](https://www.npmjs.com/package/@graphnight/native) • [Source](../crates/graphnight-node) |
| **Node.js (HTTP)** | `@graphnight/client` | `npm install @graphnight/client` | [npm](https://www.npmjs.com/package/@graphnight/client) • [Source](../node-client) |
| **Rust** | `graphnight-*` crates | `cargo add graphnight-core` | [crates.io](https://crates.io/search?q=graphnight) • [Source](../crates) |

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
- [CHANGELOG.md](../CHANGELOG.md) — 1.0.0 notes and remaining gaps
- [LAUNCH.md](../LAUNCH.md) — OSS vs production checklist
- [ARCHITECTURE.md](../ARCHITECTURE.md) — longer-term blueprint (many items vision-only)

## Incomplete features (honest status)

| Feature | Status in 1.0 |
|---------|----------------|
| Vault / cloud secret managers | Not integrated — use `env:VARNAME` refs |
| MCP / REST agent APIs | Not implemented (see ARCHITECTURE) |
| `multiStageQuery` DAG | Implemented — topological sort + cycle detection |
| `ingestModels` (DB introspection) | Implemented — PostgreSQL, MySQL, SQLite |
| Column masks on responses | Implemented — `SessionPolicy.column_masks` |
| Query timeout end-to-end | Implemented — `SessionPolicy.query_timeout_secs` |
| In-process TLS | Not implemented — terminate at a reverse proxy |
| OIDC browser login UI | JWT bearer validation only |