use blackrouter_common::unix_timestamp;
use blackrouter_config::AppConfig;
use blackrouter_rtk::{RateLimitConfig, Rtk, RtkCircuitEntry, RtkRateEntry, RtkSnapshot};
use blackrouter_storage::{RtkStateRow, Storage, UsageEntry};
use std::time::Duration;

use crate::metrics::Metrics;
use blackrouter_rtk::ResponseCache;
use serde::{Deserialize, Serialize};
use serde_json;
use std::sync::{Arc, RwLock};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RuntimeSettings {
    pub require_api_key: bool,
    pub health_probe_enabled: bool,
    pub health_probe_interval_seconds: u64,
    pub health_probe_timeout_seconds: u64,
    pub health_probe_failure_threshold: u32,
    pub semantic_memory_enabled: bool,
    pub semantic_memory_top_k: usize,
    pub semantic_memory_dim: usize,
    pub semantic_memory_embedder: String,
}

#[derive(Clone, Debug, Default, Serialize)]
pub struct ProviderProbeSummary {
    pub last_run_unix: Option<u64>,
    pub healthy: usize,
    pub unhealthy: usize,
    pub pending: usize,
}

#[derive(Clone)]
pub struct SharedStateStore {
    client: redis::Client,
    prefix: String,
}

pub struct SharedRatePermit {
    client: redis::Client,
    concurrent_key: String,
}

impl Drop for SharedRatePermit {
    fn drop(&mut self) {
        let client = self.client.clone();
        let key = self.concurrent_key.clone();
        if let Ok(runtime) = tokio::runtime::Handle::try_current() {
            runtime.spawn(async move {
                if let Ok(mut connection) = client.get_multiplexed_async_connection().await {
                    let script = redis::Script::new(
                        "local n=redis.call('DECR',KEYS[1]); if n<=0 then redis.call('DEL',KEYS[1]) end; return n",
                    );
                    let _ = script.key(key).invoke_async::<i64>(&mut connection).await;
                }
            });
        }
    }
}

#[derive(Serialize, Deserialize)]
struct SharedCacheEntry {
    body: Vec<u8>,
    content_type: Option<String>,
}

impl SharedStateStore {
    fn new(url: &str, prefix: String) -> Option<Self> {
        match redis::Client::open(url) {
            Ok(client) => Some(Self { client, prefix }),
            Err(error) => {
                tracing::warn!(%error, "invalid Redis shared-state URL; using local state");
                None
            }
        }
    }

    fn key(&self, suffix: &str) -> String {
        format!("{}:{suffix}", self.prefix)
    }

    pub async fn ping(&self) -> bool {
        let Ok(mut connection) = self.client.get_multiplexed_async_connection().await else {
            return false;
        };
        redis::cmd("PING")
            .query_async::<String>(&mut connection)
            .await
            .is_ok()
    }

    async fn load_rtk(&self) -> Option<RtkSnapshot> {
        let mut connection = self.client.get_multiplexed_async_connection().await.ok()?;
        let raw = redis::cmd("GET")
            .arg(self.key("rtk:snapshot"))
            .query_async::<Option<String>>(&mut connection)
            .await
            .ok()??;
        serde_json::from_str(&raw).ok()
    }

    async fn save_rtk(&self, snapshot: &RtkSnapshot) -> redis::RedisResult<()> {
        let mut connection = self.client.get_multiplexed_async_connection().await?;
        let raw = serde_json::to_string(snapshot).unwrap_or_default();
        redis::cmd("SET")
            .arg(self.key("rtk:snapshot"))
            .arg(raw)
            .arg("EX")
            .arg(120)
            .query_async::<()>(&mut connection)
            .await
    }

    async fn cache_get(&self, key: &str) -> Option<(Vec<u8>, Option<String>)> {
        let mut connection = self.client.get_multiplexed_async_connection().await.ok()?;
        let raw = redis::cmd("GET")
            .arg(self.key(&format!("cache:{key}")))
            .query_async::<Option<Vec<u8>>>(&mut connection)
            .await
            .ok()??;
        let entry = serde_json::from_slice::<SharedCacheEntry>(&raw).ok()?;
        Some((entry.body, entry.content_type))
    }

