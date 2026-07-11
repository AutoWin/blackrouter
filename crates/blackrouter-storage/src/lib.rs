use blackrouter_common::mask_secret;
use blackrouter_core::{parse_provider_model, RouteKind};
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use thiserror::Error;
use uuid::Uuid;

#[derive(Debug, Error)]
pub enum StorageError {
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("sqlite error: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error("json error: {0}")]
    Json(#[from] serde_json::Error),
    #[error("validation error: {0}")]
    Validation(String),
}

pub type Result<T> = std::result::Result<T, StorageError>;

const PRAGMA_SQL: &str = r#"
PRAGMA journal_mode = WAL;
PRAGMA synchronous = NORMAL;
PRAGMA temp_store = MEMORY;
PRAGMA mmap_size = 30000000;
PRAGMA cache_size = -64000;
PRAGMA foreign_keys = ON;
PRAGMA busy_timeout = 5000;
"#;

const COMPAT_TABLES: &[&str] = &[
    "_meta",
    "settings",
    "providerConnections",
    "providerNodes",
    "proxyPools",
    "apiKeys",
    "combos",
    "kv",
    "usageHistory",
    "usageDaily",
    "requestDetails",
    "modelAliases",
];

const BLACKROUTER_TABLES: &[&str] = &[
    "adminAuditLog",
    "telegramLinks",
    "runtimeEvents",
    "settingsHistory",
    "apiKeyQuotaCounters",
];

#[derive(Clone, Debug)]
pub struct Storage {
    database_path: PathBuf,
}

#[derive(Clone, Debug, Serialize)]
pub struct StorageStatus {
    pub database_path: PathBuf,
    pub existed_before_init: bool,
    pub schema_compatible: bool,
    pub missing_compat_tables: Vec<String>,
    pub table_counts: BTreeMap<String, i64>,
}

#[derive(Clone, Debug, Serialize)]
pub struct ModelListItem {
    pub id: String,
    pub object: &'static str,
    pub owned_by: &'static str,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct ApiKeyPolicy {
    #[serde(default)]
    pub requests_per_day: Option<u64>,
    #[serde(default)]
    pub tokens_per_day: Option<u64>,
    #[serde(default)]
    pub cost_per_month_usd: Option<f64>,
    #[serde(default)]
    pub provider_allowlist: Vec<String>,
    #[serde(default)]
    pub model_allowlist: Vec<String>,
}

#[derive(Clone, Debug, Serialize)]
pub struct ApiKeyRecord {
    pub id: String,
    pub key_masked: String,
    pub name: Option<String>,
    pub machine_id: Option<String>,
    pub tenant_id: Option<String>,
    pub policy: ApiKeyPolicy,
    pub is_active: bool,
    pub created_at: String,
}

#[derive(Clone, Debug, Serialize)]
pub struct CreatedApiKey {
    pub record: ApiKeyRecord,
    pub key: String,
}

#[derive(Clone, Debug, Deserialize)]
pub struct NewApiKey {
    pub name: Option<String>,
    pub machine_id: Option<String>,
    #[serde(default)]
    pub tenant_id: Option<String>,
    #[serde(default)]
    pub policy: ApiKeyPolicy,
}

#[derive(Clone, Debug)]
pub struct AuthenticatedApiKey {
    pub id: String,
    pub tenant_id: Option<String>,
    pub policy: ApiKeyPolicy,
}

#[derive(Clone, Debug, Default, Serialize)]
pub struct ApiKeyUsage {
    pub requests: u64,
    pub tokens: u64,
    pub cost: f64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SettingsVersion {
    pub version: u64,
    pub data: Value,
    pub created_at: u64,
}

#[derive(Clone, Debug, Serialize)]
pub struct ProviderConnectionRecord {
    pub id: String,
    pub provider: String,
    pub auth_type: String,
    pub name: Option<String>,
    pub email: Option<String>,
    pub priority: Option<i64>,
    pub is_active: bool,
    pub status: String,
    pub cooldown_until: Option<String>,
    pub expires_at: Option<String>,
    pub data: Value,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Clone, Debug, Deserialize)]
pub struct NewProviderConnection {
    pub provider: String,
    pub auth_type: String,
    pub name: Option<String>,
    pub email: Option<String>,
    pub priority: Option<i64>,
    pub is_active: Option<bool>,
    #[serde(default)]
    pub status: Option<String>,
    #[serde(default)]
    pub cooldown_until: Option<String>,
    #[serde(default)]
    pub expires_at: Option<String>,
    pub data: Option<Value>,
}

#[derive(Clone, Debug)]
pub struct RawProviderConnection {
    pub id: String,
    pub provider: String,
    pub auth_type: String,
    pub name: Option<String>,
    pub email: Option<String>,
    pub priority: Option<i64>,
    pub is_active: bool,
    pub status: String,
    pub cooldown_until: Option<String>,
    pub expires_at: Option<String>,
    pub data: Value,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Clone, Debug, Serialize)]
pub struct CachedProviderModels {
    pub models: Vec<String>,
    pub models_url: Option<String>,
    pub fetched_at: String,
    pub age_seconds: u64,
}

#[derive(Clone, Debug, Serialize)]
pub struct ComboRecord {
    pub id: String,
    pub name: String,
    pub kind: String,
    pub models: Vec<String>,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Clone, Debug, Deserialize)]
pub struct NewCombo {
    pub name: String,
    pub kind: Option<String>,
    pub models: Vec<String>,
}

/// Usage history entry for recording request metrics (Phase 3.1)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UsageEntry {
    pub id: String,
    pub timestamp: String,
    pub provider: String,
    pub model: String,
    pub connection_id: Option<String>,
    pub api_key: Option<String>,
    pub endpoint: String,
    pub prompt_tokens: u64,
    pub completion_tokens: u64,
    pub cost: f64,
    pub status: String,
    pub tokens: Option<String>,
    pub meta: Option<String>,
}

/// Aggregated usage row for reporting
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UsageRow {
    pub provider: String,
    pub connection_id: Option<String>,
    pub model: String,
    pub status: String,
    pub prompt_tokens: u64,
    pub completion_tokens: u64,
    pub cost: f64,
    pub count: u64,
}

/// A persisted RTK runtime state row (rate-limit / circuit-breaker snapshots).
/// `data` holds a serialized JSON snapshot; `kind` discriminates the payload.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RtkStateRow {
    pub key: String,
    pub kind: String,
    pub data: String,
    pub updated_at: u64,
}

/// A normalized model catalog entry: capability + pricing metadata used for
/// cost-aware and capability-based routing (Phase 7).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelCatalogEntry {
    pub provider: String,
    pub model: String,
    pub context_window: Option<u64>,
    /// JSON array of modality strings, e.g. ["text","vision","audio","tools"].
    pub modalities: Option<String>,
    pub price_in_per_million: Option<f64>,
    pub price_out_per_million: Option<f64>,
    pub latency_p50_ms: Option<u64>,
}

/// Request detail entry for storing full request/response metadata (Phase 3.1)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RequestDetailEntry {
    pub id: String,
    pub timestamp: String,
    pub provider: String,
    pub model: String,
    pub connection_id: Option<String>,
    pub status: String,
    pub data: String,
}

/// Model alias record (Phase 4.3 — model aliases)
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ModelAliasRecord {
    pub id: String,
    pub alias: String,
    pub target: String,
    pub created_at: String,
    pub updated_at: String,
}

/// Create a new model alias
#[derive(Clone, Debug, Deserialize)]
pub struct NewModelAlias {
    pub alias: String,
    pub target: String,
}

/// Daily usage summary stored in usageDaily table
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DailyUsage {
    pub date_key: String,
    pub total_requests: u64,
    pub successful_requests: u64,
    pub failed_requests: u64,
    pub total_prompt_tokens: u64,
    pub total_completion_tokens: u64,
    pub total_cost: f64,
    pub by_provider: serde_json::Value,
}

impl Storage {
    pub fn new(database_path: impl Into<PathBuf>) -> Self {
        Self {
            database_path: database_path.into(),
        }
    }

    pub fn database_path(&self) -> &Path {
        &self.database_path
    }

    pub fn initialize(&self) -> Result<StorageStatus> {
        let existed_before_init = self.database_path.exists();
        self.ensure_parent_dir()?;

        let conn = self.open()?;
        conn.execute_batch(PRAGMA_SQL)?;
        conn.execute_batch(COMPAT_SCHEMA_SQL)?;
        conn.execute_batch(BLACKROUTER_SCHEMA_SQL)?;
        migrate_schema(&conn)?;

        self.status_with_existing_flag(existed_before_init)
    }

    pub fn status(&self) -> Result<StorageStatus> {
        self.status_with_existing_flag(self.database_path.exists())
    }

    pub fn settings_json(&self) -> Result<Value> {
        let conn = self.open()?;
        let data = conn
            .query_row("SELECT data FROM settings WHERE id = 1", [], |row| {
                row.get::<_, String>(0)
            })
            .optional()?;

        match data {
            Some(raw) => serde_json::from_str(&raw).map_err(StorageError::from),
            None => Ok(Value::Object(Default::default())),
        }
    }

    pub fn save_settings_json(&self, settings: &Value) -> Result<Value> {
        let conn = self.open()?;
        let raw = serde_json::to_string_pretty(settings)?;
        let tx = conn.unchecked_transaction()?;
        tx.execute(
            r#"
            INSERT INTO settings (id, data)
            VALUES (1, ?1)
            ON CONFLICT(id) DO UPDATE SET data = excluded.data
            "#,
            params![&raw],
        )?;
        tx.execute(
            "INSERT INTO settingsHistory (data, createdAt) VALUES (?1, ?2)",
            params![&raw, blackrouter_common::unix_timestamp()],
        )?;
        tx.commit()?;
        Ok(settings.clone())
    }

    pub fn list_settings_versions(&self, limit: u64) -> Result<Vec<SettingsVersion>> {
        let conn = self.open()?;
        let mut stmt = conn.prepare(
            "SELECT version, data, createdAt FROM settingsHistory ORDER BY version DESC LIMIT ?1",
        )?;
        let rows = stmt.query_map(params![limit.min(100) as i64], |row| {
            let raw: String = row.get(1)?;
            Ok((row.get::<_, i64>(0)?, raw, row.get::<_, i64>(2)?))
        })?;
        rows.map(|row| {
            let (version, raw, created_at) = row?;
            Ok(SettingsVersion {
                version: version as u64,
                data: serde_json::from_str(&raw)?,
                created_at: created_at as u64,
            })
        })
        .collect()
    }

    pub fn restore_settings_version(&self, version: u64) -> Result<Value> {
        let conn = self.open()?;
        let raw = conn
            .query_row(
                "SELECT data FROM settingsHistory WHERE version = ?1",
                params![version as i64],
                |row| row.get::<_, String>(0),
            )
            .optional()?
            .ok_or_else(|| StorageError::Validation("settings version not found".to_string()))?;
        let settings = serde_json::from_str::<Value>(&raw)?;
        self.save_settings_json(&settings)
    }

    pub fn list_api_keys(&self) -> Result<Vec<ApiKeyRecord>> {
        let conn = self.open()?;
        let mut stmt = conn.prepare(
            r#"
            SELECT id, key, name, machineId, tenantId, policy, isActive, createdAt
            FROM apiKeys
            ORDER BY createdAt DESC, id DESC
            "#,
        )?;

        let rows = stmt.query_map([], |row| {
            let key: String = row.get(1)?;
            Ok(ApiKeyRecord {
                id: row.get(0)?,
                key_masked: mask_secret(&key),
                name: row.get(2)?,
                machine_id: row.get(3)?,
                tenant_id: row.get(4)?,
                policy: parse_policy(row.get::<_, Option<String>>(5)?),
                is_active: row.get::<_, i64>(6)? != 0,
                created_at: row.get(7)?,
            })
        })?;

        rows.collect::<std::result::Result<Vec<_>, _>>()
            .map_err(StorageError::from)
    }

