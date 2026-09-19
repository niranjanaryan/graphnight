# Contributing

Thanks for interest in GraphNight. This project is an **alpha** semantic layer — please keep PRs focused and honest about what is / is not implemented.

## Development setup

```bash
git clone https://github.com/niranjanaryan/graphnight.git
cd graphnight
cargo test --workspace --exclude graphnight-python
```

## Workflow

1. Open an issue (or reference an existing one) for non-trivial changes.
2. Keep changes scoped — prefer one concern per PR.
3. Run before opening a PR:
   - `cargo fmt --all`
   - `cargo clippy --workspace --exclude graphnight-python --all-targets`
   - `cargo test --workspace --exclude graphnight-python`
4. Update docs when user-facing behavior changes (`README.md`, `docs/`, `LAUNCH.md`).
5. Do not claim production readiness for unfinished governance (auth/RLS/audit).

## Project map

| Path | Role |
|------|------|
| `crates/graphnight-core` | Domain models, formulas, joins, security types |
| `crates/graphnight-sql` | SQL generation + execution |
| `crates/graphnight-storage` | YAML / SQLite / search |
| `crates/graphnight-graphql` | GraphQL schema + resolvers |
| `crates/graphnight-cli` / `graphnight-server` | Binaries |
| `examples/` | Golden-path fixtures |
| `LAUNCH.md` | Release bars (OSS vs production) |

## Code style

- Match surrounding Rust style; run `cargo fmt`.
- Prefer failing loudly for unsupported alpha APIs over silent stubs.
- Keep comments short and for non-obvious constraints only.

## License

By contributing, you agree that your contributions are licensed under the Apache License 2.0.