    async fn cache_put(
        &self,
        key: &str,
        body: &[u8],
        content_type: Option<String>,
    ) -> redis::RedisResult<()> {
        let mut connection = self.client.get_multiplexed_async_connection().await?;
        let raw = serde_json::to_vec(&SharedCacheEntry {
            body: body.to_vec(),
            content_type,
        })
        .unwrap_or_default();
        redis::cmd("SET")
            .arg(self.key(&format!("cache:{key}")))
            .arg(raw)
            .arg("EX")
            .arg(300)
            .query_async::<()>(&mut connection)
            .await
    }

    pub async fn acquire_rate_permit(
        &self,
        key: &str,
        request_limit: u64,
        token_limit: u64,
        concurrent_limit: u64,
    ) -> Result<Option<SharedRatePermit>, ()> {
        let Ok(mut connection) = self.client.get_multiplexed_async_connection().await else {
            tracing::warn!("Redis rate-limit connection failed; falling back to local RTK");
            return Ok(None);
        };
        let request_key = self.key(&format!("rate:requests:{key}"));
        let token_key = self.key(&format!("rate:tokens:{key}"));
        let concurrent_key = self.key(&format!("rate:concurrent:{key}"));
        let script = redis::Script::new(
            "local r=tonumber(redis.call('GET',KEYS[1]) or '0'); local t=tonumber(redis.call('GET',KEYS[2]) or '0'); local c=tonumber(redis.call('GET',KEYS[3]) or '0'); if r>=tonumber(ARGV[1]) or t>=tonumber(ARGV[2]) or c>=tonumber(ARGV[3]) then return 0 end; r=redis.call('INCR',KEYS[1]); if r==1 then redis.call('EXPIRE',KEYS[1],60) end; c=redis.call('INCR',KEYS[3]); if c==1 then redis.call('EXPIRE',KEYS[3],600) end; return 1",
        );
        match script
            .key(request_key)
            .key(token_key)
            .key(&concurrent_key)
            .arg(request_limit)
            .arg(token_limit)
            .arg(concurrent_limit)
            .invoke_async::<i64>(&mut connection)
            .await
        {
            Ok(1) => Ok(Some(SharedRatePermit {
                client: self.client.clone(),
                concurrent_key,
            })),
            Ok(_) => Err(()),
            Err(error) => {
                tracing::warn!(%error, "Redis rate-limit check failed; falling back to local RTK");
                Ok(None)
            }
        }
    }

    pub async fn record_tokens(&self, key: &str, tokens: u64) {
        if tokens == 0 {
            return;
        }
        let Ok(mut connection) = self.client.get_multiplexed_async_connection().await else {
            return;
        };
        let script = redis::Script::new(
            "local n=redis.call('INCRBY',KEYS[1],ARGV[1]); if n==tonumber(ARGV[1]) then redis.call('EXPIRE',KEYS[1],60) end; return n",
        );
        let _ = script
            .key(self.key(&format!("rate:tokens:{key}")))
            .arg(tokens)
            .invoke_async::<i64>(&mut connection)
            .await;
    }
}

