# Security Policy

## Supported versions

GraphNight is **alpha**. There is no supported production release yet. Security fixes are applied on a best-effort basis on `main`.

## Auth (v0.2)

Configure API keys via environment:

```bash
export GRAPHNIGHT_API_KEYS='alice:secret1,bob:secret2'
export GRAPHNIGHT_ADMIN_KEYS='admin:adminsecret'
# optional: force auth even with no keys configured
# export GRAPHNIGHT_AUTH_REQUIRED=1
# optional: explicit open mode (silences warning)
# export GRAPHNIGHT_DEV_OPEN=1
```

Clients send `Authorization: Bearer <key>` or `X-API-Key: <key>`. Optional `X-Tenant-Id` adds a forced `tenant_id` filter.

When keys are configured (and `GRAPHNIGHT_DEV_OPEN` is not set):

- Anonymous GraphQL operations are rejected
- Datasource/model write mutations require an **admin** key
- Queries run through `PolicyEnforcer` (allow/deny lists, forced filters, RLS, max rows)

## Durable audit log

Successful and failed GraphQL `query` executions append JSONL `AuditEntry` records (user, tenant, model, row count, duration, success/error).

```bash
# default: ./graphnight_data/audit.jsonl
export GRAPHNIGHT_AUDIT_LOG=/var/log/graphnight/audit.jsonl
```

## CORS

`GRAPHNIGHT_CORS_ORIGINS` controls browser access:

| Value | Behavior |
|-------|----------|
| unset / empty | **Restrictive default:** only `http://127.0.0.1:8080` and `http://localhost:8080` |
| comma-separated URLs | Exact allowlist (e.g. `https://app.example.com,https://admin.example.com`) |
| `*` | Allow any origin (**logs a warning** — avoid on shared deployments) |

## TLS

**Terminate TLS at a reverse proxy** (nginx, Caddy, Traefik, cloud load balancer). Point HTTPS at the proxy and proxy HTTP to GraphNight on localhost.

In-process TLS in `graphnight-server` is **not implemented** yet. Do not expose the raw GraphNight port to the public internet.

## Secret references (connection strings)

Prefer environment variables over plaintext warehouse credentials in config or GraphQL mutations.

### Pattern

1. Put the real connection string in an env var, conventionally `GRAPHNIGHT_DATASOURCE_<NAME>`:

```bash
export GRAPHNIGHT_DATASOURCE_DEMO='postgresql://user:pass@db:5432/analytics'
```

2. Reference it with the `env:VARNAME` form in `graphnight.toml` or GraphQL:

```toml
[datasources.demo]
driver = "postgres"
connection_string = "env:GRAPHNIGHT_DATASOURCE_DEMO"
```

```graphql
mutation {
  createDatasource(input: {
    name: "demo"
    driver: "postgres"
    connectionString: "env:GRAPHNIGHT_DATASOURCE_DEMO"
  }) { name }
}
```

`ConnectionManager` resolves `env:…` when opening pools. Config load validates that referenced vars exist at startup.

### Enforce refs on GraphQL writes

```bash
export GRAPHNIGHT_REQUIRE_SECRET_REFS=1
```

When set, `createDatasource` / `updateDatasource` reject raw connection strings and require `env:VARNAME`.

Vault / cloud secret managers are not integrated yet — inject values into the process environment (or a thin wrapper) and use `env:` refs.

## Remaining risks

- Default without keys is still **open** (dev convenience) — set keys before any shared deployment
- Audit is append-only JSONL (no tamper-evidence / central shipping yet)
- No OIDC/SSO yet
- Plaintext connection strings are still accepted unless `GRAPHNIGHT_REQUIRE_SECRET_REFS=1`
- Do not expose a GraphNight server to the public internet with production warehouse credentials

## Reporting a vulnerability

Please **do not** open a public GitHub issue for sensitive reports.

Email: **nirunitk@gmail.com** with:

- Description of the issue
- Steps to reproduce / proof of concept (non-destructive)
- Affected commit SHA or release tag if known

We will acknowledge reports as soon as practical and coordinate disclosure after a fix is available on `main`.