    pub fn create_api_key(&self, input: NewApiKey) -> Result<CreatedApiKey> {
        let conn = self.open()?;
        let id = new_id();
        let key = format!("brk_{}", Uuid::new_v4().simple());
        let created_at = now_text();
        let name = normalize_opt(input.name);
        let machine_id = normalize_opt(input.machine_id);
        let tenant_id = normalize_opt(input.tenant_id);
        validate_api_key_policy(&input.policy)?;
        let policy = serde_json::to_string(&input.policy)?;

        conn.execute(
            r#"
            INSERT INTO apiKeys (id, key, name, machineId, tenantId, policy, isActive, createdAt)
            VALUES (?1, ?2, ?3, ?4, ?5, ?6, 1, ?7)
            "#,
            params![id, key, name, machine_id, tenant_id, policy, created_at],
        )?;

        Ok(CreatedApiKey {
            record: ApiKeyRecord {
                id,
                key_masked: mask_secret(&key),
                name,
                machine_id,
                tenant_id,
                policy: input.policy,
                is_active: true,
                created_at,
            },
            key,
        })
    }

    /// Rotate an API key: deactivate old key, create new one with same metadata
    pub fn rotate_api_key(&self, key_id: &str) -> Result<CreatedApiKey> {
        let conn = self.open()?;
        // Get existing key metadata
        let (name, machine_id, tenant_id, policy): (
            Option<String>,
            Option<String>,
            Option<String>,
            Option<String>,
        ) = conn.query_row(
            "SELECT name, machineId, tenantId, policy FROM apiKeys WHERE id = ?1",
            params![key_id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        )?;

        // Deactivate old key
        conn.execute(
            "UPDATE apiKeys SET isActive = 0 WHERE id = ?1",
            params![key_id],
        )?;

        // Create new key with same metadata
        let new_id = new_id();
        let key = format!("brk_{}", Uuid::new_v4().simple());
        let created_at = now_text();

        conn.execute(
            r#"
            INSERT INTO apiKeys (id, key, name, machineId, tenantId, policy, isActive, createdAt)
            VALUES (?1, ?2, ?3, ?4, ?5, ?6, 1, ?7)
            "#,
            params![new_id, key, name, machine_id, tenant_id, policy, created_at],
        )?;

        Ok(CreatedApiKey {
            record: ApiKeyRecord {
                id: new_id,
                key_masked: mask_secret(&key),
                name,
                machine_id,
                tenant_id,
                policy: parse_policy(policy),
                is_active: true,
                created_at,
            },
            key,
        })
    }

    pub fn update_api_key_policy(
        &self,
        key_id: &str,
        tenant_id: Option<String>,
        policy: ApiKeyPolicy,
    ) -> Result<ApiKeyRecord> {
        validate_api_key_policy(&policy)?;
        let conn = self.open()?;
        let tenant_id = normalize_opt(tenant_id);
        let policy_json = serde_json::to_string(&policy)?;
        let changed = conn.execute(
            "UPDATE apiKeys SET tenantId = ?2, policy = ?3 WHERE id = ?1",
            params![key_id, tenant_id, policy_json],
        )?;
        if changed == 0 {
            return Err(StorageError::Validation("API key not found".to_string()));
        }
        self.get_api_key_record(key_id)?
            .ok_or_else(|| StorageError::Validation("API key not found".to_string()))
    }

    pub fn authenticate_api_key(&self, api_key: &str) -> Result<Option<AuthenticatedApiKey>> {
        let conn = self.open()?;
        conn.query_row(
            "SELECT id, tenantId, policy FROM apiKeys WHERE key = ?1 AND isActive = 1 LIMIT 1",
            params![api_key],
            |row| {
                Ok(AuthenticatedApiKey {
                    id: row.get(0)?,
                    tenant_id: row.get(1)?,
                    policy: parse_policy(row.get::<_, Option<String>>(2)?),
                })
            },
        )
        .optional()
        .map_err(StorageError::from)
    }

    pub fn api_key_usage_since(&self, key_id: &str, since_unix: u64) -> Result<ApiKeyUsage> {
        let conn = self.open()?;
        conn.query_row(
            r#"
            SELECT COUNT(*),
                   COALESCE(SUM(promptTokens + completionTokens), 0),
                   COALESCE(SUM(CASE WHEN status = 'success' THEN cost ELSE 0 END), 0)
            FROM usageHistory
            WHERE apiKey = ?1 AND CAST(timestamp AS INTEGER) >= ?2
            "#,
            params![key_id, since_unix],
            |row| {
                Ok(ApiKeyUsage {
                    requests: row.get::<_, i64>(0)? as u64,
                    tokens: row.get::<_, i64>(1)? as u64,
                    cost: row.get(2)?,
                })
            },
        )
        .map_err(StorageError::from)
    }

    pub fn reserve_api_key_request(
        &self,
        key_id: &str,
        period_start: u64,
        limit: u64,
    ) -> Result<bool> {
        if limit == 0 {
            return Ok(false);
        }
        let conn = self.open()?;
        let changed = conn.execute(
            r#"
            INSERT INTO apiKeyQuotaCounters (keyId, periodStart, kind, value)
            VALUES (?1, ?2, 'requests', 1)
            ON CONFLICT(keyId, periodStart, kind) DO UPDATE SET
              value = apiKeyQuotaCounters.value + 1
            WHERE apiKeyQuotaCounters.value < ?3
            "#,
            params![key_id, period_start, limit],
        )?;
        let _ = conn.execute(
            "DELETE FROM apiKeyQuotaCounters WHERE periodStart < ?1",
            params![period_start.saturating_sub(40 * 86_400)],
        );
        Ok(changed > 0)
    }

    fn get_api_key_record(&self, key_id: &str) -> Result<Option<ApiKeyRecord>> {
        let conn = self.open()?;
        conn.query_row(
            "SELECT id, key, name, machineId, tenantId, policy, isActive, createdAt FROM apiKeys WHERE id = ?1",
            params![key_id],
            |row| {
                let key: String = row.get(1)?;
                Ok(ApiKeyRecord {
                    id: row.get(0)?,
                    key_masked: mask_secret(&key),
                    name: row.get(2)?,
                    machine_id: row.get(3)?,
                    tenant_id: row.get(4)?,
                    policy: parse_policy(row.get::<_, Option<String>>(5)?),
                    is_active: row.get::<_, i64>(6)? != 0,
                    created_at: row.get(7)?,
                })
            },
        )
        .optional()
        .map_err(StorageError::from)
    }

    pub fn list_provider_connections(&self) -> Result<Vec<ProviderConnectionRecord>> {
        let conn = self.open()?;
        let mut stmt = conn.prepare(
            r#"
            SELECT id, provider, authType, name, email, priority, isActive, status, cooldownUntil, expiresAt, data, createdAt, updatedAt
            FROM providerConnections
            ORDER BY provider ASC, COALESCE(priority, 999999) ASC, createdAt DESC
            "#,
        )?;

        let rows = stmt.query_map([], |row| {
            let raw_data: String = row.get(10)?;
            let data = serde_json::from_str(&raw_data)
                .map(mask_sensitive_json)
                .unwrap_or(Value::Object(Default::default()));
            Ok(ProviderConnectionRecord {
                id: row.get(0)?,
                provider: row.get(1)?,
                auth_type: row.get(2)?,
                name: row.get(3)?,
                email: row.get(4)?,
                priority: row.get(5)?,
                is_active: row.get::<_, i64>(6)? != 0,
                status: row
                    .get::<_, Option<String>>(7)?
                    .unwrap_or_else(|| "unknown".to_string()),
                cooldown_until: row.get(8)?,
                expires_at: row.get(9)?,
                data,
                created_at: row.get(11)?,
                updated_at: row.get(12)?,
            })
        })?;

        rows.collect::<std::result::Result<Vec<_>, _>>()
            .map_err(StorageError::from)
    }

    pub fn create_provider_connection(
        &self,
        input: NewProviderConnection,
    ) -> Result<ProviderConnectionRecord> {
        let provider = input.provider.trim().to_string();
        let auth_type = input.auth_type.trim().to_string();
        if provider.is_empty() {
            return Err(StorageError::Validation("provider is required".to_string()));
        }
        if auth_type.is_empty() {
            return Err(StorageError::Validation(
                "auth_type is required".to_string(),
            ));
        }

        let conn = self.open()?;
        let id = new_id();
        let now = now_text();
        let name = normalize_opt(input.name);
        let email = normalize_opt(input.email);
        let priority = input.priority;
        let is_active = input.is_active.unwrap_or(true);
        let status = normalize_status(input.status.as_deref());
        let cooldown_until = normalize_opt(input.cooldown_until);
        let expires_at = normalize_opt(input.expires_at);
        let data = input.data.unwrap_or(Value::Object(Default::default()));
        let raw_data = serde_json::to_string_pretty(&data)?;

        conn.execute(
            r#"
            INSERT INTO providerConnections
              (id, provider, authType, name, email, priority, isActive, status, cooldownUntil, expiresAt, data, createdAt, updatedAt)
            VALUES
              (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)
            "#,
            params![
                id,
                provider,
                auth_type,
                name,
                email,
                priority,
                if is_active { 1 } else { 0 },
                status,
                cooldown_until,
                expires_at,
                raw_data,
                now,
                now
            ],
        )?;

        Ok(ProviderConnectionRecord {
            id,
            provider,
            auth_type,
            name,
            email,
            priority,
            is_active,
            status,
            cooldown_until,
            expires_at,
            data: mask_sensitive_json(data),
            created_at: now.clone(),
            updated_at: now,
        })
    }

    pub fn get_provider_connection_raw(&self, id: &str) -> Result<RawProviderConnection> {
        let conn = self.open()?;
        conn.query_row(
            r#"
            SELECT id, provider, authType, name, email, priority, isActive, status, cooldownUntil, expiresAt, data, createdAt, updatedAt
            FROM providerConnections
            WHERE id = ?1
            "#,
            params![id],
            |row| {
                let raw_data: String = row.get(10)?;
                let data = serde_json::from_str(&raw_data).unwrap_or(Value::Object(Default::default()));
                Ok(RawProviderConnection {
                    id: row.get(0)?,
                    provider: row.get(1)?,
                    auth_type: row.get(2)?,
                    name: row.get(3)?,
                    email: row.get(4)?,
                    priority: row.get(5)?,
                    is_active: row.get::<_, i64>(6)? != 0,
                    status: row
                        .get::<_, Option<String>>(7)?
                        .unwrap_or_else(|| "unknown".to_string()),
                    cooldown_until: row.get(8)?,
                    expires_at: row.get(9)?,
                    data,
                    created_at: row.get(11)?,
                    updated_at: row.get(12)?,
                })
            },
        )
        .optional()?
        .ok_or_else(|| StorageError::Validation("provider connection not found".to_string()))
    }

    pub fn get_active_provider_connection_raw(
        &self,
        provider: &str,
    ) -> Result<RawProviderConnection> {
        let conn = self.open()?;
        conn.query_row(
            r#"
            SELECT id, provider, authType, name, email, priority, isActive, status, cooldownUntil, expiresAt, data, createdAt, updatedAt
            FROM providerConnections
            WHERE provider = ?1
              AND isActive = 1
              AND COALESCE(status, 'unknown') NOT IN ('disabled', 'expired')
              AND (expiresAt IS NULL OR expiresAt = '' OR CAST(expiresAt AS INTEGER) > ?2)
            ORDER BY COALESCE(priority, 999999) ASC, createdAt DESC
            LIMIT 1
            "#,
            params![provider, now_text()],
            |row| {
                let raw_data: String = row.get(10)?;
                let data =
                    serde_json::from_str(&raw_data).unwrap_or(Value::Object(Default::default()));
                Ok(RawProviderConnection {
                    id: row.get(0)?,
                    provider: row.get(1)?,
                    auth_type: row.get(2)?,
                    name: row.get(3)?,
                    email: row.get(4)?,
                    priority: row.get(5)?,
                    is_active: row.get::<_, i64>(6)? != 0,
                    status: row
                        .get::<_, Option<String>>(7)?
                        .unwrap_or_else(|| "unknown".to_string()),
                    cooldown_until: row.get(8)?,
                    expires_at: row.get(9)?,
                    data,
                    created_at: row.get(11)?,
                    updated_at: row.get(12)?,
                })
            },
        )
        .optional()?
        .ok_or_else(|| {
            StorageError::Validation(format!(
                "active provider connection not found: {provider}"
            ))
        })
    }

    /// List all active provider connections for a given provider name, ordered by priority.
    /// Used for load balancing (Phase 4.1).
    pub fn list_active_provider_connections(
        &self,
        provider: &str,
    ) -> Result<Vec<RawProviderConnection>> {
        let conn = self.open()?;
        let mut stmt = conn.prepare(
            r#"
            SELECT id, provider, authType, name, email, priority, isActive, status, cooldownUntil, expiresAt, data, createdAt, updatedAt
            FROM providerConnections
            WHERE provider = ?1
              AND isActive = 1
              AND COALESCE(status, 'unknown') NOT IN ('disabled', 'expired')
              AND (expiresAt IS NULL OR expiresAt = '' OR CAST(expiresAt AS INTEGER) > ?2)
            ORDER BY COALESCE(priority, 999999) ASC, createdAt DESC
            "#,
        )?;
        let rows = stmt.query_map(params![provider, now_text()], |row| {
            let raw_data: String = row.get(10)?;
            let data = serde_json::from_str(&raw_data).unwrap_or(Value::Object(Default::default()));
            Ok(RawProviderConnection {
                id: row.get(0)?,
                provider: row.get(1)?,
                auth_type: row.get(2)?,
                name: row.get(3)?,
                email: row.get(4)?,
                priority: row.get(5)?,
                is_active: row.get::<_, i64>(6)? != 0,
                status: row
                    .get::<_, Option<String>>(7)?
                    .unwrap_or_else(|| "unknown".to_string()),
                cooldown_until: row.get(8)?,
                expires_at: row.get(9)?,
                data,
                created_at: row.get(11)?,
                updated_at: row.get(12)?,
            })
        })?;
        rows.collect::<std::result::Result<Vec<_>, _>>()
            .map_err(StorageError::from)
    }

    pub fn update_provider_connection(
        &self,
        id: &str,
        input: NewProviderConnection,
    ) -> Result<ProviderConnectionRecord> {
        let existing = self.get_provider_connection_raw(id)?;
        let provider = input.provider.trim().to_string();
        let auth_type = input.auth_type.trim().to_string();
        if provider.is_empty() {
            return Err(StorageError::Validation("provider is required".to_string()));
        }
        if auth_type.is_empty() {
            return Err(StorageError::Validation(
                "auth_type is required".to_string(),
            ));
        }

        let conn = self.open()?;
        let now = now_text();
        let name = normalize_opt(input.name);
        let email = normalize_opt(input.email);
        let priority = input.priority;
        let is_active = input.is_active.unwrap_or(existing.is_active);
        let status = input
            .status
            .as_deref()
            .map(|value| normalize_status(Some(value)))
            .unwrap_or(existing.status);
        let cooldown_until = input.cooldown_until.or(existing.cooldown_until);
        let expires_at = input.expires_at.or(existing.expires_at);
        let data = preserve_sensitive_json(
            existing.data,
            input.data.unwrap_or(Value::Object(Default::default())),
        );
        let raw_data = serde_json::to_string_pretty(&data)?;

        let changed = conn.execute(
            r#"
            UPDATE providerConnections
            SET provider = ?2,
                authType = ?3,
                name = ?4,
                email = ?5,
                priority = ?6,
                isActive = ?7,
                status = ?8,
                cooldownUntil = ?9,
                expiresAt = ?10,
                data = ?11,
                updatedAt = ?12
            WHERE id = ?1
            "#,
            params![
                id,
                provider,
                auth_type,
                name,
                email,
                priority,
                if is_active { 1 } else { 0 },
                status,
                cooldown_until,
                expires_at,
                raw_data,
                now
            ],
        )?;

        if changed == 0 {
            return Err(StorageError::Validation(
                "provider connection not found".to_string(),
            ));
        }

        Ok(ProviderConnectionRecord {
            id: id.to_string(),
            provider,
            auth_type,
            name,
            email,
            priority,
            is_active,
            status,
            cooldown_until,
            expires_at,
            data: mask_sensitive_json(data),
            created_at: existing.created_at,
            updated_at: now,
        })
    }

    pub fn set_provider_connection_active(
        &self,
        id: &str,
        is_active: bool,
    ) -> Result<ProviderConnectionRecord> {
        let existing = self.get_provider_connection_raw(id)?;
        let input = NewProviderConnection {
            provider: existing.provider,
            auth_type: existing.auth_type,
            name: existing.name,
            email: existing.email,
            priority: existing.priority,
            is_active: Some(is_active),
            status: Some(existing.status),
            cooldown_until: existing.cooldown_until,
            expires_at: existing.expires_at,
            data: Some(existing.data),
        };
        self.update_provider_connection(id, input)
    }

    pub fn set_provider_connection_models(
        &self,
        id: &str,
        models: Vec<String>,
        models_url: Option<String>,
    ) -> Result<ProviderConnectionRecord> {
        let existing = self.get_provider_connection_raw(id)?;
        let mut data = match existing.data {
            Value::Object(map) => map,
            _ => serde_json::Map::new(),
        };
        data.insert(
            "models".to_string(),
            Value::Array(models.into_iter().map(Value::String).collect()),
        );
        if let Some(models_url) = models_url {
            data.insert("modelsUrl".to_string(), Value::String(models_url));
        }
        data.insert("modelsFetchedAt".to_string(), Value::String(now_text()));

        let input = NewProviderConnection {
            provider: existing.provider,
            auth_type: existing.auth_type,
            name: existing.name,
            email: existing.email,
            priority: existing.priority,
            is_active: Some(existing.is_active),
            status: Some(existing.status),
            cooldown_until: existing.cooldown_until,
            expires_at: existing.expires_at,
            data: Some(Value::Object(data)),
        };
        self.update_provider_connection(id, input)
    }

    pub fn set_provider_rate_limit_snapshot(&self, id: &str, snapshot: Value) -> Result<()> {
        let existing = self.get_provider_connection_raw(id)?;
        let mut data = match existing.data {
            Value::Object(map) => map,
            _ => serde_json::Map::new(),
        };
        data.insert("rateLimit".to_string(), snapshot);
        let raw_data = serde_json::to_string_pretty(&Value::Object(data))?;
        let conn = self.open()?;
        conn.execute(
            r#"
            UPDATE providerConnections
            SET data = ?2,
                updatedAt = ?3
            WHERE id = ?1
            "#,
            params![id, raw_data, now_text()],
        )?;
        Ok(())
    }

    pub fn set_provider_oauth_access_token(
        &self,
        id: &str,
        access_token: &str,
        token_expires_at: Option<String>,
    ) -> Result<()> {
        let existing = self.get_provider_connection_raw(id)?;
        let mut data = match existing.data {
            Value::Object(map) => map,
            _ => serde_json::Map::new(),
        };
        if data.contains_key("apiKey") {
            data.insert(
                "apiKey".to_string(),
                Value::String(access_token.to_string()),
            );
        }
        data.insert(
            "accessToken".to_string(),
            Value::String(access_token.to_string()),
        );
        if let Some(token_expires_at) = token_expires_at {
            data.insert(
                "tokenExpiresAt".to_string(),
                Value::String(token_expires_at),
            );
        }
        let raw_data = serde_json::to_string_pretty(&Value::Object(data))?;
        let conn = self.open()?;
        conn.execute(
            r#"
            UPDATE providerConnections
            SET data = ?2,
                updatedAt = ?3
            WHERE id = ?1
            "#,
            params![id, raw_data, now_text()],
        )?;
        Ok(())
    }

    pub fn cached_provider_models(
        &self,
        id: &str,
        max_age_seconds: u64,
    ) -> Result<Option<CachedProviderModels>> {
        let provider = self.get_provider_connection_raw(id)?;
        let data = match provider.data {
            Value::Object(map) => map,
            _ => return Ok(None),
        };
        let models = data
            .get("models")
            .and_then(Value::as_array)
            .map(|items| {
                items
                    .iter()
                    .filter_map(Value::as_str)
                    .map(ToOwned::to_owned)
                    .collect::<Vec<_>>()
            })
            .filter(|models| !models.is_empty());
        let Some(models) = models else {
            return Ok(None);
        };

        let fetched_at = data
            .get("modelsFetchedAt")
            .or_else(|| data.get("models_fetched_at"))
            .and_then(Value::as_str)
            .and_then(|value| value.parse::<u64>().ok());
        let Some(fetched_at) = fetched_at else {
            return Ok(None);
        };

        let now = blackrouter_common::unix_timestamp();
        let age_seconds = now.saturating_sub(fetched_at);
        if age_seconds > max_age_seconds {
            return Ok(None);
        }

        let models_url = data
            .get("modelsUrl")
            .or_else(|| data.get("models_url"))
            .and_then(Value::as_str)
            .map(ToOwned::to_owned);

        Ok(Some(CachedProviderModels {
            models,
            models_url,
            fetched_at: fetched_at.to_string(),
            age_seconds,
        }))
    }

    pub fn set_provider_runtime_status(
        &self,
        id: &str,
        status: &str,
        cooldown_until: Option<String>,
        expires_at: Option<String>,
    ) -> Result<ProviderConnectionRecord> {
        let conn = self.open()?;
        let status = normalize_status(Some(status));
        let now = now_text();
        let changed = conn.execute(
            r#"
            UPDATE providerConnections
            SET status = ?2,
                cooldownUntil = ?3,
                expiresAt = ?4,
                updatedAt = ?5
            WHERE id = ?1
            "#,
            params![id, status, cooldown_until, expires_at, now],
        )?;
        if changed == 0 {
            return Err(StorageError::Validation(
                "provider connection not found".to_string(),
            ));
        }

        Ok(provider_record_from_raw(
            self.get_provider_connection_raw(id)?,
        ))
    }

    pub fn delete_provider_connection(&self, id: &str) -> Result<()> {
        let conn = self.open()?;
        let changed = conn.execute("DELETE FROM providerConnections WHERE id = ?1", params![id])?;
        if changed == 0 {
            return Err(StorageError::Validation(
                "provider connection not found".to_string(),
            ));
        }
        Ok(())
    }

    pub fn list_combos(&self) -> Result<Vec<ComboRecord>> {
        let conn = self.open()?;
        let mut stmt = conn.prepare(
            r#"
            SELECT id, name, kind, models, createdAt, updatedAt
            FROM combos
            ORDER BY name ASC
            "#,
        )?;

        let rows = stmt.query_map([], combo_from_row)?;

        rows.collect::<std::result::Result<Vec<_>, _>>()
            .map_err(StorageError::from)
    }

    pub fn get_combo(&self, id: &str) -> Result<ComboRecord> {
        let conn = self.open()?;
        conn.query_row(
            r#"
            SELECT id, name, kind, models, createdAt, updatedAt
            FROM combos
            WHERE id = ?1
            "#,
            params![id],
            combo_from_row,
        )
        .optional()?
        .ok_or_else(|| StorageError::Validation("combo not found".to_string()))
    }

    pub fn create_combo(&self, input: NewCombo) -> Result<ComboRecord> {
        let conn = self.open()?;
        let normalized = self.normalize_combo_input(&conn, input, None)?;
        let id = new_id();
        let now = now_text();
        let raw_models = serde_json::to_string_pretty(&normalized.models)?;

        conn.execute(
            r#"
            INSERT INTO combos (id, name, kind, models, createdAt, updatedAt)
            VALUES (?1, ?2, ?3, ?4, ?5, ?6)
            "#,
            params![id, normalized.name, normalized.kind, raw_models, now, now],
        )?;

        Ok(ComboRecord {
            id,
            name: normalized.name,
            kind: normalized.kind,
            models: normalized.models,
            created_at: now.clone(),
            updated_at: now,
        })
    }

    pub fn update_combo(&self, id: &str, input: NewCombo) -> Result<ComboRecord> {
        let existing = self.get_combo(id)?;
        let conn = self.open()?;
        let normalized = self.normalize_combo_input(&conn, input, Some(id))?;
        let now = now_text();
        let raw_models = serde_json::to_string_pretty(&normalized.models)?;

        let changed = conn.execute(
            r#"
            UPDATE combos
            SET name = ?2,
                kind = ?3,
                models = ?4,
                updatedAt = ?5
            WHERE id = ?1
            "#,
            params![id, normalized.name, normalized.kind, raw_models, now],
        )?;

        if changed == 0 {
            return Err(StorageError::Validation("combo not found".to_string()));
        }

        Ok(ComboRecord {
            id: id.to_string(),
            name: normalized.name,
            kind: normalized.kind,
            models: normalized.models,
            created_at: existing.created_at,
            updated_at: now,
        })
    }

    pub fn delete_combo(&self, id: &str) -> Result<()> {
        let conn = self.open()?;
        let changed = conn.execute("DELETE FROM combos WHERE id = ?1", params![id])?;
        if changed == 0 {
            return Err(StorageError::Validation("combo not found".to_string()));
        }
        Ok(())
    }

    pub fn resolve_model_route(&self, model: &str) -> Result<RouteKind> {
        let model = model.trim();
        if model.is_empty() {
            return Err(StorageError::Validation("missing model".to_string()));
        }

        if model.contains('/') {
            let model_ref = parse_provider_model(model)
                .map_err(|error| StorageError::Validation(error.to_string()))?;
            let conn = self.open()?;
            ensure_active_provider(&conn, &model_ref.provider)?;
            return Ok(RouteKind::Single(model_ref));
        }

        // Check model aliases first (Phase 4.3)
        if let Ok(conn) = self.open() {
            let alias_target: Option<String> = conn
                .query_row(
                    "SELECT target FROM modelAliases WHERE alias = ?1 LIMIT 1",
                    params![model],
                    |row| row.get(0),
                )
                .optional()?;
            if let Some(target) = alias_target {
                let model_ref = parse_provider_model(&target)
                    .map_err(|error| StorageError::Validation(error.to_string()))?;
                let conn = self.open()?;
                ensure_active_provider(&conn, &model_ref.provider)?;
                return Ok(RouteKind::Single(model_ref));
            }
        }

        let combo = self.get_combo_by_name(model)?;
        let models = combo
            .models
            .iter()
            .map(|item| {
                parse_provider_model(item)
                    .map_err(|error| StorageError::Validation(error.to_string()))
            })
            .collect::<Result<Vec<_>>>()?;

        Ok(RouteKind::Combo {
            name: combo.name,
            models,
        })
    }

    // ── Model Aliases (Phase 4.3) ──────────────────────────────────────────

    pub fn list_model_aliases(&self) -> Result<Vec<ModelAliasRecord>> {
        let conn = self.open()?;
        let mut stmt = conn.prepare(
            r#"
            SELECT id, alias, target, createdAt, updatedAt
            FROM modelAliases
            ORDER BY alias ASC
            "#,
        )?;
        let rows = stmt.query_map([], |row| {
            Ok(ModelAliasRecord {
                id: row.get(0)?,
                alias: row.get(1)?,
                target: row.get(2)?,
                created_at: row.get(3)?,
                updated_at: row.get(4)?,
            })
        })?;
        rows.collect::<std::result::Result<Vec<_>, _>>()
            .map_err(StorageError::from)
    }

    pub fn get_model_alias_by_name(&self, alias: &str) -> Result<Option<String>> {
        let conn = self.open()?;
        let target: Option<String> = conn
            .query_row(
                "SELECT target FROM modelAliases WHERE alias = ?1 LIMIT 1",
                params![alias],
                |row| row.get(0),
            )
            .optional()?;
        Ok(target)
    }

    pub fn create_model_alias(&self, input: NewModelAlias) -> Result<ModelAliasRecord> {
        let alias = input.alias.trim().to_string();
        let target = input.target.trim().to_string();
        if alias.is_empty() || target.is_empty() {
            return Err(StorageError::Validation(
                "alias and target are required".to_string(),
            ));
        }
        // Validate target format: must contain "/"
        if !target.contains('/') {
            return Err(StorageError::Validation(
                "target must be in format provider/model".to_string(),
            ));
        }
        // Check for duplicate alias name (also conflicts with combo names)
        let conn = self.open()?;
        let existing: Option<String> = conn
            .query_row(
                "SELECT id FROM modelAliases WHERE alias = ?1 LIMIT 1",
                params![alias],
                |row| row.get(0),
            )
            .optional()?;
        if existing.is_some() {
            return Err(StorageError::Validation(format!(
                "alias '{}' already exists",
                alias
            )));
        }
        let id = Uuid::new_v4().to_string();
        let now = blackrouter_common::unix_timestamp().to_string();
        conn.execute(
            r#"
            INSERT INTO modelAliases (id, alias, target, createdAt, updatedAt)
            VALUES (?1, ?2, ?3, ?4, ?5)
            "#,
            params![id, alias, target, now, now],
        )?;
        Ok(ModelAliasRecord {
            id,
            alias,
            target,
            created_at: now.clone(),
            updated_at: now,
        })
    }

    pub fn update_model_alias(&self, id: &str, input: NewModelAlias) -> Result<ModelAliasRecord> {
        let alias = input.alias.trim().to_string();
        let target = input.target.trim().to_string();
        if alias.is_empty() || target.is_empty() {
            return Err(StorageError::Validation(
                "alias and target are required".to_string(),
            ));
        }
        if !target.contains('/') {
            return Err(StorageError::Validation(
                "target must be in format provider/model".to_string(),
            ));
        }
        let conn = self.open()?;
        let existing: Option<()> = conn
            .query_row(
                "SELECT 1 FROM modelAliases WHERE id = ?1",
                params![id],
                |_| Ok(()),
            )
            .optional()?;
        if existing.is_none() {
            return Err(StorageError::Validation("alias not found".to_string()));
        }
        let now = blackrouter_common::unix_timestamp().to_string();
        conn.execute(
            r#"
            UPDATE modelAliases
            SET alias = ?2, target = ?3, updatedAt = ?4
            WHERE id = ?1
            "#,
            params![id, alias, target, now],
        )?;
        Ok(ModelAliasRecord {
            id: id.to_string(),
            alias,
            target,
            created_at: String::new(), // not needed for response
            updated_at: now,
        })
    }

    pub fn delete_model_alias(&self, id: &str) -> Result<()> {
        let conn = self.open()?;
        let changed = conn.execute("DELETE FROM modelAliases WHERE id = ?1", params![id])?;
        if changed == 0 {
            return Err(StorageError::Validation("alias not found".to_string()));
        }
        Ok(())
    }

    pub fn is_valid_api_key(&self, api_key: &str) -> Result<bool> {
        let conn = self.open()?;
        let found = conn
            .query_row(
                "SELECT 1 FROM apiKeys WHERE key = ?1 AND isActive = 1 LIMIT 1",
                params![api_key],
                |_| Ok(()),
            )
            .optional()?
            .is_some();
        Ok(found)
    }

    /// Record a usage entry to the usageHistory table (Phase 3.1)
    pub fn record_usage(&self, entry: &UsageEntry) -> Result<()> {
        let conn = self.open()?;
        conn.execute(
            "INSERT INTO usageHistory
             (timestamp, provider, model, connectionId, apiKey, endpoint,
              promptTokens, completionTokens, cost, status, tokens, meta)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
            params![
                entry.timestamp,
                entry.provider,
                entry.model,
                entry.connection_id,
                entry.api_key,
                entry.endpoint,
                entry.prompt_tokens,
                entry.completion_tokens,
                entry.cost,
                entry.status,
                entry.tokens,
                entry.meta,
            ],
        )?;
        Ok(())
    }

    /// Record multiple usage entries in a single transaction (Phase 1.4)
    pub fn record_usages_batch(&self, entries: &[UsageEntry]) -> Result<()> {
        let conn = self.open()?;
        let tx = conn.unchecked_transaction()?;
        for entry in entries {
            tx.execute(
                "INSERT INTO usageHistory
                 (timestamp, provider, model, connectionId, apiKey, endpoint,
                  promptTokens, completionTokens, cost, status, tokens, meta)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
                rusqlite::params![
                    entry.timestamp,
                    entry.provider,
                    entry.model,
                    entry.connection_id,
                    entry.api_key,
                    entry.endpoint,
                    entry.prompt_tokens,
                    entry.completion_tokens,
                    entry.cost,
                    entry.status,
                    entry.tokens,
                    entry.meta,
                ],
            )?;
        }
        tx.commit()?;
        Ok(())
    }

    /// Record multiple request detail entries in a single transaction (Phase 1.4)
    pub fn record_request_details_batch(&self, entries: &[RequestDetailEntry]) -> Result<()> {
        let conn = self.open()?;
        let tx = conn.unchecked_transaction()?;
        for entry in entries {
            tx.execute(
                "INSERT INTO requestDetails
                 (id, timestamp, provider, model, connectionId, status, data)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                rusqlite::params![
                    entry.id,
                    entry.timestamp,
                    entry.provider,
                    entry.model,
                    entry.connection_id,
                    entry.status,
                    entry.data,
                ],
            )?;
        }
        tx.commit()?;
        Ok(())
    }

    /// Get aggregated usage stats for a time range
    pub fn usage_stats(&self, since: Option<&str>) -> Result<Vec<UsageRow>> {
        let conn = self.open()?;
        let since = since.unwrap_or("1970-01-01T00:00:00Z");
        let mut stmt = conn.prepare(
            "SELECT provider, connectionId, model, status,
                    SUM(promptTokens) as pt, SUM(completionTokens) as ct,
                    SUM(cost) as cost, COUNT(*) as cnt
             FROM usageHistory
             WHERE timestamp >= ?1
             GROUP BY provider, connectionId, model, status
             ORDER BY cnt DESC",
        )?;
        let rows = stmt.query_map(params![since], |row| {
            Ok(UsageRow {
                provider: row.get::<_, Option<String>>(0)?.unwrap_or_default(),
                connection_id: row.get::<_, Option<String>>(1)?,
                model: row.get::<_, Option<String>>(2)?.unwrap_or_default(),
                status: row.get::<_, Option<String>>(3)?.unwrap_or_default(),
                prompt_tokens: row.get::<_, i64>(4)? as u64,
                completion_tokens: row.get::<_, i64>(5)? as u64,
                cost: row.get::<_, f64>(6)?,
                count: row.get::<_, i64>(7)? as u64,
            })
        })?;
        rows.collect::<std::result::Result<Vec<_>, _>>()
            .map_err(StorageError::Sqlite)
    }

    /// Load all persisted RTK state rows (rate-limit + circuit-breaker snapshots).
    pub fn load_rtk_state(&self) -> Result<Vec<RtkStateRow>> {
        let conn = self.open()?;
        let mut stmt = conn.prepare(
            "SELECT key, kind, data, updated_at FROM rtk_state ORDER BY updated_at DESC",
        )?;
        let rows = stmt.query_map([], |row| {
            Ok(RtkStateRow {
                key: row.get::<_, String>(0)?,
                kind: row.get::<_, String>(1)?,
                data: row.get::<_, String>(2)?,
                updated_at: row.get::<_, i64>(3)? as u64,
            })
        })?;
        rows.collect::<std::result::Result<Vec<_>, _>>()
            .map_err(StorageError::Sqlite)
    }

    /// Persist a batch of RTK state rows, replacing any existing rows with the
    /// same `key`. Uses a single transaction for atomicity.
    pub fn save_rtk_state(&self, rows: &[RtkStateRow]) -> Result<()> {
        if rows.is_empty() {
            return Ok(());
        }
        let conn = self.open()?;
        let tx = conn.unchecked_transaction()?;
        for row in rows {
            tx.execute(
                "INSERT INTO rtk_state (key, kind, data, updated_at)
                 VALUES (?1, ?2, ?3, ?4)
                 ON CONFLICT(key) DO UPDATE SET
                   kind = excluded.kind,
                   data = excluded.data,
                   updated_at = excluded.updated_at",
                params![row.key, row.kind, row.data, row.updated_at],
            )?;
        }
        tx.commit()?;
        Ok(())
    }

    /// Remove RTK state rows older than `before_unix` (replacement-guard so a
    /// stalled snapshotter cannot wipe a fresh snapshot).
    pub fn prune_rtk_state(&self, before_unix: u64) -> Result<()> {
        let conn = self.open()?;
        conn.execute(
            "DELETE FROM rtk_state WHERE updated_at < ?1",
            params![before_unix],
        )?;
        Ok(())
    }

    /// Upsert a batch of model catalog entries (Phase 7.1).
    pub fn upsert_model_catalog(&self, entries: &[ModelCatalogEntry]) -> Result<()> {
        if entries.is_empty() {
            return Ok(());
        }
        let conn = self.open()?;
        let tx = conn.unchecked_transaction()?;
        for e in entries {
            tx.execute(
                "INSERT INTO model_catalog \
                 (provider, model, context_window, modalities, price_in_per_million, price_out_per_million, latency_p50_ms) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7) \
                 ON CONFLICT(provider, model) DO UPDATE SET \
                   context_window = excluded.context_window,\
                   modalities = excluded.modalities,\
                   price_in_per_million = excluded.price_in_per_million,\
                   price_out_per_million = excluded.price_out_per_million,\
                   latency_p50_ms = excluded.latency_p50_ms",
                params![
                    e.provider,
                    e.model,
                    e.context_window.map(|v| v as i64),
                    e.modalities,
                    e.price_in_per_million,
                    e.price_out_per_million,
                    e.latency_p50_ms.map(|v| v as i64),
                ],
            )?;
        }
        tx.commit()?;
        Ok(())
    }

    /// Load all model catalog entries.
    pub fn load_model_catalog(&self) -> Result<Vec<ModelCatalogEntry>> {
        let conn = self.open()?;
        let mut stmt = conn.prepare(
            "SELECT provider, model, context_window, modalities, price_in_per_million, \
             price_out_per_million, latency_p50_ms FROM model_catalog",
        )?;
        let rows = stmt.query_map([], |row| {
            Ok(ModelCatalogEntry {
                provider: row.get::<_, String>(0)?,
                model: row.get::<_, String>(1)?,
                context_window: row.get::<_, Option<i64>>(2)?.map(|v| v as u64),
                modalities: row.get::<_, Option<String>>(3)?,
                price_in_per_million: row.get::<_, Option<f64>>(4)?,
                price_out_per_million: row.get::<_, Option<f64>>(5)?,
                latency_p50_ms: row.get::<_, Option<i64>>(6)?.map(|v| v as u64),
            })
        })?;
        rows.collect::<std::result::Result<Vec<_>, _>>()
            .map_err(StorageError::Sqlite)
    }

    /// Get a single catalog entry for a provider/model pair.
    pub fn get_model_catalog(
        &self,
        provider: &str,
        model: &str,
    ) -> Result<Option<ModelCatalogEntry>> {
        let conn = self.open()?;
        let mut stmt = conn.prepare(
            "SELECT provider, model, context_window, modalities, price_in_per_million, \
             price_out_per_million, latency_p50_ms FROM model_catalog \
             WHERE provider = ?1 AND model = ?2",
        )?;
        let mut rows = stmt.query_map(params![provider, model], |row| {
            Ok(ModelCatalogEntry {
                provider: row.get::<_, String>(0)?,
                model: row.get::<_, String>(1)?,
                context_window: row.get::<_, Option<i64>>(2)?.map(|v| v as u64),
                modalities: row.get::<_, Option<String>>(3)?,
                price_in_per_million: row.get::<_, Option<f64>>(4)?,
                price_out_per_million: row.get::<_, Option<f64>>(5)?,
                latency_p50_ms: row.get::<_, Option<i64>>(6)?.map(|v| v as u64),
            })
        })?;
        match rows.next() {
            Some(row) => row.map(Some).map_err(StorageError::Sqlite),
            None => Ok(None),
        }
    }

    // ── Generic key/value store (Phase 8: conversation memory) ────────────
    // Backed by the pre-existing `kv (scope, key, value)` compat table.

    /// Read a raw value from the `kv` store. Returns `Ok(None)` if absent.
    pub fn get_kv(&self, scope: &str, key: &str) -> Result<Option<String>> {
        let conn = self.open()?;
        let mut stmt = conn.prepare("SELECT value FROM kv WHERE scope = ?1 AND key = ?2")?;
        let mut rows = stmt.query_map(params![scope, key], |row| row.get::<_, String>(0))?;
        match rows.next() {
            Some(row) => row.map(Some).map_err(StorageError::Sqlite),
            None => Ok(None),
        }
    }

    /// Upsert a raw value into the `kv` store (insert or replace).
    pub fn set_kv(&self, scope: &str, key: &str, value: &str) -> Result<()> {
        let conn = self.open()?;
        conn.execute(
            "INSERT INTO kv (scope, key, value) VALUES (?1, ?2, ?3) \
             ON CONFLICT(scope, key) DO UPDATE SET value = excluded.value",
            params![scope, key, value],
        )?;
        Ok(())
    }

    /// Delete a value from the `kv` store. Idempotent (no-op if absent).
    pub fn delete_kv(&self, scope: &str, key: &str) -> Result<()> {
        let conn = self.open()?;
        conn.execute(
            "DELETE FROM kv WHERE scope = ?1 AND key = ?2",
            params![scope, key],
        )?;
        Ok(())
    }

    /// List every `(key, value)` pair in a scope, ordered by key.
    /// Used by the conversation-management control endpoints.
    pub fn list_kv(&self, scope: &str) -> Result<Vec<(String, String)>> {
        let conn = self.open()?;
        let mut stmt = conn.prepare("SELECT key, value FROM kv WHERE scope = ?1 ORDER BY key")?;
        let rows = stmt.query_map(params![scope], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })?;
        rows.collect::<std::result::Result<Vec<_>, _>>()
            .map_err(StorageError::Sqlite)
    }

