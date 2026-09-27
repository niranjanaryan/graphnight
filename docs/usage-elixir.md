# GraphNight Elixir Bindings - Usage Guide

This guide covers using GraphNight from Elixir applications via the native Rustler NIF bindings.

## Installation

```elixir
def deps do
  [
    {:graphnight, "~> 1.0"}
  ]
end
```

Run `mix deps.get && mix compile`.

## Quick Start

```elixir
# 1. Initialize the engine (creates ./graphnight_data for YAML storage)
{:ok, engine} = GraphNight.Client.init("./graphnight_data")

# 2. Create a datasource
ds = GraphNight.DataSource.new(
  "analytics",
  "postgres",
  "postgresql://user:pass@localhost/db",
  description: "Analytics database",
  pool_size: 10
)
GraphNight.Client.create_datasource(engine, ds)

# 3. Define a model
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
GraphNight.Client.create_model(engine, model)

# 4. Execute a query
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

## Core Concepts

### Engine
The `Engine` is a handle to the Rust NIF resource. It manages:
- SQL engine (dialect, connection pooling, caching)
- Storage backend (YAML, SQLite, etc.)
- Tokio runtime for async operations

Create once per application, reuse for all operations.

### DataSource
Represents a database connection:

```elixir
%GraphNight.DataSource{
  name: "analytics",
  driver: "postgres" | "mysql" | "sqlite",
  connection_string: "postgresql://...",
  description: "optional",
  models: [],  # Auto-populated
  pool_size: 10
}
```

### Model
Semantic definition of a dataset:

```elixir
%GraphNight.Model{
  name: "orders",
  datasource: "analytics",
  description: "Order transactions",
  measures: [...],
  dimensions: [...],
  time_dimensions: [...],
  joins: [...]
}
```

### Measures
Aggregations over columns:

```elixir
# Shorthand
%GraphNight.Measure{formula: "revenue:sum", aggregation: "SUM"}

# Advanced formulas
%GraphNight.Measure{formula: "running_total(revenue:sum)", aggregation: "SUM"}
%GraphNight.Measure{formula: "pct_change(revenue:sum)", aggregation: "SUM"}
%GraphNight.Measure{formula: "time_shift(revenue:sum, 1, 'MONTH')", aggregation: "SUM"}
%GraphNight.Measure{formula: "ratio(profit:sum, revenue:sum)", aggregation: "SUM"}

# Aggregation types: SUM, AVG, COUNT, MIN, MAX, COUNT_DISTINCT, CUSTOM
```

### Dimensions
Group-by columns:

```elixir
%GraphNight.Dimension{name: "status", label: "Order Status"}
%GraphNight.Dimension{name: "store_id"}
```

### Time Dimensions
Temporal grouping with granularity:

```elixir
%GraphNight.TimeDimension{
  dimension: "created_at",
  granularity: "DAY" | "HOUR" | "WEEK" | "MONTH" | "QUARTER" | "YEAR",
  label: "Month"
}
```

### Joins
Relationships to other models:

```elixir
%GraphNight.Join{
  name: "orders_to_customers",
  model: "customers",
  join_type: "LEFT" | "INNER" | "RIGHT" | "FULL",
  on: [{"customer_id", "id"}],
  alias: "customers"
}
```

## Query Building

### GraphNight.Client.build_query/1

```elixir
query = GraphNight.Client.build_query(
  name: "orders",
  source_model: %GraphNight.SourceSpec{model: "orders", alias: "o"},
  measures: [
    GraphNight.Measure.new("revenue:sum"),
    GraphNight.Measure.new("order_count:count")
  ],
  dimensions: [
    GraphNight.Dimension.new("status"),
    GraphNight.Dimension.new("store_id")
  ],
  time_dimensions: [
    GraphNight.TimeDimension.new("created_at", granularity: "MONTH")
  ],
  filters: [
    GraphNight.Filter.eq("status", "completed"),
    GraphNight.Filter.gte("created_at", "2024-01-01"),
    GraphNight.Filter.lte("created_at", "2024-12-31"),
    GraphNight.Filter.in("store_id", [1, 2, 3])
  ],
  order: [
    GraphNight.OrderBy.desc("revenue:sum"),
    GraphNight.OrderBy.asc("status")
  ],
  limit: 100,
  offset: 0,
  whole_periods_only: true,
  distinct_dimension_values: false,
  stage_ref: nil  # For multi-stage DAG queries
)
```

### Filter Operators

```elixir
GraphNight.Filter.eq(field, value)
GraphNight.Filter.neq(field, value)
GraphNight.Filter.gt(field, value)
GraphNight.Filter.gte(field, value)
GraphNight.Filter.lt(field, value)
GraphNight.Filter.lte(field, value)
GraphNight.Filter.like(field, value)
GraphNight.Filter.ilike(field, value)
GraphNight.Filter.in(field, values)
GraphNight.Filter.not_in(field, values)
GraphNight.Filter.is_null(field)
GraphNight.Filter.is_not_null(field)
GraphNight.Filter.between(field, start, finish)
GraphNight.Filter.not_between(field, start, finish)

