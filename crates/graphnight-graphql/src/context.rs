use graphnight_core::security::SessionPolicy;
use graphnight_sql::SqlEngine;
use graphnight_storage::StorageBackend;
use std::sync::Arc;

/// GraphQL request context (prefer request-scoped injection from the HTTP layer).
pub struct GraphQLContext {
    pub sql_engine: Arc<SqlEngine>,
    pub storage: Arc<dyn StorageBackend>,
    pub session_policy: Option<SessionPolicy>,
    pub user_id: Option<String>,
    pub tenant_id: Option<String>,
    pub is_admin: bool,
    /// When true, anonymous requests must be rejected by resolvers.
    pub auth_required: bool,
}

impl GraphQLContext {
    pub fn new(sql_engine: Arc<SqlEngine>, storage: Arc<dyn StorageBackend>) -> Self {
        Self {
            sql_engine,
            storage,
            session_policy: None,
            user_id: None,
            tenant_id: None,
            is_admin: false,
            auth_required: false,
        }
    }

    pub fn with_policy(mut self, policy: SessionPolicy) -> Self {
        self.session_policy = Some(policy);
        self
    }

    pub fn with_user(mut self, user_id: String, tenant_id: Option<String>) -> Self {
        self.user_id = Some(user_id);
        self.tenant_id = tenant_id;
        self
    }

    pub fn with_admin(mut self, is_admin: bool) -> Self {
        self.is_admin = is_admin;
        self
    }

    pub fn with_auth_required(mut self, required: bool) -> Self {
        self.auth_required = required;
        self
    }
}
