//! Request ID middleware: generate or propagate `x-request-id`, echo on response.

use axum::{
    extract::Request,
    http::{HeaderName, HeaderValue},
    middleware::Next,
    response::Response,
};
use uuid::Uuid;

pub const REQUEST_ID_HEADER: HeaderName = HeaderName::from_static("x-request-id");

/// Ensure every request has an `x-request-id` (client-supplied or new UUID).
/// Echoes the id on the response. Pair with [`make_trace_span`] so logs include it.
pub async fn request_id_middleware(mut request: Request, next: Next) -> Response {
    let id = request
        .headers()
        .get(&REQUEST_ID_HEADER)
        .and_then(|v| v.to_str().ok())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
        .unwrap_or_else(|| Uuid::new_v4().to_string());

    if let Ok(value) = HeaderValue::from_str(&id) {
        request.headers_mut().insert(REQUEST_ID_HEADER, value);
    }

    let mut response = next.run(request).await;
    if let Ok(value) = HeaderValue::from_str(&id) {
        response.headers_mut().insert(REQUEST_ID_HEADER, value);
    }
    response
}

/// Build an HTTP span that records `request_id` for structured tracing.
pub fn make_trace_span(request: &Request) -> tracing::Span {
    let request_id = request
        .headers()
        .get(&REQUEST_ID_HEADER)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("-");
    tracing::info_span!(
        "http",
        method = %request.method(),
        uri = %request.uri(),
        request_id = %request_id,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{body::Body, routing::get, Router};
    use tower::ServiceExt;

    #[tokio::test]
    async fn generates_and_echoes_request_id() {
        let app = Router::new()
            .route("/", get(|| async { "ok" }))
            .layer(axum::middleware::from_fn(request_id_middleware));

        let response = app
            .oneshot(Request::builder().uri("/").body(Body::empty()).unwrap())
            .await
            .unwrap();

        let id = response
            .headers()
            .get(&REQUEST_ID_HEADER)
            .and_then(|v| v.to_str().ok())
            .unwrap();
        assert!(!id.is_empty());
        assert!(Uuid::parse_str(id).is_ok());
    }

    #[tokio::test]
    async fn propagates_client_request_id() {
        let app = Router::new()
            .route("/", get(|| async { "ok" }))
            .layer(axum::middleware::from_fn(request_id_middleware));

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/")
                    .header("x-request-id", "client-req-123")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(
            response
                .headers()
                .get(&REQUEST_ID_HEADER)
                .and_then(|v| v.to_str().ok()),
            Some("client-req-123")
        );
    }
}
