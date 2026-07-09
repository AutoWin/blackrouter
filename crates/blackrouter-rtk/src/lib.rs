use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::RwLock;

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
pub struct CompressionStats {
    pub original_bytes: usize,
    pub compressed_bytes: usize,
}

impl CompressionStats {
    pub fn saved_bytes(&self) -> usize {
        self.original_bytes.saturating_sub(self.compressed_bytes)
    }

    pub fn saved_ratio(&self) -> f64 {
        if self.original_bytes == 0 {
            return 0.0;
        }

        self.saved_bytes() as f64 / self.original_bytes as f64
    }
}

/// Request tracking key for rate limiting and metrics
#[derive(Clone, Debug, Hash, Eq, PartialEq, Serialize, Deserialize)]
pub struct RequestKey {
    pub provider: String,
    pub model: String,
    pub api_key: Option<String>,
}

/// Rate limit configuration
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RateLimitConfig {
    /// Maximum requests per minute per key
    pub requests_per_minute: u32,
    /// Maximum tokens per minute per key
    pub tokens_per_minute: u64,
    /// Maximum concurrent requests per key
    pub max_concurrent: u32,
    /// Enable rate limiting
    pub enabled: bool,
}

impl Default for RateLimitConfig {
    fn default() -> Self {
        Self {
            requests_per_minute: 60,
            tokens_per_minute: 100_000,
            max_concurrent: 10,
            enabled: true,
        }
    }
}

/// Request metrics for tracking usage
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct RequestMetrics {
    pub total_requests: u64,
    pub successful_requests: u64,
    pub failed_requests: u64,
    pub total_prompt_tokens: u64,
    pub total_completion_tokens: u64,
    pub total_cost: f64,
    pub avg_latency_ms: f64,
    pub p95_latency_ms: f64,
    pub p99_latency_ms: f64,
}

/// Per-key rate limit state
#[derive(Clone, Debug)]
struct RateLimitState {
    requests: Vec<Instant>,
    tokens: Vec<(Instant, u64)>,
    concurrent: u32,
}

impl RateLimitState {
    fn new() -> Self {
        Self {
            requests: Vec::new(),
            tokens: Vec::new(),
            concurrent: 0,
        }
    }

    fn cleanup(&mut self, window: Duration) {
        let cutoff = Instant::now() - window;
        self.requests.retain(|t| *t > cutoff);
        self.tokens.retain(|(t, _)| *t > cutoff);
    }

    fn can_proceed(&self, config: &RateLimitConfig) -> bool {
        if !config.enabled {
            return true;
        }

        if self.concurrent >= config.max_concurrent {
            return false;
        }

        if self.requests.len() as u32 >= config.requests_per_minute {
            return false;
        }

        let window_tokens: u64 = self.tokens.iter().map(|(_, t)| t).sum();
        if window_tokens >= config.tokens_per_minute {
            return false;
        }

        true
    }
}

// ── Circuit Breaker (Phase 4.1) ────────────────────────────────────────

/// Circuit breaker state
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CircuitState {
    /// Normal operation — requests pass through
    Closed,
    /// Failing — requests are rejected
    Open,
    /// Testing after cooldown — one request allowed
    HalfOpen,
}

/// Per-provider circuit breaker state
#[derive(Clone, Debug)]
struct CircuitBreakerEntry {
    state: CircuitState,
    consecutive_failures: u32,
    last_failure_time: Option<Instant>,
    opened_at: Option<Instant>,
}

impl Default for CircuitBreakerEntry {
    fn default() -> Self {
        Self {
            state: CircuitState::Closed,
            consecutive_failures: 0,
            last_failure_time: None,
            opened_at: None,
        }
    }
}

/// Configuration for circuit breaker
#[derive(Clone, Debug)]
pub struct CircuitBreakerConfig {
    /// Number of consecutive failures before opening circuit
    pub failure_threshold: u32,
    /// Time to wait before trying again (half-open state)
    pub cooldown: Duration,
    /// Number of successes in half-open before closing circuit
    pub half_open_max_successes: u32,
}

impl Default for CircuitBreakerConfig {
    fn default() -> Self {
        Self {
            failure_threshold: 5,
            cooldown: Duration::from_secs(30),
            half_open_max_successes: 2,
        }
    }
}

// ── Load Balancer (Phase 4.1) ──────────────────────────────────────────

