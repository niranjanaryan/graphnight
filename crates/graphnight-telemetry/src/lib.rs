//! OpenTelemetry tracing for the GraphNight binaries.
//!
//! Spans themselves are emitted with plain [`tracing`] macros from wherever the
//! work happens — `sql.plan` in the generator, `sql.execute` in the engine,
//! plus the HTTP request span in the server. This crate is only responsible for
//! turning those into OTLP traces when an endpoint is configured.
//!
//! The split matters: the engine is also linked into the Python and Node SDKs,
//! which must not inherit an OpenTelemetry stack (and its dependency weight) just
//! because the server has one. Instrumentation travels with the `tracing`
//! macros; the exporter lives in the binaries.
//!
//! With no endpoint configured nothing here installs an OTLP layer, so there is
//! no exporter thread and no behaviour change for a default deployment.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use opentelemetry::trace::TracerProvider as _;
use opentelemetry_otlp::{SpanExporter, WithExportConfig, WithHttpConfig};
use opentelemetry_sdk::trace::{Sampler, SdkTracerProvider};
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;
use tracing_subscriber::EnvFilter;

/// Keeps the tracer provider alive and, on drop, flushes buffered spans.
///
/// A batch exporter buffers spans in memory, so a process that exits without
/// flushing loses whatever had not yet been sent. `main` should hold this until
/// the end of the run, or call [`TelemetryGuard::flush`] before returning.
#[derive(Clone, Default)]
pub struct TelemetryGuard {
    provider: Option<SdkTracerProvider>,
    shut_down: Arc<std::sync::atomic::AtomicBool>,
}

impl TelemetryGuard {
    fn is_shut_down(&self) -> bool {
        self.shut_down.load(Ordering::Relaxed)
    }

    /// Push any buffered spans to the collector without shutting the exporter
    /// down.
    ///
    /// A no-op once [`TelemetryGuard::shutdown`] has run, since the SDK reports
    /// that as an error and a clean exit should not print one.
    pub fn flush(&self) {
        if self.is_shut_down() {
            return;
        }
        if let Some(provider) = &self.provider {
            if let Err(e) = provider.force_flush() {
                eprintln!("tracing: flush failed: {e}");
            }
        }
    }

    /// Flush and stop the exporter, waiting at most [`SHUTDOWN_FLUSH_TIMEOUT`]
    /// for delivery.
    ///
    /// A server should call this on the way out. Without it, spans still sitting
    /// in the batch queue are dropped when the process exits, so the last few
    /// requests before a deploy are exactly the ones that go missing — which is
    /// when a trace is most useful.
    pub fn shutdown(&self) {
        // Set first: a concurrent `flush` must see the shutdown and skip rather
        // than race the provider into an error.
        self.shut_down.store(true, Ordering::Relaxed);
        if let Some(provider) = &self.provider {
            if let Err(e) = provider.shutdown_with_timeout(SHUTDOWN_FLUSH_TIMEOUT) {
                eprintln!("tracing: shutdown failed: {e}");
            }
        }
    }
}

impl Drop for TelemetryGuard {
    fn drop(&mut self) {
        self.flush();
    }
}

/// Resolved tracing configuration, kept separate from `init` so it can be
/// inspected and tested without installing global state.
#[derive(Debug, Clone, PartialEq)]
pub struct TelemetryConfig {
    /// OTLP/HTTP endpoint for spans. `None` disables exporting.
    pub endpoint: Option<String>,
    /// `service.name` reported on every span.
    pub service_name: String,
    /// Fraction of traces to record, `0.0..=1.0`.
    pub sample_ratio: f64,
    /// Extra headers for the collector, e.g. an auth token.
    pub headers: HashMap<String, String>,
}

