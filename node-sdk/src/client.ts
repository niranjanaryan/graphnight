// GraphNight Node.js Client

import { GraphQLClient, gql } from 'graphql-request';
import type {
  GraphNightClientConfig,
  Model,
  DataSource,
  QueryInput,
  QueryResult,
  MultiStageQueryInput,
  MultiStageQueryResult,
  DryRunResult,
  ModelListResponse,
  DataSourceListResponse,
  SearchResponse,
  IngestionReport,
  SessionPolicy,
  MemoryItem,
} from './types';

/** GraphNight Client for both local and server modes */
export class GraphNightClient {
  private client: GraphQLClient | null = null;
  private storagePath: string;
  private isLocalMode: boolean;

  constructor(config: GraphNightClientConfig = {}) {
    this.storagePath = config.storagePath || './graphnight_data';
    this.isLocalMode = !config.url;

    if (!this.isLocalMode && config.url) {
      this.client = new GraphQLClient(config.url, {
        headers: config.headers || {},
        fetch: config.timeout 
          ? async (url, options) => {
              const controller = new AbortController();
              const timeoutId = setTimeout(() => controller.abort(), config.timeout);
              try {
                return await fetch(url, { ...options, signal: controller.signal });
              } finally {
                clearTimeout(timeoutId);
              }
            }
          : undefined,
      });
    }
  }

  // ============================================
  // Local Mode Helpers (YAML storage)
  // ============================================

  private async localRequest<T>(operation: () => Promise<T>): Promise<T> {
    if (!this.isLocalMode) {
      throw new Error('Operation only available in local mode. Use GraphQL server for remote operations.');
    }
    // In local mode, we'd use the Rust binary or native bindings
    // For now, this is a placeholder - actual implementation would use the CLI or native module
    return operation();
  }

  // ============================================
  // Model Operations
  // ============================================

  /** List all models */
  async listModels(): Promise<ModelListResponse> {
    if (this.isLocalMode) {
      return this.localRequest(async () => {
        // Placeholder - would call local storage
        return { models: [], total: 0 };
      });
    }

    const query = gql`
      query ListModels {
        models {
          name
          datasource
          description
          measures {
            formula { expression label }
            aggregation
          }
          dimensions { name label }
          timeDimensions { dimension granularity }
          joins { name model joinType on alias }
        }
      }
    `;

    const data = await this.client!.request<{ models: Model[] }>(query);
    return { models: data.models, total: data.models.length };
  }

  /** Get a single model by name */
  async getModel(name: string): Promise<Model> {
    if (this.isLocalMode) {
      return this.localRequest(async () => {
        throw new Error('Not implemented in local mode');
      });
    }

    const query = gql`
      query GetModel($name: String!) {
        model(name: $name) {
          name
          datasource
          description
          measures { formula { expression label } aggregation }
          dimensions { name label }
          timeDimensions { dimension granularity }
          joins { name model joinType on alias }
        }
      }
    `;

    const data = await this.client!.request<{ model: Model }>(query, { name });
    return data.model;
  }

  /** Create a new model (admin only) */
  async createModel(model: Model): Promise<Model> {
    if (this.isLocalMode) {
      return this.localRequest(async () => {
        throw new Error('Not implemented in local mode');
      });
    }

    const mutation = gql`
      mutation CreateModel($input: ModelInput!) {
        createModel(input: $input) {
          name
          datasource
          description
          measures { formula { expression label } aggregation }
          dimensions { name label }
          timeDimensions { dimension granularity }
          joins { name model joinType on alias }
        }
      }
    `;

    const data = await this.client!.request<{ createModel: Model }>(mutation, { input: model });
    return data.createModel;
  }

  /** Update an existing model (admin only) */
  async updateModel(name: string, model: Partial<Model>): Promise<Model> {
    if (this.isLocalMode) {
      return this.localRequest(async () => {
        throw new Error('Not implemented in local mode');
      });
    }

    const mutation = gql`
      mutation UpdateModel($name: String!, $input: ModelInput!) {
        updateModel(name: $name, input: $input) {
          name
          datasource
          description
          measures { formula { expression label } aggregation }
          dimensions { name label }
          timeDimensions { dimension granularity }
          joins { name model joinType on alias }
        }
      }
    `;

    const data = await this.client!.request<{ updateModel: Model }>(mutation, { name, input: model });
    return data.updateModel;
  }