/// Load balancing strategy
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LoadBalanceStrategy {
    /// Rotate through providers sequentially
    RoundRobin,
    /// Use priority as weight (higher priority = more requests)
    WeightedRoundRobin,
    /// Select provider with fewest active connections
    LeastConnections,
    /// Select provider with lowest average response time
    ResponseTime,
}

impl Default for LoadBalanceStrategy {
    fn default() -> Self {
        Self::RoundRobin
    }
}

// ── Response Cache (Phase 4.2) ─────────────────────────────────────────

/// A cached response entry
struct CacheEntry {
    body: Vec<u8>,
    content_type: Option<String>,
    inserted_at: Instant,
}

/// Response cache with TTL and LRU eviction (Phase 4.2)
#[derive(Clone)]
pub struct ResponseCache {
    entries: Arc<RwLock<std::collections::HashMap<String, CacheEntry>>>,
    ttl: Duration,
    max_entries: usize,
}

impl ResponseCache {
    pub fn new(ttl: Duration, max_entries: usize) -> Self {
        Self {
            entries: Arc::new(RwLock::new(std::collections::HashMap::new())),
            ttl,
            max_entries,
        }
    }

    /// Try to get a cached response. Returns None if not cached or expired.
    pub async fn get(&self, key: &str) -> Option<(Vec<u8>, Option<String>)> {
        let entries = self.entries.read().await;
        if let Some(entry) = entries.get(key) {
            if entry.inserted_at.elapsed() < self.ttl {
                return Some((entry.body.clone(), entry.content_type.clone()));
            }
        }
        None
    }

    /// Store a response in the cache
    pub async fn put(&self, key: String, body: Vec<u8>, content_type: Option<String>) {
        let mut entries = self.entries.write().await;
        // LRU eviction: remove oldest if at capacity
        if entries.len() >= self.max_entries {
            if let Some(oldest_key) = entries
                .iter()
                .min_by_key(|(_, e)| e.inserted_at)
                .map(|(k, _)| k.clone())
            {
                entries.remove(&oldest_key);
            }
        }
        entries.insert(
            key,
            CacheEntry {
                body,
                content_type,
                inserted_at: Instant::now(),
            },
        );
    }

    /// Clear all cached entries
    pub async fn clear(&self) {
        let mut entries = self.entries.write().await;
        entries.clear();
    }

    /// Get current cache size
    pub async fn len(&self) -> usize {
        self.entries.read().await.len()
    }
}

/// Real-time tracker for request metrics and rate limiting
#[derive(Clone)]
pub struct Rtk {
    inner: Arc<RtkInner>,
}

struct RtkInner {
    /// Global metrics
    metrics: RwLock<RequestMetrics>,
    /// Per-key rate limit states
    rate_limits: RwLock<HashMap<RequestKey, RateLimitState>>,
    /// Rate limit configuration
    config: RwLock<RateLimitConfig>,
    /// Latency samples for percentile calculation (circular buffer)
    latency_samples: RwLock<Vec<f64>>,
    /// Start time for uptime calculation
    started_at: Instant,
    /// Atomic counters for lock-free increments
    total_requests: AtomicU64,
    successful_requests: AtomicU64,
    failed_requests: AtomicU64,
    /// Circuit breaker state per provider connection ID
    circuit_breakers: RwLock<HashMap<String, CircuitBreakerEntry>>,
    /// Circuit breaker configuration
    cb_config: RwLock<CircuitBreakerConfig>,
    /// Round-robin counters per provider name
    rr_counters: RwLock<HashMap<String, u64>>,
    /// Load balancing strategy
    lb_strategy: RwLock<LoadBalanceStrategy>,
    /// Per-provider average response times (for ResponseTime strategy)
    response_times: RwLock<HashMap<String, f64>>,
}

impl Rtk {
    pub fn new(config: RateLimitConfig) -> Self {
        Self {
            inner: Arc::new(RtkInner {
                metrics: RwLock::new(RequestMetrics::default()),
                rate_limits: RwLock::new(HashMap::new()),
                config: RwLock::new(config),
                latency_samples: RwLock::new(Vec::with_capacity(10000)),
                started_at: Instant::now(),
                total_requests: AtomicU64::new(0),
                successful_requests: AtomicU64::new(0),
                failed_requests: AtomicU64::new(0),
                circuit_breakers: RwLock::new(HashMap::new()),
                cb_config: RwLock::new(CircuitBreakerConfig::default()),
                rr_counters: RwLock::new(HashMap::new()),
                lb_strategy: RwLock::new(LoadBalanceStrategy::default()),
                response_times: RwLock::new(HashMap::new()),
            }),
        }
    }