    /// Total cost (USD) recorded since the given unix timestamp.
    pub fn total_cost_since(&self, since_unix: u64) -> Result<f64> {
        let conn = self.open()?;
        let total = conn.query_row(
            r#"
            SELECT COALESCE(SUM(cost), 0)
            FROM usageHistory
            WHERE CAST(timestamp AS INTEGER) >= ?1
              AND COALESCE(status, '') = 'success'
            "#,
            params![since_unix],
            |row| row.get::<_, f64>(0),
        )?;
        Ok(total)
    }

    /// Record a request detail entry to the requestDetails table (Phase 3.1)
    pub fn record_request_details(&self, entry: &RequestDetailEntry) -> Result<()> {
        let conn = self.open()?;
        conn.execute(
            "INSERT INTO requestDetails
             (id, timestamp, provider, model, connectionId, status, data)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![
                entry.id,
                entry.timestamp,
                entry.provider,
                entry.model,
                entry.connection_id,
                entry.status,
                entry.data,
            ],
        )?;
        Ok(())
    }

    /// Aggregate usage for a specific date (YYYY-MM-DD) from usageHistory into usageDaily
    pub fn aggregate_daily_usage(&self, date_key: &str) -> Result<DailyUsage> {
        let conn = self.open()?;
        let start = format!("{}T00:00:00Z", date_key);
        let end = format!("{}T23:59:59Z", date_key);

        // Aggregate from usageHistory
        let total_requests: i64 = conn.query_row(
            "SELECT COUNT(*) FROM usageHistory WHERE timestamp >= ?1 AND timestamp <= ?2",
            params![start, end],
            |row| row.get(0),
        )?;

        let successful_requests: i64 = conn.query_row(
            "SELECT COUNT(*) FROM usageHistory WHERE timestamp >= ?1 AND timestamp <= ?2 AND status = 'success'",
            params![start, end],
            |row| row.get(0),
        )?;

        let failed_requests: i64 = conn.query_row(
            "SELECT COUNT(*) FROM usageHistory WHERE timestamp >= ?1 AND timestamp <= ?2 AND status != 'success'",
            params![start, end],
            |row| row.get(0),
        )?;

        let total_prompt_tokens: i64 = conn.query_row(
            "SELECT COALESCE(SUM(promptTokens), 0) FROM usageHistory WHERE timestamp >= ?1 AND timestamp <= ?2",
            params![start, end],
            |row| row.get(0),
        )?;

        let total_completion_tokens: i64 = conn.query_row(
            "SELECT COALESCE(SUM(completionTokens), 0) FROM usageHistory WHERE timestamp >= ?1 AND timestamp <= ?2",
            params![start, end],
            |row| row.get(0),
        )?;

        let total_cost: f64 = conn.query_row(
            "SELECT COALESCE(SUM(cost), 0.0) FROM usageHistory WHERE timestamp >= ?1 AND timestamp <= ?2",
            params![start, end],
            |row| row.get(0),
        )?;

        // Aggregate by provider
        let mut stmt = conn.prepare(
            "SELECT provider, COUNT(*), SUM(promptTokens), SUM(completionTokens), SUM(cost)
             FROM usageHistory WHERE timestamp >= ?1 AND timestamp <= ?2
             GROUP BY provider",
        )?;
        let provider_rows = stmt.query_map(params![start, end], |row| {
            Ok(serde_json::json!({
                "provider": row.get::<_, Option<String>>(0)?.unwrap_or_default(),
                "requests": row.get::<_, i64>(1)?,
                "prompt_tokens": row.get::<_, i64>(2)? as u64,
                "completion_tokens": row.get::<_, i64>(3)? as u64,
                "cost": row.get::<_, f64>(4)?,
            }))
        })?;
        let by_provider: Vec<Value> = provider_rows.filter_map(|r| r.ok()).collect();

        let daily = DailyUsage {
            date_key: date_key.to_string(),
            total_requests: total_requests as u64,
            successful_requests: successful_requests as u64,
            failed_requests: failed_requests as u64,
            total_prompt_tokens: total_prompt_tokens as u64,
            total_completion_tokens: total_completion_tokens as u64,
            total_cost,
            by_provider: Value::Array(by_provider),
        };

        // Upsert into usageDaily
        let data_json = serde_json::to_string(&daily).unwrap_or_else(|_| "{}".to_string());
        conn.execute(
            "INSERT INTO usageDaily (dateKey, data) VALUES (?1, ?2)
             ON CONFLICT(dateKey) DO UPDATE SET data = ?2",
            params![date_key, data_json],
        )?;

        Ok(daily)
    }

