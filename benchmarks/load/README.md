# E5: Throughput / Load characteristics

Replicates the **E5** evaluation in the paper. The metrics below exercise the
real HTTP surface (axum + tracing + request-id + rate-limit + CORS layers) of
`graphnight-server`.

## Reference run (this paper)

Hardware: macOS 13, Apple Silicon (aarch64), release build.
Driver: `wrk` `-t8 -c128 -d10s`, single server process, YAML storage (empty),
auth required with one API key.

| Endpoint                                                       | req/s | avg latency | notes |
| -------------------------------------------------------------- | ----- | ----------- | ----- |
| `GET /health`                                                  | 175 354 | 0.75 ms | middleware + health body |
| `GET /metrics`                                                 | 180 864 | 0.66 ms | prometheus rendering |
| `GET /api/v1/models`                                           | 180 461 | 0.70 ms | storage read + serialization |
| `POST /api/v1/query` (authed, policy + compile)                | 182 489 | 0.63 ms | auth resolve + policy + model lookup + compile deny |
| `POST /api/v1/query` (anonymous → 401)                         | 187 116 | 0.55 ms | auth reject fast path |

Tail latency at saturation ≈ 20–36 ms max; ≥ 92 % of requests below 1 ms.

## Reproduce

```bash
cargo build --release -p graphnight-server

# serve: empty YAML storage, auth required, one key
export GRAPHNIGHT_DEV_OPEN=1
export GRAPHNIGHT_AUTH_REQUIRED=1
export GRAPHNIGHT_API_KEYS='loadtest=secret'
export GRAPHNIGHT_AUDIT_LOG_PATH=/tmp/gn-load/audit.jsonl
./target/release/graphnight-server --config graphnight.toml &
```

`wrk` scripts (checked in next to this README):

```bash
wrk -t8 -c128 -d10s http://127.0.0.1:8090/health
wrk -t8 -c128 -d10s http://127.0.0.1:8090/metrics
wrk -t8 -c128 -d10s http://127.0.0.1:8090/api/v1/models
wrk -t8 -c128 -d10s -s wrk_query.lua http://127.0.0.1:8090   # authed, 404 path
wrk -t8 -c128 -d10s -s wrk_anon.lua http://127.0.0.1:8090    # anonymous, 401 path
```

### Query-path load with a real datasource

The in-tree server currently binds a Postgres dialect for execution, so live
SQL query-path load requires a Postgres fixture:

1. `create schema` with the example `orders`/`customers` tables (see
   `examples/data/models.yaml`).
2. Point `datasources.yaml` / `graphnight.toml` at it.
3. `wrk -s wrk_query_live.lua` (list measures based on the fixture).

Until then, plan-compile and execution-throughput evidence comes from E1/E3
(`cargo bench`); this E5 table covers the HTTP + auth + policy surface.