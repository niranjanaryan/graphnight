# Concepts

## Model

A semantic table (or SQL view) with measures, dimensions, time dimensions, and optional joins. Stored in:

- YAML (`models.yaml` under the storage path)
- SQLite metadata backend
- Shared Postgres metadata (`storage.type = "postgres"`) for HA
- Or via GraphQL `createModel` (admin when auth is required)

Example shape (see `examples/data/models.yaml`):

```yaml
- name: orders
  datasource: demo
  measures:
    - formula: { expression: amount_usd, label: Revenue (USD) }
      aggregation: sum
  dimensions:
    - name: status
  joins:
    - name: customers
      model: customers
      join_type: left
      on: [[customer_id, id]]
```

## Measure

An aggregation over a field or formula:

- Structured: expression `amount_usd` + aggregation `sum`
- Shorthand formula string: `revenue:sum`, `*:count`
- Functions: `ratio(revenue:sum, cost:sum)`, `time_shift(revenue:sum, -1, 'month')`

Invalid aggregations (e.g. `revenue:nope`) fail at SQL generation time.

## Dimension / time dimension

Group-by fields. Time dimensions use `DATE_TRUNC` with a granularity (`day`, `month`, …).

## Join

Declared on a model. The SQL generator uses the join graph walker to emit `JOIN` clauses for declared targets (multi-hop paths when models are registered together).

## Query

JSON (CLI / Python) or GraphQL input selecting a model (`name` or `sourceModel`), measures, dimensions, filters, limit.

- **Dry-run** (`query dry-run` / GraphQL `dryRun: true`) returns SQL only — no warehouse connection required
- Live execute needs a reachable datasource and valid credentials

## Auth modes

Configured via environment (see [auth.md](auth.md)):

| Mode | When | Behavior |
|------|------|----------|
| Open (default) | No keys, no OIDC, `DEV_OPEN` unset | GraphQL accepts anonymous requests; server logs a warning |
| Explicit open | `GRAPHNIGHT_DEV_OPEN=1` | Same as open; warning silenced — local demos only |
| API keys | `GRAPHNIGHT_API_KEYS` / `GRAPHNIGHT_ADMIN_KEYS` | Bearer or `X-API-Key`; anonymous rejected |
| OIDC JWT | `GRAPHNIGHT_OIDC_ISSUER` set | JWKS validation for JWT-shaped Bearer tokens; falls back to API keys |
| Force closed | `GRAPHNIGHT_AUTH_REQUIRED=1` | Auth required even with no keys configured |

When auth is required: datasource/model write mutations need an **admin** identity; queries run through `PolicyEnforcer`.

`X-Tenant-Id` (or an OIDC tenant claim) adds a forced `tenant_id` equality filter via `SessionPolicy`.

## PolicyEnforcer

On the live GraphQL query path, `PolicyEnforcer` applies the session policy:

- Model / datasource allow and deny lists
- Forced filters (including tenant)
- RLS `row_filter`
- Max rows (default policy cap 10_000)

**Incomplete:** column masks on response payloads and end-to-end query timeout enforcement are not fully wired (fields exist on `SessionPolicy`).

## Storage backends

Configured under `[storage]` in `graphnight.toml` (and/or `--storage-path`):

| `type` | Use | Notes |
|--------|-----|-------|
| `yaml` | Local / single-node | Directory with `models.yaml`, `datasources.yaml`; Tantivy search when available |
| `sqlite` | Local embedded metadata | File-backed store |
| `postgres` | Multi-replica / HA | Shared metadata; `path` or `GRAPHNIGHT_METADATA_DATABASE_URL`; search is `ILIKE`-only |

`path` may be a filesystem path, a raw `postgresql://…` URL, or `env:VARNAME`. Schema tables for Postgres are created on connect.

Warehouse datasources are separate from metadata storage (e.g. demo SQLite warehouse vs Postgres metadata DB).

## Caching

In-process caches on `SqlEngine` (server / SQL crate):

| Cache | Default | Behavior |
|-------|---------|----------|
| Plan cache | LRU ~512 | Caches generated SQL for identical query keys |
| Result cache | LRU ~256, TTL ~60s | Caches row sets; invalidated on model mutations |

Hit/miss counters export on Prometheus `GET /metrics`. Caches are **per process** — not shared across replicas. Prefer Postgres metadata for shared model definitions; do not assume shared result cache in HA.

## Unsupported / incomplete

| Feature | Status |
|---------|--------|
| `ingestModels` (DB introspection) | Error — use YAML / `createModel` |
| `multiStageQuery` DAG | Error — not implemented |
| Vault / cloud secret managers | Not integrated — use `env:` refs |
| MCP / REST agent APIs | Not implemented |
| Live subscription usefulness | WebSocket route exists; treat as early |
| Column masks / full query timeout | Incomplete |
| Python remote GraphQL client | Local YAML/SQL bindings only |