    /// Get daily usage for a specific date
    pub fn get_daily_usage(&self, date_key: &str) -> Result<Option<DailyUsage>> {
        let conn = self.open()?;
        let result = conn
            .query_row(
                "SELECT data FROM usageDaily WHERE dateKey = ?1 LIMIT 1",
                params![date_key],
                |row| row.get::<_, String>(0),
            )
            .optional()?;

        match result {
            Some(json) => {
                let daily: DailyUsage = serde_json::from_str(&json).map_err(StorageError::from)?;
                Ok(Some(daily))
            }
            None => Ok(None),
        }
    }

    /// List daily usage entries
    pub fn list_daily_usage(&self, limit: u32) -> Result<Vec<DailyUsage>> {
        let conn = self.open()?;
        let mut stmt =
            conn.prepare("SELECT data FROM usageDaily ORDER BY dateKey DESC LIMIT ?1")?;
        let rows = stmt.query_map(params![limit], |row| row.get::<_, String>(0))?;
        rows.filter_map(|r| r.ok())
            .filter_map(|json| serde_json::from_str::<DailyUsage>(&json).ok())
            .collect::<Vec<_>>()
            .into_iter()
            .map(Ok)
            .collect()
    }

    pub fn list_model_shell_items(&self) -> Result<Vec<ModelListItem>> {
        let conn = self.open()?;
        let mut items = Vec::new();

        if table_exists(&conn, "combos")? {
            let mut stmt = conn.prepare("SELECT name FROM combos ORDER BY name ASC")?;
            let rows = stmt.query_map([], |row| row.get::<_, String>(0))?;
            for row in rows {
                items.push(ModelListItem {
                    id: row?,
                    object: "model",
                    owned_by: "combo",
                });
            }
        }

        // Add model aliases to the model list
        if table_exists(&conn, "modelAliases")? {
            let mut stmt = conn.prepare("SELECT alias FROM modelAliases ORDER BY alias ASC")?;
            let rows = stmt.query_map([], |row| row.get::<_, String>(0))?;
            for row in rows {
                items.push(ModelListItem {
                    id: row?,
                    object: "model",
                    owned_by: "alias",
                });
            }
        }

        Ok(items)
    }

