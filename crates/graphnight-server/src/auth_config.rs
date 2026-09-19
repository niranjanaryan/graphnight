use graphnight_core::security::SessionPolicy;
use std::collections::HashMap;
use std::env;

/// Parsed API key identity.
#[derive(Debug, Clone)]
pub struct Identity {
    pub user_id: String,
    pub is_admin: bool,
    pub tenant_id: Option<String>,
}

/// Server auth configuration loaded from environment.
#[derive(Debug, Clone)]
pub struct AuthConfig {
    /// user_id → api key
    pub api_keys: HashMap<String, String>,
    /// admin user ids
    pub admin_users: std::collections::HashSet<String>,
    /// When true, anonymous GraphQL is rejected.
    pub auth_required: bool,
}

impl AuthConfig {
    /// Load from env:
    /// - `GRAPHNIGHT_DEV_OPEN=1` → auth not required (explicit open mode)
    /// - `GRAPHNIGHT_API_KEYS=alice:secret,bob:secret2`
    /// - `GRAPHNIGHT_ADMIN_KEYS=admin:adminsecret` (also registered as API keys)
    /// - `GRAPHNIGHT_AUTH_REQUIRED=1` → force auth even if no keys (fails closed)
    ///
    /// If any keys are configured and DEV_OPEN is not set, auth is required.
    pub fn from_env() -> Self {
        let dev_open = env::var("GRAPHNIGHT_DEV_OPEN")
            .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
            .unwrap_or(false);
        let force_required = env::var("GRAPHNIGHT_AUTH_REQUIRED")
            .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
            .unwrap_or(false);

        let mut api_keys = HashMap::new();
        let mut admin_users = std::collections::HashSet::new();

        parse_key_pairs(
            env::var("GRAPHNIGHT_API_KEYS").ok().as_deref(),
            &mut api_keys,
        );
        parse_key_pairs(
            env::var("GRAPHNIGHT_ADMIN_KEYS").ok().as_deref(),
            &mut api_keys,
        );
        if let Ok(admin_keys) = env::var("GRAPHNIGHT_ADMIN_KEYS") {
            for part in admin_keys.split(',') {
                let part = part.trim();
                if part.is_empty() {
                    continue;
                }
                if let Some((user, _)) = part.split_once(':') {
                    admin_users.insert(user.trim().to_string());
                }
            }
        }

        let auth_required = if dev_open {
            false
        } else {
            force_required || !api_keys.is_empty()
        };

        Self {
            api_keys,
            admin_users,
            auth_required,
        }
    }

    pub fn authenticate(
        &self,
        raw_key: Option<&str>,
        tenant_id: Option<String>,
    ) -> Option<Identity> {
        let key = raw_key?.trim();
        if key.is_empty() {
            return None;
        }
        for (user_id, secret) in &self.api_keys {
            if secrets_equal(secret, key) {
                return Some(Identity {
                    user_id: user_id.clone(),
                    is_admin: self.admin_users.contains(user_id),
                    tenant_id,
                });
            }
        }
        None
    }

    pub fn default_policy(&self, identity: Option<&Identity>) -> SessionPolicy {
        let mut policy = SessionPolicy::new();
        if let Some(id) = identity {
            if let Some(tenant) = &id.tenant_id {
                policy = policy.with_forced_filter(graphnight_core::models::Filter::new(
                    "tenant_id",
                    graphnight_core::models::FilterOperator::Eq,
                    serde_json::json!(tenant),
                ));
            }
        }
        policy
    }
}

fn parse_key_pairs(raw: Option<&str>, out: &mut HashMap<String, String>) {
    let Some(raw) = raw else {
        return;
    };
    for part in raw.split(',') {
        let part = part.trim();
        if part.is_empty() {
            continue;
        }
        if let Some((user, key)) = part.split_once(':') {
            out.insert(user.trim().to_string(), key.trim().to_string());
        }
    }
}

fn secrets_equal(a: &str, b: &str) -> bool {
    // Constant-time-ish compare for equal lengths; otherwise false.
    if a.len() != b.len() {
        return false;
    }
    a.bytes()
        .zip(b.bytes())
        .fold(0u8, |acc, (x, y)| acc | (x ^ y))
        == 0
}

/// Extract bearer token or X-API-Key value from headers.
pub fn extract_api_key(headers: &axum::http::HeaderMap) -> Option<String> {
    if let Some(auth) = headers.get(axum::http::header::AUTHORIZATION) {
        if let Ok(value) = auth.to_str() {
            let value = value.trim();
            if let Some(token) = value.strip_prefix("Bearer ") {
                return Some(token.trim().to_string());
            }
            if let Some(token) = value.strip_prefix("bearer ") {
                return Some(token.trim().to_string());
            }
        }
    }
    headers
        .get("x-api-key")
        .and_then(|v| v.to_str().ok())
        .map(|s| s.trim().to_string())
}

pub fn extract_tenant(headers: &axum::http::HeaderMap) -> Option<String> {
    headers
        .get("x-tenant-id")
        .and_then(|v| v.to_str().ok())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}