    /// Check if a request is allowed under rate limits
    pub async fn check_rate_limit(&self, key: &RequestKey) -> bool {
        let config = self.inner.config.read().await;
        if !config.enabled {
            return true;
        }

        let mut rate_limits = self.inner.rate_limits.write().await;
        let state = rate_limits
            .entry(key.clone())
            .or_insert_with(RateLimitState::new);

        state.cleanup(Duration::from_secs(60));
        state.can_proceed(&config)
    }

    /// Record the start of a request (increment concurrent count)
    pub async fn record_request_start(&self, key: &RequestKey) {
        let mut rate_limits = self.inner.rate_limits.write().await;
        let state = rate_limits
            .entry(key.clone())
            .or_insert_with(RateLimitState::new);

        state.requests.push(Instant::now());
        state.concurrent += 1;

        self.inner.total_requests.fetch_add(1, Ordering::Relaxed);
    }

    /// Record the completion of a request
    pub async fn record_request_end(
        &self,
        key: &RequestKey,
        success: bool,
        latency: Duration,
        prompt_tokens: u64,
        completion_tokens: u64,
        cost: f64,
    ) {
        // Update rate limit state
        {
            let mut rate_limits = self.inner.rate_limits.write().await;
            if let Some(state) = rate_limits.get_mut(key) {
                state.concurrent = state.concurrent.saturating_sub(1);
                state
                    .tokens
                    .push((Instant::now(), prompt_tokens + completion_tokens));
            }
        }

        // Update metrics
        {
            let mut metrics = self.inner.metrics.write().await;
            metrics.total_prompt_tokens += prompt_tokens;
            metrics.total_completion_tokens += completion_tokens;
            metrics.total_cost += cost;

            if success {
                self.inner
                    .successful_requests
                    .fetch_add(1, Ordering::Relaxed);
                metrics.successful_requests =
                    self.inner.successful_requests.load(Ordering::Relaxed);
            } else {
                self.inner.failed_requests.fetch_add(1, Ordering::Relaxed);
                metrics.failed_requests = self.inner.failed_requests.load(Ordering::Relaxed);
            }

            metrics.total_requests = self.inner.total_requests.load(Ordering::Relaxed);
        }

        // Update latency
        {
            let latency_ms = latency.as_secs_f64() * 1000.0;
            let mut samples = self.inner.latency_samples.write().await;
            samples.push(latency_ms);

            // Keep only last 10000 samples
            if samples.len() > 10000 {
                let drain_count = samples.len() - 10000;
                samples.drain(0..drain_count);
            }

            // Calculate percentiles
            let mut sorted = samples.clone();
            sorted.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));

