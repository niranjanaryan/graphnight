use graphnight_sql::SqlEngine;
use graphnight_storage::StorageBackend;
use std::sync::Arc;

/// GraphQL context
pub struct GraphQLContext {
    pub sql_engine: Arc<SqlEngine>,
    pub storage: Arc<dyn StorageBackend>,
    pub session_policy: Option<graphnight_core::security::SessionPolicy>,
    pub user_id: Option<String>,
    pub tenant_id: Option<String>,
}

impl GraphQLContext {
    pub fn new(sql_engine: Arc<SqlEngine>, storage: Arc<dyn StorageBackend>) -> Self {
        Self {
            sql_engine,
            storage,
            session_policy: None,
            user_id: None,
            tenant_id: None,
        }
    }

    pub fn with_policy(mut self, policy: graphnight_core::security::SessionPolicy) -> Self {
        self.session_policy = Some(policy);
        self
    }

    pub fn with_user(mut self, user_id: String, tenant_id: Option<String>) -> Self {
        self.user_id = Some(user_id);
        self.tenant_id = tenant_id;
        self
    }
}