# OR conditions
GraphNight.Filter.eq("status", "completed") |> GraphNight.Filter.or()
```

## Query Execution

### Execute Query
```elixir
{:ok, result} = GraphNight.Client.execute_query(engine, query)

# result.data      - List of rows as maps
# result.columns   - Column names
# result.sql       - Generated SQL
# result.execution_time_ms - Execution time in ms
```

### Dry Run (SQL Generation Only)
```elixir
{:ok, result} = GraphNight.Client.dry_run_query(engine, query)
IO.puts(result.sql)
# SELECT status, SUM(revenue) AS "revenue:sum"
# FROM orders orders
# WHERE status = 'completed'
# GROUP BY status
```

### Multi-Stage DAG Queries
Execute multiple queries as a DAG with dependencies:

```elixir
# Stage 1: Top 10 customers by revenue
stage1 = GraphNight.Client.build_query(
  name: "orders",
  measures: [GraphNight.Measure.new("revenue:sum")],
  dimensions: [GraphNight.Dimension.new("customer_id")],
  order: [GraphNight.OrderBy.desc("revenue:sum")],
  limit: 10
)

# Stage 2: Drill down into top customers' orders (references stage1)
stage2 = GraphNight.Client.build_query(
  name: "orders",
  measures: [GraphNight.Measure.new("revenue:sum")],
  dimensions: [GraphNight.Dimension.new("product_id")],
  filters: [GraphNight.Filter.eq("customer_id", "{{stage1.customer_id}}")],
  stage_ref: "stage1"
)

{:ok, %{results: results}} = GraphNight.Client.execute_dag(engine, [stage1, stage2])

# results[0] - top 10 customers
# results[1] - products for each top customer
```

## Security: Session Policies

Apply row-level filters, column masks, and limits:

```elixir
policy = %GraphNight.SessionPolicy{}
  |> GraphNight.SessionPolicy.with_forced_filter(%GraphNight.Filter{
    field: "tenant_id",
    operator: "EQ",
    value: "tenant_123"
  })
  |> GraphNight.SessionPolicy.with_column_mask("email", &GraphNight.Masks.email_mask/1)
  |> GraphNight.SessionPolicy.with_column_mask("ssn", fn _ -> "***-**-****" end)
  |> GraphNight.SessionPolicy.with_max_rows(10000)
  |> GraphNight.SessionPolicy.with_query_timeout_secs(30)

# Apply policy during query execution
GraphNight.Client.execute_query_with_policy(engine, query, policy)
```

### Built-in Column Masks
```elixir
GraphNight.Masks.email_mask("john.doe@example.com")
# "j***e@example.com"
```

## Error Handling

```elixir
case GraphNight.Client.execute_query(engine, query) do
  {:ok, result} ->
    Enum.each(result.data, &process_row/1)
  
  {:error, reason} ->
    case reason do
      "Model not found: ..." -> {:error, :model_not_found}
      "Datasource not found: ..." -> {:error, :datasource_not_found}
      "Query timeout exceeded" -> {:error, :timeout}
      "Query must have a name or source_model" -> {:error, :invalid_query}
      _ -> {:error, :unknown, reason}
    end
end
```

## Phoenix Integration

### Controller Example
```elixir
defmodule MyAppWeb.AnalyticsController do
  use MyAppWeb, :controller

  def revenue_by_month(conn, %{"customer_id" => customer_id}) do
    {:ok, engine} = GraphNight.Client.init("./graphnight_data")
    
    query = GraphNight.Client.build_query(
      name: "orders",
      measures: [GraphNight.Measure.new("revenue:sum")],
      time_dimensions: [GraphNight.TimeDimension.new("created_at", granularity: "MONTH")],
      filters: [
        GraphNight.Filter.eq("customer_id", customer_id),
        GraphNight.Filter.gte("created_at", Date.to_string(Date.add(Date.utc_today(), -365)))
      ]
    )
    
    case GraphNight.Client.execute_query(engine, query) do
      {:ok, result} ->
        render(conn, "revenue.json", data: result.data)
      {:error, reason} ->
        conn |> put_status(500) |> json(%{error: reason})
    end
  end
