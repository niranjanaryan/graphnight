#!/usr/bin/env python3
"""Create a temp datasource + model via the Python API, then dry-run a query.

Does not require an external database — only SQL generation is exercised.

Usage:
  python examples/python/03_init_and_query.py
"""

from __future__ import annotations

import tempfile
from pathlib import Path

from graphnight import GraphNightClient


def main() -> int:
    with tempfile.TemporaryDirectory(prefix="graphnight-py-") as td:
        storage = Path(td)
        client = GraphNightClient(storage_path=str(storage))

        client.create_datasource(
            {
                "name": "demo",
                "driver": "postgres",
                "connection_string": "postgresql://localhost/demo",
                "description": "Ephemeral demo datasource",
            }
        )
        client.create_model(
            {
                "name": "orders",
                "datasource": "demo",
                "description": "Orders created from Python",
                "measures": [
                    {
                        "formula": {"expression": "amount_usd", "label": "Revenue"},
                        "aggregation": "sum",
                    },
                    {"formula": "*", "aggregation": "count"},
                ],
                "dimensions": ["status", {"name": "customer_id", "label": "Customer"}],
                "time_dimensions": [
                    {"dimension": "created_at", "granularity": "day"},
                ],
            }
        )

        print("models:", [m["name"] for m in client.list_models()])
        print("datasources:", [d["name"] for d in client.list_datasources()])

        result = client.dry_run_query(
            {
                "name": "orders",
                "measures": [
                    {
                        "formula": {"expression": "amount_usd"},
                        "aggregation": "sum",
                    }
                ],
                "dimensions": [{"name": "status"}],
                "filters": [
                    {"field": "status", "operator": "eq", "value": "completed"},
                ],
                "limit": 25,
            }
        )
        print("--- dry-run SQL ---")
        print(result["sql"])
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
