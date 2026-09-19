# GraphNight launch checklist

Two bars: **OSS v0.1 (usable alpha)** and **production customer launch**.  
Do not claim production readiness until the production section is complete.

Last reviewed: 2026-09-20

---

## A. OSS v0.1 — usable public alpha

Goal: strangers can clone, build, run a dry-run query, and understand limits.

### Repo hygiene

- [x] Apache-2.0 `LICENSE` at repo root
- [x] Workspace crate license aligned to Apache-2.0
- [x] Root `.gitignore` (target, `.DS_Store`, `codedb.snapshot`, venv, `.env`)
- [x] `README.md` with alpha warning + quickstart
- [x] Example config / models / queries under `examples/`
- [x] GitHub Actions CI (fmt / clippy / test / binary build)
- [x] Engine sources committed on local `main`
- [x] Push local `main` to `origin/main`
- [x] Tag `v0.1.0-alpha` after push
- [x] GitHub description + topics (`semantic-layer`, `graphql`, `rust`, `sql`, `analytics`)
- [x] Empty `api/` / placeholder dirs removed; workspace at repo root (`crates/`)
- [x] Root `pyproject.toml` is a non-publishable workspace marker (Python = experimental)
- [x] `CONTRIBUTING.md` + `SECURITY.md`

### Product honesty

- [x] README states auth/RLS not enforced on live path
- [x] `ARCHITECTURE.md` bannered as vision (not status)
- [x] `tasks.txt` points at this file
- [x] `ingestModels` / `multiStageQuery` fail loudly (unsupported in alpha)
- [x] Formula parser + join walker wired into SQL generation
- [x] CLI `graphnight init` + `docs/getting-started.md`

### Minimum quality gate

- [x] `cargo test --workspace --exclude graphnight-python` passes locally
- [x] Example YAML dry-run integration test
- [ ] CI green on `main`
- [x] Documented security warning for open GraphQL + connection strings

**Exit criteria:** clone → build → dry-run GraphQL/CLI works from docs alone; alpha labeling is unmistakable.

---

## B. Production customer launch

Goal: safe to put in front of real tenant data behind a controlled deployment.

### B1. Security (hard blockers)

- [x] AuthN middleware (API key) populates `user_id` / `tenant_id` / `is_admin`
- [x] AuthZ when keys configured (fail closed); open mode only without keys or with `GRAPHNIGHT_DEV_OPEN=1`
- [x] Wire `PolicyEnforcer` into GraphQL execute path:
  - [x] model / datasource allowlists
  - [x] forced filters
  - [x] RLS `row_filter`
  - [x] max rows
  - [ ] query timeout enforced end-to-end
  - [ ] column masks on response
- [x] Lock down datasource/model mutations (admin-only when auth required)
- [ ] Secret references (env / vault) instead of plaintext connection strings in APIs
- [ ] TLS (terminate at proxy or in-process); CORS is permissive today
- [ ] Rate limiting that actually enforces quotas
- [ ] Parameterized / safely bound SQL values; SQL injection review
- [x] Durable audit log (JSONL via `GRAPHNIGHT_AUDIT_LOG`, default `./graphnight_data/audit.jsonl`)

### B2. Observability & ops

- [ ] Deep `/health` (storage + DB pool checks)
- [ ] Prometheus `/metrics` (QPS, latency, errors, pool usage)
- [ ] OpenTelemetry traces for plan → SQL → execute
- [ ] Structured logging with request IDs
- [ ] Dockerfile + compose (and optionally Helm)
- [ ] Config via env; no secrets in git
- [ ] Backup / restore for metadata storage
- [ ] Runbook: deploy, rollback, rotate credentials, incident response
- [ ] SBOM + dependency vulnerability scanning in CI

### B3. Reliability & performance

- [ ] Integration tests against real Postgres/MySQL (testcontainers)
- [x] Connection pool acquire/idle + statement timeouts (Postgres/MySQL)
- [x] Query plan cache + result cache with invalidation on model mutate
- [x] Streaming fetch in executor (GraphQL still buffers each response payload)
- [ ] Multi-stage DAG with topological sort (not sequential stub)
- [x] Criterion bench baseline (`graphnight-sql`)
- [ ] HA story for metadata (shared store, not single-node YAML only)
- [x] Prometheus `/metrics` counters (queries, cache hits/misses, rows)

### B4. Product surface (production-expected)

- [ ] Stable versioned API + changelog
- [ ] REST and/or MCP for agent integrations (as committed in blueprint)
- [ ] Python package with working tests (or explicitly unsupported)
- [ ] Importers or migration guides (dbt / Cube) as needed by customers
- [ ] Support channel + security contact (`SECURITY.md`)

**Exit criteria:** security is enforced on the live path; metrics/audit/health are real; containerized deploy is documented; load and integration tests pass; customers are not relying on alpha warnings.

---

## Suggested sequence

```text
1. Finish OSS A (commit engine, push, tag alpha)     ← current focus
2. B1 auth + PolicyEnforcer on query path
3. B2 metrics / Docker / CI security scanning
4. B3 cache + e2e + timeouts
5. B4 APIs / Python / importers as demand requires
6. Production launch announcement
```

## Explicit non-goals until B is done

- Hosting a multi-tenant SaaS on this binary
- Attaching production warehouse credentials to an exposed GraphQL port
- Marketing as “governed semantic layer” without live policy enforcement
