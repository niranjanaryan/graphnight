"""Unit tests for the GraphNight Python client (local YAML storage).

Requires a built extension module, e.g.:

    cd crates/graphnight-python
    maturin develop --uv  # or: maturin develop
    pytest
"""

from __future__ import annotations

import tempfile
from pathlib import Path

import pytest

graphnight = pytest.importorskip("graphnight")
GraphNightClient = graphnight.GraphNightClient


@pytest.fixture
def client(tmp_path: Path) -> GraphNightClient:
    return GraphNightClient(storage_path=str(tmp_path))


def _seed(client: GraphNightClient) -> None:
    client.create_datasource(
        {
            "name": "postgres",
            "driver": "postgres",
            "connection_string": "postgresql://localhost/test",
            "description": "Test warehouse",
            "pool_size": 5,
        }
    )
    client.create_model(
        {
            "name": "orders",
            "datasource": "postgres",
            "description": "Test orders model",
            "measures": [
                {
                    "formula": {"expression": "amount_usd", "label": "Revenue"},
                    "aggregation": "sum",
                },
                {
                    "formula": {"expression": "*", "label": "Orders"},
                    "aggregation": "count",
                },
            ],
            "dimensions": [
                {"name": "status", "label": "Status"},
                {"name": "store_id"},
            ],
            "time_dimensions": [
                {"dimension": "created_at", "granularity": "day"},
            ],
        }
    )


def test_client_new_temp_storage(tmp_path: Path) -> None:
    client = GraphNightClient(storage_path=str(tmp_path))
    assert client.list_models() == []
    assert client.list_datasources() == []


def test_create_and_list_models(client: GraphNightClient) -> None:
    _seed(client)
    models = client.list_models()
    assert len(models) == 1
    assert models[0]["name"] == "orders"
    assert models[0]["datasource"] == "postgres"
    assert "amount_usd" in models[0]["measures"]
    assert "status" in models[0]["dimensions"]

    by_ds = client.list_models(datasource="postgres")
    assert len(by_ds) == 1
    assert client.list_models(datasource="missing") == []


def test_get_model(client: GraphNightClient) -> None:
    _seed(client)
    model = client.get_model("orders")
    assert model is not None
    assert model["name"] == "orders"
    assert client.get_model("nope") is None


def test_list_datasources(client: GraphNightClient) -> None:
    _seed(client)
    ds = client.list_datasources()
    assert len(ds) == 1
    assert ds[0]["name"] == "postgres"
    assert ds[0]["driver"] == "postgres"


def test_generate_sql_and_dry_run(client: GraphNightClient) -> None:
    _seed(client)
    query = {
        "name": "orders",
        "measures": [
            {
                "formula": {"expression": "amount_usd", "label": "Revenue"},
                "aggregation": "sum",
            }
        ],
        "dimensions": [{"name": "status"}],
        "limit": 10,
    }
    sql = client.generate_sql(query)
    assert isinstance(sql, str)
    assert len(sql) > 0

    dry = client.dry_run_query(query)
    assert dry["sql"] == sql
    assert dry["name"] == "orders"


def test_memories(client: GraphNightClient) -> None:
    saved = client.save_memory(
        "Revenue spikes on Black Friday",
        ["revenue:sum"],
        id="mem_1",
        description="seasonal note",
    )
    assert saved["id"] == "mem_1"
    listed = client.list_memories(limit=10, offset=0)
    assert any(m["id"] == "mem_1" for m in listed)
    forgotten = client.forget_memory("mem_1")
    assert forgotten["success"] is True


def test_search(client: GraphNightClient) -> None:
    _seed(client)
    # YAML backend search may return empty; call must not raise.
    results = client.search("orders", limit=5)
    assert isinstance(results, list)


def test_storage_persists_on_disk(tmp_path: Path) -> None:
    first = GraphNightClient(storage_path=str(tmp_path))
    _seed(first)
    second = GraphNightClient(storage_path=str(tmp_path))
    assert len(second.list_models()) == 1
    assert len(second.list_datasources()) == 1


def test_tempfile_storage_roundtrip() -> None:
    with tempfile.TemporaryDirectory() as td:
        client = GraphNightClient(storage_path=td)
        client.create_datasource(
            {
                "name": "demo",
                "driver": "sqlite",
                "connection_string": "sqlite:///:memory:",
            }
        )
        client.create_model(
            {
                "name": "events",
                "datasource": "demo",
                "measures": [{"formula": "count", "aggregation": "count"}],
                "dimensions": ["kind"],
            }
        )
        assert client.list_models()[0]["name"] == "events"
