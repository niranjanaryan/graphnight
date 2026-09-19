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
- [ ] Engine sources committed and pushed to `origin/main`
- [ ] Tag `v0.1.0-alpha` after push
- [ ] GitHub description, topics (`semantic-layer`, `graphql`, `rust`), homepage
- [ ] Remove or clearly archive empty `api/` placeholder
- [ ] Decide fate of root `pyproject.toml` (aspirational FastAPI vs Rust-only for now)

### Product honesty

- [x] README states auth/RLS not enforced on live path
- [ ] Keep `ARCHITECTURE.md` as vision; mark unimplemented sections clearly
- [ ] Update `tasks.txt` or retire it in favor of this file

### Minimum quality gate

- [x] `cargo test --workspace --exclude graphnight-python` passes locally
- [ ] CI green on `main`
- [ ] Documented security warning for open GraphQL + connection strings

**Exit criteria:** clone → build → dry-run GraphQL/CLI works from docs alone; alpha labeling is unmistakable.

---

## B. Production customer launch

Goal: safe to put in front of real tenant data behind a controlled deployment.

### B1. Security (hard blockers)

- [ ] AuthN middleware (API key / JWT / OIDC) populates `user_id` / `tenant_id`
- [ ] AuthZ on every query and mutation (fail closed)
- [ ] Wire `PolicyEnforcer` into GraphQL/CLI execute path:
  - [ ] model / datasource allowlists
  - [ ] forced filters
  - [ ] RLS `row_filter`
  - [ ] max rows + query timeout
  - [ ] column masks on response
- [ ] Lock down datasource mutations (admin-only; no raw secrets from clients)
- [ ] Secret references (env / vault) instead of plaintext connection strings in APIs
- [ ] TLS (terminate at proxy or in-process); explicit CORS policy
- [ ] Rate limiting that actually enforces quotas
- [ ] Parameterized / safely bound SQL values; SQL injection review
- [ ] Durable audit log (not in-memory only)

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
- [ ] Connection pool acquire/idle/statement timeouts
- [ ] Query plan cache + result cache with invalidation
- [ ] True streaming for large results (no full materialization)
- [ ] Multi-stage DAG with topological sort (not sequential stub)
- [ ] Load test baselines and regression budgets
- [ ] HA story for metadata (shared store, not single-node YAML only)

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
