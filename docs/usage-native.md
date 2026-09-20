# Native Node.js SDK Usage

Package: [`@graphnight/native`](https://www.npmjs.com/package/@graphnight/native) **1.0.0** — Native Rust bindings for GraphNight via NAPI-RS.

The native SDK provides **zero-copy direct access** to the GraphNight Rust engine. No HTTP server required — runs entirely in-process.

## Install

```bash
npm install @graphnight/native
# or
yarn add @graphnight/native
# or
pnpm add @graphnight/native
```

Requires Node.js 18+.

## Quick Start

```typescript
import { createClient } from '@graphnight/native';

// Create client with local YAML storage
const client = createClient('./graphnight_data');

// Execute queries directly against your database
const result = await client.query({
  name: 'orders',
  measures: [
    { formula: { expression: 'amount_usd', label: 'Revenue' }, aggregation: 'sum' },
  ],
  dimensions: [{ name: 'status' }],
  time_dimensions: [{ dimension: 'created_at', granularity: 'month' }],
  filters: [{ field: 'status', operator: 'eq', value: 'completed' }],
});

// Result contains data as JSON string, columns, SQL, and timing
console.log(result.sql);
const rows = JSON.parse(result.data); // Parse to get array of objects
console.log(rows);
```

## Client Configuration

```typescript
const client = createClient(storagePath?: string);
// storagePath: Path to YAML storage directory (default: "./graphnight_data")
```

## Query Operations

### Execute Query

```typescript
const result = await client.query({
  name: 'orders',
  measures: [
    { formula: { expression: 'amount_usd', label: 'Revenue' }, aggregation: 'sum' },
    { formula: { expression: '*', label: 'Orders' }, aggregation: 'count' },
  ],
  dimensions: [{ name: 'status' }],
  time_dimensions: [{ dimension: 'created_at', granularity: 'month' }],
  filters: [
    { field: 'status', operator: 'eq', value: 'completed' },
    { field: 'amount_usd', operator: 'gte', value: '100' },
    { field: 'store_id', operator: 'in', value: '[1, 2, 3]' },
    { field: 'created_at', operator: 'between', value: '["2024-01-01", "2024-12-31"]' },
  ],
  order: [
    { field: 'created_at', descending: false },
    { field: 'amount_usd:sum', descending: true },
  ],
  limit: 100,
  offset: 0,
});

// Result structure
interface QueryResult {
  data: string;           // JSON string of array of objects
  columns: string[];      // Column names
  sql: string;            // Generated SQL
  execution_time_ms: number;
  row_count: number;
}

// Parse data
const rows = JSON.parse(result.data);
```

### Dry Run (SQL Generation Only)

```typescript
const sql = await client.generateSql({
  name: 'orders',
  measures: [{ formula: { expression: 'amount_usd' }, aggregation: 'sum' }],
  dimensions: [{ name: 'status' }],
});
console.log(sql);
// SELECT status, SUM(amount_usd) FROM orders GROUP BY status
```

### Dry Run with Full Result

```typescript
const result = await client.dryRun({
  name: 'orders',
  measures: [{ formula: { expression: 'amount_usd' }, aggregation: 'sum' }],
  dimensions: [{ name: 'status' }],
});
console.log(result.sql);
```

## Model Operations

### List Models

```typescript
const models = await client.listModels(); // or listModels('datasource_name')
// models: ModelSummary[]
```

### Get Model

```typescript
const model = await client.getModel('orders');
// ModelSummary | null
```

### Create Model

```typescript
await client.createModel({
  name: 'orders',
  datasource: 'analytics',
  description: 'Order transactions',
  measures: [
    { formula: { expression: 'amount_usd', label: 'Revenue' }, aggregation: 'sum' },
  ],
  dimensions: [{ name: 'status' }],
  time_dimensions: [{ dimension: 'created_at', granularity: 'day' }],
  joins: [],
});
```

## DataSource Operations

### List DataSources

```typescript
const datasources = await client.listDataSources();
// DataSourceSummary[]
```

### Create DataSource

```typescript
await client.createDataSource({
  name: 'analytics',
  driver: 'postgres',
  connection_string: 'postgresql://user:pass@localhost/db',
  pool_size: 10,
});
```

## Search

```typescript
const results = await client.search('revenue', 10);
// SearchResult[]
```

## Memory Operations

```typescript
// Save learning
const memory = await client.saveMemory(
  'Customers from NY prefer premium tier',
  ['customers', 'orders'],
  'ny-premium-insight',
  'Geographic preference analysis'
);

// List memories
const memories = await client.listMemories('premium', 'customers', 10, 0);

// Delete memory
await client.deleteMemory('ny-premium-insight');
```

## Advanced Query Examples

### Window Functions

```typescript
const result = await client.query({
  name: 'orders',
  measures: [
    { formula: { expression: 'running_total(amount_usd:sum)', label: 'Running Revenue' }, aggregation: 'sum' },
    { formula: { expression: 'pct_change(amount_usd:sum)', label: 'Pct Change' }, aggregation: 'sum' },
  ],
  time_dimensions: [{ dimension: 'created_at', granularity: 'month' }],
});
```

### Time Comparisons

```typescript
const result = await client.query({
  name: 'orders',
  measures: [
    { formula: { expression: 'time_shift(amount_usd:sum, 1, "month")', label: 'Prev Month' }, aggregation: 'sum' },
    { formula: { expression: 'ratio(amount_usd:sum, time_shift(amount_usd:sum, 1, "month"))', label: 'MoM Ratio' }, aggregation: 'sum' },
  ],
  time_dimensions: [{ dimension: 'created_at', granularity: 'month' }],
});
```

### Complex Filters

```typescript
const result = await client.query({
  name: 'orders',
  measures: [{ formula: { expression: 'amount_usd' }, aggregation: 'sum' }],
  dimensions: [{ name: 'status' }],
  filters: [
    { field: 'status', operator: 'in', value: '["completed", "shipped"]' },
    { field: 'amount_usd', operator: 'gte', value: '50' },
    { field: 'customer_email', operator: 'like', value: '"%@company.com"' },
    { field: 'status', operator: 'eq', value: 'pending', or_condition: true },
  ],
});
```

### Joins

```typescript
// Joins defined in model YAML
const result = await client.query({
  name: 'orders',
  measures: [
    { formula: { expression: 'amount_usd', label: 'Revenue' }, aggregation: 'sum' },
    { formula: { expression: 'customers.lifetime_value', label: 'Customer LTV' }, aggregation: 'sum' },
  ],
  dimensions: [
    { name: 'status' },
    { name: 'customers.tier' },
  ],
});
```

## TypeScript Types

Full type definitions included:

```typescript
import type {
  QueryInput,
  MeasureInput,
  FilterInput,
  ModelInput,
  DataSourceInput,
  QueryResult,
  ModelSummary,
  DataSourceSummary,
  SearchResult,
  MemoryItem,
  Aggregation,
  Granularity,
  FilterOperator,
  JoinType,
} from '@graphnight/native';
```

## Native vs HTTP SDK

| Feature | @graphnight/native | @graphnight/sdk |
|---------|-------------------|-----------------|
| **Transport** | Native (NAPI-RS) | HTTP/GraphQL |
| **Server required** | No | Yes |
| **Local execution** | ✅ Direct | ❌ Via server |
| **Performance** | Zero-copy, <1ms overhead | Network latency |
| **TypeScript** | ✅ Full | ✅ Full |
| **Multi-stage DAG** | Planned | ✅ |
| **Schema ingestion** | Planned | ✅ |
| **Auth/Policies** | Via local config | ✅ Full |

## When to Use

- **@graphnight/native** — Embedding in Node.js apps, local development, serverless functions, edge computing, high-performance analytics
- **@graphnight/sdk** — Connecting to remote GraphNight server, multi-tenant deployments, GraphQL subscriptions, shared server

## Development

```bash
# Build native module
npm run build

# Build debug
npm run build:debug

# Run tests
npm test
```

## See Also

- [Node.js HTTP SDK](usage-node.md)
- [Python Usage](usage-python.md)
- [Elixir Usage](usage-elixir.md)
- [CLI Usage](usage-cli.md)
- [GraphQL Usage](usage-graphql.md)