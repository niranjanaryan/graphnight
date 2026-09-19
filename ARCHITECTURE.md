# GraphNight Architecture Blueprint

> **Vision document — not a status report.**  
> Many modules, paths, and phases below describe the intended design. What ships today is a smaller alpha surface (YAML models, formula/join core, SQL generation, GraphQL query/dry-run, CLI). See [README.md](README.md) and [LAUNCH.md](LAUNCH.md) for current capability and release bars. Treat unchecked items in `LAUNCH.md` as not ready for production.

## Overview

GraphNight is a high-performance, embeddable semantic layer for AI agents and humans, inspired by SLayer but built with Rust for performance and GraphQL for flexible querying. It enables governed, shared access to data and metrics across databases.

## Core Goals (from SLayer)

1. **Expressive semantic queries** - Define metrics once, query with flexible expressions (`revenue:sum`, `revenue:sum / *:count`, `time_shift(revenue:sum, -1, 'year')`)
2. **Agent-first search → inspect → query flow** - Search tool for discovery, memory store for business context
3. **Multi-interface** - GraphQL, REST, Python, MCP, Flight SQL, Postgres wire protocol
4. **Governance** - Row-level security, forced filters, audit trails
5. **Embeddable** - Standalone tool or Python/Rust library
6. **Data stack agnostic** - Importers for dbt, Cube, Ossie

## Architecture

```
┌─────────────────────────────────────────────────────────────────────┐
│                        GraphNight Architecture                       │
├─────────────────────────────────────────────────────────────────────┤
│                                                                      │
│  ┌──────────────┐  ┌──────────────┐  ┌──────────────┐              │
│  │   GraphQL    │  │    REST      │  │   Python     │  ← Interfaces │
│  │   (async-    │  │   (axum/     │  │   (PyO3      │              │
│  │   graphql)   │  │   tide)      │  │   bindings)  │              │
│  └──────┬───────┘  └──────┬───────┘  └──────┬───────┘              │
│         │                 │                 │                        │
│         └─────────────────┼─────────────────┘                        │
│                           ▼                                           │
│              ┌────────────────────────┐                              │
│              │   Query Planner        │                              │
│              │   (Rust core)          │                              │
│              └───────────┬────────────┘                              │
│                          │                                            │
│         ┌───────────────┼───────────────┐                            │
│         ▼               ▼               ▼                            │
│  ┌────────────┐ ┌────────────┐ ┌────────────┐                       │
│  │  Formula   │ │   Join     │ │  Security  │                       │
│  │  Engine    │ │  Walker    │ │  Layer     │                       │
│  └────────────┘ └────────────┘ └────────────┘                       │
│         │               │               │                            │
│         └───────────────┼───────────────┘                            │
│                         ▼                                             │
│              ┌────────────────────────┐                              │
│              │   SQL Generator        │  (DataFusion / custom)       │
│              └───────────┬────────────┘                              │
│                          │                                            │
│         ┌───────────────┼───────────────┐                            │
│         ▼               ▼               ▼                            │
│  ┌────────────┐ ┌────────────┐ ┌────────────┐                       │
│  │ PostgreSQL │ │   MySQL    │ │  SQLite/   │                       │
│  │  (sqlx)    │ │  (sqlx)    │ │  DuckDB    │                       │
│  └────────────┘ └────────────┘ └────────────┘                       │
│                                                                      │
│  ┌──────────────────────────────────────────────────────────────┐   │
│  │                    Storage Layer (YAML/JSON/SQLite)           │   │
│  │  Models │ Datasources │ Memories │ Search Index │ Config      │   │
│  └──────────────────────────────────────────────────────────────┘   │
└─────────────────────────────────────────────────────────────────────┘
```

## Rust Workspace Structure

Shipped layout (repo root workspace):

```text
Cargo.toml
crates/
  graphnight-core/       # models, formula, join, security types
  graphnight-sql/        # dialects, generator, executor
  graphnight-storage/    # yaml, sqlite, tantivy
  graphnight-graphql/    # schema, resolvers, directives
  graphnight-server/     # GraphQL server binary (Tide today)
  graphnight-cli/        # CLI (init, query, model, …)
  graphnight-python/     # early PyO3 bindings
examples/
docs/
```

