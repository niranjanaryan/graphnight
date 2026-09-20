# @graphnight/native

[![npm version](https://badge.fury.io/js/@graphnight%2Fnative.svg)](https://www.npmjs.com/package/@graphnight/native)
[![Build Status](https://github.com/niranjanaryan/graphnight/workflows/CI/badge.svg)](https://github.com/niranjanaryan/graphnight/actions)
[![License: MIT](https://img.shields.io/badge/License-MIT-yellow.svg)](https://opensource.org/licenses/MIT)

**Native Node.js bindings for GraphNight** — Zero-copy access to the GraphNight Rust engine via NAPI-RS.

This package provides native Node.js bindings that expose the GraphNight semantic layer engine directly to Node.js, without HTTP overhead. Built with [napi-rs](https://napi.rs/) for maximum performance and type safety.

## Features

- 🚀 **Zero-copy native performance** — Direct Rust engine access, no HTTP round-trips
- 📦 **Self-contained** — No external GraphQL server needed for local execution
- 🔒 **Type-safe** — Full TypeScript definitions included
- 🌐 **Cross-platform** — Pre-built binaries for Linux, macOS, Windows, FreeBSD
- 🔌 **Compatible API** — Same interface as the HTTP-based `@graphnight/sdk`

## Installation

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

// Result contains data (as JSON string), columns, SQL, and timing
console.log(result.sql);
console.log(JSON.parse(result.data)); // Parse the JSON string to get data array
```

## API Reference

### Client Configuration

```typescript
const client = createClient(storagePath?: string);
// storagePath: Path to YAML storage directory (default: "./graphnight_data")
```

### Query Operations

```typescript
// Execute a query
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
const result = await client.generateSql({
  name: 'orders',
  measures: [{ formula: { expression: 'amount_usd' }, aggregation: 'sum' }],
  dimensions: [{ name: 'status' }],
});
console.log(result); // SELECT status, SUM(amount_usd) FROM orders GROUP BY status
```

### Model Operations

```typescript
// List models
const models = await client.listModels(); // or listModels('datasource_name')

// Get single model
const model = await client.getModel('orders');

// Create model
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

### DataSource Operations

```typescript
// List datasources
const datasources = await client.listDataSources();

// Create datasource
await client.createDataSource({
  name: 'analytics',
  driver: 'postgres',
  connection_string: 'postgresql://user:pass@localhost/db',
  pool_size: 10,
});
```

### Search

```typescript
const results = await client.search('revenue', 10);
// results: SearchResult[]
```

### Memory Operations

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
// Joins defined in model
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

## TypeScript

Full TypeScript definitions included:

```typescript
import type {
  QueryInput,
  MeasureInput,
  FilterInput,
  ModelInput,
  DataSourceInput,
  QueryResult,
  ModelSummary,
} from '@graphnight/native';
```

## Comparison: @graphnight/native vs @graphnight/sdk

| Feature | @graphnight/native | @graphnight/sdk |
|---------|-------------------|-----------------|
| **Transport** | Native (NAPI-RS) | HTTP/GraphQL |
| **Server required** | No | Yes |
| **Local execution** | ✅ Direct | ❌ Via server |
| **Performance** | Zero-copy, <1ms overhead | Network latency |
| **TypeScript** | ✅ Full | ✅ Full |
| **Multi-stage DAG** | Planned | ✅ |
| **Schema ingestion** | Planned | ✅ |

## When to Use Which

- **@graphnight/native** — Embedding in Node.js apps, local development, serverless functions, edge computing
- **@graphnight/sdk** — Connecting to remote GraphNight server, multi-tenant deployments, GraphQL subscriptions

## Development

```bash
# Build native module
npm run build

# Build debug
npm run build:debug

# Run tests
npm test
```

## Publishing

```bash
npm run build
npm test
npm publish
```

## License

MIT — See [LICENSE](../LICENSE) for details.

## Links

- [GraphNight GitHub](https://github.com/niranjanaryan/graphnight)
- [Python SDK](https://pypi.org/project/graphnight/)
- [Elixir SDK](https://hex.pm/packages/graphnight)
- [HTTP SDK (@graphnight/sdk)](https://www.npmjs.com/package/@graphnight/sdk)
- [Documentation](https://github.com/niranjanaryan/graphnight/tree/main/docs)