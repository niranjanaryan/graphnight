# HTTP Client Usage

Package: [`@graphnight/client`](https://www.npmjs.com/package/@graphnight/client) **1.0.0** — TypeScript client for GraphNight GraphQL API.

The HTTP client talks to a remote GraphNight GraphQL server. For local execution without a server, use the [Native SDK](usage-native.md).

## Install

```bash
npm install @graphnight/client graphql@^16.8.1
# or
yarn add @graphnight/client graphql@^16.8.1
# or
pnpm add @graphnight/client graphql@^16.8.1
```

Requires Node.js 18+.

## Quick Start

```typescript
import { createClient } from '@graphnight/client';

const client = createClient({
  url: 'http://localhost:8080/graphql',
  headers: {
    Authorization: 'Bearer YOUR_TOKEN',
  },
});

// List models
const { models } = await client.listModels();
console.log(models);

// Execute a query
const result = await client.query({
  name: 'orders',
  measures: [
    { formula: { expression: 'amount_usd', label: 'Revenue' }, aggregation: 'sum' },
  ],
  dimensions: [{ name: 'status' }],
  limit: 10,
});

console.log(result.sql);
console.log(result.data);
```

## Client Configuration

```typescript
const client = createClient({
  url: 'http://localhost:8080/graphql',     // GraphQL endpoint (required)
  headers: {                                 // Auth headers
    Authorization: 'Bearer <token>',
    'X-Tenant-ID': 'tenant-123',
  },
  timeout: 30000,                            // Request timeout in ms (default: 30s)
});
```

## Model Operations

### List Models

```typescript
const { models, total } = await client.listModels();
// models: Model[]
// total: number
```

### Get Model

```typescript
const model = await client.getModel('orders');
// Model with measures, dimensions, time_dimensions, joins
```

### Create Model (Admin)

```typescript
const model = await client.createModel({
  name: 'orders',
  datasource: 'analytics',
  description: 'Order transactions',
  measures: [
    { formula: { expression: 'amount_usd', label: 'Revenue' }, aggregation: 'sum' },
    { formula: { expression: '*', label: 'Orders' }, aggregation: 'count' },
  ],
  dimensions: [
    { name: 'status', label: 'Order Status' },
    { name: 'store_id', label: 'Store' },
  ],
  time_dimensions: [
    { dimension: 'created_at', granularity: 'DAY' },
  ],
  joins: [],
});
```

### Update Model (Admin)

```typescript
const updated = await client.updateModel('orders', { description: 'Updated description' });
```

### Delete Model (Admin)

```typescript
await client.deleteModel('orders');
```

## DataSource Operations

### List DataSources

```typescript
const { datasources, total } = await client.listDataSources();
```

### Get DataSource

```typescript
const ds = await client.getDataSource('analytics');
```

### Create DataSource (Admin)

```typescript
await client.createDataSource({
  name: 'analytics',
  driver: 'postgres',
  connection_string: 'postgresql://user:pass@localhost/db',
  description: 'Analytics database',
  models: ['orders', 'customers'],
  pool_size: 10,
});
```

### Delete DataSource (Admin)

```typescript
await client.deleteDataSource('analytics');
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
  time_dimensions: [
    { dimension: 'created_at', granularity: 'MONTH' },
  ],
  filters: [
    { field: 'status', operator: 'eq', value: 'completed' },
    { field: 'amount_usd', operator: 'gte', value: 100 },
    { field: 'store_id', operator: 'in', value: [1, 2, 3] },
    { field: 'created_at', operator: 'between', value: ['2024-01-01', '2024-12-31'] },
    { field: 'customer_email', operator: 'like', value: '%@company.com' },
  ],
  order: [
    { field: 'created_at', direction: 'asc' },
    { field: 'amount_usd:sum', direction: 'desc' },
  ],
  limit: 100,
  offset: 0,
});

// Result structure:
interface QueryResult {
  data: Record<string, unknown>[];
  columns: string[];
  sql: string;
  execution_time_ms: number;
  row_count: number;
}
```

### Dry Run (SQL Generation Only)

```typescript
const { sql } = await client.dryRun({
  name: 'orders',
  measures: [{ formula: { expression: 'amount_usd', label: 'Revenue' }, aggregation: 'sum' }],
  dimensions: [{ name: 'status' }],
  filters: [{ field: 'status', operator: 'eq', value: 'completed' }],
});

console.log(sql);
// SELECT status, SUM(amount_usd) AS "Revenue" FROM orders WHERE status = 'completed' GROUP BY status
```

### Multi-Stage DAG Queries

```typescript
const result = await client.multiStageQuery({
  stages: [
    {
      // Stage 1: Top 10 customers by revenue
      name: 'orders',
      measures: [{ formula: { expression: 'amount_usd', label: 'Revenue' }, aggregation: 'sum' }],
      dimensions: [{ name: 'customer_id' }],
      order: [{ field: 'amount_usd:sum', direction: 'desc' }],
      limit: 10,
    },
    {
      // Stage 2: Products for each top customer (references stage1)
      name: 'orders',
      measures: [{ formula: { expression: 'amount_usd', label: 'Revenue' }, aggregation: 'sum' }],
      dimensions: [{ name: 'product_id' }],
      filters: [{ field: 'customer_id', operator: 'eq', value: '{{stage1.customer_id}}' }],
      stage_ref: 'stage1',
    },
  ],
});

// result.results[0] - top 10 customers
// result.results[1] - products per customer
// result.execution_time_ms - total time
```

## Search

```typescript
const { results, total } = await client.search('revenue', 10);

// results: SearchResult[]
// SearchResult: { model: Model, score: number, matches: string[] }
```

## Admin Operations

### Ingest Models from Database Schema

```typescript
const report = await client.ingestModels('analytics');

// IngestionReport: { modelsCreated: number, modelsUpdated: number, errors: string[] }
```

## Advanced Query Examples

### Window Functions

```typescript
const result = await client.query({
  name: 'orders',
  measures: [
    { formula: { expression: 'running_total(amount_usd:sum)', label: 'Running Revenue' }, aggregation: 'sum' },
    { formula: { expression: 'pct_change(amount_usd:sum)', label: 'Pct Change' }, aggregation: 'sum' },
    { formula: { expression: 'time_shift(amount_usd:sum, 1, "MONTH")', label: 'Prev Month' }, aggregation: 'sum' },
    { formula: { expression: 'ratio(amount_usd:sum, time_shift(amount_usd:sum, 1, "MONTH"))', label: 'MoM Ratio' }, aggregation: 'sum' },
  ],
  time_dimensions: [{ dimension: 'created_at', granularity: 'MONTH' }],
});
```

### Time Shift & Comparison

```typescript
const result = await client.query({
  name: 'orders',
  measures: [
    { formula: { expression: 'amount_usd', label: 'Current Month' }, aggregation: 'sum' },
    { formula: { expression: 'time_shift(amount_usd:sum, 1, "MONTH")', label: 'Previous Month' }, aggregation: 'sum' },
    { formula: { expression: 'time_shift(amount_usd:sum, 12, "MONTH")', label: 'Same Month Last Year' }, aggregation: 'sum' },
  ],
  time_dimensions: [{ dimension: 'created_at', granularity: 'MONTH' }],
  filters: [{ field: 'created_at', operator: 'gte', value: '2024-01-01' }],
});
```

### Complex Filters with OR Conditions

```typescript
const result = await client.query({
  name: 'orders',
  measures: [{ formula: { expression: 'amount_usd', label: 'Revenue' }, aggregation: 'sum' }],
  dimensions: [{ name: 'status' }],
  filters: [
    { field: 'status', operator: 'in', value: ['completed', 'shipped'] },
    { field: 'amount_usd', operator: 'gte', value: 50 },
    { field: 'store_id', operator: 'in', value: [1, 2, 3] },
    { field: 'customer_tier', operator: 'eq', value: 'premium' },
    // OR condition
    { field: 'status', operator: 'eq', value: 'pending', or_condition: true },
  ],
});
```

### Joins (Defined in Model)

```typescript
// Join defined in model YAML:
// joins:
//   - name: customers
//     model: customers
//     join_type: left
//     on: orders.customer_id = customers.id
//     alias: customers

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
  Model,
  DataSource,
  QueryInput,
  QueryResult,
  MultiStageQueryInput,
  MultiStageQueryResult,
  Measure,
  Dimension,
  TimeDimension,
  Filter,
  OrderBy,
  Aggregation,
  Granularity,
  Driver,
  FilterOperator,
  JoinType,
  SortDirection,
  SessionPolicy,
  IngestionReport,
} from '@graphnight/client';
```

### Type-Safe Query Building

```typescript
const query: QueryInput = {
  name: 'orders',
  measures: [
    { formula: { expression: 'amount_usd', label: 'Revenue' }, aggregation: 'sum' as Aggregation },
  ],
  dimensions: [{ name: 'status' }],
  time_dimensions: [{ dimension: 'created_at', granularity: 'MONTH' as Granularity }],
  filters: [
    { field: 'status', operator: 'eq' as FilterOperator, value: 'completed' },
  ],
  order: [{ field: 'amount_usd:sum', direction: 'desc' as SortDirection }],
  limit: 100,
};
```

## Error Handling

```typescript
import { GraphNightClient } from '@graphnight/client';

try {
  const result = await client.query({...});
} catch (error) {
  if (error instanceof Error) {
    if (error.message.includes('Model not found')) {
      // Handle missing model
    } else if (error.message.includes('Datasource not found')) {
      // Handle missing datasource
    } else if (error.message.includes('Query timeout')) {
      // Handle timeout
    } else if (error.message.includes('Unauthorized')) {
      // Handle auth error
    }
  }
  throw error;
}
```

## Authentication

### API Key

```typescript
const client = createClient({
  url: 'http://localhost:8080/graphql',
  headers: {
    Authorization: 'Bearer gn_live_abc123...',
  },
});
```

### OIDC JWT

```typescript
const client = createClient({
  url: 'http://localhost:8080/graphql',
  headers: {
    Authorization: 'Bearer eyJhbGciOiJIUzI1NiIs...',
  },
});
```

### Tenant Header (Multi-tenancy)

```typescript
const client = createClient({
  url: 'http://localhost:8080/graphql',
  headers: {
    Authorization: 'Bearer <token>',
    'X-Tenant-ID': 'tenant-123',
  },
});
```

## Development

### Local Development with CLI

```bash
# Terminal 1: Start GraphNight server
graphnight-server --storage-path ./graphnight_data --port 8080

# Terminal 2: Run Node.js app
npm run dev
```

### Using with TypeScript

```bash
# Initialize TypeScript project
npm init -y
npm install @graphnight/client
npm install -D typescript @types/node
npx tsc --init
```

```json
// tsconfig.json
{
  "compilerOptions": {
    "target": "ES2022",
    "module": "NodeNext",
    "moduleResolution": "NodeNext",
    "strict": true,
    "esModuleInterop": true,
    "skipLibCheck": true
  }
}
```

## Next Steps

- [Native SDK Usage](usage-native.md) — Local execution with native bindings
- [GraphQL Usage](usage-graphql.md) — Raw GraphQL operations
- [Python Usage](usage-python.md) — Python SDK for local execution
- [Elixir Usage](usage-elixir.md) — Elixir bindings with Ecto
- [CLI Usage](usage-cli.md) — Command-line interface
- [Authentication](auth.md) — Auth configuration
- [Deployment](deploy.md) — Production deployment