  /** Delete a model (admin only) */
  async deleteModel(name: string): Promise<boolean> {
    if (this.isLocalMode) {
      return this.localRequest(async () => {
        throw new Error('Not implemented in local mode');
      });
    }

    const mutation = gql`
      mutation DeleteModel($name: String!) {
        deleteModel(name: $name)
      }
    `;

    const data = await this.client!.request<{ deleteModel: boolean }>(mutation, { name });
    return data.deleteModel;
  }

  // ============================================
  // DataSource Operations
  // ============================================

  /** List all datasources */
  async listDataSources(): Promise<DataSourceListResponse> {
    if (this.isLocalMode) {
      return this.localRequest(async () => {
        return { datasources: [], total: 0 };
      });
    }

    const query = gql`
      query ListDataSources {
        datasources {
          name
          driver
          connectionString
          description
          models
          poolSize
        }
      }
    `;

    const data = await this.client!.request<{ datasources: DataSource[] }>(query);
    return { datasources: data.datasources, total: data.datasources.length };
  }

  /** Get a datasource by name */
  async getDataSource(name: string): Promise<DataSource> {
    if (this.isLocalMode) {
      return this.localRequest(async () => {
        throw new Error('Not implemented in local mode');
      });
    }

    const query = gql`
      query GetDataSource($name: String!) {
        datasource(name: $name) {
          name
          driver
          connectionString
          description
          models
          poolSize
        }
      }
    `;

    const data = await this.client!.request<{ datasource: DataSource }>(query, { name });
    return data.datasource;
  }

  /** Create a datasource (admin only) */
  async createDataSource(datasource: DataSource): Promise<DataSource> {
    if (this.isLocalMode) {
      return this.localRequest(async () => {
        throw new Error('Not implemented in local mode');
      });
    }

    const mutation = gql`
      mutation CreateDataSource($input: DataSourceInput!) {
        createDataSource(input: $input) {
          name
          driver
          connectionString
          description
          models
          poolSize
        }
      }
    `;

    const data = await this.client!.request<{ createDataSource: DataSource }>(mutation, { input: datasource });
    return data.createDataSource;
  }

  /** Delete a datasource (admin only) */
  async deleteDataSource(name: string): Promise<boolean> {
    if (this.isLocalMode) {
      return this.localRequest(async () => {
        throw new Error('Not implemented in local mode');
      });
    }

    const mutation = gql`
      mutation DeleteDataSource($name: String!) {
        deleteDataSource(name: $name)
      }
    `;

    const data = await this.client!.request<{ deleteDataSource: boolean }>(mutation, { name });
    return data.deleteDataSource;
  }

  // ============================================
  // Query Operations
  // ============================================

  /** Execute a query */
  async query(input: QueryInput): Promise<QueryResult> {
    if (this.isLocalMode) {
      return this.localRequest(async () => {
        throw new Error('Not implemented in local mode. Use CLI or Python SDK for local execution.');
      });
    }

    const query = gql`
      mutation ExecuteQuery($input: QueryInput!) {
        query(input: $input) {
          data
          columns
          sql
          executionTimeMs
          rowCount
        }
      }
    `;

    const data = await this.client!.request<{ query: QueryResult }>(query, { input });
    return data.query;
  }

  /** Dry run - generate SQL only */
  async dryRun(input: QueryInput): Promise<DryRunResult> {
    if (this.isLocalMode) {
      return this.localRequest(async () => {
        throw new Error('Not implemented in local mode');
      });
    }

    const query = gql`
      mutation DryRun($input: QueryInput!) {
        dryRun(input: $input) {
          sql
        }
      }
    `;

    const data = await this.client!.request<{ dryRun: DryRunResult }>(query, { input });
    return data.dryRun;
  }

