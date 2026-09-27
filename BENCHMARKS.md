# Benchmarks

Reproducible benchmark harness for the GraphNight research paper. All numbers
in `docs/paper/paper.tex` (Tables 1–5) are produced by the commands below on a
macOS / Apple Silicon developer machine, Rust release profile
(`lto=true`, `codegen-units=1`).

## Layout

| Path | What |
|------|------|
| `crates/graphnight-sql/benches/sql_generate.rs` | E1: compilation latency, plan cache, query families |
| `crates/graphnight-sql/benches/policy_enforce.rs` | E3: policy enforcement overhead |
| `tests/e2e/tests/conformance.rs` | E2: cross-dialect execution conformance (SQLite vs DuckDB) |
| `tests/e2e/tests/pipeline_testcontainers.rs` | E2: Postgres/MySQL via testcontainers |
| `benchmarks/load/` | E5: HTTP throughput (`wrk` scripts + README) |
| `benchmarks/compare/README.md` | E4: governance matrix vs dbt MetricFlow / Cube |

## E1 — Compilation latency and cache behavior

```bash
cargo bench -p graphnight-sql --bench sql_generate
```

Measures (Criterion, 100 samples, release build):

- `sql_generate_orders` — cold compile of an `orders` query with a `ratio`
  measure plus a declarative join.
- `sql_plan_cache_hit` — warm path, identical query served from the plan cache.
- `e1_measures_{1,5,10,25,50}_cold` / `e1_dims_{1,5,10,20}_cold` /
  `e1_joins_{1,2,4}_cold` — query families scaling measures, dimensions, and
  declarative joins independently.
- `e1_measures50_dims20_joins4_cache_hit` — warm cache hit for the largest
  family.

### Measured medians (macOS / Apple Silicon, release build)

**Headline pair:**

| Path | Median latency |
|------|---------------|
| Cold SQL compile (ratio measure + join) | 43.8 µs |
| Plan-cache hit (fingerprint + canonical query) | 428 ns |
| Warm cache hit, 50 measures / 20 dims / 4 joins | 5.4 µs |

**Query families (cold compile, µs):**

| Family | 1 | 5 | 10 | 20 | 50 |
|--------|---|---|----|----|----|
| Measures | 44.6 | 46.6 | 48.8 | 56.7 | 69.4 |
| Dimensions | 44.9 | 46.7 | 59.1 | 53.9 | — |
| Joins (1/2/4) | 46.1 | 48.6 | 54.9 | | |

Cold compilation is dominated by query canonicalization + plan building
($\approx$44~\textmu s); the warm path is a hash + LRU lookup at 428 ns. The
largest family still serves from cache in 5.4~\textmu s — a $\approx$13$\times$
speedup over its own cold compile and $\approx$100$\times$ over the baseline cold
compile. These are single-node numbers on a dev machine; the relative ordering
matters more than the absolutes.

## E2 — Cross-dialect semantic conformance

```bash
cargo test -p graphnight-e2e --test conformance
```

Compiles six scenario queries (basic group-by, IN/GTE filters, day-granularity
time dimension, `ratio` formula with alias, joined-model dimension, compiled
policy forced-filter) for SQLite and DuckDB, executes both against identical
seeded data, and diffs canonical result tables:

| Scenario | SQLite | DuckDB |
|----------|:------:|:------:|
| Basic group-by + sum | ✓ | ✓ |
| Filters (IN / GTE) | ✓ | ✓ |
| Time dimension (day) | ✓ | ✓ |
| Ratio formula, aliased | ✓ | ✓ |
| Joined-model dimension | ✓ | ✓ |
| Policy forced filter | ✓ | ✓ |

Postgres and MySQL run via testcontainers in CI
(`tests/e2e/tests/pipeline_testcontainers.rs`); soft-skip when Docker is
unavailable. Building the harness surfaced five real cross-dialect bugs, all
since fixed and covered by the harness and unit tests.

## E3 — Policy enforcement overhead

