//! OIDC JWT bearer validation via issuer discovery + JWKS.

use crate::auth_config::Identity;
use jsonwebtoken::{decode, decode_header, jwk::JwkSet, Algorithm, DecodingKey, Validation};
use serde::Deserialize;
use serde_json::Value;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::RwLock;
use tracing::{debug, warn};

const JWKS_TTL: Duration = Duration::from_secs(3600);

/// OIDC settings loaded from environment.
#[derive(Debug, Clone)]
pub struct OidcSettings {
    pub issuer: String,
    pub audience: Option<String>,
    pub client_id: Option<String>,
    /// Claim name holding admin role(s), e.g. `roles` or `groups`.
    pub admin_claim: String,
    /// Values in `admin_claim` that grant admin (comma-separated in env).
    pub admin_values: Vec<String>,
    /// Claim for tenant/org id (default `tenant_id`; falls back to `org_id` when unset in token).
    pub tenant_claim: String,
}

impl OidcSettings {
    pub fn from_env() -> Option<Self> {
        let issuer = std::env::var("GRAPHNIGHT_OIDC_ISSUER").ok()?;
        let issuer = issuer.trim().trim_end_matches('/').to_string();
        if issuer.is_empty() {
            return None;
        }

        let audience = std::env::var("GRAPHNIGHT_OIDC_AUDIENCE")
            .ok()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty());
        let client_id = std::env::var("GRAPHNIGHT_OIDC_CLIENT_ID")
            .ok()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty());

        let admin_claim = std::env::var("GRAPHNIGHT_OIDC_ADMIN_CLAIM")
            .ok()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| "roles".to_string());

        let admin_values = std::env::var("GRAPHNIGHT_OIDC_ADMIN_VALUES")
            .ok()
            .map(|s| {
                s.split(',')
                    .map(|p| p.trim().to_string())
                    .filter(|p| !p.is_empty())
                    .collect::<Vec<_>>()
            })
            .filter(|v| !v.is_empty())
            .unwrap_or_else(|| vec!["admin".to_string()]);

        let tenant_claim = std::env::var("GRAPHNIGHT_OIDC_TENANT_CLAIM")
            .ok()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| "tenant_id".to_string());

        Some(Self {
            issuer,
            audience,
            client_id,
            admin_claim,
            admin_values,
            tenant_claim,
        })
    }

    fn expected_audiences(&self) -> Vec<String> {
        let mut out = Vec::new();
        if let Some(a) = &self.audience {
            out.push(a.clone());
        }
        if let Some(c) = &self.client_id {
            if !out.iter().any(|x| x == c) {
                out.push(c.clone());
            }
        }
        out
    }
}

#[derive(Clone)]
enum KeySource {
    /// Fetch JWKS from OIDC discovery (`issuer` + `/.well-known/openid-configuration`).
    Discovery,
    /// Static HMAC secret for unit tests (HS256).
    #[cfg(test)]
    Hmac(String),
}

struct CachedJwks {
    set: JwkSet,
    fetched_at: Instant,
}

/// Validates Bearer JWTs issued by a configured OIDC provider.
pub struct OidcValidator {
    settings: OidcSettings,
    key_source: KeySource,
    http: reqwest::Client,
    jwks: RwLock<Option<CachedJwks>>,
}