            let mut metrics = self.inner.metrics.write().await;
            metrics.avg_latency_ms = sorted.iter().sum::<f64>() / sorted.len() as f64;
            metrics.p95_latency_ms = percentile(&sorted, 0.95);
            metrics.p99_latency_ms = percentile(&sorted, 0.99);
        }
    }

    /// Get current metrics snapshot
    pub async fn metrics(&self) -> RequestMetrics {
        self.inner.metrics.read().await.clone()
    }

    /// Update rate limit configuration
    pub async fn update_config(&self, config: RateLimitConfig) {
        let mut current = self.inner.config.write().await;
        *current = config;
    }

    /// Get uptime duration
    pub fn uptime(&self) -> Duration {
        self.inner.started_at.elapsed()
    }

    /// Check if a specific key is currently rate limited
    pub async fn is_rate_limited(&self, key: &RequestKey) -> bool {
        let config = self.inner.config.read().await;
        if !config.enabled {
            return false;
        }

        let mut rate_limits = self.inner.rate_limits.write().await;
        if let Some(state) = rate_limits.get_mut(key) {
            state.cleanup(Duration::from_secs(60));
            !state.can_proceed(&config)
        } else {
            false
        }
    }

    /// Get rate limit status for a key
    pub async fn rate_limit_status(&self, key: &RequestKey) -> RateLimitStatus {
        let config = self.inner.config.read().await;
        if !config.enabled {
            return RateLimitStatus {
                limited: false,
                requests_remaining: config.requests_per_minute,
                tokens_remaining: config.tokens_per_minute,
                concurrent_remaining: config.max_concurrent,
                retry_after: None,
            };
        }

        let mut rate_limits = self.inner.rate_limits.write().await;
        let state = rate_limits
            .entry(key.clone())
            .or_insert_with(RateLimitState::new);
        state.cleanup(Duration::from_secs(60));

        let window_tokens: u64 = state.tokens.iter().map(|(_, t)| t).sum();
        let requests_remaining = config
            .requests_per_minute
            .saturating_sub(state.requests.len() as u32);
        let tokens_remaining = config.tokens_per_minute.saturating_sub(window_tokens);
        let concurrent_remaining = config.max_concurrent.saturating_sub(state.concurrent);

        let limited = !state.can_proceed(&config);
        let retry_after = if limited {
            // Calculate retry-after based on oldest request in window
            state.requests.first().map(|oldest| {
                let elapsed = oldest.elapsed();
                if elapsed < Duration::from_secs(60) {
                    Duration::from_secs(60) - elapsed
                } else {
                    Duration::ZERO
                }
            })
        } else {
            None
        };

        RateLimitStatus {
            limited,
            requests_remaining,
            tokens_remaining,
            concurrent_remaining,
            retry_after,
        }
    }

    /// Reset metrics for a specific key
    pub async fn reset_key(&self, key: &RequestKey) {
        let mut rate_limits = self.inner.rate_limits.write().await;
        rate_limits.remove(key);
    }

    /// Reset all metrics
    pub async fn reset_all(&self) {
        let mut rate_limits = self.inner.rate_limits.write().await;
        rate_limits.clear();

        let mut metrics = self.inner.metrics.write().await;
        *metrics = RequestMetrics::default();

        self.inner.total_requests.store(0, Ordering::Relaxed);
        self.inner.successful_requests.store(0, Ordering::Relaxed);
        self.inner.failed_requests.store(0, Ordering::Relaxed);

        let mut samples = self.inner.latency_samples.write().await;
        samples.clear();
    }

    // ── Circuit Breaker methods (Phase 4.1) ───────────────────────────

    /// Check if the circuit breaker is open for a provider connection
    pub async fn is_circuit_open(&self, connection_id: &str) -> bool {
        let config = self.inner.cb_config.read().await;
        let mut breakers = self.inner.circuit_breakers.write().await;
        let entry = breakers
            .entry(connection_id.to_string())
            .or_insert_with(CircuitBreakerEntry::default);

        match entry.state {
            CircuitState::Open => {
                // Check if cooldown has passed → transition to HalfOpen
                if let Some(opened_at) = entry.opened_at {
                    if opened_at.elapsed() >= config.cooldown {
                        entry.state = CircuitState::HalfOpen;
                        return false; // Allow request through (half-open)
                    }
                }
                true // Circuit is open, reject
            }
            CircuitState::HalfOpen | CircuitState::Closed => false,
        }
    }

    /// Record a successful request — may close a half-open circuit
    pub async fn record_circuit_success(&self, connection_id: &str) {
        let mut breakers = self.inner.circuit_breakers.write().await;
        if let Some(entry) = breakers.get_mut(connection_id) {
            entry.consecutive_failures = 0;
            entry.state = CircuitState::Closed;
            entry.opened_at = None;
        }
    }

    /// Record a failed request — may open the circuit
    pub async fn record_circuit_failure(&self, connection_id: &str) {
        let config = self.inner.cb_config.read().await;
        let mut breakers = self.inner.circuit_breakers.write().await;
        let entry = breakers
            .entry(connection_id.to_string())
            .or_insert_with(CircuitBreakerEntry::default);

        entry.consecutive_failures += 1;
        entry.last_failure_time = Some(Instant::now());

        if entry.consecutive_failures >= config.failure_threshold {
            entry.state = CircuitState::Open;
            entry.opened_at = Some(Instant::now());
            tracing::warn!(
                "Circuit breaker opened for connection {}: {} consecutive failures",
                connection_id,
                entry.consecutive_failures
            );
        }
    }

    /// Get circuit breaker state for a connection
    pub async fn circuit_state(&self, connection_id: &str) -> CircuitState {
        let breakers = self.inner.circuit_breakers.read().await;
        breakers
            .get(connection_id)
            .map(|e| e.state.clone())
            .unwrap_or(CircuitState::Closed)
    }

    /// Update circuit breaker configuration
    pub async fn update_cb_config(&self, config: CircuitBreakerConfig) {
        let mut current = self.inner.cb_config.write().await;
        *current = config;
    }

    // ── Load Balancer methods (Phase 4.1) ─────────────────────────────

    /// Select an index from a list of providers using the configured strategy
    pub async fn select_provider_index(&self, provider_name: &str, count: usize) -> usize {
        if count <= 1 {
            return 0;
        }

        let strategy = self.inner.lb_strategy.read().await.clone();
        match strategy {
            LoadBalanceStrategy::RoundRobin => {
                let mut counters = self.inner.rr_counters.write().await;
                let counter = counters.entry(provider_name.to_string()).or_insert(0);
                let idx = (*counter % count as u64) as usize;
                *counter += 1;
                idx
            }
            LoadBalanceStrategy::WeightedRoundRobin => {
                // For weighted, we use priority-based selection
                // Higher priority (lower number) gets more requests
                // Simple approach: still round-robin but skip lower-priority items sometimes
                // For now, use round-robin (weight info comes from storage)
                let mut counters = self.inner.rr_counters.write().await;
                let counter = counters.entry(provider_name.to_string()).or_insert(0);
                let idx = (*counter % count as u64) as usize;
                *counter += 1;
                idx
            }
            LoadBalanceStrategy::LeastConnections => {
                // Use the rate limit state to find the provider with fewest concurrent requests
                // Since we don't have per-connection tracking here, fall back to round-robin
                let mut counters = self.inner.rr_counters.write().await;
                let counter = counters.entry(provider_name.to_string()).or_insert(0);
                let idx = (*counter % count as u64) as usize;
                *counter += 1;
                idx
            }
            LoadBalanceStrategy::ResponseTime => {
                // Select the provider with lowest average response time
                // For now, fall back to round-robin if no response time data
                let _response_times = self.inner.response_times.read().await;
                // Without per-connection IDs here, we can't select by response time
                // This is a placeholder — the API layer will handle this
                let mut counters = self.inner.rr_counters.write().await;
                let counter = counters.entry(provider_name.to_string()).or_insert(0);
                let idx = (*counter % count as u64) as usize;
                *counter += 1;
                idx
            }
        }
    }

    /// Update the load balancing strategy
    pub async fn update_lb_strategy(&self, strategy: LoadBalanceStrategy) {
        let mut current = self.inner.lb_strategy.write().await;
        *current = strategy;
    }

    /// Get the current load balancing strategy
    pub async fn lb_strategy(&self) -> LoadBalanceStrategy {
        self.inner.lb_strategy.read().await.clone()
    }

    /// Record response time for a provider connection (for ResponseTime strategy)
    pub async fn record_response_time(&self, connection_id: &str, duration: Duration) {
        let mut times = self.inner.response_times.write().await;
        let current = times.entry(connection_id.to_string()).or_insert(0.0);
        // Exponential moving average
        *current = *current * 0.8 + duration.as_secs_f64() * 1000.0 * 0.2;
    }
}