impl RuntimeSettings {
    fn load(config: &AppConfig, storage: &Storage) -> Self {
        let settings = storage.settings_json().unwrap_or_default();
        let probe = settings
            .get("healthProbe")
            .or_else(|| settings.get("health_probe"));
        Self {
            require_api_key: settings
                .get("requireApiKey")
                .or_else(|| settings.get("require_api_key"))
                .and_then(serde_json::Value::as_bool)
                .unwrap_or(config.require_api_key),
            health_probe_enabled: probe
                .and_then(|value| value.get("enabled"))
                .and_then(serde_json::Value::as_bool)
                .unwrap_or(true),
            health_probe_interval_seconds: probe
                .and_then(|value| value.get("intervalSeconds"))
                .and_then(serde_json::Value::as_u64)
                .unwrap_or(30)
                .max(5),
            health_probe_timeout_seconds: probe
                .and_then(|value| value.get("timeoutSeconds"))
                .and_then(serde_json::Value::as_u64)
                .unwrap_or(5)
                .max(1),
            health_probe_failure_threshold: probe
                .and_then(|value| value.get("failureThreshold"))
                .and_then(serde_json::Value::as_u64)
                .unwrap_or(3)
                .max(1) as u32,
            semantic_memory_enabled: settings
                .get("semanticMemory")
                .and_then(|value| value.get("enabled"))
                .and_then(serde_json::Value::as_bool)
                .unwrap_or(false),
            semantic_memory_top_k: settings
                .get("semanticMemory")
                .and_then(|value| value.get("topK"))
                .and_then(serde_json::Value::as_u64)
                .unwrap_or(3)
                .max(1) as usize,
            semantic_memory_dim: settings
                .get("semanticMemory")
                .and_then(|value| value.get("dim"))
                .and_then(serde_json::Value::as_u64)
                .unwrap_or(256)
                .max(16) as usize,
            semantic_memory_embedder: settings
                .get("semanticMemory")
                .and_then(|value| value.get("embedder"))
                .and_then(serde_json::Value::as_str)
                .unwrap_or("local")
                .to_string(),
        }
    }
}

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
    runtime_settings: Arc<RwLock<RuntimeSettings>>,
    provider_probe_summary: Arc<RwLock<ProviderProbeSummary>>,
    pub shared_state: Option<SharedStateStore>,
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
        let shared_state = config
            .redis_url
            .as_deref()
            .and_then(|url| SharedStateStore::new(url, config.shared_state_prefix.clone()));

        // Restore durable RTK state after a restart (Phase 5.3) so rate-limit
        // windows and circuit breakers are not lost (which would otherwise let
        // a burst of traffic through or immediately flood an unhealthy
        // provider). Snapshots are reconstructed conservatively (window treated
        // as just-started) so we never over-report remaining capacity.
        let shared_snapshot = match &shared_state {
            Some(shared) => shared.load_rtk().await,
            None => None,
        };
        if let Some(snapshot) = shared_snapshot {
            rtk.restore(&snapshot).await;
            tracing::info!("restored RTK state from Redis shared state");
        } else if let Ok(rows) = storage.load_rtk_state() {
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
        let snap_shared = shared_state.clone();
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(Duration::from_secs(10));
            loop {
                interval.tick().await;
                let snapshot = snap_rtk.snapshot().await;
                if let Some(shared) = &snap_shared {
                    if let Err(error) = shared.save_rtk(&snapshot).await {
                        tracing::warn!(%error, "failed to persist RTK state to Redis");
                    }
                }
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

        let runtime_settings = Arc::new(RwLock::new(RuntimeSettings::load(&config, &storage)));

        let state = Self {
            config,
            storage,
            started_at_unix: unix_timestamp(),
            rtk,
            http_client,
            usage_tx,
            metrics: Metrics::new(),
            response_cache: ResponseCache::new(Duration::from_secs(300), 1000),
            runtime_settings,
            provider_probe_summary: Arc::new(RwLock::new(ProviderProbeSummary::default())),
            shared_state,
        };
        crate::spawn_provider_health_prober(state.clone());
        state
    }

    pub fn runtime_settings(&self) -> RuntimeSettings {
        self.runtime_settings
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone()
    }

    pub fn reload_runtime_settings(&self) -> RuntimeSettings {
        let loaded = RuntimeSettings::load(&self.config, &self.storage);
        *self
            .runtime_settings
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = loaded.clone();
        loaded
    }

    pub fn provider_probe_summary(&self) -> ProviderProbeSummary {
        self.provider_probe_summary
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone()
    }

    pub fn set_provider_probe_summary(&self, summary: ProviderProbeSummary) {
        *self
            .provider_probe_summary
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = summary;
    }

    pub async fn cache_get(&self, key: &str) -> Option<(Vec<u8>, Option<String>)> {
        if let Some(value) = self.response_cache.get(key).await {
            return Some(value);
        }
        let value = match &self.shared_state {
            Some(shared) => shared.cache_get(key).await,
            None => None,
        };
        if let Some((body, content_type)) = &value {
            self.response_cache
                .put(key.to_string(), body.clone(), content_type.clone())
                .await;
        }
        value
    }

    pub async fn cache_put(&self, key: String, body: Vec<u8>, content_type: Option<String>) {
        self.response_cache
            .put(key.clone(), body.clone(), content_type.clone())
            .await;
        if let Some(shared) = &self.shared_state {
            if let Err(error) = shared.cache_put(&key, &body, content_type).await {
                tracing::warn!(%error, "failed to persist response cache to Redis");
            }
        }
    }
}
