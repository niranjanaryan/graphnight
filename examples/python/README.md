# GraphNight Python examples

These scripts use the local YAML-backed `GraphNightClient` from the PyO3 package
in `crates/graphnight-python/`.

## Setup

```bash
# From the repo root (needs Rust + Python 3.11+)
cd crates/graphnight-python
python3.11 -m venv .venv
source .venv/bin/activate
pip install maturin pytest
maturin develop
# optional: pip install -e '.[dev]'
```

Reuse an existing Cargo target dir to save disk:

```bash
export CARGO_TARGET_DIR=/path/to/graphnight/target
maturin develop
```

## Run

```bash
# From repo root, with the venv activated
python examples/python/01_list_models.py
python examples/python/02_dry_run_query.py
python examples/python/03_init_and_query.py
```

| Script | What it shows |
|--------|----------------|
| `01_list_models.py` | Load `examples/data` and list models |
| `02_dry_run_query.py` | `generate_sql` / `dry_run_query` on demo `orders` |
| `03_init_and_query.py` | `create_datasource` + `create_model` in a temp dir, then dry-run |

Executing against a live warehouse uses `client.query(...)` and needs a real
`connection_string` on the datasource.
