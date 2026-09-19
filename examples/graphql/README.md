# GraphQL examples

Documents under this directory are GraphQL operations for `POST /graphql`.

| File | Operation |
|------|-----------|
| [`dry-run.graphql`](dry-run.graphql) | Semantic query with `dryRun: true` |
| [`list-models.graphql`](list-models.graphql) | List models |
| [`create-model.graphql`](create-model.graphql) | Admin `createModel` mutation |

Root also keeps [`../query.graphql`](../query.graphql) as a short combined sample.

## Curl one-liners

Start the server (open auth for local demos):

```bash
cargo run -p graphnight-server -- \
  --config examples/graphnight.toml \
  --storage-path examples/data \
  --host 127.0.0.1 --port 8080
```

**Dry-run** (inline JSON):

```bash
curl -s http://127.0.0.1:8080/graphql \
  -H 'content-type: application/json' \
  -d '{"query":"query { query(input: { name: \"orders\", measures: [{ formula: \"amount_usd\", aggregation: SUM, label: \"Revenue\" }], dimensions: [{ name: \"status\" }], limit: 10 }, dryRun: true) { sql columns executionTimeMs } }"}'
```

**List models:**

```bash
curl -s http://127.0.0.1:8080/graphql \
  -H 'content-type: application/json' \
  -d '{"query":"query { models { name datasource description } }"}'
```

**Create model** (admin key required when auth is on):

```bash
export GRAPHNIGHT_ADMIN_KEYS='admin:adminsecret'
# restart server with keys set…

QUERY=$(python3 -c 'import json,pathlib; print(json.dumps({"query": pathlib.Path("examples/graphql/create-model.graphql").read_text()}))')
curl -s http://127.0.0.1:8080/graphql \
  -H 'content-type: application/json' \
  -H 'Authorization: Bearer adminsecret' \
  -d "$QUERY"
```

Or paste the mutation body into GraphiQL / any GraphQL client pointed at `http://127.0.0.1:8080/graphql`.

Authenticated variants: [../auth/](../auth/).
