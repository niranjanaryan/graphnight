// GraphNight HTTP Client - Main Entry Point

// Types
export * from './types';

// Client
export { GraphNightClient, createClient } from './client';

// Re-export commonly used GraphQL utilities
export { gql } from 'graphql-request';

// Version
export const VERSION = '1.0.0';

/**
 * GraphNight HTTP Client
 * 
 * @example
 * ```typescript
 * import { GraphNightClient, createClient } from '@graphnight/client';
 * 
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
 * ```
 */