    fn get_combo_by_name(&self, name: &str) -> Result<ComboRecord> {
        let conn = self.open()?;
        conn.query_row(
            r#"
            SELECT id, name, kind, models, createdAt, updatedAt
            FROM combos
            WHERE name = ?1
            "#,
            params![name],
            combo_from_row,
        )
        .optional()?
        .ok_or_else(|| StorageError::Validation("model route not found".to_string()))
    }

    fn normalize_combo_input(
        &self,
        conn: &Connection,
        input: NewCombo,
        existing_id: Option<&str>,
    ) -> Result<NormalizedComboInput> {
        let name = input.name.trim().to_string();
        if name.is_empty() {
            return Err(StorageError::Validation(
                "combo name is required".to_string(),
            ));
        }
        if name.contains('/') {
            return Err(StorageError::Validation(
                "combo name must not contain /".to_string(),
            ));
        }

        let kind = input
            .kind
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
            .unwrap_or_else(|| "llm".to_string());

        let models = input
            .models
            .into_iter()
            .map(|model| model.trim().to_string())
            .filter(|model| !model.is_empty())
            .collect::<Vec<_>>();
        if models.is_empty() {
            return Err(StorageError::Validation(
                "combo models are required".to_string(),
            ));
        }

        let duplicate_id = conn
            .query_row(
                "SELECT id FROM combos WHERE name = ?1 LIMIT 1",
                params![name],
                |row| row.get::<_, String>(0),
            )
            .optional()?;
        if duplicate_id
            .as_deref()
            .map(|id| Some(id) != existing_id)
            .unwrap_or(false)
        {
            return Err(StorageError::Validation(
                "combo name already exists".to_string(),
            ));
        }

        for model in &models {
            let model_ref = parse_provider_model(model)
                .map_err(|error| StorageError::Validation(error.to_string()))?;
            ensure_active_provider(conn, &model_ref.provider)?;
        }

        Ok(NormalizedComboInput { name, kind, models })
    }