  /** Execute multi-stage DAG query */
  async multiStageQuery(input: MultiStageQueryInput): Promise<MultiStageQueryResult> {
    if (this.isLocalMode) {
      return this.localRequest(async () => {
        throw new Error('Not implemented in local mode');
      });
    }

    const mutation = gql`
      mutation MultiStageQuery($input: MultiStageQueryInput!) {
        multiStageQuery(input: $input) {
          results {
            data
            columns
            sql
            executionTimeMs
            rowCount
          }
          executionTimeMs
        }
      }
    `;

    const data = await this.client!.request<{ multiStageQuery: MultiStageQueryResult }>(mutation, { input });
    return data.multiStageQuery;
  }

  // ============================================
  // Admin Operations
  // ============================================

  /** Ingest models from database schema (admin only) */
  async ingestModels(datasource: string): Promise<IngestionReport> {
    if (this.isLocalMode) {
      return this.localRequest(async () => {
        throw new Error('Not implemented in local mode');
      });
    }

    const mutation = gql`
      mutation IngestModels($datasource: String!) {
        ingestModels(datasource: $datasource) {
          modelsCreated
          modelsUpdated
          errors
        }
      }
    `;

    const data = await this.client!.request<{ ingestModels: IngestionReport }>(mutation, { datasource });
    return data.ingestModels;
  }

  // ============================================
  // Search
  // ============================================

  /** Search models */
  async search(query: string, limit = 10): Promise<SearchResponse> {
    if (this.isLocalMode) {
      return this.localRequest(async () => {
        return { results: [], total: 0 };
      });
    }

    const gqlQuery = gql`
      query Search($query: String!, $limit: Int!) {
        search(query: $query, limit: $limit) {
          model {
            name
            datasource
            description
            measures { formula { expression label } aggregation }
            dimensions { name label }
            timeDimensions { dimension granularity }
            joins { name model joinType on alias }
          }
          score
          matches
        }
      }
    `;

    const data = await this.client!.request<{ search: SearchResponse['results'] }>(gqlQuery, { query, limit });
    return { results: data.search, total: data.search.length };
  }

  // ============================================
  // Memory Operations (Local only)
  // ============================================

  /** Save a memory item */
  async saveMemory(key: string, value: unknown): Promise<MemoryItem> {
    if (!this.isLocalMode) {
      throw new Error('Memory operations only available in local mode');
    }
    return this.localRequest(async () => {
      throw new Error('Not implemented');
    });
  }

  /** Get a memory item */
  async getMemory(key: string): Promise<MemoryItem | null> {
    if (!this.isLocalMode) {
      throw new Error('Memory operations only available in local mode');
    }
    return this.localRequest(async () => {
      throw new Error('Not implemented');
    });
  }

  /** List memory items */
  async listMemory(): Promise<MemoryItem[]> {
    if (!this.isLocalMode) {
      throw new Error('Memory operations only available in local mode');
    }
    return this.localRequest(async () => {
      throw new Error('Not implemented');
    });
  }

  /** Delete a memory item */
  async deleteMemory(key: string): Promise<boolean> {
    if (!this.isLocalMode) {
      throw new Error('Memory operations only available in local mode');
    }
    return this.localRequest(async () => {
      throw new Error('Not implemented');
    });
  }

  // ============================================
  // Utility
  // ============================================

  /** Check if connected to server */
  isConnected(): boolean {
    return !this.isLocalMode && this.client !== null;
  }

  /** Get storage path (local mode) */
  getStoragePath(): string {
    return this.storagePath;
  }

  /** Set authentication headers (server mode) */
  setHeaders(headers: Record<string, string>): void {
    if (this.client) {
      // graphql-request doesn't support dynamic header updates easily
      // Would need to recreate client or use fetch directly
      console.warn('Header updates require client recreation');
    }
  }
}

/** Factory function for creating client */
export function createClient(config?: GraphNightClientConfig): GraphNightClient {
  return new GraphNightClient(config);
}