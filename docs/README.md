# GraphNight docs

Practical guides for GraphNight **1.0.0**. Start with getting started, then pick a surface (CLI, GraphQL, Python) or ops topic (auth, deploy).

## Index

| Doc | Contents |
|-----|----------|
| [getting-started.md](getting-started.md) | Build or install, scaffold, dry-run, server, Docker, pip |
| [concepts.md](concepts.md) | Models, measures, joins, auth modes, storage, caching |
| [usage-cli.md](usage-cli.md) | `init`, `model list`, dry-run, search, `serve` tip |
| [usage-graphql.md](usage-graphql.md) | curl, `dryRun`, auth headers, GraphiQL |
| [usage-python.md](usage-python.md) | `pip install graphnight` and client examples |
| [auth.md](auth.md) | API keys, OIDC JWT, PolicyEnforcer, tenant header, `DEV_OPEN` |
| [deploy.md](deploy.md) | Docker, compose HA profile, reverse-proxy TLS, env cheat sheet |

## Examples in-repo

| Path | Purpose |
|------|---------|
| [`examples/graphnight.toml`](../examples/graphnight.toml) | YAML metadata server config |
| [`examples/graphnight.postgres.toml`](../examples/graphnight.postgres.toml) | Postgres metadata (HA) |
| [`examples/data/`](../examples/data/) | Sample `models.yaml` / `datasources.yaml` |
| [`examples/query.json`](../examples/query.json) | CLI query payload |
| [`examples/query.graphql`](../examples/query.graphql) | GraphQL dry-run + list models |

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
| `multiStageQuery` DAG | Errors — not implemented |
| `ingestModels` (DB introspection) | Errors — use YAML / `createModel` |
| Column masks on responses | Types exist; not applied end-to-end |
| Query timeout end-to-end | Policy field exists; full enforcement incomplete |
| In-process TLS | Not implemented — terminate at a reverse proxy |
| OIDC browser login UI | JWT bearer validation only |