    fn status_with_existing_flag(&self, existed_before_init: bool) -> Result<StorageStatus> {
        self.ensure_parent_dir()?;
        let conn = self.open()?;
        conn.execute_batch(PRAGMA_SQL)?;

        let mut missing_compat_tables = Vec::new();
        let mut table_counts = BTreeMap::new();

        for table in COMPAT_TABLES.iter().chain(BLACKROUTER_TABLES.iter()) {
            if table_exists(&conn, table)? {
                table_counts.insert((*table).to_string(), count_rows(&conn, table)?);
            } else if COMPAT_TABLES.contains(table) {
                missing_compat_tables.push((*table).to_string());
            }
        }

        Ok(StorageStatus {
            database_path: self.database_path.clone(),
            existed_before_init,
            schema_compatible: missing_compat_tables.is_empty(),
            missing_compat_tables,
            table_counts,
        })
    }

    fn open(&self) -> Result<Connection> {
        self.ensure_parent_dir()?;
        Connection::open(&self.database_path).map_err(StorageError::from)
    }

    fn ensure_parent_dir(&self) -> Result<()> {
        if let Some(parent) = self.database_path.parent() {
            fs::create_dir_all(parent)?;
        }
        Ok(())
    }
}

fn new_id() -> String {
    Uuid::new_v4().to_string()
}

fn now_text() -> String {
    blackrouter_common::unix_timestamp().to_string()
}

fn normalize_opt(value: Option<String>) -> Option<String> {
    value
        .map(|item| item.trim().to_string())
        .filter(|item| !item.is_empty())
}

fn parse_policy(raw: Option<String>) -> ApiKeyPolicy {
    raw.and_then(|value| serde_json::from_str(&value).ok())
        .unwrap_or_default()
}

fn validate_api_key_policy(policy: &ApiKeyPolicy) -> Result<()> {
    if policy.cost_per_month_usd.is_some_and(|value| value < 0.0) {
        return Err(StorageError::Validation(
            "cost_per_month_usd cannot be negative".to_string(),
        ));
    }
    if policy
        .provider_allowlist
        .iter()
        .any(|value| value.trim().is_empty())
        || policy
            .model_allowlist
            .iter()
            .any(|value| value.trim().is_empty())
    {
        return Err(StorageError::Validation(
            "allowlist entries cannot be empty".to_string(),
        ));
    }
    Ok(())
}

fn normalize_status(value: Option<&str>) -> String {
    value
        .map(|item| item.trim().to_ascii_lowercase())
        .filter(|item| !item.is_empty())
        .unwrap_or_else(|| "unknown".to_string())
}

struct NormalizedComboInput {
    name: String,
    kind: String,
    models: Vec<String>,
}

fn combo_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<ComboRecord> {
    let raw_models: String = row.get(3)?;
    let models = serde_json::from_str::<Vec<String>>(&raw_models).unwrap_or_default();
    Ok(ComboRecord {
        id: row.get(0)?,
        name: row.get(1)?,
        kind: row
            .get::<_, Option<String>>(2)?
            .filter(|value| !value.trim().is_empty())
            .unwrap_or_else(|| "llm".to_string()),
        models,
        created_at: row.get(4)?,
        updated_at: row.get(5)?,
    })
}

fn provider_record_from_raw(raw: RawProviderConnection) -> ProviderConnectionRecord {
    ProviderConnectionRecord {
        id: raw.id,
        provider: raw.provider,
        auth_type: raw.auth_type,
        name: raw.name,
        email: raw.email,
        priority: raw.priority,
        is_active: raw.is_active,
        status: raw.status,
        cooldown_until: raw.cooldown_until,
        expires_at: raw.expires_at,
        data: mask_sensitive_json(raw.data),
        created_at: raw.created_at,
        updated_at: raw.updated_at,
    }
}

fn ensure_active_provider(conn: &Connection, provider: &str) -> Result<()> {
    let now = now_text();
    let found = conn
        .query_row(
            r#"
            SELECT 1
            FROM providerConnections
            WHERE provider = ?1
              AND isActive = 1
              AND COALESCE(status, 'unknown') NOT IN ('disabled', 'expired')
              AND (expiresAt IS NULL OR expiresAt = '' OR CAST(expiresAt AS INTEGER) > ?2)
            LIMIT 1
            "#,
            params![provider, now],
            |_| Ok(()),
        )
        .optional()?
        .is_some();

    if found {
        Ok(())
    } else {
        Err(StorageError::Validation(format!(
            "provider is not active or does not exist: {provider}"
        )))
    }
}

fn mask_sensitive_json(value: Value) -> Value {
    match value {
        Value::Object(map) => Value::Object(
            map.into_iter()
                .map(|(key, value)| {
                    let masked = if is_sensitive_key(&key) {
                        value
                            .as_str()
                            .map(mask_secret)
                            .map(Value::String)
                            .unwrap_or(Value::String("***".to_string()))
                    } else {
                        mask_sensitive_json(value)
                    };
                    (key, masked)
                })
                .collect(),
        ),
        Value::Array(items) => Value::Array(items.into_iter().map(mask_sensitive_json).collect()),
        other => other,
    }
}

fn preserve_sensitive_json(existing: Value, incoming: Value) -> Value {
    match (existing, incoming) {
        (Value::Object(existing), Value::Object(incoming)) => {
            let mut out = serde_json::Map::new();
            for (key, incoming_value) in incoming {
                let value = if is_sensitive_key(&key)
                    && incoming_value
                        .as_str()
                        .map(|value| value.contains("...") || value == "***" || value.is_empty())
                        .unwrap_or(false)
                {
                    existing.get(&key).cloned().unwrap_or(incoming_value)
                } else {
                    let existing_child = existing.get(&key).cloned().unwrap_or(Value::Null);
                    preserve_sensitive_json(existing_child, incoming_value)
                };
                out.insert(key, value);
            }
            Value::Object(out)
        }
        (_, incoming) => incoming,
    }
}

fn is_sensitive_key(key: &str) -> bool {
    let key = key.to_ascii_lowercase();
    key.contains("apikey")
        || key.contains("api_key")
        || key.contains("token")
        || key.contains("secret")
        || key.contains("password")
        || key == "authorization"
}

fn table_exists(conn: &Connection, table: &str) -> Result<bool> {
    let exists = conn
        .query_row(
            "SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = ?1 LIMIT 1",
            params![table],
            |_| Ok(()),
        )
        .optional()?
        .is_some();
    Ok(exists)
}

fn column_exists(conn: &Connection, table: &str, column: &str) -> Result<bool> {
    let mut stmt = conn.prepare(&format!("PRAGMA table_info({table})"))?;
    let rows = stmt.query_map([], |row| row.get::<_, String>(1))?;
    for row in rows {
        if row? == column {
            return Ok(true);
        }
    }
    Ok(false)
}

fn add_column_if_missing(
    conn: &Connection,
    table: &str,
    column: &str,
    definition: &str,
) -> Result<()> {
    if !column_exists(conn, table, column)? {
        conn.execute(&format!("ALTER TABLE {table} ADD COLUMN {definition}"), [])?;
    }
    Ok(())
}

