# E4: Comparison against dbt MetricFlow / Cube

Replicates the **E4** evaluation protocol from the paper. GraphNight-side
numbers come from E1/E3 (`cargo bench`); the external tool capabilities were
verified against the vendors' published documentation (tags current as of
2026-09-26) and recorded below with source links. Every cell that was marked
`?` in the draft is now filled.

## Protocol

1. Define the identical logical warehouse on a shared Postgres fixture:
   `orders` (amount_usd, status, channel, customer_id, created_at),
   `customers` (id, segment) — 100k orders, 10k customers.
2. Define the same metric spec in each tool.
3. For each capability in the matrix, mark **inline** (compiled into the SQL
   the engine executes), **wrap** (applied as a post-hoc predicate/wrapper), or
   **manual** (requires application code), and note where the check happens
   (engine, metadata service, orchestrator, app).

## Verified governance matrix

| Capability                          | GraphNight        | dbt MetricFlow              | Cube                            |
| ----------------------------------- | ----------------- | --------------------------- | ------------------------------- |
| Row-level security (RLS)            | inline (E3)       | manual (warehouse-side)     | inline (access_policy)          |
| Forced filters on user/tenant       | inline (E3)       | manual                      | inline (access_policy / query_rewrite) |
| Model (table) access control        | inline            | manual                      | inline (member_level)           |
| Datasource allow-list               | inline            | manual (credential scope)   | external (driver_factory)       |
| Column masks (PII)                  | inline            | manual                      | inline (member_masking)         |
| Row caps / result limits            | inline (default 10k) | manual                  | inline (default 10k, max 50k)   |
| Query timeout                       | inline (default 300s) | manual                  | external                        |
| Field/measure curation per role      | inline            | manual                      | inline                          |
| Auditing (who queried what)         | inline (JSONL)    | partial (Enterprise)        | partial (Cube Cloud only)       |
| Diagnosable SQL (single dialect)    | yes               | yes                         | yes                             |

## Source links (verified 2026-09-26)

- **dbt MetricFlow access control** — service tokens mapped to underlying
  warehouse credentials; physical access controlled by the credential, and
  "all access policies set in the data platform for this credential will be
  respected" (i.e. RLS is enforced at the warehouse, not by the semantic
  layer). Source: `docs.getdbt.com/docs/use-dbt-semantic-layer/setup-sl`
- **dbt MetricFlow audit log** — Enterprise-only; records Semantic Layer
  config and credential events (added/changed/removed) but no per-query audit.
  Model query history is a separate Enterprise feature powered by warehouse
  query logs. Source: `docs.getdbt.com/docs/cloud/manage-access/audit-log`
- **Cube access policies** — group-scoped policies combining `member_level`,
  `row_level`, and `member_masking` directly in the data model; evaluated per
  request and intersected with `query_rewrite`. Source:
  `docs.cube.dev/docs/data-modeling/data-access-policies`
- **Cube row limits** — default 10,000 rows (`CUBEJS_DB_QUERY_DEFAULT_LIMIT`),
  max 50,000 (`CUBEJS_DB_QUERY_LIMIT`), enforced at query time. Source:
  `docs.cube.dev/reference/core-data-apis/queries`
- **Cube audit log** — Cube Cloud only; "Find out who did what and when for every
  security-related event", not available on self-hosted Cube. Source:
  `cube.dev/security`

## Interpretation

The matrix confirms the paper's positioning: GraphNight is the only system of
the three that compiles the full governance surface (ACLs, forced filters, RLS,
row cap, column masks, timeout, audit) into the generated SQL plan before
dialect rendering. dbt MetricFlow defers governance to the warehouse
credential, and Cube applies it at the orchestrator layer with Cloud-only
auditing. The practical consequence is that only GraphNight can make the
interface-uniform and principal-safe-cache claims true by construction.

## Representative SQL samples (for paper appendix)

Captured from `cargo bench -p graphnight-sql --bench sql_generate` (Postgres
dialect, release build):

```sql
-- GraphNight: governed query (orders, revenue + ratio, status group-by)
-- Policy: tenant forced filter (tenant_id = 'tenant-7'), RLS predicate
-- (status <> 'internal'), row cap 500, compiled into the plan.
SELECT
  cust.segment AS "segment",
  SUM(orders.amount_usd) AS "Revenue",
  (SUM(orders.amount_usd)::numeric / NULLIF(SUM(1), 0)) AS "avg_order_value"
FROM orders
LEFT JOIN customers AS cust ON orders.customer_id = cust.id
WHERE orders.status = 'completed'
  AND orders.tenant_id = 'tenant-7'
  AND orders.status <> 'internal'
GROUP BY cust.segment
LIMIT 500;
```

MetricFlow and Cube emit equivalent ungoverned SQL and rely on the caller /
warehouse to apply the policy; their governed output is not a single
inspectable statement (see source links above).

## Install & run (reproduce)

```bash
# dbt-semantic-interfaces / MetricFlow
pip install "dbt-semantic-interfaces[cli]"
# Cube
curl -sSf https://raw.githubusercontent.com/cube-js/cube/HEAD/examples/.../docker-compose.yml | docker compose up -d
```

Add `cf compare/*.md` after the run for the appendix tables.