/// Rate limit status response
#[derive(Clone, Debug, Serialize)]
pub struct RateLimitStatus {
    pub limited: bool,
    pub requests_remaining: u32,
    pub tokens_remaining: u64,
    pub concurrent_remaining: u32,
    pub retry_after: Option<Duration>,
}

/// Usage statistics for a time period
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct UsageStats {
    pub period_start: u64,
    pub period_end: u64,
    pub total_requests: u64,
    pub successful_requests: u64,
    pub failed_requests: u64,
    pub total_prompt_tokens: u64,
    pub total_completion_tokens: u64,
    pub total_cost: f64,
    pub by_provider: HashMap<String, ProviderStats>,
    pub by_model: HashMap<String, ModelStats>,
}

/// Provider-level statistics
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct ProviderStats {
    pub requests: u64,
    pub tokens: u64,
    pub cost: f64,
    pub avg_latency_ms: f64,
    pub error_rate: f64,
}

/// Model-level statistics
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct ModelStats {
    pub requests: u64,
    pub prompt_tokens: u64,
    pub completion_tokens: u64,
    pub cost: f64,
    pub avg_latency_ms: f64,
}

fn percentile(sorted: &[f64], p: f64) -> f64 {
    if sorted.is_empty() {
        return 0.0;
    }

    let index = (p * (sorted.len() - 1) as f64) as usize;
    sorted.get(index).copied().unwrap_or(0.0)
}

