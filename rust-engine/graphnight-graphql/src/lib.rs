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

/// Build the GraphQL schema
pub fn build_schema(
    sql_engine: Arc<SqlEngine>,
    storage: Arc<dyn StorageBackend>,
) -> Schema<QueryRoot, MutationRoot, SubscriptionRoot> {
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