impl OidcValidator {
    pub fn from_settings(settings: OidcSettings) -> Result<Arc<Self>, String> {
        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(10))
            .user_agent("graphnight-server/oidc")
            .build()
            .map_err(|e| format!("OIDC HTTP client: {e}"))?;
        Ok(Arc::new(Self {
            settings,
            key_source: KeySource::Discovery,
            http,
            jwks: RwLock::new(None),
        }))
    }

    /// Build a validator that verifies HS256 tokens with a shared secret (tests).
    #[cfg(test)]
    pub fn from_hmac(settings: OidcSettings, secret: impl Into<String>) -> Arc<Self> {
        Arc::new(Self {
            settings,
            key_source: KeySource::Hmac(secret.into()),
            http: reqwest::Client::new(),
            jwks: RwLock::new(None),
        })
    }

    pub async fn validate(&self, token: &str) -> Result<Identity, String> {
        let header = decode_header(token).map_err(|e| format!("JWT header: {e}"))?;
        let alg = header.alg;

        let key = match &self.key_source {
            #[cfg(test)]
            KeySource::Hmac(secret) => {
                if !matches!(alg, Algorithm::HS256 | Algorithm::HS384 | Algorithm::HS512) {
                    return Err(format!("HMAC test validator does not support {alg:?}"));
                }
                DecodingKey::from_secret(secret.as_bytes())
            }
            KeySource::Discovery => {
                self.decoding_key_from_jwks(header.kid.as_deref(), alg)
                    .await?
            }
        };

        let mut validation = Validation::new(alg);
        validation.set_issuer(std::slice::from_ref(&self.settings.issuer));
        let audiences = self.settings.expected_audiences();
        if audiences.is_empty() {
            validation.validate_aud = false;
        } else {
            validation.set_audience(&audiences);
        }

        let data = decode::<OidcClaims>(token, &key, &validation)
            .map_err(|e| format!("JWT validate: {e}"))?;

        let claims = data.claims;
        if claims.sub.trim().is_empty() {
            return Err("JWT missing sub".into());
        }

        let is_admin = claim_matches_admin(
            &claims.extra,
            &self.settings.admin_claim,
            &self.settings.admin_values,
        );
        let tenant_id = extract_tenant(&claims.extra, &self.settings.tenant_claim);

        Ok(Identity {
            user_id: claims.sub,
            is_admin,
            tenant_id,
        })
    }

    async fn decoding_key_from_jwks(
        &self,
        kid: Option<&str>,
        alg: Algorithm,
    ) -> Result<DecodingKey, String> {
        let set = self.get_jwks(false).await?;
        if let Some(key) = find_decoding_key(&set, kid, alg)? {
            return Ok(key);
        }
        // Kid miss / stale set: force refresh once.
        debug!("JWKS kid miss or incompatible; refreshing");
        let set = self.get_jwks(true).await?;
        find_decoding_key(&set, kid, alg)?
            .ok_or_else(|| format!("No JWKS key for kid={kid:?} alg={alg:?}"))
    }

    async fn get_jwks(&self, force: bool) -> Result<JwkSet, String> {
        {
            let guard = self.jwks.read().await;
            if !force {
                if let Some(cached) = guard.as_ref() {
                    if cached.fetched_at.elapsed() < JWKS_TTL {
                        return Ok(cached.set.clone());
                    }
                }
            }
        }

        let set = self.fetch_jwks().await?;
        let mut guard = self.jwks.write().await;
        *guard = Some(CachedJwks {
            set: set.clone(),
            fetched_at: Instant::now(),
        });
        Ok(set)
    }

    async fn fetch_jwks(&self) -> Result<JwkSet, String> {
        let discovery_url = format!(
            "{}/.well-known/openid-configuration",
            self.settings.issuer.trim_end_matches('/')
        );
        let discovery: OidcDiscovery = self
            .http
            .get(&discovery_url)
            .send()
            .await
            .map_err(|e| format!("OIDC discovery fetch: {e}"))?
            .error_for_status()
            .map_err(|e| format!("OIDC discovery status: {e}"))?
            .json()
            .await
            .map_err(|e| format!("OIDC discovery JSON: {e}"))?;

        if discovery.jwks_uri.trim().is_empty() {
            return Err("OIDC discovery missing jwks_uri".into());
        }

        let set: JwkSet = self
            .http
            .get(&discovery.jwks_uri)
            .send()
            .await
            .map_err(|e| format!("JWKS fetch: {e}"))?
            .error_for_status()
            .map_err(|e| format!("JWKS status: {e}"))?
            .json()
            .await
            .map_err(|e| format!("JWKS JSON: {e}"))?;

        if set.keys.is_empty() {
            warn!("JWKS from {} contained zero keys", discovery.jwks_uri);
        }
        Ok(set)
    }
}