Vision-only modules (not all present as separate files yet): query planner/optimizer,
DataFusion engine, Redis/result cache, axum REST, MCP, Flight SQL, importers.
See [LAUNCH.md](LAUNCH.md) for what is actually gated for release.

## Data Models

### Query Model (Rust)

```rust
pub struct Query {
    pub name: Option<String>,                    // Run by name
    pub source_model: Option<SourceSpec>,        // For multi-stage
    pub measures: Vec<Measure>,                  // revenue:sum, time_shift(...)
    pub dimensions: Vec<Dimension>,              // Group by fields
    pub time_dimensions: Vec<TimeDimension>,     // DATE_TRUNC granularity
    pub filters: Vec<Filter>,                    // WHERE clauses
    pub order: Vec<OrderBy>,                     // ORDER BY
    pub limit: Option<usize>,
    pub offset: Option<usize>,
    pub whole_periods_only: Option<bool>,        // Complete time periods
    pub distinct_dimension_values: Option<bool>, // Distinct on dimensions
    pub stage_ref: Option<String>,               // Multi-stage reference
}
```

### Formula Expressions

| Expression | Meaning |
|------------|---------|
| `revenue:sum` | Sum of revenue |
| `revenue:avg` | Average revenue |
| `revenue:sum / *:count` | Revenue per row |
| `time_shift(revenue:sum, -1, 'month')` | Prior month revenue |
| `ratio(revenue:sum, cost:sum)` | Revenue/cost ratio |
| `pct_change(revenue:sum)` | Month-over-month % change |
| `running_total(revenue:sum)` | Cumulative sum |
| `cohort(users, 'month', revenue:sum)` | Cohort analysis |

### Model Definition (YAML)

```yaml
name: orders
datasource: postgres_primary
description: "Order facts table"
measures:
  - name: revenue
    expression: "amount_usd:sum"
    label: "Revenue (USD)"
    format: "currency_usd"
  - name: order_count
    expression: "*:count"
    label: "Orders"
  - name: aov
    expression: "ratio(revenue:sum, order_count:count)"
    label: "Average Order Value"
dimensions:
  - name: status
    label: "Order Status"
  - name: customer_id
    label: "Customer"
time_dimensions:
  - name: created_at
    label: "Order Date"
joins:
  - name: customers
    model: customers
    type: left
    on: [customer_id, id]
    alias: cust
```

## GraphQL Schema

```graphql
type Query {
  # Execute semantic query
  query(input: QueryInput!, dryRun: Boolean, explain: Boolean): QueryResponse!
  
  # Multi-stage DAG query
  multiStageQuery(inputs: [QueryInput!]!, dryRun: Boolean): MultiStageResponse!
  
  # Introspection
  models(datasource: String): [ModelInfo!]!
  model(name: String!, datasource: String): ModelInfo
  datasources: [DatasourceInfo!]!
  
  # Search & discovery
  search(q: String!, limit: Int): [SearchResult!]!
  inspect(model: String!, datasource: String): InspectResult!
  
  # Memories
  memories(filter: MemoryFilter): [Memory!]!
  memory(id: ID!): Memory
}

type Mutation {
  # Model management
  createModel(input: CreateModelInput!): ModelInfo!
  updateModel(name: String!, input: UpdateModelInput!): ModelInfo!
  deleteModel(name: String!, datasource: String): Boolean!
  
  # Datasource management
  createDatasource(input: CreateDatasourceInput!): DatasourceInfo!
  updateDatasource(name: String!, input: UpdateDatasourceInput!): DatasourceInfo!
  
  # Memories
  saveMemory(input: SaveMemoryInput!): Memory!
  forgetMemory(id: ID!): ForgetMemoryResponse!
  
  # Ingestion
  ingestModels(datasource: String!): IngestionReport!
}

type Subscription {
  # Live query results
  liveQuery(input: QueryInput!, intervalMs: Int!): QueryResponse!
  
  # Model changes
  modelChanges(datasource: String): ModelChangeEvent!
}

# Input types for flexible query construction
input QueryInput {
  name: String
  sourceModel: SourceSpecInput
  measures: [MeasureInput!]
  dimensions: [DimensionInput!]
  timeDimensions: [TimeDimensionInput!]
  filters: [FilterInput!]
  order: [OrderByInput!]
  limit: Int
  offset: Int
  wholePeriodsOnly: Boolean
  distinctDimensionValues: Boolean
}

input MeasureInput {
  formula: String!           # "revenue:sum" or "time_shift(revenue:sum, -1, 'month')"
  label: String
  format: String
  aggregation: AggregationType
}

# ... other input types

enum AggregationType {
  SUM
  AVG
  COUNT
  MIN
  MAX
  COUNT_DISTINCT
}

type QueryResponse {
  data: [JSON!]!
  columns: [String!]!
  sql: String
  attributes: ResponseAttributes
  population: Int
  populationInferred: Boolean
  executionTimeMs: Float!
}
```