fn migrate_schema(conn: &Connection) -> Result<()> {
    add_column_if_missing(
        conn,
        "providerConnections",
        "status",
        "status TEXT DEFAULT 'unknown'",
    )?;
    add_column_if_missing(
        conn,
        "providerConnections",
        "cooldownUntil",
        "cooldownUntil TEXT",
    )?;
    add_column_if_missing(conn, "providerConnections", "expiresAt", "expiresAt TEXT")?;
    add_column_if_missing(conn, "apiKeys", "tenantId", "tenantId TEXT")?;
    add_column_if_missing(
        conn,
        "apiKeys",
        "policy",
        "policy TEXT NOT NULL DEFAULT '{}'",
    )?;
    conn.execute(
        "CREATE INDEX IF NOT EXISTS idx_ak_tenant ON apiKeys(tenantId)",
        [],
    )?;
    Ok(())
}

fn count_rows(conn: &Connection, table: &str) -> Result<i64> {
    let sql = format!("SELECT COUNT(*) FROM {table}");
    conn.query_row(&sql, [], |row| row.get::<_, i64>(0))
        .map_err(StorageError::from)
}

const COMPAT_SCHEMA_SQL: &str = r#"
CREATE TABLE IF NOT EXISTS _meta (
  key TEXT PRIMARY KEY,
  value TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS settings (
  id INTEGER PRIMARY KEY CHECK (id = 1),
  data TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS providerConnections (
  id TEXT PRIMARY KEY,
  provider TEXT NOT NULL,
  authType TEXT NOT NULL,
  name TEXT,
  email TEXT,
  priority INTEGER,
  isActive INTEGER DEFAULT 1,
  status TEXT DEFAULT 'unknown',
  cooldownUntil TEXT,
  expiresAt TEXT,
  data TEXT NOT NULL,
  createdAt TEXT NOT NULL,
  updatedAt TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_pc_provider ON providerConnections(provider);
CREATE INDEX IF NOT EXISTS idx_pc_provider_active ON providerConnections(provider, isActive);
CREATE INDEX IF NOT EXISTS idx_pc_priority ON providerConnections(provider, priority);

CREATE TABLE IF NOT EXISTS providerNodes (
  id TEXT PRIMARY KEY,
  type TEXT,
  name TEXT,
  data TEXT NOT NULL,
  createdAt TEXT NOT NULL,
  updatedAt TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_pn_type ON providerNodes(type);

CREATE TABLE IF NOT EXISTS proxyPools (
  id TEXT PRIMARY KEY,
  isActive INTEGER DEFAULT 1,
  testStatus TEXT,
  data TEXT NOT NULL,
  createdAt TEXT NOT NULL,
  updatedAt TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_pp_active ON proxyPools(isActive);
CREATE INDEX IF NOT EXISTS idx_pp_status ON proxyPools(testStatus);

CREATE TABLE IF NOT EXISTS apiKeys (
  id TEXT PRIMARY KEY,
  key TEXT UNIQUE NOT NULL,
  name TEXT,
  machineId TEXT,
  tenantId TEXT,
  policy TEXT NOT NULL DEFAULT '{}',
  isActive INTEGER DEFAULT 1,
  createdAt TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_ak_key ON apiKeys(key);

CREATE TABLE IF NOT EXISTS combos (
  id TEXT PRIMARY KEY,
  name TEXT UNIQUE NOT NULL,
  kind TEXT,
  models TEXT NOT NULL,
  createdAt TEXT NOT NULL,
  updatedAt TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_combo_name ON combos(name);

CREATE TABLE IF NOT EXISTS kv (
  scope TEXT NOT NULL,
  key TEXT NOT NULL,
  value TEXT NOT NULL,
  PRIMARY KEY (scope, key)
);
CREATE INDEX IF NOT EXISTS idx_kv_scope ON kv(scope);

CREATE TABLE IF NOT EXISTS usageHistory (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  timestamp TEXT NOT NULL,
  provider TEXT,
  model TEXT,
  connectionId TEXT,
  apiKey TEXT,
  endpoint TEXT,
  promptTokens INTEGER DEFAULT 0,
  completionTokens INTEGER DEFAULT 0,
  cost REAL DEFAULT 0,
  status TEXT,
  tokens TEXT,
  meta TEXT
);
CREATE INDEX IF NOT EXISTS idx_uh_ts ON usageHistory(timestamp DESC);
CREATE INDEX IF NOT EXISTS idx_uh_provider ON usageHistory(provider);
CREATE INDEX IF NOT EXISTS idx_uh_model ON usageHistory(model);
CREATE INDEX IF NOT EXISTS idx_uh_conn ON usageHistory(connectionId);

CREATE TABLE IF NOT EXISTS usageDaily (
  dateKey TEXT PRIMARY KEY,
  data TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS requestDetails (
  id TEXT PRIMARY KEY,
  timestamp TEXT NOT NULL,
  provider TEXT,
  model TEXT,
  connectionId TEXT,
  status TEXT,
  data TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_rd_ts ON requestDetails(timestamp DESC);
CREATE INDEX IF NOT EXISTS idx_rd_provider ON requestDetails(provider);
CREATE INDEX IF NOT EXISTS idx_rd_model ON requestDetails(model);
CREATE INDEX IF NOT EXISTS idx_rd_conn ON requestDetails(connectionId);

CREATE TABLE IF NOT EXISTS modelAliases (
  id TEXT PRIMARY KEY,
  alias TEXT UNIQUE NOT NULL,
  target TEXT NOT NULL,
  createdAt TEXT NOT NULL,
  updatedAt TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_ma_alias ON modelAliases(alias);
"#;

const BLACKROUTER_SCHEMA_SQL: &str = r#"
CREATE TABLE IF NOT EXISTS apiKeyQuotaCounters (
  keyId TEXT NOT NULL,
  periodStart INTEGER NOT NULL,
  kind TEXT NOT NULL,
  value INTEGER NOT NULL DEFAULT 0,
  PRIMARY KEY (keyId, periodStart, kind)
);
CREATE INDEX IF NOT EXISTS idx_api_key_quota_period ON apiKeyQuotaCounters(periodStart);

CREATE TABLE IF NOT EXISTS settingsHistory (
  version INTEGER PRIMARY KEY AUTOINCREMENT,
  data TEXT NOT NULL,
  createdAt INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_settings_history_created ON settingsHistory(createdAt DESC);

CREATE TABLE IF NOT EXISTS adminAuditLog (
  id TEXT PRIMARY KEY,
  timestamp TEXT NOT NULL,
  actorType TEXT NOT NULL,
  actorId TEXT NOT NULL,
  action TEXT NOT NULL,
  target TEXT,
  status TEXT NOT NULL,
  meta TEXT
);

CREATE TABLE IF NOT EXISTS telegramLinks (
  id TEXT PRIMARY KEY,
  chatId TEXT NOT NULL,
  displayName TEXT,
  isActive INTEGER DEFAULT 1,
  createdAt TEXT NOT NULL,
  updatedAt TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_telegram_links_chat ON telegramLinks(chatId);

CREATE TABLE IF NOT EXISTS runtimeEvents (
  id TEXT PRIMARY KEY,
  timestamp TEXT NOT NULL,
  level TEXT NOT NULL,
  source TEXT NOT NULL,
  message TEXT NOT NULL,
  meta TEXT
);
CREATE INDEX IF NOT EXISTS idx_runtime_events_ts ON runtimeEvents(timestamp DESC);

CREATE TABLE IF NOT EXISTS rtk_state (
  key TEXT PRIMARY KEY,
  kind TEXT NOT NULL,
  data TEXT NOT NULL,
  updated_at INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_rtk_state_kind ON rtk_state(kind);

CREATE TABLE IF NOT EXISTS model_catalog (
  provider TEXT NOT NULL,
  model TEXT NOT NULL,
  context_window INTEGER,
  modalities TEXT,
  price_in_per_million REAL,
  price_out_per_million REAL,
  latency_p50_ms INTEGER,
  PRIMARY KEY (provider, model)
);
CREATE INDEX IF NOT EXISTS idx_model_catalog_provider ON model_catalog(provider);
"#;

#[cfg(test)]
mod tests {
    use super::{ApiKeyPolicy, NewApiKey, NewCombo, NewProviderConnection, Storage, UsageEntry};
    use blackrouter_core::{ModelRef, RouteKind};
    use serde_json::json;
    use std::fs;

    fn temp_storage(label: &str) -> (Storage, std::path::PathBuf) {
        let path = std::env::temp_dir().join(format!(
            "blackrouter-storage-{label}-{}-{}.sqlite",
            std::process::id(),
            uuid::Uuid::new_v4().simple()
        ));

        let storage = Storage::new(&path);
        storage.initialize().expect("schema initializes");
        (storage, path)
    }

    fn active_provider(provider: &str) -> NewProviderConnection {
        NewProviderConnection {
            provider: provider.to_string(),
            auth_type: "none".to_string(),
            name: Some(provider.to_string()),
            email: None,
            priority: None,
            is_active: Some(true),
            status: None,
            cooldown_until: None,
            expires_at: None,
            data: Some(json!({
                "baseUrl": "http://127.0.0.1:20130/health",
                "format": "openai"
            })),
        }
    }

    #[test]
    fn initializes_schema() {
        let (storage, path) = temp_storage("schema");
        let status = storage.status().expect("storage status");

        assert!(status.schema_compatible);
        assert!(status.table_counts.contains_key("settings"));

        let _ = fs::remove_file(path);
    }

    #[test]
    fn migrates_legacy_api_keys_before_creating_tenant_index() {
        let path = std::env::temp_dir().join(format!(
            "blackrouter-storage-legacy-api-keys-{}-{}.sqlite",
            std::process::id(),
            uuid::Uuid::new_v4().simple()
        ));
        let conn = rusqlite::Connection::open(&path).unwrap();
        conn.execute_batch(
            "CREATE TABLE apiKeys (id TEXT PRIMARY KEY, key TEXT UNIQUE NOT NULL, name TEXT, machineId TEXT, isActive INTEGER DEFAULT 1, createdAt TEXT NOT NULL);\
             INSERT INTO apiKeys (id, key, isActive, createdAt) VALUES ('legacy', 'brk_legacy', 1, '1');",
        )
        .unwrap();
        drop(conn);

        let storage = Storage::new(&path);
        storage.initialize().expect("legacy schema migrates");
        let authenticated = storage.authenticate_api_key("brk_legacy").unwrap().unwrap();
        assert_eq!(authenticated.id, "legacy");
        assert_eq!(authenticated.policy, ApiKeyPolicy::default());
        let _ = fs::remove_file(path);
    }

    #[test]
    fn api_key_policy_round_trips_and_rotation_preserves_scope() {
        let (storage, path) = temp_storage("api-key-policy");
        let policy = ApiKeyPolicy {
            requests_per_day: Some(10),
            tokens_per_day: Some(1_000),
            cost_per_month_usd: Some(12.5),
            provider_allowlist: vec!["openai".to_string()],
            model_allowlist: vec!["openai/gpt-5".to_string()],
        };
        let created = storage
            .create_api_key(NewApiKey {
                name: Some("tenant key".to_string()),
                machine_id: None,
                tenant_id: Some("tenant-a".to_string()),
                policy: policy.clone(),
            })
            .unwrap();
        let authenticated = storage.authenticate_api_key(&created.key).unwrap().unwrap();
        assert_eq!(authenticated.tenant_id.as_deref(), Some("tenant-a"));
        assert_eq!(authenticated.policy, policy);

        let rotated = storage.rotate_api_key(&created.record.id).unwrap();
        assert!(storage
            .authenticate_api_key(&created.key)
            .unwrap()
            .is_none());
        let authenticated = storage.authenticate_api_key(&rotated.key).unwrap().unwrap();
        assert_eq!(authenticated.policy, policy);
        assert_eq!(authenticated.tenant_id.as_deref(), Some("tenant-a"));
        let _ = fs::remove_file(path);
    }

    #[test]
    fn settings_history_and_key_usage_are_queryable() {
        let (storage, path) = temp_storage("settings-history-usage");
        storage
            .save_settings_json(&json!({"requireApiKey": false}))
            .unwrap();
        storage
            .save_settings_json(&json!({"requireApiKey": true}))
            .unwrap();
        let versions = storage.list_settings_versions(10).unwrap();
        assert_eq!(versions.len(), 2);
        assert_eq!(versions[0].data["requireApiKey"], true);

        storage
            .record_usage(&UsageEntry {
                id: "ignored".to_string(),
                timestamp: blackrouter_common::unix_timestamp().to_string(),
                provider: "openai".to_string(),
                model: "gpt-5".to_string(),
                connection_id: None,
                api_key: Some("key-id".to_string()),
                endpoint: "/v1/chat/completions".to_string(),
                prompt_tokens: 10,
                completion_tokens: 5,
                cost: 0.25,
                status: "success".to_string(),
                tokens: None,
                meta: None,
            })
            .unwrap();
        let usage = storage.api_key_usage_since("key-id", 0).unwrap();
        assert_eq!(usage.requests, 1);
        assert_eq!(usage.tokens, 15);
        assert_eq!(usage.cost, 0.25);
        let period = blackrouter_common::unix_timestamp() / 86_400 * 86_400;
        assert!(storage
            .reserve_api_key_request("key-id", period, 2)
            .unwrap());
        assert!(storage
            .reserve_api_key_request("key-id", period, 2)
            .unwrap());
        assert!(!storage
            .reserve_api_key_request("key-id", period, 2)
            .unwrap());
        let _ = fs::remove_file(path);
    }

    #[test]
    fn creates_combo_for_active_provider_models() {
        let (storage, path) = temp_storage("combo-create");
        storage
            .create_provider_connection(active_provider("cline"))
            .expect("provider creates");

        let combo = storage
            .create_combo(NewCombo {
                name: "code-high".to_string(),
                kind: Some("llm".to_string()),
                models: vec!["cline/claude-sonnet-4".to_string()],
            })
            .expect("combo creates");

        assert_eq!(combo.name, "code-high");
        assert_eq!(combo.kind, "llm");
        assert_eq!(combo.models, vec!["cline/claude-sonnet-4"]);
        assert_eq!(
            storage.list_model_shell_items().unwrap()[0].owned_by,
            "combo"
        );

        let _ = fs::remove_file(path);
    }

    #[test]
    fn rejects_invalid_combo_payloads() {
        let (storage, path) = temp_storage("combo-invalid");
        storage
            .create_provider_connection(active_provider("cline"))
            .expect("provider creates");

        let empty_name = storage.create_combo(NewCombo {
            name: " ".to_string(),
            kind: None,
            models: vec!["cline/model-a".to_string()],
        });
        assert!(empty_name.is_err());

        let slash_name = storage.create_combo(NewCombo {
            name: "cline/model-a".to_string(),
            kind: None,
            models: vec!["cline/model-a".to_string()],
        });
        assert!(slash_name.is_err());

        let empty_models = storage.create_combo(NewCombo {
            name: "empty-models".to_string(),
            kind: None,
            models: vec![],
        });
        assert!(empty_models.is_err());

        let bad_model = storage.create_combo(NewCombo {
            name: "bad-model".to_string(),
            kind: None,
            models: vec!["model-without-provider".to_string()],
        });
        assert!(bad_model.is_err());

        let _ = fs::remove_file(path);
    }

    #[test]
    fn rejects_missing_or_disabled_provider_models() {
        let (storage, path) = temp_storage("combo-provider");

        let missing = storage.create_combo(NewCombo {
            name: "missing-provider".to_string(),
            kind: None,
            models: vec!["missing/model-a".to_string()],
        });
        assert!(missing.is_err());

        let mut disabled = active_provider("cline");
        disabled.is_active = Some(false);
        storage
            .create_provider_connection(disabled)
            .expect("provider creates");

        let disabled = storage.create_combo(NewCombo {
            name: "disabled-provider".to_string(),
            kind: None,
            models: vec!["cline/model-a".to_string()],
        });
        assert!(disabled.is_err());

        let _ = fs::remove_file(path);
    }

    #[test]
    fn provider_cooldown_keeps_active_routes_but_expiry_blocks_them() {
        let (storage, path) = temp_storage("provider-runtime-status");
        let future = (blackrouter_common::unix_timestamp() + 60).to_string();
        let mut provider = active_provider("cline");
        provider.status = Some("cooldown".to_string());
        provider.cooldown_until = Some(future);
        storage
            .create_provider_connection(provider)
            .expect("provider creates");

        assert!(storage.get_active_provider_connection_raw("cline").is_ok());
        assert!(storage.resolve_model_route("cline/model-a").is_ok());

        let record = storage.list_provider_connections().unwrap().remove(0);
        storage
            .set_provider_runtime_status(&record.id, "healthy", None, None)
            .expect("runtime status updates");
        assert!(storage.get_active_provider_connection_raw("cline").is_ok());

        let past = (blackrouter_common::unix_timestamp().saturating_sub(1)).to_string();
        storage
            .set_provider_runtime_status(&record.id, "expired", None, Some(past))
            .expect("expiry updates");
        assert!(storage.get_active_provider_connection_raw("cline").is_err());

        let _ = fs::remove_file(path);
    }

    #[test]
    fn cached_provider_models_respects_ttl() {
        let (storage, path) = temp_storage("provider-model-cache");
        let fetched_at = (blackrouter_common::unix_timestamp().saturating_sub(100)).to_string();
        let mut provider = active_provider("cline");
        provider.data = Some(json!({
            "baseUrl": "http://127.0.0.1:20130/health",
            "format": "openai",
            "models": ["cline/model-a", "cline/model-b"],
            "modelsUrl": "https://example.test/models",
            "modelsFetchedAt": fetched_at
        }));
        let record = storage
            .create_provider_connection(provider)
            .expect("provider creates");

        let cached = storage
            .cached_provider_models(&record.id, 200)
            .expect("cache lookup")
            .expect("fresh cache exists");
        assert_eq!(
            cached.models,
            vec!["cline/model-a".to_string(), "cline/model-b".to_string()]
        );
        assert_eq!(
            cached.models_url.as_deref(),
            Some("https://example.test/models")
        );

        assert!(storage
            .cached_provider_models(&record.id, 10)
            .expect("cache lookup")
            .is_none());

        let _ = fs::remove_file(path);
    }

    #[test]
    fn stores_provider_rate_limit_snapshot_without_dropping_data() {
        let (storage, path) = temp_storage("provider-rate-limit");
        let mut provider = active_provider("openai");
        provider.data = Some(json!({
            "baseUrl": "https://api.openai.com/v1/chat/completions",
            "format": "openai",
            "models": ["gpt-5.5"]
        }));
        let record = storage
            .create_provider_connection(provider)
            .expect("provider creates");

        storage
            .set_provider_rate_limit_snapshot(
                &record.id,
                json!({
                    "model": "gpt-5.5",
                    "headers": {
                        "x-ratelimit-remaining-requests": "59"
                    }
                }),
            )
            .expect("snapshot stores");

        let provider = storage
            .get_provider_connection_raw(&record.id)
            .expect("provider loads");
        assert_eq!(provider.data["models"][0], "gpt-5.5");
        assert_eq!(
            provider.data["rateLimit"]["headers"]["x-ratelimit-remaining-requests"],
            "59"
        );

        let _ = fs::remove_file(path);
    }

    #[test]
    fn updates_provider_oauth_access_token_without_dropping_refresh_metadata() {
        let (storage, path) = temp_storage("provider-oauth-token");
        let mut provider = active_provider("antigravity");
        provider.auth_type = "oauth".to_string();
        provider.data = Some(json!({
            "baseUrl": "https://cloudcode-pa.googleapis.com",
            "format": "antigravity",
            "apiKey": "old-access",
            "refreshToken": "refresh-token",
            "projectId": "blackrouter",
            "models": ["gemini-3-flash-agent"]
        }));
        let record = storage
            .create_provider_connection(provider)
            .expect("provider creates");

        storage
            .set_provider_oauth_access_token(&record.id, "new-access", Some("12345".to_string()))
            .expect("access token updates");

        let provider = storage
            .get_provider_connection_raw(&record.id)
            .expect("provider loads");
        assert_eq!(provider.data["apiKey"], "new-access");
        assert_eq!(provider.data["accessToken"], "new-access");
        assert_eq!(provider.data["refreshToken"], "refresh-token");
        assert_eq!(provider.data["projectId"], "blackrouter");
        assert_eq!(provider.data["tokenExpiresAt"], "12345");

        let _ = fs::remove_file(path);
    }

    #[test]
    fn updates_combo_and_enforces_unique_names() {
        let (storage, path) = temp_storage("combo-update");
        storage
            .create_provider_connection(active_provider("cline"))
            .expect("provider creates");
        storage
            .create_provider_connection(active_provider("commandcode"))
            .expect("provider creates");

        let first = storage
            .create_combo(NewCombo {
                name: "code-high".to_string(),
                kind: None,
                models: vec!["cline/model-a".to_string()],
            })
            .expect("first combo creates");
        let _second = storage
            .create_combo(NewCombo {
                name: "code-backup".to_string(),
                kind: None,
                models: vec!["cline/model-a".to_string()],
            })
            .expect("second combo creates");

        let updated = storage
            .update_combo(
                &first.id,
                NewCombo {
                    name: "code-high".to_string(),
                    kind: Some("llm".to_string()),
                    models: vec![
                        "commandcode/model-b".to_string(),
                        "cline/model-a".to_string(),
                    ],
                },
            )
            .expect("combo updates");
        assert_eq!(
            updated.models,
            vec![
                "commandcode/model-b".to_string(),
                "cline/model-a".to_string()
            ]
        );

        let duplicate = storage.update_combo(
            &first.id,
            NewCombo {
                name: "code-backup".to_string(),
                kind: None,
                models: vec!["cline/model-a".to_string()],
            },
        );
        assert!(duplicate.is_err());

        let _ = fs::remove_file(path);
    }

    #[test]
    fn deletes_combo_by_id() {
        let (storage, path) = temp_storage("combo-delete");
        storage
            .create_provider_connection(active_provider("cline"))
            .expect("provider creates");
        let combo = storage
            .create_combo(NewCombo {
                name: "code-high".to_string(),
                kind: None,
                models: vec!["cline/model-a".to_string()],
            })
            .expect("combo creates");

        storage.delete_combo(&combo.id).expect("combo deletes");
        assert!(storage.get_combo(&combo.id).is_err());

        let _ = fs::remove_file(path);
    }

    #[test]
    fn resolves_single_and_combo_routes() {
        let (storage, path) = temp_storage("combo-resolve");
        storage
            .create_provider_connection(active_provider("cline"))
            .expect("provider creates");
        storage
            .create_provider_connection(active_provider("commandcode"))
            .expect("provider creates");
        storage
            .create_combo(NewCombo {
                name: "code-high".to_string(),
                kind: None,
                models: vec![
                    "cline/model-a".to_string(),
                    "commandcode/model-b".to_string(),
                ],
            })
            .expect("combo creates");

        assert_eq!(
            storage.resolve_model_route("cline/model-a").unwrap(),
            RouteKind::Single(ModelRef {
                provider: "cline".to_string(),
                model: "model-a".to_string()
            })
        );
        assert_eq!(
            storage.resolve_model_route("code-high").unwrap(),
            RouteKind::Combo {
                name: "code-high".to_string(),
                models: vec![
                    ModelRef {
                        provider: "cline".to_string(),
                        model: "model-a".to_string()
                    },
                    ModelRef {
                        provider: "commandcode".to_string(),
                        model: "model-b".to_string()
                    },
                ]
            }
        );
        assert!(storage.resolve_model_route("unknown-model").is_err());

        let _ = fs::remove_file(path);
    }
}