```bash
cargo bench -p graphnight-sql --bench policy_enforce
```

Measures `enforce_policy` cost as a function of allow-list size (1, 100, 1k,
10k) and the incremental compile cost of a governed vs ungoverned query at a
1k allow-list.

**Enforcement cost by allow-list size:**

| Allow-list size | 1 | 100 | 1k | 10k |
|-----------------|-----|-----|-----|-----|
| Policy enforcement | 223 ns | 243 ns | 512 ns | 3.37 µs |

**Compile cost, 1k allow-list:**

| Path | Median |
|------|--------|
| Compile with policy (ACL + forced filter + RLS + cap) | 4.49 µs |
| Compile without policy | 2.17 µs |

Enforcement is in the high-nanosecond to low-microsecond range even for policy
tables two orders of magnitude larger than a typical tenant's — 512 ns at a
1,000-rule allow-list, which is <2% of the E1 cold-compile cost. Compiling under
a 1k-rule policy adds 2.3 µs over the ungoverned path, so governed compilation
remains dominated by the underlying SQL generation, not the policy machinery.

## E4 — Governance comparison

See `benchmarks/compare/README.md`. The governance matrix cells were verified
against vendor documentation (2026-09-26); source links and a representative
governed SQL sample are recorded there.

## E5 — End-to-end server throughput

```bash
cargo build --release -p graphnight-server

# empty YAML storage, auth required, one API key
GRAPHNIGHT_DEV_OPEN=1 \
GRAPHNIGHT_AUTH_REQUIRED=1 \
GRAPHNIGHT_API_KEYS='loadtest=secret' \
GRAPHNIGHT_AUDIT_LOG_PATH=/tmp/gn-load/audit.jsonl \
  ./target/release/graphnight-server --config examples/graphnight.toml &

wrk -t8 -c128 -d10s http://127.0.0.1:8080/health
wrk -t8 -c128 -d10s http://127.0.0.1:8080/metrics
wrk -t8 -c128 -d10s http://127.0.0.1:8080/api/v1/models
wrk -t8 -c128 -d10s -s benchmarks/load/wrk_query.lua http://127.0.0.1:8080
wrk -t8 -c128 -d10s -s benchmarks/load/wrk_anon.lua http://127.0.0.1:8080
```

Scripts and a reference run are in `benchmarks/load/`. The in-tree server
binds a Postgres dialect for execution, so live SQL query-path load requires a
Postgres fixture; the tables below cover the HTTP + auth + policy surface.

**Reference run (this paper):** macOS / Apple Silicon (aarch64), release build,
`wrk -t8 -c128 -d10s`, single server process, empty YAML storage, auth required
with one API key.

| Endpoint | req/s | avg latency |
|----------|-------|-------------|
| `GET /health` | 175 354 | 0.75 ms |
| `GET /metrics` | 180 864 | 0.66 ms |
| `GET /api/v1/models` | 180 461 | 0.70 ms |
| `POST /api/v1/query` (authed, policy + compile) | 182 489 | 0.63 ms |
| `POST /api/v1/query` (anonymous → 401) | 187 116 | 0.55 ms |

Rates cluster around 180k req/s regardless of surface, i.e. the service is
rate-limiter and protocol bound in this configuration, not policy or compile
bound: the measured compile+policy work (E1 + E3, tens of µs) is far below the
per-request service floor, so policy compilation does not register in
server-level latency.

## Reproducibility

All benchmark code is committed and version-pinned (Rust 1.75, edition 2021,
Apache-2.0). To reproduce every number in the paper from a clean checkout:

```bash
cargo bench -p graphnight-sql            # E1, E3
cargo test -p graphnight-e2e --test conformance  # E2
cargo build --release -p graphnight-server && <E5 recipe above>
```

Numbers vary between machines; the relative ordering and the headline claims
(tens of microseconds cold compile, sub-microsecond cache hit, sub-microsecond
policy enforcement, principal-safe caches) are the stable findings.