# High-availability metadata (Postgres)

Use shared Postgres metadata so multiple GraphNight replicas read/write the same models and datasources.

## Config

See [`../graphnight.postgres.toml`](../graphnight.postgres.toml):

```toml
[storage]
type = "postgres"
path = "env:GRAPHNIGHT_METADATA_DATABASE_URL"
```

## Docker Compose (`--profile ha`)

```bash
# metadata on host :5433, server on :8081
docker compose --profile ha up --build metadata-db server-postgres
```

- `metadata-db` — Postgres 16 (`graphnight` / `graphnight` / `graphnight_meta`)
- `server-postgres` — GraphNight with `examples/graphnight.postgres.toml` mounted; listens on **8081**

```bash
curl -s http://127.0.0.1:8081/health
export GRAPHNIGHT_METADATA_DATABASE_URL='postgresql://graphnight:graphnight@127.0.0.1:5433/graphnight_meta'
```

## Local binary + Compose DB only

```bash
docker compose --profile ha up -d metadata-db
export GRAPHNIGHT_METADATA_DATABASE_URL='postgresql://graphnight:graphnight@127.0.0.1:5433/graphnight_meta'
cargo run -p graphnight-server -- \
  --config examples/graphnight.postgres.toml \
  --host 127.0.0.1 --port 8080
```

Schema tables are created on connect (`CREATE TABLE IF NOT EXISTS`). Model search on this backend is SQL `ILIKE` (not Tantivy). Warehouse datasources remain separate from the metadata DB.
