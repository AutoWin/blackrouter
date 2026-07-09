use blackrouter_common::unix_timestamp;
use blackrouter_config::AppConfig;
use blackrouter_rtk::{RateLimitConfig, Rtk};
use blackrouter_storage::{Storage, UsageEntry};
use std::time::Duration;

use crate::metrics::Metrics;
use blackrouter_rtk::ResponseCache;

#[derive(Clone)]
pub struct AppState {
    pub config: AppConfig,
    pub storage: Storage,
    pub started_at_unix: u64,
    pub rtk: Rtk,
    pub http_client: reqwest::Client,
    pub usage_tx: tokio::sync::mpsc::UnboundedSender<UsageEntry>,
    pub metrics: Metrics,
    pub response_cache: ResponseCache,
}

impl AppState {
    pub fn new(config: AppConfig, storage: Storage) -> Self {
        let rtk_config = RateLimitConfig {
            requests_per_minute: 60,
            tokens_per_minute: 100_000,
            max_concurrent: 10,
            enabled: true,
        };

        // Shared HTTP client with connection pooling (Phase 1.2)
        let http_client = reqwest::Client::builder()
            .pool_max_idle_per_host(100)
            .pool_idle_timeout(Duration::from_secs(90))
            .tcp_keepalive(Duration::from_secs(60))
            .tcp_nodelay(true)
            .timeout(Duration::from_secs(600))
            .redirect(reqwest::redirect::Policy::limited(3))
            .build()
            .expect("Failed to build shared HTTP client");

        // Batch usage writer (Phase 1.4): buffer entries, flush every 5s or at 100 entries
        let (usage_tx, mut usage_rx) = tokio::sync::mpsc::unbounded_channel::<UsageEntry>();
        let batch_storage = storage.clone();
        tokio::spawn(async move {
            let mut buffer: Vec<UsageEntry> = Vec::with_capacity(128);
            let mut interval = tokio::time::interval(Duration::from_secs(5));
            loop {
                tokio::select! {
                    Some(entry) = usage_rx.recv() => {
                        buffer.push(entry);
                        if buffer.len() >= 100 {
                            let batch = std::mem::take(&mut buffer);
                            if let Err(e) = batch_storage.record_usages_batch(&batch) {
                                tracing::warn!("batch usage write failed: {e}");
                            }
                        }
                    }
                    _ = interval.tick() => {
                        if !buffer.is_empty() {
                            let batch = std::mem::take(&mut buffer);
                            if let Err(e) = batch_storage.record_usages_batch(&batch) {
                                tracing::warn!("batch usage flush failed: {e}");
                            }
                        }
                    }
                }
            }
        });

        Self {
            config,
            storage,
            started_at_unix: unix_timestamp(),
            rtk: Rtk::new(rtk_config),
            http_client,
            usage_tx,
            metrics: Metrics::new(),
            response_cache: ResponseCache::new(Duration::from_secs(300), 1000),
        }
    }
}
