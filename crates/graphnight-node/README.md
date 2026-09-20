# @graphnight/native

Native Node.js bindings for **GraphNight** — high-performance semantic layer with zero-copy access to the Rust engine via NAPI.

## Features

- 🚀 **Native performance** — Direct Rust engine access, no HTTP overhead
- 🔌 **Zero-copy** — Efficient data transfer between Rust and Node.js
- 📦 **Prebuilt binaries** — Linux (x64, ARM64), macOS (x64, ARM64), Windows (x64), FreeBSD
- 🔒 **Type-safe** — Full TypeScript definitions included
- 🎯 **Same API** — Identical interface to the HTTP-based `@graphnight/sdk`

## Installation

```bash
npm install @graphnight/native
# or
yarn add @graphnight/native
# or
pnpm add @graphnight/native
```

**Requires Node.js 18+**

## Quick Start

```typescript
import { GraphNightClient, createClient } from '@graphnight/native';

// Create client with local YAML storage
const client = createClient('./graphnight_data');

// List models
const models = await client.listModels();
console.log(models);

// Execute a query
const result = await client.query({
  name: 'orders',
  measures: [
    { formula: { expression: 'amount_usd', label: 'Revenue' }, aggregation: 'sum' },
  ],
  dimensions: [{ name: 'status' }],
  time_dimensions: [{ dimension: 'created_at', granularity: 'month' }],
  filters: [{ field: 'status', operator: 'eq', value: 'completed' }],
  limit: 10,
});

// Result contains data (JSON string), columns, SQL, execution time
console.log(result.sql);
const data = JSON.parse(result.data);
console.log(data);
```

## API Reference

### Client Configuration

```typescript
const client = new GraphNightClient(storagePath?: string);
```

- `storagePath` — Path to YAML storage directory (default: `./graphnight_data`)

### Query Operations

#### Execute Query

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

// QueryResult
interface QueryResult {
  data: string;        // JSON string: [{status: "completed", Revenue: 10000}, ...]
  columns: string[];   // ["status", "Revenue"]
  sql: string;         // Generated SQL
  execution_time_ms: number;
  row_count: number;
}
```

#### Generate SQL Only (Dry Run)

```typescript
const sql = await client.generateSql({
  name: 'orders',
  measures: [{ formula: { expression: 'amount_usd' }, aggregation: 'sum' }],
  dimensions: [{ name: 'status' }],
});
console.log(sql);
// SELECT status, SUM(amount_usd) AS "Revenue" FROM orders GROUP BY status
```

#### Dry Run with Full Result

```typescript
const result = await client.dryRun({
  name: 'orders',
  measures: [{ formula: { expression: 'amount_usd' }, aggregation: 'sum' }],
  dimensions: [{ name: 'status' }],
});
// result.sql contains the SQL, result.data is "[]"
```

### Model Operations

```typescript
// List all models
const models = await client.listModels('analytics'); // optional datasource filter

// Get single model
const model = await client.getModel('orders', 'analytics');

// Create model
await client.createModel({
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
    { dimension: 'created_at', granularity: 'day' },
  ],
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
  description: 'Analytics database',
  models: ['orders', 'customers'],
  pool_size: 10,
});
```

### Search

```typescript
const results = await client.search('revenue', 10);
// [{ model_name: 'orders', datasource: 'analytics', score: 0.95, matched_fields: ['measures', 'description'], snippet: '...' }]
```

### Memory Operations

```typescript
// Save memory
const memory = await client.saveMemory(
  'Customer prefers premium tier',
  ['customers', 'orders'],
  'mem-123',
  'User preference'
);

// List memories
const memories = await client.listMemories('premium', 'customers', 10, 0);

// Delete memory
await client.deleteMemory('mem-123');
```

## Advanced Queries

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

## Filter Operators

| Operator | Description | Example |
|----------|-------------|---------|
| `eq` | Equal | `{ field: 'status', operator: 'eq', value: 'completed' }` |
| `neq` | Not equal | `{ field: 'status', operator: 'neq', value: 'cancelled' }` |
| `gt` | Greater than | `{ field: 'amount_usd', operator: 'gt', value: '100' }` |
| `gte` | Greater than or equal | `{ field: 'amount_usd', operator: 'gte', value: '100' }` |
| `lt` | Less than | `{ field: 'amount_usd', operator: 'lt', value: '1000' }` |
| `lte` | Less than or equal | `{ field: 'amount_usd', operator: 'lte', value: '1000' }` |
| `in` | In array | `{ field: 'store_id', operator: 'in', value: '[1, 2, 3]' }` |
| `not_in` | Not in array | `{ field: 'status', operator: 'not_in', value: '["cancelled", "refunded"]' }` |
| `like` | SQL LIKE | `{ field: 'email', operator: 'like', value: '%@company.com' }` |
| `ilike` | Case-insensitive LIKE | `{ field: 'name', operator: 'ilike', value: 'john%' }` |
| `is_null` | Is NULL | `{ field: 'deleted_at', operator: 'is_null' }` |
| `is_not_null` | Is NOT NULL | `{ field: 'confirmed_at', operator: 'is_not_null' }` |
| `between` | Between range | `{ field: 'created_at', operator: 'between', value: '["2024-01-01", "2024-12-31"]' }` |

## Time Granularities

- `second`, `minute`, `hour`, `day`, `week`, `month`, `quarter`, `year`

## Aggregations

- `sum`, `count`, `avg`, `min`, `max`, `count_distinct`

## Join Types

- `inner`, `left`, `right`, `full`

## TypeScript

Full TypeScript definitions included — just import and use:

```typescript
import { GraphNightClient, QueryInput, QueryResult, ModelInput } from '@graphnight/native';

const query: QueryInput = {
  name: 'orders',
  measures: [
    { formula: { expression: 'amount_usd', label: 'Revenue' }, aggregation: 'sum' },
  ],
  dimensions: [{ name: 'status' }],
  filters: [
    { field: 'status', operator: 'eq', value: 'completed' }, // Type-safe!
  ],
};
```

## @graphnight/native vs @graphnight/sdk

| Feature | @graphnight/native | @graphnight/sdk |
|---------|-------------------|-----------------|
| **Transport** | Native NAPI | HTTP/GraphQL |
| **Server Required** | No | Yes |
| **Performance** | ~10-100x faster | Network latency |
| **Local Execution** | ✅ | ❌ (requires server) |
| **TypeScript** | ✅ | ✅ |
| **Multi-platform** | ✅ | ✅ |

Use `@graphnight/native` for:
- Local development and testing
- Embedded analytics in Node.js apps
- Maximum performance
- Offline/air-gapped environments

Use `@graphnight/sdk` for:
- Remote GraphNight server
- Multi-language environments
- Serverless functions
- When you need the full GraphQL API

## Building from Source

```bash
# Requires Rust toolchain
cd crates/graphnight-node
cargo build --release

# Build npm package
napi build --platform --release
```

## License

MIT — See [LICENSE](../../LICENSE) for details.