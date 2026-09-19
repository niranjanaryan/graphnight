#!/usr/bin/env python3
"""List semantic models from a GraphNight YAML storage directory.

Usage:
  python examples/python/01_list_models.py [storage_path]

Default storage_path is examples/data (repo demo models).
"""

from __future__ import annotations

import sys
from pathlib import Path

from graphnight import GraphNightClient


def main() -> int:
    root = Path(__file__).resolve().parents[2]
    default_storage = root / "examples" / "data"
    storage = Path(sys.argv[1]) if len(sys.argv) > 1 else default_storage

    client = GraphNightClient(storage_path=str(storage))
    models = client.list_models()
    print(f"storage={storage} models={len(models)}")
    for m in models:
        print(f"- {m['name']} (datasource={m['datasource']})")
        if m.get("description"):
            print(f"  {m['description']}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