impl TelemetryConfig {
    /// Read configuration from the environment.
    ///
    /// Every setting accepts a `GRAPHNIGHT_`-prefixed variable as well as the
    /// standard OpenTelemetry name, so an existing collector setup works
    /// unchanged and GraphNight-specific deployments can stay namespaced.
    ///
    /// | Setting | GraphNight | OpenTelemetry |
    /// |---|---|---|
    /// | endpoint | `GRAPHNIGHT_OTEL_EXPORTER_OTLP_ENDPOINT` | `OTEL_EXPORTER_OTLP_ENDPOINT` |
    /// | service name | `GRAPHNIGHT_OTEL_SERVICE_NAME` | `OTEL_SERVICE_NAME` |
    /// | sample ratio | `GRAPHNIGHT_OTEL_TRACES_SAMPLER_ARG` | `OTEL_TRACES_SAMPLER_ARG` |
    /// | headers | `GRAPHNIGHT_OTEL_EXPORTER_OTLP_HEADERS` | `OTEL_EXPORTER_OTLP_HEADERS` |
    ///
    /// Tracing is off unless an endpoint is set. A ratio of `1.0` (the default)
    /// records everything, which is right for a debugging deployment and
    /// expensive for a busy one.
    pub fn from_env(default_service_name: &str) -> Self {
        let endpoint = first_env(&[
            "GRAPHNIGHT_OTEL_EXPORTER_OTLP_ENDPOINT",
            "OTEL_EXPORTER_OTLP_ENDPOINT",
        ])
        // A collector with a sidecar or a default gRPC port is common enough
        // that an explicit opt-in beats a silent exporter that hangs.
        .filter(|v| !v.trim().is_empty());

        let service_name = first_env(&["GRAPHNIGHT_OTEL_SERVICE_NAME", "OTEL_SERVICE_NAME"])
            .filter(|v| !v.trim().is_empty())
            .unwrap_or_else(|| default_service_name.to_string());

        let sample_ratio = first_env(&[
            "GRAPHNIGHT_OTEL_TRACES_SAMPLER_ARG",
            "OTEL_TRACES_SAMPLER_ARG",
        ])
        .and_then(|v| v.trim().parse::<f64>().ok())
        // Guard the input rather than trusting it: a ratio of 9 or -1 would
        // otherwise be handed straight to the sampler.
        .map(|r| r.clamp(0.0, 1.0))
        .unwrap_or(1.0);

        let headers = parse_headers(&first_env(&[
            "GRAPHNIGHT_OTEL_EXPORTER_OTLP_HEADERS",
            "OTEL_EXPORTER_OTLP_HEADERS",
        ]));

        Self {
            endpoint,
            service_name,
            sample_ratio,
            headers,
        }
    }

    /// Whether spans should be exported at all.
    pub fn is_enabled(&self) -> bool {
        self.endpoint.is_some()
    }
}

/// First non-empty value among `names`.
fn first_env(names: &[&str]) -> Option<String> {
    names
        .iter()
        .find_map(|name| std::env::var(name).ok())
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
}

/// Parse the OTLP headers format: `key1=value1,key2=value2`.
fn parse_headers(raw: &Option<String>) -> HashMap<String, String> {
    let Some(raw) = raw else {
        return HashMap::new();
    };
    raw.split(',')
        .filter_map(|pair| {
            let (key, value) = pair.split_once('=')?;
            let (key, value) = (key.trim(), value.trim());
            if key.is_empty() {
                return None;
            }
            Some((key.to_string(), value.trim_matches('"').to_string()))
        })
        .collect()
}

/// Install the global subscriber: human-readable logs always, OTLP traces when
/// configured.
///
/// Safe to call from a synchronous `fn main`: the batch processor owns a
/// dedicated thread and the OTLP client is a blocking one, so no Tokio runtime
/// is required to be in scope.
///
/// `log_filter` is an `EnvFilter` directive set such as `info` or
/// `graphnight_sql=debug`. It applies to the log output only, so a deployment
/// can keep logs quiet and traces verbose or vice versa.
///
/// Returns a guard the caller should keep alive. When exporting is disabled the
/// guard is inert and no exporter thread is started.
///
/// # Errors
/// Fails if no global subscriber can be installed, which means something else
/// already set one. That is a programming error, not a runtime condition.
pub fn init(log_filter: EnvFilter, config: &TelemetryConfig) -> anyhow::Result<TelemetryGuard> {
    let fmt_layer = tracing_subscriber::fmt::layer()
        .with_target(true)
        .with_writer(std::io::stderr);

    if !config.is_enabled() {
        tracing_subscriber::registry()
            .with(log_filter)
            .with(fmt_layer)
            .try_init()
            .map_err(|e| anyhow::anyhow!("failed to install tracing subscriber: {e}"))?;
        return Ok(TelemetryGuard::default());
    }

    let endpoint = config
        .endpoint
        .clone()
        .expect("checked by is_enabled above");

    let exporter = SpanExporter::builder()
        .with_http()
        .with_endpoint(endpoint.clone())
        .with_headers(config.headers.clone())
        .build()
        .map_err(|e| anyhow::anyhow!("invalid OTLP endpoint {endpoint}: {e}"))?;

    let sampler = if config.sample_ratio >= 1.0 {
        Sampler::AlwaysOn
    } else {
        Sampler::ParentBased(Box::new(Sampler::TraceIdRatioBased(config.sample_ratio)))
    };

    let resource = opentelemetry_sdk::Resource::builder()
        .with_service_name(config.service_name.clone())
        .build();

    let provider = SdkTracerProvider::builder()
        .with_batch_exporter(exporter)
        .with_sampler(sampler)
        .with_resource(resource)
        .build();

    let tracer = provider.tracer("graphnight");
    let otel_layer = tracing_opentelemetry::layer().with_tracer(tracer);

    tracing_subscriber::registry()
        .with(log_filter)
        .with(fmt_layer)
        .with(otel_layer)
        .try_init()
        .map_err(|e| anyhow::anyhow!("failed to install tracing subscriber: {e}"))?;

    Ok(TelemetryGuard {
        provider: Some(provider),
        shut_down: Arc::new(AtomicBool::new(false)),
    })
}

