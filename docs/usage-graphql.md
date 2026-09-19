# GraphQL usage

Default endpoint: `POST http://127.0.0.1:8080/graphql`  
Browser IDE: open the same URL in a browser for **GraphiQL** (Axum serves GraphiQL on `GET /graphql`).

Also:

| Path | Purpose |
|------|---------|
| `GET /health` | Deep health (JSON; `503` if metadata storage fails) |
| `GET /metrics` | Prometheus counters (queries, cache hits/misses, rows) |
| `/graphql/ws` | GraphQL WebSocket subscriptions (early) |

## Dry-run with curl

No warehouse required when `dryRun: true`:

```bash
curl -s http://127.0.0.1:8080/graphql \
  -H 'content-type: application/json' \
  -d @- <<'EOF'
{
  "query": "query { query(input: { name: \"orders\", measures: [{ formula: \"amount_usd\", aggregation: SUM, label: \"Revenue\" }], dimensions: [{ name: \"status\" }], limit: 10 }, dryRun: true) { sql columns executionTimeMs } }"
}
EOF
```

Or send a document file (see `examples/query.graphql`):

```bash
# GraphiQL paste, or wrap the operation in a JSON body:
curl -s http://127.0.0.1:8080/graphql \
  -H 'content-type: application/json' \
  -d '{"query":"query { models { name datasource description } }"}'
```

## Auth headers

When API keys or OIDC are configured (and `GRAPHNIGHT_DEV_OPEN` is not set), anonymous operations fail.

API key (either header):

```bash
curl -s http://127.0.0.1:8080/graphql \
  -H 'content-type: application/json' \
  -H 'Authorization: Bearer secret1' \
  -d '{"query":"query { models { name } }"}'

curl -s http://127.0.0.1:8080/graphql \
  -H 'content-type: application/json' \
  -H 'X-API-Key: secret1' \
  -d '{"query":"query { models { name } }"}'
```

OIDC: send `Authorization: Bearer <jwt>`. JWT-shaped tokens are validated via JWKS when `GRAPHNIGHT_OIDC_ISSUER` is set; otherwise / on failure the server falls back to API key matching.

Optional tenant header (forced `tenant_id` filter when policy applies):

```bash
-H 'X-Tenant-Id: acme'
```

Admin keys (`GRAPHNIGHT_ADMIN_KEYS`) are required for datasource/model write mutations when auth is required.

Full matrix: [auth.md](auth.md).

## Example operations

### List models

```graphql
query ListModels {
  models {
    name
    datasource
    description
  }
}
```

### Dry-run semantic query

```graphql
query DryRunOrders {
  query(
    input: {
      name: "orders"
      measures: [
        { formula: "amount_usd", aggregation: SUM, label: "Revenue (USD)" }
        { formula: "*", aggregation: COUNT, label: "Orders" }
      ]
      dimensions: [{ name: "status", label: "Order Status" }]
      limit: 100
    }
    dryRun: true
  ) {
    sql
    columns
    executionTimeMs
  }
}
```

Omit `dryRun` (or set `false`) to execute against the model's datasource.

### Unsupported mutations / queries

These fail loudly in 1.0:

- `ingestModels` — use YAML or `createModel`
- `multiStageQuery` — DAG not implemented

## GraphiQL

1. Start `graphnight-server` on `127.0.0.1:8080`
2. Open `http://127.0.0.1:8080/graphql`
3. For authenticated servers, set HTTP headers in GraphiQL (e.g. `Authorization: Bearer secret1`)

CORS defaults to localhost origins only. Override with `GRAPHNIGHT_CORS_ORIGINS` — see [deploy.md](deploy.md).
