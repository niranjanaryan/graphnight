#!/usr/bin/env python3
"""Dry-run a semantic query and print generated SQL (no database required).

Usage:
  python examples/python/02_dry_run_query.py [storage_path]
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
    query = {
        "name": "orders",
        "measures": [
            {
                "formula": {"expression": "amount_usd", "label": "Revenue (USD)"},
                "aggregation": "sum",
            },
            {
                "formula": {"expression": "*", "label": "Orders"},
                "aggregation": "count",
            },
        ],
        "dimensions": [{"name": "status", "label": "Order Status"}],
        "limit": 100,
    }

    # generate_sql returns a string; dry_run_query returns {sql, name?}
    sql = client.generate_sql(query)
    dry = client.dry_run_query(query)
    print(f"model={dry.get('name')}")
    print("--- SQL ---")
    print(dry["sql"])
    assert dry["sql"] == sql
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