/// Convenience wrapper: read config from the environment, then [`init`].
///
/// Exporter setup failures are reported as warnings and tracing is left in
/// plain-log mode rather than aborting startup. A collector that is
/// misconfigured should not take the semantic layer down with it.
pub fn init_from_env(log_filter: EnvFilter, default_service_name: &str) -> TelemetryGuard {
    let config = TelemetryConfig::from_env(default_service_name);
    match init(log_filter.clone(), &config) {
        Ok(guard) => guard,
        Err(e) => {
            eprintln!("tracing: {e}");
            let _ = tracing_subscriber::registry()
                .with(log_filter)
                .with(tracing_subscriber::fmt::layer().with_writer(std::io::stderr))
                .try_init();
            TelemetryGuard::default()
        }
    }
}

/// How long [`TelemetryGuard::shutdown`] waits for buffered spans to reach the
/// collector before giving up.
pub const SHUTDOWN_FLUSH_TIMEOUT: Duration = Duration::from_secs(5);

/// Load `.env` into the process environment, if present.
///
/// This is what makes `.env.example` actionable: a developer copies it to
/// `.env`, fills it in, and the binary picks it up with no export boilerplate.
/// `.env` is git-ignored, so credentials stay out of history.
///
/// Existing variables win. A value already exported into the environment is
/// the deliberate one, and silently overwriting it from a file in the working
/// directory is the kind of surprise that costs an afternoon. A missing `.env`
/// is not an error, since production injects real environment variables.
///
/// # Errors
/// Fails only when a `.env` file exists but cannot be parsed, which is worth
/// surfacing: a typo in a secret name should not look like "not configured".
pub fn load_dotenv() -> anyhow::Result<()> {
    match dotenvy::dotenv() {
        Ok(_) => Ok(()),
        Err(dotenvy::Error::Io(e)) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(anyhow::anyhow!("failed to read .env: {e}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Env-var tests share one process, so they take a lock rather than running
    // in parallel with each other.
    static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    const OTEL_VARS: &[&str] = &[
        "GRAPHNIGHT_OTEL_EXPORTER_OTLP_ENDPOINT",
        "OTEL_EXPORTER_OTLP_ENDPOINT",
        "GRAPHNIGHT_OTEL_SERVICE_NAME",
        "OTEL_SERVICE_NAME",
        "GRAPHNIGHT_OTEL_TRACES_SAMPLER_ARG",
        "OTEL_TRACES_SAMPLER_ARG",
        "GRAPHNIGHT_OTEL_EXPORTER_OTLP_HEADERS",
        "OTEL_EXPORTER_OTLP_HEADERS",
    ];

    fn with_clean_env(f: impl FnOnce()) {
        let _guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        for var in OTEL_VARS {
            std::env::remove_var(var);
        }
        f();
        for var in OTEL_VARS {
            std::env::remove_var(var);
        }
    }

    #[test]
    fn tracing_is_off_without_an_endpoint() {
        with_clean_env(|| {
            let config = TelemetryConfig::from_env("graphnight-server");
            assert!(!config.is_enabled());
            assert_eq!(config.endpoint, None);
            assert_eq!(config.service_name, "graphnight-server");
        });
    }

    #[test]
    fn endpoint_enables_tracing() {
        with_clean_env(|| {
            std::env::set_var(
                "GRAPHNIGHT_OTEL_EXPORTER_OTLP_ENDPOINT",
                "http://collector:4318/v1/traces",
            );
            let config = TelemetryConfig::from_env("graphnight-server");
            assert!(config.is_enabled());
            assert_eq!(
                config.endpoint.as_deref(),
                Some("http://collector:4318/v1/traces")
            );
        });
    }

    #[test]
    fn standard_otel_vars_are_honored() {
        with_clean_env(|| {
            // The common case: a deployment already configures its collector
            // the standard way and GraphNight should not need its own vars.
            std::env::set_var("OTEL_EXPORTER_OTLP_ENDPOINT", "http://otel:4318");
            std::env::set_var("OTEL_SERVICE_NAME", "semantic-layer");
            let config = TelemetryConfig::from_env("graphnight-server");
            assert!(config.is_enabled());
            assert_eq!(config.service_name, "semantic-layer");
        });
    }

    #[test]
    fn graphnight_vars_win_over_standard() {
        with_clean_env(|| {
            std::env::set_var("OTEL_EXPORTER_OTLP_ENDPOINT", "http://otel:4318");
            std::env::set_var("OTEL_SERVICE_NAME", "from-otel");
            std::env::set_var("GRAPHNIGHT_OTEL_SERVICE_NAME", "from-graphnight");
            let config = TelemetryConfig::from_env("graphnight-server");
            assert_eq!(config.service_name, "from-graphnight");
        });
    }

    #[test]
    fn blank_endpoint_does_not_enable_tracing() {
        with_clean_env(|| {
            // Set but empty/whitespace must not start an exporter that then
            // fails to reach "nothing".
            std::env::set_var("GRAPHNIGHT_OTEL_EXPORTER_OTLP_ENDPOINT", "   ");
            assert!(!TelemetryConfig::from_env("x").is_enabled());
            std::env::set_var("GRAPHNIGHT_OTEL_EXPORTER_OTLP_ENDPOINT", "");
            assert!(!TelemetryConfig::from_env("x").is_enabled());
        });
    }

    #[test]
    fn sample_ratio_defaults_to_full_and_clamps() {
        with_clean_env(|| {
            assert_eq!(TelemetryConfig::from_env("x").sample_ratio, 1.0);

            std::env::set_var("GRAPHNIGHT_OTEL_TRACES_SAMPLER_ARG", "0.05");
            assert!((TelemetryConfig::from_env("x").sample_ratio - 0.05).abs() < f64::EPSILON);

            // Garbage and out-of-range values must not be passed through to the
            // sampler, which expects a probability.
            std::env::set_var("GRAPHNIGHT_OTEL_TRACES_SAMPLER_ARG", "lots");
            assert_eq!(TelemetryConfig::from_env("x").sample_ratio, 1.0);
            std::env::set_var("GRAPHNIGHT_OTEL_TRACES_SAMPLER_ARG", "9");
            assert_eq!(TelemetryConfig::from_env("x").sample_ratio, 1.0);
            std::env::set_var("GRAPHNIGHT_OTEL_TRACES_SAMPLER_ARG", "-1");
            assert_eq!(TelemetryConfig::from_env("x").sample_ratio, 0.0);
        });
    }

    #[test]
    fn headers_parse_into_pairs() {
        with_clean_env(|| {
            std::env::set_var(
                "GRAPHNIGHT_OTEL_EXPORTER_OTLP_HEADERS",
                "authorization=Bearer tok123, x-tenant=acme",
            );
            let headers = TelemetryConfig::from_env("x").headers;
            assert_eq!(headers.get("authorization").unwrap(), "Bearer tok123");
            assert_eq!(headers.get("x-tenant").unwrap(), "acme");
        });
    }

    #[test]
    fn quoted_and_malformed_headers() {
        with_clean_env(|| {
            std::env::set_var(
                "GRAPHNIGHT_OTEL_EXPORTER_OTLP_HEADERS",
                "authorization=\"Bearer quoted\",broken,=novalue",
            );
            let headers = TelemetryConfig::from_env("x").headers;
            // Quotes are stripped, and fragments without a key are dropped
            // rather than producing an empty header name.
            assert_eq!(headers.get("authorization").unwrap(), "Bearer quoted");
            assert_eq!(headers.len(), 1);
        });
    }

    #[test]
    fn dotenv_never_overrides_a_real_environment_variable() {
        // The precedence rule is the whole reason this is safe to do
        // unconditionally at startup: an exported value must win over .env.
        let _guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        std::env::remove_var("GRAPHNIGHT_OTEL_EXPORTER_OTLP_ENDPOINT");
        std::env::set_var(
            "GRAPHNIGHT_OTEL_EXPORTER_OTLP_ENDPOINT",
            "http://from-shell:4318",
        );
        // No .env in the crate directory, so this is a no-op that must not fail.
        load_dotenv().expect("a missing .env is not an error");
        assert_eq!(
            std::env::var("GRAPHNIGHT_OTEL_EXPORTER_OTLP_ENDPOINT").unwrap(),
            "http://from-shell:4318"
        );
        std::env::remove_var("GRAPHNIGHT_OTEL_EXPORTER_OTLP_ENDPOINT");
    }

    #[test]
    fn init_without_endpoint_installs_logs_only() {
        with_clean_env(|| {
            let config = TelemetryConfig::from_env("graphnight-test");
            let guard =
                init(EnvFilter::new("warn"), &config).expect("plain logging should install");
            assert!(guard.provider.is_none());
            // A flush with no exporter is a no-op, and must not panic.
            guard.flush();
            // Shutting down with no exporter is also a no-op.
            guard.shutdown();
            guard.flush();
        });
    }
}
