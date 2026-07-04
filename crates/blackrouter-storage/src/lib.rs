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
];

const BLACKROUTER_TABLES: &[&str] = &["adminAuditLog", "telegramLinks", "runtimeEvents"];

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

#[derive(Clone, Debug, Serialize)]
pub struct ApiKeyRecord {
    pub id: String,
    pub key_masked: String,
    pub name: Option<String>,
    pub machine_id: Option<String>,
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
    pub data: Value,
    pub created_at: String,
    pub updated_at: String,
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
        conn.execute(
            r#"
            INSERT INTO settings (id, data)
            VALUES (1, ?1)
            ON CONFLICT(id) DO UPDATE SET data = excluded.data
            "#,
            params![raw],
        )?;
        Ok(settings.clone())
    }

    pub fn list_api_keys(&self) -> Result<Vec<ApiKeyRecord>> {
        let conn = self.open()?;
        let mut stmt = conn.prepare(
            r#"
            SELECT id, key, name, machineId, isActive, createdAt
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
                is_active: row.get::<_, i64>(4)? != 0,
                created_at: row.get(5)?,
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

        conn.execute(
            r#"
            INSERT INTO apiKeys (id, key, name, machineId, isActive, createdAt)
            VALUES (?1, ?2, ?3, ?4, 1, ?5)
            "#,
            params![id, key, name, machine_id, created_at],
        )?;

        Ok(CreatedApiKey {
            record: ApiKeyRecord {
                id,
                key_masked: mask_secret(&key),
                name,
                machine_id,
                is_active: true,
                created_at,
            },
            key,
        })
    }

    pub fn list_provider_connections(&self) -> Result<Vec<ProviderConnectionRecord>> {
        let conn = self.open()?;
        let mut stmt = conn.prepare(
            r#"
            SELECT id, provider, authType, name, email, priority, isActive, data, createdAt, updatedAt
            FROM providerConnections
            ORDER BY provider ASC, COALESCE(priority, 999999) ASC, createdAt DESC
            "#,
        )?;

        let rows = stmt.query_map([], |row| {
            let raw_data: String = row.get(7)?;
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
                data,
                created_at: row.get(8)?,
                updated_at: row.get(9)?,
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
        let data = input.data.unwrap_or(Value::Object(Default::default()));
        let raw_data = serde_json::to_string_pretty(&data)?;

        conn.execute(
            r#"
            INSERT INTO providerConnections
              (id, provider, authType, name, email, priority, isActive, data, createdAt, updatedAt)
            VALUES
              (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)
            "#,
            params![
                id,
                provider,
                auth_type,
                name,
                email,
                priority,
                if is_active { 1 } else { 0 },
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
            data: mask_sensitive_json(data),
            created_at: now.clone(),
            updated_at: now,
        })
    }

    pub fn get_provider_connection_raw(&self, id: &str) -> Result<RawProviderConnection> {
        let conn = self.open()?;
        conn.query_row(
            r#"
            SELECT id, provider, authType, name, email, priority, isActive, data, createdAt, updatedAt
            FROM providerConnections
            WHERE id = ?1
            "#,
            params![id],
            |row| {
                let raw_data: String = row.get(7)?;
                let data = serde_json::from_str(&raw_data).unwrap_or(Value::Object(Default::default()));
                Ok(RawProviderConnection {
                    id: row.get(0)?,
                    provider: row.get(1)?,
                    auth_type: row.get(2)?,
                    name: row.get(3)?,
                    email: row.get(4)?,
                    priority: row.get(5)?,
                    is_active: row.get::<_, i64>(6)? != 0,
                    data,
                    created_at: row.get(8)?,
                    updated_at: row.get(9)?,
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
            SELECT id, provider, authType, name, email, priority, isActive, data, createdAt, updatedAt
            FROM providerConnections
            WHERE provider = ?1 AND isActive = 1
            ORDER BY COALESCE(priority, 999999) ASC, createdAt DESC
            LIMIT 1
            "#,
            params![provider],
            |row| {
                let raw_data: String = row.get(7)?;
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
                    data,
                    created_at: row.get(8)?,
                    updated_at: row.get(9)?,
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
                data = ?8,
                updatedAt = ?9
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
            data: Some(Value::Object(data)),
        };
        self.update_provider_connection(id, input)
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

fn ensure_active_provider(conn: &Connection, provider: &str) -> Result<()> {
    let found = conn
        .query_row(
            "SELECT 1 FROM providerConnections WHERE provider = ?1 AND isActive = 1 LIMIT 1",
            params![provider],
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
"#;

const BLACKROUTER_SCHEMA_SQL: &str = r#"
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
"#;

#[cfg(test)]
mod tests {
    use super::{NewCombo, NewProviderConnection, Storage};
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