/// Builder for creating a custom Rtk instance
pub struct RtkBuilder {
    config: RateLimitConfig,
}

impl RtkBuilder {
    pub fn new() -> Self {
        Self {
            config: RateLimitConfig::default(),
        }
    }

    pub fn requests_per_minute(mut self, rpm: u32) -> Self {
        self.config.requests_per_minute = rpm;
        self
    }

    pub fn tokens_per_minute(mut self, tpm: u64) -> Self {
        self.config.tokens_per_minute = tpm;
        self
    }

    pub fn max_concurrent(mut self, max: u32) -> Self {
        self.config.max_concurrent = max;
        self
    }

    pub fn enabled(mut self, enabled: bool) -> Self {
        self.config.enabled = enabled;
        self
    }

    pub fn build(self) -> Rtk {
        Rtk::new(self.config)
    }
}

impl Default for RtkBuilder {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_rate_limiting() {
        let rtk = RtkBuilder::new()
            .requests_per_minute(2)
            .enabled(true)
            .build();

        let key = RequestKey {
            provider: "test".to_string(),
            model: "model".to_string(),
            api_key: None,
        };

        assert!(rtk.check_rate_limit(&key).await);

        rtk.record_request_start(&key).await;
        rtk.record_request_start(&key).await;

        assert!(!rtk.check_rate_limit(&key).await);

        rtk.record_request_end(&key, true, Duration::from_millis(100), 100, 50, 0.001)
            .await;

        assert!(!rtk.check_rate_limit(&key).await);
    }

    #[tokio::test]
    async fn test_metrics_tracking() {
        let rtk = Rtk::new(RateLimitConfig::default());

        let key = RequestKey {
            provider: "test".to_string(),
            model: "model".to_string(),
            api_key: Some("key1".to_string()),
        };

        rtk.record_request_start(&key).await;
        rtk.record_request_end(&key, true, Duration::from_millis(100), 100, 50, 0.001)
            .await;

        let metrics = rtk.metrics().await;
        assert_eq!(metrics.total_requests, 1);
        assert_eq!(metrics.successful_requests, 1);
        assert_eq!(metrics.total_prompt_tokens, 100);
        assert_eq!(metrics.total_completion_tokens, 50);
    }

    #[tokio::test]
    async fn test_rate_limit_status() {
        let rtk = RtkBuilder::new()
            .requests_per_minute(10)
            .tokens_per_minute(10000)
            .max_concurrent(5)
            .enabled(true)
            .build();

        let key = RequestKey {
            provider: "test".to_string(),
            model: "model".to_string(),
            api_key: None,
        };

        let status = rtk.rate_limit_status(&key).await;
        assert!(!status.limited);
        assert_eq!(status.requests_remaining, 10);
        assert_eq!(status.tokens_remaining, 10000);
        assert_eq!(status.concurrent_remaining, 5);
    }

    #[tokio::test]
    async fn test_disabled_rate_limiting() {
        let rtk = RtkBuilder::new()
            .requests_per_minute(1)
            .enabled(false)
            .build();

        let key = RequestKey {
            provider: "test".to_string(),
            model: "model".to_string(),
            api_key: None,
        };

        // Should allow unlimited requests when disabled
        for _ in 0..100 {
            assert!(rtk.check_rate_limit(&key).await);
            rtk.record_request_start(&key).await;
            rtk.record_request_end(&key, true, Duration::from_millis(10), 10, 5, 0.0001)
                .await;
        }
    }

    #[tokio::test]
    async fn test_reset() {
        let rtk = Rtk::new(RateLimitConfig::default());

        let key = RequestKey {
            provider: "test".to_string(),
            model: "model".to_string(),
            api_key: None,
        };

        rtk.record_request_start(&key).await;
        rtk.record_request_end(&key, true, Duration::from_millis(100), 100, 50, 0.001)
            .await;

        let metrics_before = rtk.metrics().await;
        assert_eq!(metrics_before.total_requests, 1);

        rtk.reset_all().await;

        let metrics_after = rtk.metrics().await;
        assert_eq!(metrics_after.total_requests, 0);
        assert_eq!(metrics_after.total_prompt_tokens, 0);
    }
}
