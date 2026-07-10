use blackrouter_common::unix_timestamp;
use blackrouter_config::AppConfig;
use blackrouter_rtk::{RateLimitConfig, Rtk, RtkCircuitEntry, RtkRateEntry, RtkSnapshot};
use blackrouter_storage::{RtkStateRow, Storage, UsageEntry};
use std::time::Duration;

use crate::metrics::Metrics;
use blackrouter_rtk::ResponseCache;
use serde_json;

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
    pub async fn new(config: AppConfig, storage: Storage) -> Self {
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

        let rtk = Rtk::new(rtk_config);

        // Restore durable RTK state after a restart (Phase 5.3) so rate-limit
        // windows and circuit breakers are not lost (which would otherwise let
        // a burst of traffic through or immediately flood an unhealthy
        // provider). Snapshots are reconstructed conservatively (window treated
        // as just-started) so we never over-report remaining capacity.
        if let Ok(rows) = storage.load_rtk_state() {
            let mut snapshot = RtkSnapshot::default();
            for row in &rows {
                match row.kind.as_str() {
                    "rate_limit" => {
                        if let Ok(entry) = serde_json::from_str::<RtkRateEntry>(&row.data) {
                            snapshot.rate_limits.push(entry);
                        }
                    }
                    "circuit" => {
                        if let Ok(entry) = serde_json::from_str::<RtkCircuitEntry>(&row.data) {
                            snapshot.circuit_breakers.push(entry);
                        }
                    }
                    _ => {}
                }
            }
            if !snapshot.rate_limits.is_empty() || !snapshot.circuit_breakers.is_empty() {
                rtk.restore(&snapshot).await;
                tracing::info!(
                    "restored {} rate-limit and {} circuit-breaker entries from durable state",
                    snapshot.rate_limits.len(),
                    snapshot.circuit_breakers.len()
                );
            }
        }

        // Periodically snapshot RTK state to SQLite so it survives restarts.
        let snap_rtk = rtk.clone();
        let snap_storage = storage.clone();
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(Duration::from_secs(10));
            loop {
                interval.tick().await;
                let snapshot = snap_rtk.snapshot().await;
                let mut rows: Vec<RtkStateRow> = snapshot
                    .rate_limits
                    .iter()
                    .map(|e| RtkStateRow {
                        key: format!(
                            "rl:{}\u{0}{}\u{0}{}",
                            e.key.provider,
                            e.key.model,
                            e.key.api_key.as_deref().unwrap_or("")
                        ),
                        kind: "rate_limit".to_string(),
                        data: serde_json::to_string(e).unwrap_or_default(),
                        updated_at: snapshot.captured_at_unix,
                    })
                    .collect();
                rows.extend(snapshot.circuit_breakers.iter().map(|e| RtkStateRow {
                    key: format!("cb:{}", e.connection_id),
                    kind: "circuit".to_string(),
                    data: serde_json::to_string(e).unwrap_or_default(),
                    updated_at: snapshot.captured_at_unix,
                }));
                if let Err(error) = snap_storage.save_rtk_state(&rows) {
                    tracing::warn!("failed to persist RTK state: {error}");
                }
                if let Err(error) =
                    snap_storage.prune_rtk_state(snapshot.captured_at_unix.saturating_sub(60))
                {
                    tracing::warn!("failed to prune RTK state: {error}");
                }
            }
        });

        Self {
            config,
            storage,
            started_at_unix: unix_timestamp(),
            rtk,
            http_client,
            usage_tx,
            metrics: Metrics::new(),
            response_cache: ResponseCache::new(Duration::from_secs(300), 1000),
        }
    }
}
