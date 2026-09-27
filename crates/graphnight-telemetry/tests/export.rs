//! End-to-end check that spans reach a collector.
//!
//! The unit tests in `lib.rs` cover configuration and guard behaviour, but the
//! claim in the launch checklist is that `plan → SQL → execute` is actually
//! traceable end to end. That only holds if a span emitted with a `tracing`
//! macro somewhere in the SQL crate survives the bridge to the OTLP exporter
//! and lands as an HTTP request. This test wires all of that against a stub
//! collector and inspects what arrives.

use std::sync::{Arc, Mutex};

use axum::Router;
use graphnight_telemetry::{init, TelemetryConfig};

/// One export request as the stub collector saw it.
type Request = (String, Vec<u8>);

/// Stub OTLP/HTTP collector. Records the path and body of every request.
#[derive(Clone, Default)]
struct Received {
    requests: Arc<Mutex<Vec<Request>>>,
}

async fn collect_traces(
    axum::extract::State(state): axum::extract::State<Received>,
    uri: axum::http::Uri,
    body: axum::body::Bytes,
) -> impl axum::response::IntoResponse {
    state
        .requests
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .push((uri.path().to_string(), body.to_vec()));
    axum::http::StatusCode::OK
}

/// A single test rather than two: installing a tracing subscriber is a
/// process-global operation, so only the first caller wins and concurrent tests
/// would race. Both assertions share one exporter for the same reason.
#[tokio::test]
async fn spans_are_exported_to_the_collector() {
    let received = Received::default();
    let app = Router::new()
        .route("/v1/traces", axum::routing::post(collect_traces))
        .with_state(received.clone());

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind stub collector");
    let port = listener.local_addr().unwrap().port();
    let server = tokio::spawn(async move {
        axum::serve(listener, app)
            .await
            .expect("serve stub collector");
    });

    let config = TelemetryConfig {
        endpoint: Some(format!("http://127.0.0.1:{port}/v1/traces")),
        service_name: "graphnight-export-test".to_string(),
        sample_ratio: 1.0,
        headers: Default::default(),
    };

    let guard = init(tracing_subscriber::EnvFilter::new("info"), &config)
        .expect("installing the subscriber must succeed");

    // Stand in for the real instrumented path. The name and fields mirror
    // `sql.plan` / `sql.execute` in graphnight-sql; what matters is that a
    // span created here is picked up by the same pipeline.
    {
        let span = tracing::info_span!(
            "sql.execute",
            otel.name = "plan demo",
            db.system = "sqlite",
            db.rows = 3,
        );
        let _guard = span.enter();
        tracing::info!(cache.hit = false, "statement executed");
    }

    // The batch exporter runs on a background thread with a scheduling delay,
    // so a flush is what makes the assertion deterministic. Poll afterwards in
    // case the request is still in flight when the flush returns.
    guard.flush();
    for _ in 0..40 {
        if !received.requests.lock().unwrap().is_empty() {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(250)).await;
    }
    drop(guard);
    server.abort();

    let requests = received
        .requests
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .clone();

    assert_eq!(requests.len(), 1, "expected exactly one export request");
    let (path, body) = &requests[0];
    assert_eq!(path, "/v1/traces", "endpoint path was not honoured");

    // OTLP is protobuf, but string fields are plain UTF-8 in the wire format,
    // so the span name is assertable without pulling in a protobuf decoder.
    let body = String::from_utf8_lossy(body);
    // `otel.name` wins over the tracing span name, so that is what a collector
    // shows. The real `sql.plan` span sets it to "plan <model>".
    assert!(
        body.contains("plan demo"),
        "exported payload did not contain the span name: {body}"
    );
    assert!(
        body.contains("graphnight-export-test"),
        "exported payload did not contain the configured service name: {body}"
    );
    assert!(
        body.contains("sqlite"),
        "exported payload did not contain the db.system field: {body}"
    );
}
