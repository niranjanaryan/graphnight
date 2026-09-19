# Getting started

GraphNight alpha: define semantic models in YAML, generate SQL, query via CLI or GraphQL.

## 1. Build

```bash
cd rust-engine
cargo build -p graphnight-cli -p graphnight-server
```

## 2. Scaffold a project

```bash
./target/debug/graphnight init ./my-project
cd my-project
```

This writes `graphnight.toml`, `graphnight_data/models.yaml`, `graphnight_data/datasources.yaml`, and `query.json`.

## 3. Dry-run a query (no database required)

```bash
graphnight --storage-path ./graphnight_data query dry-run --file ./query.json
```

You should see dialect SQL with `SUM` / `COUNT` / `GROUP BY`.

## 4. List models

```bash
graphnight --storage-path ./graphnight_data model list
```

## 5. Start the GraphQL server (optional)

```bash
graphnight-server \
  --config ./graphnight.toml \
  --storage-path ./graphnight_data \
  --host 127.0.0.1 \
  --port 8080
```

Then `POST http://127.0.0.1:8080/graphql` with `dryRun: true` (see `examples/query.graphql`).

## Security

Alpha servers have **no authentication**. Do not bind to a public interface or attach production credentials. Row-level security types exist in-library but are **not enforced** on the live path yet.

## Next

- [concepts.md](concepts.md) — models, measures, joins, formulas
- [../LAUNCH.md](../LAUNCH.md) — OSS vs production checklist
