# Authentication and policy

GraphNight 1.0 supports API keys and OIDC JWT bearer validation. Browser login redirect / authorization-code UI is **not** implemented.

Authoritative detail also lives in [SECURITY.md](../SECURITY.md).

## Modes

| Situation | `auth_required` | Notes |
|-----------|-----------------|-------|
| No keys, no OIDC | false | Open GraphQL; warning logged |
| `GRAPHNIGHT_DEV_OPEN=1` | false | Explicit open; warning silenced — **local only** |
| API keys and/or OIDC set | true | Anonymous rejected |
| `GRAPHNIGHT_AUTH_REQUIRED=1` | true | Fail closed even with empty key maps |

When auth is required:

- Every GraphQL operation needs a resolved identity
- Datasource / model **write** mutations need an **admin** identity
- Queries run through `PolicyEnforcer`

## API keys

```bash
export GRAPHNIGHT_API_KEYS='alice:secret1,bob:secret2'
export GRAPHNIGHT_ADMIN_KEYS='admin:adminsecret'
```

Format: comma-separated `user_id:secret` pairs. Admin keys are also registered as API keys; those `user_id`s are marked admin.

Clients send either:

```http
Authorization: Bearer secret1
```

or

```http
X-API-Key: secret1
```

## OIDC JWT

Library validation of Bearer JWTs (JWKS discovery). No hosted login UI.

```bash
export GRAPHNIGHT_OIDC_ISSUER='https://login.example.com/realms/app'
# optional:
# export GRAPHNIGHT_OIDC_AUDIENCE='graphnight-api'
# export GRAPHNIGHT_OIDC_CLIENT_ID='graphnight'
# export GRAPHNIGHT_OIDC_ADMIN_CLAIM='roles'
# export GRAPHNIGHT_OIDC_ADMIN_VALUES='admin,Admin'
# export GRAPHNIGHT_OIDC_TENANT_CLAIM='tenant_id'
```

At runtime GraphNight fetches `{issuer}/.well-known/openid-configuration`, then the JWKS URI, and validates signature / `iss` / `exp` (and `aud` when configured).

Request handling order:

1. If `Authorization: Bearer` looks like a JWT (exactly two `.` separators) **and** OIDC is configured → validate via JWKS first
2. Otherwise fall back to API key matching
3. Map `sub` → `user_id`; admin from the admin claim/values; tenant from the tenant claim (or `X-Tenant-Id` if the token has none)

When OIDC is configured and `GRAPHNIGHT_DEV_OPEN` is not set, `auth_required` is **true** even with no API keys.

## Tenant header

```http
X-Tenant-Id: acme
```

If the identity has a `tenant_id` (from header and/or OIDC claim), the default session policy adds a forced filter `tenant_id = <value>`.

## PolicyEnforcer

Applied on the live GraphQL `query` path via `SessionPolicy`:

- `allowed_models` / `denied_models`
- `allowed_datasources`
- Forced filters (including tenant)
- RLS `row_filter`
- `max_rows` (default policy uses 10_000)

**Not complete yet:** column masks on response data and end-to-end query timeout enforcement (policy fields exist; do not rely on them for production guarantees).

## DEV_OPEN

```bash
export GRAPHNIGHT_DEV_OPEN=1
```

Forces open mode even if keys/OIDC env vars are present in the environment (e.g. docker-compose defaults). Default compose sets `GRAPHNIGHT_DEV_OPEN=1` for local demos — **clear it** for shared deployments and set real keys or OIDC.

## Audit

Successful and failed GraphQL `query` executions append JSONL `AuditEntry` records:

```bash
# default: ./graphnight_data/audit.jsonl
export GRAPHNIGHT_AUDIT_LOG=/var/log/graphnight/audit.jsonl
```

Append-only file; no tamper-evidence or central shipping built in.

## Quick checklist for shared deploys

1. Unset `GRAPHNIGHT_DEV_OPEN` (or set to `0`)
2. Set `GRAPHNIGHT_API_KEYS` + `GRAPHNIGHT_ADMIN_KEYS`, and/or `GRAPHNIGHT_OIDC_ISSUER`
3. Prefer `env:` datasource refs + `GRAPHNIGHT_REQUIRE_SECRET_REFS=1`
4. Bind the process to localhost and terminate TLS at a reverse proxy
5. Set `GRAPHNIGHT_CORS_ORIGINS` to an explicit allowlist (not `*`)
