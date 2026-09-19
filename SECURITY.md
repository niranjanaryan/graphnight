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

## Remaining risks

- Default without keys is still **open** (dev convenience) — set keys before any shared deployment
- Audit log is tracing-only (not durable)
- No OIDC/SSO yet
- Do not expose a GraphNight server to the public internet with production warehouse credentials

## Reporting a vulnerability

Please **do not** open a public GitHub issue for sensitive reports.

Email: **nirunitk@gmail.com** with:

- Description of the issue
- Steps to reproduce / proof of concept (non-destructive)
- Affected commit SHA or release tag if known

We will acknowledge reports as soon as practical and coordinate disclosure after a fix is available on `main`.
