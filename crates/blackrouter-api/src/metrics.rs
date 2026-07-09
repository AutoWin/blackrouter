use prometheus::{Encoder, HistogramVec, IntCounterVec, IntGauge, Registry, TextEncoder};
use std::sync::Arc;

#[derive(Clone)]
pub struct Metrics {
    pub registry: Arc<Registry>,
    pub requests_total: IntCounterVec,
    pub request_duration: HistogramVec,
    pub stream_ttfb: HistogramVec,
    pub tokens_total: IntCounterVec,
    pub open_connections: IntGauge,
}

impl Metrics {
    pub fn new() -> Self {
        let registry = Arc::new(Registry::new());

        // Use IntCounterVec::new / HistogramVec::new / IntGauge::new so we
        // register into our *custom* registry only — the `register_*!` macros
        // would also register into the global default registry, causing
        // double-registration panics in tests and polluting `/metrics`.
        let requests_total = IntCounterVec::new(
            prometheus::Opts::new("blackrouter_requests_total", "Total number of requests"),
            &["provider", "model", "status"],
        )
        .unwrap();

        let request_duration = HistogramVec::new(
            prometheus::HistogramOpts::new(
                "blackrouter_request_duration_seconds",
                "Request duration in seconds",
            )
            .buckets(vec![0.01, 0.05, 0.1, 0.5, 1.0, 5.0, 10.0, 30.0, 60.0]),
            &["provider", "model"],
        )
        .unwrap();

        let stream_ttfb = HistogramVec::new(
            prometheus::HistogramOpts::new(
                "blackrouter_stream_ttfb_seconds",
                "Time to first byte for streaming requests",
            )
            .buckets(vec![0.05, 0.1, 0.25, 0.5, 1.0, 2.0, 5.0]),
            &["provider", "model"],
        )
        .unwrap();

        let tokens_total = IntCounterVec::new(
            prometheus::Opts::new("blackrouter_tokens_total", "Total tokens processed"),
            &["provider", "model", "type"],
        )
        .unwrap();

        let open_connections =
            IntGauge::new("blackrouter_open_connections", "Current open connections").unwrap();

        // Register process collector into the custom registry (not global).
        let process_collector = prometheus::process_collector::ProcessCollector::for_self();
        registry
            .register(Box::new(process_collector))
            .expect("failed to register process collector");

        // Register all metrics into the custom registry only.
        registry
            .register(Box::new(requests_total.clone()))
            .expect("failed to register requests_total");
        registry
            .register(Box::new(request_duration.clone()))
            .expect("failed to register request_duration");
        registry
            .register(Box::new(stream_ttfb.clone()))
            .expect("failed to register stream_ttfb");
        registry
            .register(Box::new(tokens_total.clone()))
            .expect("failed to register tokens_total");
        registry
            .register(Box::new(open_connections.clone()))
            .expect("failed to register open_connections");

        Self {
            registry,
            requests_total,
            request_duration,
            stream_ttfb,
            tokens_total,
            open_connections,
        }
    }

    pub fn encode(&self) -> String {
        let mut buffer = vec![];
        let encoder = TextEncoder::new();
        let metric_families = self.registry.gather();
        encoder.encode(&metric_families, &mut buffer).ok();
        String::from_utf8(buffer).unwrap_or_default()
    }
}

impl Default for Metrics {
    fn default() -> Self {
        Self::new()
    }
}
