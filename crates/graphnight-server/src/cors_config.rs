//! Configurable CORS from `GRAPHNIGHT_CORS_ORIGINS`.

use axum::http::{header, HeaderName, HeaderValue, Method};
use tower_http::cors::{AllowOrigin, Any, CorsLayer};
use tracing::{info, warn};

/// Default browser origins when `GRAPHNIGHT_CORS_ORIGINS` is unset or empty.
pub const DEFAULT_LOCAL_ORIGINS: &[&str] =
    &["http://127.0.0.1:8080", "http://localhost:8080"];

/// Build a `CorsLayer` from the environment.
///
/// - Unset / empty → allow only `http://127.0.0.1:8080` and `http://localhost:8080`
/// - `*` → allow any origin (logs a warning)
/// - comma-separated list → allow those exact origins
pub fn cors_layer_from_env() -> CorsLayer {
    cors_layer_from_value(std::env::var("GRAPHNIGHT_CORS_ORIGINS").ok().as_deref())
}

pub fn cors_layer_from_value(raw: Option<&str>) -> CorsLayer {
    let trimmed = raw.map(str::trim).filter(|s| !s.is_empty());

    match trimmed {
        Some("*") => {
            warn!(
                "GRAPHNIGHT_CORS_ORIGINS=* enables permissive CORS (any origin). \
                 Prefer an explicit allowlist for shared deployments."
            );
            CorsLayer::new()
                .allow_origin(Any)
                .allow_methods(Any)
                .allow_headers(Any)
        }
        Some(list) => {
            let origins = parse_origin_list(list);
            info!(
                "CORS allowlist: {}",
                origins
                    .iter()
                    .filter_map(|h| h.to_str().ok())
                    .collect::<Vec<_>>()
                    .join(", ")
            );
            allowlist_layer(origins)
        }
        None => {
            let origins = parse_origin_list(&DEFAULT_LOCAL_ORIGINS.join(","));
            info!(
                "CORS default (localhost only). Set GRAPHNIGHT_CORS_ORIGINS to override \
                 (comma-separated), or GRAPHNIGHT_CORS_ORIGINS=* for open CORS."
            );
            allowlist_layer(origins)
        }
    }
}

fn allowlist_layer(origins: Vec<HeaderValue>) -> CorsLayer {
    CorsLayer::new()
        .allow_origin(AllowOrigin::list(origins))
        .allow_methods([Method::GET, Method::POST, Method::OPTIONS])
        .allow_headers([
            header::AUTHORIZATION,
            header::CONTENT_TYPE,
            header::ACCEPT,
            HeaderName::from_static("x-api-key"),
            HeaderName::from_static("x-tenant-id"),
        ])
}

fn parse_origin_list(raw: &str) -> Vec<HeaderValue> {
    raw.split(',')
        .map(str::trim)
        .filter(|s| !s.is_empty() && *s != "*")
        .filter_map(|s| match HeaderValue::from_str(s) {
            Ok(v) => Some(v),
            Err(_) => {
                warn!("Ignoring invalid CORS origin: {s}");
                None
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_parses_localhost() {
        let origins = parse_origin_list(&DEFAULT_LOCAL_ORIGINS.join(","));
        assert_eq!(origins.len(), 2);
        assert_eq!(origins[0].to_str().unwrap(), "http://127.0.0.1:8080");
        assert_eq!(origins[1].to_str().unwrap(), "http://localhost:8080");
    }

    #[test]
    fn custom_list() {
        let origins = parse_origin_list("https://app.example.com, https://admin.example.com");
        assert_eq!(origins.len(), 2);
    }

    #[test]
    fn builds_without_panic() {
        let _ = cors_layer_from_value(None);
        let _ = cors_layer_from_value(Some("*"));
        let _ = cors_layer_from_value(Some("http://localhost:3000"));
    }
}