## Query Execution Pipeline

```
Query Input (GraphQL/REST/Python)
         │
         ▼
┌────────────────────────┐
│  Semantic Validation   │  ← Check model exists, fields valid, types match
└───────────┬────────────┘
            │
            ▼
┌────────────────────────┐
│   Formula Expansion    │  ← Resolve shorthand, time_shift, ratio, etc.
│   (FormulaRegistry)    │
└───────────┬────────────┘
            │
            ▼
┌────────────────────────┐
│   Join Resolution      │  ← Find join paths, detect cycles, pick join types
│   (JoinWalker)         │
└───────────┬────────────┘
            │
            ▼
┌────────────────────────┐
│  Security Policy Check │  ← Apply forced filters, RLS, column masking
└───────────┬────────────┘
            │
            ▼
┌────────────────────────┐
│   Logical Plan         │  ← Project, Filter, Aggregate, Join, Sort, Limit
│   (DataFusion/Custom)  │
└───────────┬────────────┘
            │
            ▼
┌────────────────────────┐
│   Plan Optimization    │  ← Predicate pushdown, join reorder, partition pruning
└───────────┬────────────┘
            │
            ▼
┌────────────────────────┐
│   SQL Generation       │  ← Dialect-specific SQL (Postgres, MySQL, etc.)
└───────────┬────────────┘
            │
            ▼
┌────────────────────────┐
│   Async Execution      │  ← sqlx connection pool, streaming results
└───────────┬────────────┘
            │
            ▼
┌────────────────────────┐
│   Response Formatting  │  ← Arrow → JSON, metadata, population estimate
└────────────────────────┘
```

## Performance Optimizations

| Optimization | Implementation |
|--------------|----------------|
| **Query plan cache** | LRU cache keyed by normalized query hash |
| **Connection pooling** | sqlx pool per datasource, configurable size |
| **Streaming results** | Async streams, avoid materializing large results |
| **Predicate pushdown** | Push filters into SQL WHERE clauses |
| **Partition pruning** | Detect time filters, prune partitions |
| **Join optimization** | Reorder joins by cardinality estimates |
| **Vectorized execution** | DataFusion for complex analytics |
| **Prepared statements** | Cache parameterized queries |
| **Result caching** | Redis/In-memory with TTL, invalidation on model change |

## Security Model

```rust
pub struct SessionPolicy {
    pub forced_filters: Vec<Filter>,      // Always applied
    pub allowed_models: Option<Vec<String>>,  // Whitelist
    pub denied_models: Option<Vec<String>>,   // Blacklist
    pub column_masks: HashMap<String, MaskFn>, // PII masking
    pub row_filter: Option<Filter>,       // RLS predicate
    pub max_rows: Option<usize>,          // Result limit
    pub allowed_datasources: Option<Vec<String>>,
}

// Applied automatically in query planner
impl QueryPlanner {
    fn apply_policy(&self, query: Query, policy: &SessionPolicy) -> Result<Query> {
        // 1. Merge forced filters
        // 2. Validate model access
        // 3. Apply column masks to SELECT
        // 4. Add RLS to WHERE
        // 5. Enforce row limit
    }
}
```

## Multi-Stage Queries (DAG)

