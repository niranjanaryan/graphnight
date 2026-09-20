# GraphNight Elixir Bindings

High-performance semantic layer for Elixir applications, powered by Rust via Rustler NIFs.

## Features

- **Native Performance**: Core engine runs in Rust, called via NIFs (zero-copy where possible)
- **Ecto Integration**: Optional helpers for using GraphNight with Ecto
- **Full GraphNight API**: Models, DataSources, Queries, Dry-run, Multi-stage DAG queries
- **Type Safety**: Elixir structs mirroring Rust types
- **Async**: Built on Tokio runtime

## Installation

### From Hex (when published)

```elixir
def deps do
  [
    {:graphnight, "~> 1.0"}
  ]
end
```

### From GitHub

```elixir
def deps do
  [
    {:graphnight, git: "https://github.com/niranjanaryan/graphnight.git", sparse: "elixir"}
  ]
end
```

Then run:

```bash
mix deps.get
mix compile
```

## Quick Start

```elixir
# Initialize the engine
{:ok, engine} = GraphNight.Client.init("./graphnight_data")

# Create a datasource
ds = GraphNight.DataSource.new(
  "analytics",
  "postgres",
  "postgresql://user:pass@localhost/db",
  description: "Analytics database"
)

{:ok, _ds} = GraphNight.Client.create_datasource(engine, ds)

# Create a model
model = %GraphNight.Model{
  name: "orders",
  datasource: "analytics",
  description: "Order transactions",
  measures: [
    %GraphNight.Measure{formula: "revenue:sum", aggregation: "SUM"},
    %GraphNight.Measure{formula: "order_count:count", aggregation: "COUNT"}
  ],
  dimensions: [
    %GraphNight.Dimension{name: "status"},
    %GraphNight.Dimension{name: "store_id"}
  ],
  time_dimensions: [
    %GraphNight.TimeDimension{dimension: "created_at", granularity: "DAY"}
  ],
  joins: []
}

{:ok, _model} = GraphNight.Client.create_model(engine, model)

# Execute a query
query = GraphNight.Client.build_query(
  name: "orders",
  measures: [GraphNight.Measure.new("revenue:sum")],
  dimensions: [GraphNight.Dimension.new("status")],
  filters: [GraphNight.Filter.eq("status", "completed")]
)

{:ok, result} = GraphNight.Client.execute_query(engine, query)

# Result contains:
# result.data - list of rows as maps
# result.columns - column names
# result.sql - generated SQL
# result.execution_time_ms - execution time
```

## Dry Run

```elixir
{:ok, result} = GraphNight.Client.dry_run_query(engine, query)
# result.sql contains the generated SQL without executing
```

## Ecto Integration

```elixir
# Convert GraphNight model to Ecto schema
schema_code = GraphNight.Ecto.model_to_ecto_schema(model)
# Generates Ecto schema code

# Convert GraphNight query to Ecto.Query
ecto_query = GraphNight.Ecto.query_to_ecto(query, MyApp.Orders)
# Returns an Ecto.Query that can be executed with Repo.all/1
```

## Architecture

```
┌─────────────────┐     NIF      ┌──────────────────┐
│   Elixir App    │ ──────────▶ │  Rust Engine     │
│                 │             │                  │
│  GraphNight.*   │             │  graphnight-core │
│  modules        │             │  graphnight-sql  │
│                 │             │  graphnight-     │
└─────────────────┘             │  storage         │
                                └──────────────────┘
```

The Rust engine handles:
- SQL generation (PostgreSQL, MySQL, SQLite, DuckDB)
- Formula parsing (revenue:sum, ratio(), time_shift(), etc.)
- Join walking and query optimization
- Connection pooling
- Query caching
- Tantivy full-text search

## Requirements

- Elixir 1.14+
- Rust 1.70+ (for compilation)
- Rustler 0.29+

## License

Apache-2.0