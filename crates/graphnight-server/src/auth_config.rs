use crate::oidc::{looks_like_jwt, OidcSettings, OidcValidator};
use graphnight_core::security::SessionPolicy;
use std::collections::HashMap;
use std::env;
use std::sync::Arc;
use tracing::warn;

/// Parsed API key / OIDC identity.
#[derive(Debug, Clone)]
pub struct Identity {
    pub user_id: String,
    pub is_admin: bool,
    pub tenant_id: Option<String>,
}

/// Server auth configuration loaded from environment.
#[derive(Clone)]
pub struct AuthConfig {
    /// user_id → api key
    pub api_keys: HashMap<String, String>,
    /// admin user ids
    pub admin_users: std::collections::HashSet<String>,
    /// When true, anonymous GraphQL is rejected.
    pub auth_required: bool,
    /// Optional OIDC JWT validator (Bearer tokens that look like JWTs).
    pub oidc: Option<Arc<OidcValidator>>,
}

impl std::fmt::Debug for AuthConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AuthConfig")
            .field("api_keys_len", &self.api_keys.len())
            .field("admin_users", &self.admin_users)
            .field("auth_required", &self.auth_required)
            .field("oidc", &self.oidc.is_some())
            .finish()
    }
}

impl AuthConfig {
    /// Load from env:
    /// - `GRAPHNIGHT_DEV_OPEN=1` → auth not required (explicit open mode)
    /// - `GRAPHNIGHT_API_KEYS=alice:secret,bob:secret2`
    /// - `GRAPHNIGHT_ADMIN_KEYS=admin:adminsecret` (also registered as API keys)
    /// - `GRAPHNIGHT_AUTH_REQUIRED=1` → force auth even if no keys (fails closed)
    /// - `GRAPHNIGHT_OIDC_ISSUER=https://…` → enable OIDC JWT bearer validation
    ///
    /// If any keys or OIDC are configured and DEV_OPEN is not set, auth is required.
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

        let oidc = match OidcSettings::from_env() {
            Some(settings) => match OidcValidator::from_settings(settings) {
                Ok(v) => Some(v),
                Err(e) => {
                    warn!("OIDC configured but failed to initialize: {e}");
                    None
                }
            },
            None => None,
        };

        let auth_required = if dev_open {
            false
        } else {
            force_required || !api_keys.is_empty() || oidc.is_some()
        };

        Self {
            api_keys,
            admin_users,
            auth_required,
            oidc,
        }
    }

    /// Resolve identity: JWT-shaped Bearer → OIDC first (when configured), else API key.
    /// `tenant_hint` from `X-Tenant-Id` fills tenant when the credential does not supply one.
    pub async fn resolve_identity(
        &self,
        raw_token: Option<&str>,
        tenant_hint: Option<String>,
    ) -> Option<Identity> {
        let token = raw_token.map(str::trim).filter(|t| !t.is_empty())?;

        if looks_like_jwt(token) {
            if let Some(oidc) = &self.oidc {
                match oidc.validate(token).await {
                    Ok(mut id) => {
                        if id.tenant_id.is_none() {
                            id.tenant_id = tenant_hint;
                        }
                        return Some(id);
                    }
                    Err(e) => {
                        warn!("OIDC JWT validation failed, falling back to API key: {e}");
                    }
                }
            }
        }

        self.authenticate(Some(token), tenant_hint)
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::oidc::OidcSettings;
    use jsonwebtoken::{encode, Algorithm, EncodingKey, Header};
    use serde::Serialize;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[derive(Serialize)]
    struct Claims {
        sub: String,
        iss: String,
        aud: String,
        exp: usize,
        iat: usize,
        roles: Vec<String>,
        tenant_id: String,
    }

    #[tokio::test]
    async fn resolve_prefers_oidc_for_jwt_bearer() {
        let secret = "unit-test-hmac-secret-key!!!!";
        let settings = OidcSettings {
            issuer: "https://idp.test".into(),
            audience: Some("aud".into()),
            client_id: None,
            admin_claim: "roles".into(),
            admin_values: vec!["admin".into()],
            tenant_claim: "tenant_id".into(),
        };
        let oidc = OidcValidator::from_hmac(settings, secret);

        let mut api_keys = HashMap::new();
        // Deliberately use a key that is NOT the JWT string.
        api_keys.insert("alice".into(), "alice-secret".into());

        let auth = AuthConfig {
            api_keys,
            admin_users: Default::default(),
            auth_required: true,
            oidc: Some(oidc),
        };

        let t = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs() as usize;
        let token = encode(
            &Header::new(Algorithm::HS256),
            &Claims {
                sub: "oidc-user".into(),
                iss: "https://idp.test".into(),
                aud: "aud".into(),
                exp: t + 600,
                iat: t,
                roles: vec!["admin".into()],
                tenant_id: "t1".into(),
            },
            &EncodingKey::from_secret(secret.as_bytes()),
        )
        .unwrap();

        let id = auth
            .resolve_identity(Some(&token), Some("header-tenant".into()))
            .await
            .unwrap();
        assert_eq!(id.user_id, "oidc-user");
        assert!(id.is_admin);
        assert_eq!(id.tenant_id.as_deref(), Some("t1"));
    }

    #[tokio::test]
    async fn resolve_falls_back_to_api_key_for_non_jwt() {
        let mut api_keys = HashMap::new();
        api_keys.insert("alice".into(), "alice-secret".into());
        let auth = AuthConfig {
            api_keys,
            admin_users: ["alice".into()].into_iter().collect(),
            auth_required: true,
            oidc: None,
        };
        let id = auth
            .resolve_identity(Some("alice-secret"), Some("ten".into()))
            .await
            .unwrap();
        assert_eq!(id.user_id, "alice");
        assert!(id.is_admin);
        assert_eq!(id.tenant_id.as_deref(), Some("ten"));
    }
}