```yaml
# Example: Cohort analysis
queries:
  - name: cohort_sizes
    measures: ["users:count"]
    dimensions: ["cohort_month"]
    
  - name: cohort_revenue
    source_model: 
      model: cohort_sizes  # Reference previous stage
      alias: cohorts
    measures: ["revenue:sum"]
    dimensions: ["cohort_month", "months_since_first_order"]
```

Execution: Topological sort → Execute stages in parallel where possible → Stream results

## Python API (PyO3)

```python
# High-level client
from graphnight import GraphNightClient

client = GraphNightClient("http://localhost:8080")

# Simple query
result = client.query({
    "measures": ["revenue:sum", "orders:count"],
    "dimensions": ["store", "month"],
    "time_dimensions": [{"dimension": "created_at", "granularity": "month"}],
    "filters": [{"field": "status", "operator": "eq", "value": "completed"}],
    "order": [{"field": "revenue:sum", "descending": True}],
    "limit": 100
})

# DataFrame support
df = client.query_df(query)  # Returns pandas.DataFrame

# Model management
client.create_model({
    "name": "orders",
    "datasource": "postgres",
    "measures": [{"formula": "amount:sum", "label": "Revenue"}],
    "dimensions": [{"name": "status"}],
})

# Memories
client.save_memory(
    learning="Revenue spikes on Black Friday",
    linked_entities=["revenue:sum", "orders:count"]
)

# Search
results = client.search("monthly revenue by store")
```

## Configuration

```toml
# graphnight.toml
[server]
host = "0.0.0.0"
port = 8080
workers = 4

[storage]
type = "sqlite"  # yaml, sqlite, postgres
path = "./graphnight.db"

[datasources]
# Defined via API or CLI

[security]
enable_rls = true
default_max_rows = 10000
audit_log = true

[cache]
enabled = true
type = "redis"  # memory, redis
ttl_seconds = 300
max_size_mb = 512

[observability]
metrics_port = 9090
tracing_level = "info"
```

## Deployment Options

| Mode | Use Case |
|------|----------|
| **Embedded (Python)** | `import graphnight; engine = GraphNightEngine(storage=YAMLStorage())` |
| **Standalone Server** | `graphnight-server --config graphnight.toml` |
| **Docker** | `docker run -p 8080:8080 graphnight/server` |
| **Kubernetes** | Helm chart with horizontal scaling |
| **Serverless** | AWS Lambda / Cloudflare Workers (Rust → WASM) |

## Testing Strategy

```
tests/
├── unit/
│   ├── core/           # Formula parsing, validation
│   ├── sql/            # SQL generation per dialect
│   └── storage/        # Backend implementations
├── integration/
│   ├── postgres/       # Testcontainers
│   ├── mysql/
│   └── sqlite/
├── graphql/            # Schema validation, resolver tests
├── python/             # PyO3 binding tests
└── benchmarks/         # Criterion.rs benchmarks
```

## Roadmap

### Phase 1: Core Engine (Week 1-2)
- [ ] Rust workspace setup
- [ ] Core models (Query, Measure, Dimension, Model)
- [ ] Formula parser & registry
- [ ] SQL generator (PostgreSQL dialect)
- [ ] YAML storage backend
- [ ] Basic query execution

### Phase 2: GraphQL API (Week 2-3)
- [ ] async-graphql schema
- [ ] Query/Mutation resolvers
- [ ] Tide server integration
- [ ] Subscription support (WebSocket)

### Phase 3: Python Bindings (Week 3-4)
- [ ] PyO3 module
- [ ] Client class with async/sync API
- [ ] pandas DataFrame support
- [ ] Type stubs (.pyi)

### Phase 4: Advanced Features (Week 4-6)
- [ ] Join walker & graph
- [ ] Security policies & RLS
- [ ] Multi-stage DAG queries
- [ ] Search (tantivy embeddings)
- [ ] Memories
- [ ] MCP server

### Phase 5: Production Hardening (Week 6-8)
- [ ] Connection pooling
- [ ] Query plan cache
- [ ] Observability (metrics, tracing)
- [ ] Benchmarks & optimization
- [ ] Documentation
- [ ] CI/CD pipeline