end
```

### LiveView Real-time Dashboard
```elixir
defmodule MyAppWeb.DashboardLive do
  use MyAppWeb, :live_view

  @impl true
  def mount(_params, _session, socket) do
    {:ok, engine} = GraphNight.Client.init("./graphnight_data")
    {:ok, socket |> assign(engine: engine) |> assign(:metrics, load_metrics(engine))}
  end

  defp load_metrics(engine) do
    queries = [
      %{"title" => "Total Revenue", "query" => build_kpi("revenue:sum")},
      %{"title" => "Orders Today", "query" => build_kpi("order_count:count")},
    ]
    
    Enum.map(queries, fn q ->
      case GraphNight.Client.execute_query(engine, q["query"]) do
        {:ok, result} -> Map.put(q, "value", hd(result.data))
        {:error, _} -> Map.put(q, "value", "N/A")
      end
    end)
  end
  
  defp build_kpi(formula) do
    GraphNight.Client.build_query(
      name: "orders",
      measures: [GraphNight.Measure.new(formula)],
      filters: [GraphNight.Filter.gte("created_at", Date.to_string(Date.utc_today()))]
    )
  end
end
```

## Oban Background Jobs

```elixir
defmodule MyApp.Jobs.DailyRollup do
  use Oban.Worker, queue: :analytics
  
  @impl true
  def perform(%Oban.Job{args: %{"date" => date}}) do
    {:ok, engine} = GraphNight.Client.init("./graphnight_data")
    
    query = GraphNight.Client.build_query(
      name: "orders",
      measures: [
        GraphNight.Measure.new("revenue:sum"),
        GraphNight.Measure.new("order_count:count")
      ],
      dimensions: [GraphNight.Dimension.new("store_id")],
      time_dimensions: [GraphNight.TimeDimension.new("created_at", granularity: "DAY")],
      filters: [GraphNight.Filter.eq("created_at", date)]
    )
    
    {:ok, result} = GraphNight.Client.execute_query(engine, query)
    
    Enum.each(result.data, fn row ->
      MyApp.Repo.insert(
        %MyApp.DailyMetric{
          date: Date.from_iso8601!(date),
          store_id: row["store_id"],
          revenue: Decimal.from_float(row["revenue:sum"]),
          order_count: row["order_count:count"]
        },
        on_conflict: :replace_all,
        conflict_target: [:date, :store_id]
      )
    end)
    
    {:ok, result.data}
  end
end
```

## Testing

```elixir
# test_helper.exs
{:ok, @engine} = GraphNight.Client.init("./test_data")

# Create test datasource with in-memory SQLite
ds = GraphNight.DataSource.new("test", "sqlite", "file::memory:?cache=shared")
GraphNight.Client.create_datasource(@engine, ds)

# Test query
{:ok, result} = GraphNight.Client.execute_query(@engine, test_query)
assert result.columns == ["status", "revenue:sum"]
```

## Configuration

Environment variables (for server mode):

```bash
GRAPHNIGHT_DEV_OPEN=1              # Disable auth (dev only)
GRAPHNIGHT_API_KEYS="alice:secret,bob:secret2"
GRAPHNIGHT_ADMIN_KEYS="admin:adminsecret"
GRAPHNIGHT_AUTH_REQUIRED=1         # Force auth even without keys
GRAPHNIGHT_OIDC_ISSUER="https://..."  # Enable OIDC JWT validation
```

## Performance Tips

1. **Connection Pooling**: Configure `pool_size` on datasource (default: 5)
2. **Query Caching**: Results cached automatically; invalidate with `engine.sql_engine.invalidate_result_cache()`
3. **Limit Results**: Always use `limit` for large datasets
4. **Indexes**: Ensure database has indexes on filtered/joined columns
5. **Dry Run First**: Use `dry_run_query` to verify SQL before execution

## Ecto Integration

Convert GraphNight models to Ecto schemas:

```elixir
schema_code = GraphNight.Ecto.model_to_ecto_schema(model)
# Generates:
# defmodule Orders do
#   use Ecto.Schema
#   @primary_key {:id, :id, autogenerate: true}
#   schema "orders" do
#     field :status, :string
#     field :store_id, :string
#     field :created_at, :utc_datetime
#     # measure: revenue:sum (SUM)
#     timestamps()
#   end
# end
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