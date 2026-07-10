//! OpenTelemetry tracing integration (Phase 6.1).
//!
//! Traces are only exported when `OTEL_EXPORTER_OTLP_ENDPOINT` is set.
//! When it is unset, `init_layer()` returns `None` and the application keeps
//! its plain `fmt` subscriber (zero overhead, no external dependency).

use opentelemetry::trace::TracerProvider as _;
use opentelemetry_otlp::WithExportConfig;
use opentelemetry_sdk::runtime;
use opentelemetry_sdk::trace::TracerProvider;
use tracing_subscriber::layer::Layer;
use tracing_subscriber::registry::Registry;

/// Process-lifetime handle to the tracer provider so it is never dropped
/// (dropping it would shut down the export pipeline).
static PROVIDER: std::sync::OnceLock<TracerProvider> = std::sync::OnceLock::new();

/// Build an OpenTelemetry tracing layer, or `None` when tracing is disabled.
///
/// The returned layer is `Box<dyn Layer>` so callers can attach it
/// unconditionally (pairing it with an `Identity` no-op when disabled).
pub fn init_layer() -> Option<Box<dyn Layer<Registry> + Send + Sync>> {
    let endpoint = std::env::var("OTEL_EXPORTER_OTLP_ENDPOINT").ok()?;
    if endpoint.is_empty() {
        return None;
    }

    let exporter = opentelemetry_otlp::SpanExporter::builder()
        .with_tonic()
        .with_endpoint(endpoint)
        .build()
        .map_err(|error| {
            tracing::warn!("failed to build OpenTelemetry exporter: {error}");
        })
        .ok()?;

    let provider = TracerProvider::builder()
        .with_batch_exporter(exporter, runtime::Tokio)
        .build();

    // Keep the provider alive for the process lifetime.
    let _ = PROVIDER.set(provider);
    let provider = PROVIDER.get().expect("provider just set");

    let tracer = provider.tracer("blackrouter");
    let layer = tracing_opentelemetry::layer().with_tracer(tracer);
    Some(Box::new(layer))
}
