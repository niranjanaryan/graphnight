# @graphnight/client

[![npm version](https://badge.fury.io/js/@graphnight%2Fclient.svg)](https://www.npmjs.com/package/@graphnight/client)
[![Build Status](https://github.com/niranjanaryan/graphnight/workflows/CI/badge.svg)](https://github.com/niranjanaryan/graphnight/actions)
[![License: MIT](https://img.shields.io/badge/License-MIT-yellow.svg)](https://opensource.org/licenses/MIT)

TypeScript HTTP/GraphQL client for **GraphNight** — connects to a remote GraphNight server.

## Features

- 🔌 **GraphQL API** — Connects to GraphNight server via HTTP
- 📝 **Full TypeScript** — Complete type definitions with Zod validation
- 🔐 **Auth support** — API keys, OIDC JWT, custom headers
- 📊 **Rich query API** — Measures, dimensions, time dimensions, filters, joins
- 🔄 **Multi-stage DAG queries** — Complex analytical workflows

## Installation

```bash
npm install @graphnight/client graphql@^16.8.1
# or
yarn add @graphnight/client graphql@^16.8.1
# or
pnpm add @graphnight/client graphql@^16.8.1
```

**Peer dependency required:** `graphql@^16.8.1`

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
    { formula: { expression: '*', label: 'Orders' }, aggregation: 'count' },
  ],
  dimensions: [{ name: 'status' }],
  time_dimensions: [{ dimension: 'created_at', granularity: 'MONTH' }],
  filters: [
    { field: 'status', operator: 'eq', value: 'completed' },
    { field: 'created_at', operator: 'gte', value: '2024-01-01' },
  ],
  order: [{ field: 'amount_usd:sum', direction: 'desc' }],
  limit: 100,
});

console.log(result.sql);
console.log(result.data);
```

## API Reference

### Client Configuration

```typescript
interface GraphNightClientConfig {
  url: string;                      // GraphQL server URL (required)
  headers?: Record<string, string>; // Auth headers
  timeout?: number;                 // Request timeout in ms (default: 30000)
}
```

### Model Operations

```typescript
// List all models
const { models, total } = await client.listModels();

// Get single model
const model = await client.getModel('orders');

// Create model (admin)
const newModel = await client.createModel({
  name: 'orders',
  datasource: 'analytics',
  measures: [...],
  dimensions: [...],
});

// Update model (admin)
const updated = await client.updateModel('orders', { description: 'New desc' });

// Delete model (admin)
await client.deleteModel('orders');
```

### DataSource Operations

```typescript
// List datasources
const { datasources } = await client.listDataSources();

// Get datasource
const ds = await client.getDataSource('analytics');

// Create datasource (admin)
await client.createDataSource({
  name: 'analytics',
  driver: 'postgres',
  connection_string: 'postgresql://...',
  pool_size: 10,
});

// Delete datasource (admin)
await client.deleteDataSource('analytics');
```

### Query Operations

```typescript
// Execute query
const result = await client.query({
  name: 'orders',
  measures: [
    { formula: { expression: 'amount_usd', label: 'Revenue' }, aggregation: 'sum' },
    { formula: { expression: '*', label: 'Orders' }, aggregation: 'count' },
  ],
  dimensions: [{ name: 'status' }],
  time_dimensions: [{ dimension: 'created_at', granularity: 'MONTH' }],
  filters: [
    { field: 'status', operator: 'eq', value: 'completed' },
    { field: 'amount_usd', operator: 'gte', value: 100 },
    { field: 'store_id', operator: 'in', value: [1, 2, 3] },
    { field: 'created_at', operator: 'between', value: ['2024-01-01', '2024-12-31'] },
  ],
  order: [
    { field: 'created_at', direction: 'asc' },
    { field: 'amount_usd:sum', direction: 'desc' },
  ],
  limit: 100,
  offset: 0,
});

// Dry run (SQL generation only)
const { sql } = await client.dryRun({
  name: 'orders',
  measures: [{ formula: { expression: 'amount_usd', label: 'Revenue' }, aggregation: 'sum' }],
  dimensions: [{ name: 'status' }],
});
console.log(sql);
// SELECT status, SUM(amount_usd) AS "Revenue" FROM orders GROUP BY status
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
```

### Search

```typescript
const { results, total } = await client.search('revenue', 10);
```

### Admin Operations

```typescript
// Ingest models from database schema
const report = await client.ingestModels('analytics');
// { modelsCreated: 5, modelsUpdated: 2, errors: [] }
```

## TypeScript

Full TypeScript definitions included:

```typescript
import type {
  Model,
  DataSource,
  QueryInput,
  QueryResult,
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

## When to Use

| Package | Use Case |
|---------|----------|
| `@graphnight/native` | Local execution, embedded, serverless, high performance |
| `@graphnight/client` | Remote GraphNight server, multi-tenant, GraphQL subscriptions |

## Development

```bash
npm install
npm run build
npm test
npm run lint
```

## License

MIT