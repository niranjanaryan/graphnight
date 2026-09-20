// GraphNight Node.js SDK - Main Entry Point

// Types
export * from './types';

// Client
export { GraphNightClient, createClient } from './client';

// Re-export commonly used GraphQL utilities
export { gql } from 'graphql-request';

// Version
export const VERSION = '1.0.0';

/**
 * GraphNight Node.js SDK
 * 
 * @example
 * ```typescript
 * import { GraphNightClient, createClient } from '@graphnight/sdk';
 * 
 * // Server mode (GraphQL API)
 * const client = createClient({
 *   url: 'http://localhost:8080/graphql',
 *   headers: { Authorization: 'Bearer <token>' }
 * });
 * 
 * const models = await client.listModels();
 * const result = await client.query({
 *   name: 'orders',
 *   measures: [{ formula: { expression: 'amount_usd', label: 'Revenue' }, aggregation: 'sum' }],
 *   dimensions: [{ name: 'status' }]
 * });
 * 
 * // Local mode (YAML storage - future)
 * const localClient = createClient({ storagePath: './graphnight_data' });
 * ```
 */