#[derive(Debug, Deserialize)]
struct OidcDiscovery {
    jwks_uri: String,
}

#[derive(Debug, Deserialize)]
struct OidcClaims {
    sub: String,
    #[serde(flatten)]
    extra: HashMap<String, Value>,
}

fn find_decoding_key(
    set: &JwkSet,
    kid: Option<&str>,
    alg: Algorithm,
) -> Result<Option<DecodingKey>, String> {
    let candidates: Vec<&jsonwebtoken::jwk::Jwk> = if let Some(kid) = kid {
        set.find(kid).into_iter().collect()
    } else if set.keys.len() == 1 {
        vec![&set.keys[0]]
    } else {
        set.keys.iter().collect()
    };

    for jwk in candidates {
        if let Some(jwk_alg) = jwk.common.key_algorithm {
            // Skip keys that advertise a different algorithm when we can tell.
            let mapped = key_algorithm_to_jwt(jwk_alg);
            if let Some(mapped) = mapped {
                if mapped != alg {
                    continue;
                }
            }
        }
        match DecodingKey::from_jwk(jwk) {
            Ok(key) => return Ok(Some(key)),
            Err(e) => {
                debug!("skip JWKS key: {e}");
            }
        }
    }
    Ok(None)
}

fn key_algorithm_to_jwt(alg: jsonwebtoken::jwk::KeyAlgorithm) -> Option<Algorithm> {
    use jsonwebtoken::jwk::KeyAlgorithm::*;
    Some(match alg {
        HS256 => Algorithm::HS256,
        HS384 => Algorithm::HS384,
        HS512 => Algorithm::HS512,
        RS256 => Algorithm::RS256,
        RS384 => Algorithm::RS384,
        RS512 => Algorithm::RS512,
        ES256 => Algorithm::ES256,
        ES384 => Algorithm::ES384,
        PS256 => Algorithm::PS256,
        PS384 => Algorithm::PS384,
        PS512 => Algorithm::PS512,
        EdDSA => Algorithm::EdDSA,
        _ => return None,
    })
}

fn claim_matches_admin(
    extra: &HashMap<String, Value>,
    claim: &str,
    admin_values: &[String],
) -> bool {
    let Some(value) = extra.get(claim) else {
        return false;
    };
    let needles: Vec<&str> = admin_values.iter().map(|s| s.as_str()).collect();
    match value {
        Value::String(s) => needles.iter().any(|n| s.eq_ignore_ascii_case(n)),
        Value::Array(items) => items.iter().any(|item| {
            item.as_str()
                .map(|s| needles.iter().any(|n| s.eq_ignore_ascii_case(n)))
                .unwrap_or(false)
        }),
        Value::Bool(true) if claim == "admin" || claim == "is_admin" => true,
        _ => false,
    }
}

fn extract_tenant(extra: &HashMap<String, Value>, tenant_claim: &str) -> Option<String> {
    if let Some(v) = string_claim(extra, tenant_claim) {
        return Some(v);
    }
    // Sensible fallback when using the default claim name.
    if tenant_claim == "tenant_id" {
        return string_claim(extra, "org_id");
    }
    None
}

fn string_claim(extra: &HashMap<String, Value>, name: &str) -> Option<String> {
    extra
        .get(name)
        .and_then(|v| match v {
            Value::String(s) => Some(s.trim().to_string()),
            Value::Number(n) => Some(n.to_string()),
            _ => None,
        })
        .filter(|s| !s.is_empty())
}

/// JWT compact serialization has exactly two dots (three base64url segments).
pub fn looks_like_jwt(token: &str) -> bool {
    let token = token.trim();
    if token.is_empty() {
        return false;
    }
    token.bytes().filter(|&b| b == b'.').count() == 2
}

