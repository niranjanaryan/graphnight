# Deploy

## Docker (single node)

```bash
docker compose up --build
# http://127.0.0.1:8080/health
# GraphiQL: http://127.0.0.1:8080/graphql
```

Optional auth:

```bash
GRAPHNIGHT_API_KEYS=alice:secret1 GRAPHNIGHT_ADMIN_KEYS=admin:adminsecret \
  GRAPHNIGHT_DEV_OPEN=0 \
  docker compose up --build
```

Compose defaults `GRAPHNIGHT_DEV_OPEN=1` for local demos — clear it for shared use.

- Image includes `graphnight-server` and the `graphnight` CLI
- Data persists in the `graphnight-data` volume (`/data` in the container)
- Default config: `/app/examples/graphnight.toml`

Build-only:

```bash
docker build -t graphnight:local .
docker run --rm -p 8080:8080 \
  -e GRAPHNIGHT_DEV_OPEN=1 \
  -v graphnight-data:/data \
  graphnight:local
```

## Compose HA profile (Postgres metadata)

Shared metadata for multi-replica / HA:

```bash
docker compose --profile ha up --build metadata-db server-postgres
# metadata Postgres on host :5433
# server on host :8081 → container :8080
```

`server-postgres` mounts `examples/graphnight.postgres.toml` and sets:

```text
GRAPHNIGHT_METADATA_DATABASE_URL=postgresql://graphnight:graphnight@metadata-db:5432/graphnight_meta
```

From the host (e.g. a binary outside compose):

```bash
export GRAPHNIGHT_METADATA_DATABASE_URL='postgresql://graphnight:graphnight@127.0.0.1:5433/graphnight_meta'
./target/release/graphnight-server --config examples/graphnight.postgres.toml --host 127.0.0.1
```

Notes:

- Metadata tables are created on connect
- Model search on Postgres metadata is `ILIKE`-only (not Tantivy)
- Plan/result caches remain **per process** — not shared across replicas
- Demo warehouse datasource in the sample config is still local SQLite; point `[datasources.*]` at real warehouses for production

## Reverse proxy and TLS

In-process TLS in `graphnight-server` is **not implemented**. Terminate TLS at nginx, Caddy, Traefik, or a cloud load balancer and proxy HTTP to GraphNight on localhost.

Example Caddy sketch:

```caddy
graphnight.example.com {
  reverse_proxy 127.0.0.1:8080
}
```

Example nginx sketch:

```nginx
server {
  listen 443 ssl;
  server_name graphnight.example.com;
  # ssl_certificate …;
  location / {
    proxy_pass http://127.0.0.1:8080;
    proxy_http_version 1.1;
    proxy_set_header Host $host;
    proxy_set_header X-Forwarded-For $proxy_add_x_forwarded_for;
    proxy_set_header X-Forwarded-Proto $scheme;
  }
}
```

Do not expose the raw GraphNight port to the public internet with production warehouse credentials.

## CORS

| `GRAPHNIGHT_CORS_ORIGINS` | Behavior |
|---------------------------|----------|
| unset / empty | Only `http://127.0.0.1:8080` and `http://localhost:8080` |
| comma-separated URLs | Exact allowlist |
| `*` | Any origin (**logs a warning**) |

## Health and metrics

```bash
curl -s http://127.0.0.1:8080/health
curl -s http://127.0.0.1:8080/metrics
```

`/health` returns `503` when metadata storage is unreachable.

## Env vars cheat sheet

| Variable | Purpose |
|----------|---------|
| `GRAPHNIGHT_API_KEYS` | `user:secret,…` API keys |
| `GRAPHNIGHT_ADMIN_KEYS` | Admin `user:secret,…` (also registered as API keys) |
| `GRAPHNIGHT_AUTH_REQUIRED` | `1` force auth even with no keys |
| `GRAPHNIGHT_DEV_OPEN` | `1` explicit open mode (silence warning) |
| `GRAPHNIGHT_OIDC_ISSUER` | Enable OIDC JWT / JWKS |
| `GRAPHNIGHT_OIDC_AUDIENCE` | Optional `aud` check |
| `GRAPHNIGHT_OIDC_CLIENT_ID` | Optional client id / `aud` |
| `GRAPHNIGHT_OIDC_ADMIN_CLAIM` | Admin claim name (default `roles`) |
| `GRAPHNIGHT_OIDC_ADMIN_VALUES` | Comma-separated admin claim values |
| `GRAPHNIGHT_OIDC_TENANT_CLAIM` | Tenant claim (default `tenant_id`) |
| `GRAPHNIGHT_CORS_ORIGINS` | Browser origin allowlist or `*` |
| `GRAPHNIGHT_AUDIT_LOG` | JSONL audit path (default `./graphnight_data/audit.jsonl`) |
| `GRAPHNIGHT_METADATA_DATABASE_URL` | Postgres metadata URL for `storage.type = "postgres"` |
| `GRAPHNIGHT_REQUIRE_SECRET_REFS` | `1` require `env:VAR` on GraphQL datasource writes |
| `GRAPHNIGHT_DATASOURCE_*` | Conventional env vars for `env:GRAPHNIGHT_DATASOURCE_*` refs |
| `GRAPHNIGHT_HOST` / `GRAPHNIGHT_PORT` / `GRAPHNIGHT_STORAGE_PATH` | Documented in `.env.example` (prefer CLI flags when both apply) |

See `.env.example` at the repo root for a copy-paste template.

## Secrets

Prefer:

```toml
[datasources.demo]
driver = "postgres"
connection_string = "env:GRAPHNIGHT_DATASOURCE_DEMO"
```

Vault / AWS Secrets Manager / GCP Secret Manager are **not** integrated — inject values into the process environment (or a thin wrapper) and use `env:` refs.

## Incomplete ops surface

- No Helm chart in-tree yet
- No OpenTelemetry traces
- No built-in rate limiting
- MCP / REST gateways not shipped (see ARCHITECTURE)
