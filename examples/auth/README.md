# Auth examples

Scripts that call a running `graphnight-server` with API keys or OIDC JWTs.

| File | Purpose |
|------|---------|
| [`curl-api-key.sh`](curl-api-key.sh) | `Authorization: Bearer <api-key>` (or `X-API-Key`) |
| [`curl-oidc.sh.example`](curl-oidc.sh.example) | Bearer JWT validated via OIDC JWKS |

See also [SECURITY.md](../../SECURITY.md).

## Quick start (API keys)

```bash
export GRAPHNIGHT_API_KEYS='alice:secret1'
export GRAPHNIGHT_ADMIN_KEYS='admin:adminsecret'
# unset GRAPHNIGHT_DEV_OPEN so auth is required when keys are set

cargo run -p graphnight-server -- \
  --config examples/graphnight.toml \
  --storage-path examples/data \
  --host 127.0.0.1 --port 8080

API_KEY=secret1 ./examples/auth/curl-api-key.sh
# optional tenant hint → forced tenant_id filter on queries:
TENANT_ID=acme API_KEY=secret1 ./examples/auth/curl-api-key.sh
```

Admin mutations (create/update model or datasource) need an admin key. See
[`../graphql/README.md`](../graphql/README.md) for wrapping `create-model.graphql`
into a JSON body, e.g.:

```bash
QUERY=$(python3 -c 'import json,pathlib; print(json.dumps({"query": pathlib.Path("examples/graphql/create-model.graphql").read_text()}))')
curl -s http://127.0.0.1:8080/graphql \
  -H 'content-type: application/json' \
  -H 'Authorization: Bearer adminsecret' \
  -d "$QUERY"
```

## Policy notes (`PolicyEnforcer` / `SessionPolicy`)

When API keys or OIDC are configured (and `GRAPHNIGHT_DEV_OPEN` is not set), GraphQL queries run through `PolicyEnforcer`:

| Capability | Behavior today |
|------------|----------------|
| Auth gate | Anonymous ops rejected when `auth_required` |
| Admin gate | Datasource/model **writes** need `is_admin` |
| Forced filters | `X-Tenant-Id` or OIDC tenant claim → equality filter on `tenant_id` |
| Allow / deny models | Enforced when set on `SessionPolicy` (default policy is open allow-list) |
| RLS (`row_filter`) | Applied when present on the session policy |
| Max rows | Default cap `10000` unless the query limit is lower |
| Datasource allow-list | Enforced when set on the session policy |

Default server policy (see `AuthConfig::default_policy`) only injects the tenant forced filter. Richer allow/deny / RLS policies are available in-library (`graphnight_core::security::SessionPolicy`) for embedders; the stock server wires tenant + max-rows.

Audit: successful and failed `query` executions append JSONL to `GRAPHNIGHT_AUDIT_LOG` (default `./graphnight_data/audit.jsonl`).