#[cfg(test)]
mod tests {
    use super::*;
    use jsonwebtoken::{encode, EncodingKey, Header};
    use serde::Serialize;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[derive(Serialize)]
    struct TestClaims {
        sub: String,
        iss: String,
        aud: String,
        exp: usize,
        iat: usize,
        #[serde(skip_serializing_if = "Option::is_none")]
        roles: Option<Vec<String>>,
        #[serde(skip_serializing_if = "Option::is_none")]
        tenant_id: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        org_id: Option<String>,
    }

    fn now() -> usize {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs() as usize
    }

    fn settings() -> OidcSettings {
        OidcSettings {
            issuer: "https://issuer.example".into(),
            audience: Some("graphnight".into()),
            client_id: None,
            admin_claim: "roles".into(),
            admin_values: vec!["admin".into()],
            tenant_claim: "tenant_id".into(),
        }
    }

    fn mint(claims: &TestClaims, secret: &str) -> String {
        encode(
            &Header::new(Algorithm::HS256),
            claims,
            &EncodingKey::from_secret(secret.as_bytes()),
        )
        .unwrap()
    }

    #[test]
    fn looks_like_jwt_detects_three_segments() {
        assert!(looks_like_jwt("aaa.bbb.ccc"));
        assert!(!looks_like_jwt("not-a-jwt"));
        assert!(!looks_like_jwt("only.one"));
        assert!(!looks_like_jwt("a.b.c.d"));
    }

    #[tokio::test]
    async fn hmac_valid_admin_and_tenant() {
        let secret = "test-hmac-secret-key-32bytes!!";
        let validator = OidcValidator::from_hmac(settings(), secret);
        let t = now();
        let token = mint(
            &TestClaims {
                sub: "user-1".into(),
                iss: "https://issuer.example".into(),
                aud: "graphnight".into(),
                exp: t + 3600,
                iat: t,
                roles: Some(vec!["admin".into(), "reader".into()]),
                tenant_id: Some("ten-9".into()),
                org_id: None,
            },
            secret,
        );
        let id = validator.validate(&token).await.unwrap();
        assert_eq!(id.user_id, "user-1");
        assert!(id.is_admin);
        assert_eq!(id.tenant_id.as_deref(), Some("ten-9"));
    }

    #[tokio::test]
    async fn hmac_org_id_fallback() {
        let secret = "test-hmac-secret-key-32bytes!!";
        let validator = OidcValidator::from_hmac(settings(), secret);
        let t = now();
        let token = mint(
            &TestClaims {
                sub: "user-2".into(),
                iss: "https://issuer.example".into(),
                aud: "graphnight".into(),
                exp: t + 3600,
                iat: t,
                roles: None,
                tenant_id: None,
                org_id: Some("org-42".into()),
            },
            secret,
        );
        let id = validator.validate(&token).await.unwrap();
        assert!(!id.is_admin);
        assert_eq!(id.tenant_id.as_deref(), Some("org-42"));
    }

    #[tokio::test]
    async fn hmac_rejects_bad_signature() {
        let validator = OidcValidator::from_hmac(settings(), "correct-secret-xxxxxxxxxxxxxxxx");
        let t = now();
        let token = mint(
            &TestClaims {
                sub: "user-1".into(),
                iss: "https://issuer.example".into(),
                aud: "graphnight".into(),
                exp: t + 3600,
                iat: t,
                roles: None,
                tenant_id: None,
                org_id: None,
            },
            "wrong-secret-yyyyyyyyyyyyyyyyyy",
        );
        assert!(validator.validate(&token).await.is_err());
    }

    #[tokio::test]
    async fn hmac_rejects_expired() {
        let secret = "test-hmac-secret-key-32bytes!!";
        let validator = OidcValidator::from_hmac(settings(), secret);
        let t = now();
        let token = mint(
            &TestClaims {
                sub: "user-1".into(),
                iss: "https://issuer.example".into(),
                aud: "graphnight".into(),
                // Outside default jsonwebtoken leeway (60s).
                exp: t.saturating_sub(120),
                iat: t.saturating_sub(200),
                roles: None,
                tenant_id: None,
                org_id: None,
            },
            secret,
        );
        assert!(validator.validate(&token).await.is_err());
    }
}
