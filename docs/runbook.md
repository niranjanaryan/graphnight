# GraphNight operations runbook

Practical ops for the Axum `graphnight-server` binary (v1.0). Terminate TLS at a reverse proxy.

## Deploy

1. Build or pull the image (`Dockerfile` / `docker compose up --build`).
2. Mount durable storage (`/data` in the image maps to YAML/SQLite metadata + audit log).
3. Set secrets via env (never commit). Prefer `env:VARNAME` refs in `graphnight.toml`.
4. For HA metadata, use compose profile `ha` and `GRAPHNIGHT_METADATA_DATABASE_URL` (or `storage.type = "postgres"`).
5. Confirm:
   - `GET /health` → `200` with `"status":"ok"`
   - `GET /metrics` → Prometheus text
   - Authenticated `POST /graphql` with a dry-run query

Example:

```bash
export GRAPHNIGHT_API_KEYS='app:REDACTED'
export GRAPHNIGHT_ADMIN_KEYS='admin:REDACTED'
export GRAPHNIGHT_AUDIT_LOG=/data/audit.jsonl
export GRAPHNIGHT_CORS_ORIGINS=https://app.example.com
export GRAPHNIGHT_RATE_LIMIT_RPS=50
docker compose up -d --build
curl -sf http://127.0.0.1:8080/health
```

Config precedence for bind/storage: CLI flags > `graphnight.toml` > defaults.

## Rollback

1. Redeploy the previous image tag / binary (keep the same volume and env).
2. If a bad migration of metadata YAML/SQLite occurred, restore the volume snapshot taken before deploy.
3. Postgres metadata: restore from your DB backup; schema is `CREATE TABLE IF NOT EXISTS` on connect (no separate migrator yet).
4. Verify `/health` and a known GraphQL query; check audit log for unexpected writes during the bad window.

## Rotate API keys

1. Add the new `user:secret` pair to `GRAPHNIGHT_API_KEYS` / `GRAPHNIGHT_ADMIN_KEYS` (comma-separated) **alongside** the old key.
2. Restart the server (env is read at process start).
3. Roll clients to the new secret.
4. Remove the old pair from env and restart again.
5. Treat leaked keys as compromised: rotate immediately and review `GRAPHNIGHT_AUDIT_LOG` for that user id.

## Rotate / change OIDC

1. Update IdP client / audience as needed.
2. Set or change `GRAPHNIGHT_OIDC_*` (`ISSUER` required to enable; optional `AUDIENCE`, `CLIENT_ID`, admin/tenant claims).
3. Restart the server (JWKS client is built at startup).
4. Confirm a JWT Bearer request succeeds; keep API keys as a break-glass path if you rely on both.

## Disk audit log

- Path: `GRAPHNIGHT_AUDIT_LOG` (default `./graphnight_data/audit.jsonl`, or `/data/audit.jsonl` in Docker when storage is `/data`).
- Format: append-only JSONL. Rotate with `logrotate` / ship to your SIEM; do not truncate while the process holds the file open without coordinating a restart.
- Ensure the volume has disk headroom; a full disk stops durable audit appends.
- Sample inspect: `tail -n 50 "$GRAPHNIGHT_AUDIT_LOG" | jq .`

## Health and metrics checks

| Check | Expect |
| --- | --- |
| `GET /health` | `200`, `status=ok`, `storage.ok=true` |
| `GET /health` when metadata broken | `503`, `status=unavailable` |
| `GET /metrics` | Prometheus counters (queries, cache, rows) |
| Logs | HTTP spans include `request_id` (also echoed as `x-request-id`) |

Synthetic probe (auth optional when open / DEV_OPEN):

```bash
curl -sf -H "x-request-id: probe-$(date +%s)" http://127.0.0.1:8080/health
curl -sf http://127.0.0.1:8080/metrics | head
```

Rate limit (`GRAPHNIGHT_RATE_LIMIT_RPS`): when exceeded, GraphQL returns `429` with `Retry-After`. `/health` and `/metrics` are exempt.

## Incident basics

1. **Capture**: note `x-request-id` from client/proxy logs; grep process logs and audit JSONL for that id / user / tenant.
2. **Contain**: disable public ingress; set `GRAPHNIGHT_AUTH_REQUIRED=1` if somehow open; rotate keys/OIDC client secret if credential leak suspected.
3. **Rate abuse**: lower `GRAPHNIGHT_RATE_LIMIT_RPS` or block offender IPs at the proxy; restart to clear in-process buckets if needed.
4. **Bad query / warehouse load**: kill long DB sessions on the warehouse side; GraphNight statement timeouts help but warehouse-side limits are authoritative.
5. **Corrupt / missing metadata**: restore volume or Postgres backup; confirm `/health` model count.
6. **Post-incident**: rotate credentials, preserve audit log slice, file GitHub security advisory if a product vulnerability is confirmed (`SECURITY.md`).

## Env quick reference

See `.env.example` for the full list (auth, OIDC, CORS, audit, rate limit, metadata URL, secret refs).
