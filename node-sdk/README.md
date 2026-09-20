# GraphNight Node.js SDK

[![npm version](https://badge.fury.io/js/@graphnight%2Fsdk.svg)](https://www.npmjs.com/package/@graphnight/sdk)
[![Build Status](https://github.com/niranjanaryan/graphnight/workflows/CI/badge.svg)](https://github.com/niranjanaryan/graphnight/actions)
[![License: MIT](https://img.shields.io/badge/License-MIT-yellow.svg)](https://opensource.org/licenses/MIT)

TypeScript client for **GraphNight** — a high-performance semantic layer with GraphQL API for analytics.

## Features

- 🚀 **TypeScript-first** — Full type safety with Zod validation
- 🔌 **Dual mode** — Local (YAML) or Server (GraphQL) operation
- 📊 **Rich query API** — Measures, dimensions, time dimensions, filters, joins
- 🔄 **Multi-stage DAG queries** — Complex analytical workflows
- 🔐 **Security built-in** — Session policies, column masks, row-level filters
- 📦 **Zero dependencies** — Lightweight (~50KB gzipped)

## Installation

```bash
npm install @graphnight/sdk
# or
yarn add @graphnight/sdk
# or
pnpm add @graphnight/sdk
```

**Peer dependency** (required):
```bash
npm install graphql@^16.8.1
```

## Quick Start

### Server Mode (GraphQL API)

```typescript
import { createClient } from '@graphnight/sdk';

const client = createClient({
  url: 'http://localhost:8080/graphql',
  headers: {
    Authorization: 'Bearer YOUR_TOKEN',
  },
  timeout: 30000,
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

### Local Mode (YAML Storage)

```typescript
import { createClient } from '@graphnight/sdk';

// Local mode - uses YAML files (requires graphnight CLI installed)
const client = createClient({
  storagePath: './graphnight_data',
});

// Note: Local mode query execution requires the GraphNight Rust binary
// Use Python SDK or CLI for local execution
const models = await client.listModels(); // Works with local storage
```

## API Reference

### Client Configuration

```typescript
interface GraphNightClientConfig {
  url?: string;              // GraphQL server URL (server mode)
  storagePath?: string;      // Local YAML storage path (local mode)
  headers?: Record<string, string>;  // Auth headers (server mode)
  timeout?: number;          // Request timeout in ms
}
```

### Query Building

```typescript
interface QueryInput {
  name: string;                    // Model name
  measures: Measure[];             // Required: at least one measure
  dimensions?: Dimension[];        // Group by fields
  time_dimensions?: TimeDimension[]; // Time-based grouping
  filters?: Filter[];              // WHERE conditions
  order?: OrderBy[];               // ORDER BY
  limit?: number;                  // LIMIT
  offset?: number;                 // OFFSET
  stage_ref?: string;              // For multi-stage queries
}
```

### Measures

```typescript
interface Measure {
  formula: Formula;                // Expression + optional label
  aggregation: 'sum' | 'count' | 'avg' | 'min' | 'max' | 'count_distinct';
}

interface Formula {
  expression: string;              // SQL expression or column name
  label?: string;                  // Display label
}
```

### Advanced Formulas

```typescript
// Window functions
{ formula: { expression: 'running_total(amount_usd:sum)', label: 'Running Total' }, aggregation: 'sum' }
{ formula: { expression: 'pct_change(amount_usd:sum)', label: 'Pct Change' }, aggregation: 'sum' }

// Time comparisons
{ formula: { expression: "time_shift(amount_usd:sum, 1, 'MONTH')", label: 'Prev Month' }, aggregation: 'sum' }
{ formula: { expression: "ratio(amount_usd:sum, time_shift(amount_usd:sum, 1, 'MONTH'))", label: 'MoM Ratio' }, aggregation: 'sum' }

// Conditional
{ formula: { expression: "CASE WHEN tier = 'premium' THEN amount_usd * 1.1 ELSE amount_usd END", label: 'Adjusted' }, aggregation: 'sum' }
```

### Filters

```typescript
// Basic
{ field: 'status', operator: 'eq', value: 'completed' }
{ field: 'amount_usd', operator: 'gte', value: 100 }

// IN / NOT IN
{ field: 'store_id', operator: 'in', value: [1, 2, 3] }
{ field: 'status', operator: 'not_in', value: ['cancelled', 'refunded'] }

// Pattern matching
{ field: 'email', operator: 'like', value: '%@company.com' }
{ field: 'name', operator: 'ilike', value: 'john%' }

// NULL checks
{ field: 'deleted_at', operator: 'is_null' }
{ field: 'confirmed_at', operator: 'is_not_null' }

// Range
{ field: 'created_at', operator: 'between', value: ['2024-01-01', '2024-12-31'] }

// OR conditions
{ field: 'status', operator: 'eq', value: 'pending', or_condition: true }
```

### Time Dimensions

```typescript
time_dimensions: [
  { dimension: 'created_at', granularity: 'HOUR' },
  { dimension: 'created_at', granularity: 'DAY' },
  { dimension: 'created_at', granularity: 'WEEK' },
  { dimension: 'created_at', granularity: 'MONTH' },
  { dimension: 'created_at', granularity: 'QUARTER' },
  { dimension: 'created_at', granularity: 'YEAR' },
]
```

### Multi-Stage DAG Queries

```typescript
const result = await client.multiStageQuery({
  stages: [
    // Stage 1: Top 10 customers
    {
      name: 'orders',
      measures: [{ formula: { expression: 'amount_usd', label: 'Revenue' }, aggregation: 'sum' }],
      dimensions: [{ name: 'customer_id' }],
      order: [{ field: 'amount_usd:sum', direction: 'desc' }],
      limit: 10,
    },
    // Stage 2: Products per top customer (references stage1)
    {
      name: 'orders',
      measures: [{ formula: { expression: 'amount_usd', label: 'Revenue' }, aggregation: 'sum' }],
      dimensions: [{ name: 'product_id' }],
      filters: [{ field: 'customer_id', operator: 'eq', value: '{{stage1.customer_id}}' }],
      stage_ref: 'stage1',
    },
  ],
});

// result.results[0] - Top 10 customers
// result.results[1] - Products for each top customer
```

### Model Management (Admin)

```typescript
// Create model
await client.createModel({
  name: 'orders',
  datasource: 'analytics',
  measures: [{ formula: { expression: 'amount_usd', label: 'Revenue' }, aggregation: 'sum' }],
  dimensions: [{ name: 'status' }],
});

// Update model
await client.updateModel('orders', { description: 'Updated' });

// Delete model
await client.deleteModel('orders');
```

### DataSource Management (Admin)

```typescript
// Create datasource
await client.createDataSource({
  name: 'analytics',
  driver: 'postgres',
  connection_string: 'postgresql://user:pass@localhost/db',
  pool_size: 10,
});
```

### Schema Ingestion (Admin)

```typescript
// Auto-generate models from database schema
const report = await client.ingestModels('analytics');
console.log(report); // { modelsCreated: 5, modelsUpdated: 0, errors: [] }
```

### Search

```typescript
const { results } = await client.search('revenue', 10);
```

## Error Handling

```typescript
import { GraphNightClient, GraphNightError } from '@graphnight/sdk';

try {
  const result = await client.query({ name: 'orders', measures: [...] });
} catch (error) {
  if (error instanceof GraphNightError) {
    switch (error.code) {
      case 'MODEL_NOT_FOUND':
        console.error('Model does not exist');
        break;
      case 'DATASOURCE_NOT_FOUND':
        console.error('Datasource not configured');
        break;
      case 'QUERY_TIMEOUT':
        console.error('Query took too long');
        break;
      case 'UNAUTHORIZED':
        console.error('Invalid or expired token');
        break;
      default:
        console.error('GraphNight error:', error.message);
    }
  }
}
```

## TypeScript

Full TypeScript support with strict types:

```typescript
// All query inputs are fully typed
const query: QueryInput = {
  name: 'orders',
  measures: [
    { formula: { expression: 'amount_usd', label: 'Revenue' }, aggregation: 'sum' },
  ],
  // TypeScript enforces valid operators, aggregations, granularities
  filters: [
    { field: 'status', operator: 'eq', value: 'completed' }, // ✓
    { field: 'status', operator: 'invalid', value: 'x' },    // ✗ Type error
  ],
};
```

## Requirements

- Node.js 18+
- GraphNight server (for server mode) or graphnight CLI (for local mode)

## Related Packages

- **Python**: `pip install graphnight` — [PyPI](https://pypi.org/project/graphnight/)
- **Elixir**: `{:graphnight, "~> 1.0"}` — [Hex](https://hex.pm/packages/graphnight)
- **Rust**: `graphnight` crate — [crates.io](https://crates.io/crates/graphnight)

## License

MIT © GraphNight Contributors