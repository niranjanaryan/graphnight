# Getting started

GraphNight alpha: define semantic models in YAML, generate SQL, query via CLI or GraphQL.

## 1. Build

```bash
cargo build -p graphnight-cli -p graphnight-server
```

## 2. Scaffold a project

```bash
cargo run -p graphnight-cli -- init ./my-project
cd my-project
```

This writes `graphnight.toml`, `graphnight_data/models.yaml`, `graphnight_data/datasources.yaml`, and `query.json`.

## 3. Dry-run a query (no database required)

```bash
cargo run -p graphnight-cli -- \
  --storage-path ./graphnight_data \
  query dry-run --file ./query.json
```

(From inside `my-project`, use the installed binary path or run from the repo with absolute storage paths.)

Using the repo binary after `cargo build`:

```bash
../target/debug/graphnight --storage-path ./graphnight_data query dry-run --file ./query.json
```

You should see dialect SQL with `SUM` / `COUNT` / `GROUP BY` (and joins when models declare them).

## 4. List models

```bash
../target/debug/graphnight --storage-path ./graphnight_data model list
```

## 5. Start the GraphQL server (optional)

```bash
../target/debug/graphnight-server \
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
