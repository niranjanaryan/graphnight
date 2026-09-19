pub mod auth;
pub mod context;
pub mod directives;
pub mod resolvers;
pub mod schema;

use crate::context::GraphQLContext;
use crate::directives::{AuthDirective, CacheDirective, RateLimitDirective};
use crate::resolvers::{MutationRoot, QueryRoot, SubscriptionRoot};
use async_graphql::*;
use graphnight_sql::SqlEngine;
use graphnight_storage::StorageBackend;
use std::sync::Arc;

/// GraphQL schema type used by the HTTP server.
pub type AppSchema = Schema<QueryRoot, MutationRoot, SubscriptionRoot>;

/// Build the GraphQL schema
pub fn build_schema(sql_engine: Arc<SqlEngine>, storage: Arc<dyn StorageBackend>) -> AppSchema {
    Schema::build(
        QueryRoot::new(sql_engine.clone(), storage.clone()),
        MutationRoot::new(sql_engine.clone(), storage.clone()),
        SubscriptionRoot,
    )
    .directive(AuthDirective)
    .directive(RateLimitDirective)
    .directive(CacheDirective)
    .data(GraphQLContext::new(sql_engine, storage))
    .finish()
}
