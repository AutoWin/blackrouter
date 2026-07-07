const state = {
  health: null,
  version: null,
  runtime: null,
  models: null,
  setupConfig: null,
  apiKeys: null,
  providers: null,
  providerCatalog: null,
  combos: null,
};

const ANTIGRAVITY_MODELS = [
  "gemini-3-flash-agent",
  "gemini-3.5-flash-low",
  "gemini-3.5-flash-extra-low",
  "gemini-pro-agent",
  "gemini-3.1-pro-low",
  "claude-sonnet-4-6",
  "claude-opus-4-6-thinking",
  "gpt-oss-120b-medium",
  "gemini-3-flash",
  "gemini-2.0-flash-lite",
  "gemini-2.0-flash",
  "gemini-2.5-flash",
  "gemini-1.5-flash",
  "gemini-1.5-pro",
  "gemini-2.5-pro",
  "gemini-3-flash-preview",
  "gemini-3-pro-preview",
];

const $ = (id) => document.getElementById(id);

function setValue(id, value) {
  const node = $(id);
  if (node) node.value = value ?? "";
}

function setText(id, value) {
  const node = $(id);
  if (node) node.textContent = value ?? "";
}

function setBadge(id, label, tone) {
  const node = $(id);
  if (!node) return;
  node.textContent = label;
  node.className = `badge ${tone}`;
}

function formatBool(value) {
  return value ? "On" : "Off";
}

function formatSeconds(seconds) {
  const value = Number(seconds || 0);
  if (value < 60) return `${value}s`;
  if (value < 3600) return `${Math.floor(value / 60)}m ${value % 60}s`;
  return `${Math.floor(value / 3600)}h ${Math.floor((value % 3600) / 60)}m`;
}

async function getJson(path) {
  const response = await fetch(path, { cache: "no-store" });
  if (!response.ok) throw new Error(`${path} ${response.status}`);
  return response.json();
}

async function sendJson(path, method, body) {
  const options = {
    method,
    headers: { "content-type": "application/json" },
  };
  if (body !== undefined) options.body = JSON.stringify(body);
  const response = await fetch(path, options);
  const payload = await response.json().catch(() => ({}));
  if (!response.ok) {
    const message = payload?.error?.message || `${path} ${response.status}`;
    throw new Error(message);
  }
  return payload;
}

async function refresh() {
  setBadge("healthBadge", "Checking", "muted");

  const [
    health,
    version,
    runtime,
    models,
    setupConfig,
    apiKeys,
    providers,
    providerCatalog,
    combos,
  ] = await Promise.all([
    getJson("/health"),
    getJson("/version"),
    getJson("/api/runtime/status"),
    getJson("/v1/models"),
    getJson("/api/setup/config"),
    getJson("/api/setup/api-keys"),
    getJson("/api/setup/providers"),
    getJson("/api/setup/provider-catalog"),
    getJson("/api/setup/combos"),
  ]);

  state.health = health;
  state.version = version;
  state.runtime = runtime;
  state.models = models;
  state.setupConfig = setupConfig;
  state.apiKeys = apiKeys;
  state.providers = providers;
  state.providerCatalog = providerCatalog;
  state.combos = combos;

  render();
}

function render() {
  const runtime = state.runtime || {};
  const config = runtime.config || {};
  const storage = runtime.storage || {};
  const telegram = config.telegram || {};
  const savedSettings = state.setupConfig?.settings || {};
  const savedTelegram = savedSettings.telegram || {};
  const models = Array.isArray(state.models?.data) ? state.models.data : [];
  const apiKeys = Array.isArray(state.apiKeys?.data) ? state.apiKeys.data : [];
  const providers = Array.isArray(state.providers?.data)
    ? state.providers.data
    : [];
  const catalog = Array.isArray(state.providerCatalog)
    ? state.providerCatalog
    : [];
  const combos = Array.isArray(state.combos?.data) ? state.combos.data : [];
  const tableCounts = storage.table_counts || {};

  setText("versionLabel", state.version?.version || "0.1.0");
  setText(
    "endpointLabel",
    `${config.host || "127.0.0.1"}:${config.port || ""}`,
  );
  setText("uptimeLabel", formatSeconds(runtime.uptime_seconds));

  setBadge(
    "healthBadge",
    state.health?.status === "ok" ? "Online" : "Degraded",
    state.health?.status === "ok" ? "ok" : "warn",
  );

  setValue("hostValue", config.host);
  setValue("portValue", config.port);
  setValue("dataDirValue", config.data_dir);
  setValue("databaseUrlValue", config.database_url);
  setValue(
    "databasePathValue",
    storage.database_path || state.health?.database?.path,
  );
  setValue("compatValue", formatBool(config.compat_9router_db));

  setBadge(
    "databaseBadge",
    storage.schema_compatible ? "Compatible" : "Missing Tables",
    storage.schema_compatible ? "ok" : "warn",
  );

  const savedRequireApiKey = Boolean(savedSettings.requireApiKey);
  $("configRequireApiKey").checked = savedRequireApiKey;
  setValue("requireApiKeyValue", formatBool(savedRequireApiKey));
  setValue("apiKeyCountValue", apiKeys.length);
  setBadge(
    "apiKeyBadge",
    savedRequireApiKey ? "Required" : "Local Mode",
    savedRequireApiKey ? "ok" : "warn",
  );

  $("telegramEnabledInput").checked = Boolean(
    savedTelegram.enabled ?? telegram.enabled,
  );
  $("telegramAdminInput").value = Array.isArray(savedTelegram.adminIds)
    ? savedTelegram.adminIds.join(",")
    : "";
  $("telegramWebhookInput").checked = Boolean(
    savedTelegram.useWebhook ?? telegram.use_webhook,
  );
  $("telegramTtlInput").value =
    savedTelegram.linkCodeTtlSeconds || telegram.link_code_ttl_seconds || 300;
  $("telegramWebhookUrlInput").value = savedTelegram.webhookUrl || "";
  setBadge(
    "telegramBadge",
    $("telegramEnabledInput").checked ? "Enabled" : "Disabled",
    $("telegramEnabledInput").checked ? "ok" : "muted",
  );

  setBadge(
    "modelsBadge",
    `${models.length} models`,
    models.length ? "ok" : "muted",
  );
  setBadge(
    "combosBadge",
    `${combos.length} combos`,
    combos.length ? "ok" : "muted",
  );

  renderProviderCatalog(catalog);
  renderRows(
    "tableCounts",
    Object.entries(tableCounts).map(([name, count]) => ({
      name,
      value: count,
    })),
    "No tables",
  );
  renderRows(
    "apiKeysList",
    apiKeys.map((key) => ({ name: key.name || key.id, value: key.key_masked })),
    "No API keys",
  );
  renderProviders(providers);
  renderRows(
    "modelsList",
    models.map((model) => ({
      name: model.id,
      value: model.owned_by || "blackrouter",
    })),
    "No models",
  );
  renderComboModelOptions(providers);
  renderCombos(combos);
  renderRows(
    "comboModelsList",
    models.map((model) => ({
      name: model.id,
      value: model.owned_by || "blackrouter",
    })),
    "No models",
  );
}

function renderProviderCatalog(catalog) {
  const select = $("providerPresetInput");
  if (!select || select.dataset.rendered === "true") return;

  select.innerHTML = [
    `<option value="">Custom provider</option>`,
    ...catalog.map(providerOption),
  ].join("");
  select.dataset.rendered = "true";
}

function providerOption(item) {
  return `<option value="${escapeHtml(item.id)}">${escapeHtml(item.name)} (${escapeHtml(item.alias)})</option>`;
}

function renderRows(id, rows, emptyLabel) {
  const root = $(id);
  if (!root) return;

  if (!rows.length) {
    root.innerHTML = `<div class="empty">${escapeHtml(emptyLabel)}</div>`;
    return;
  }

  root.innerHTML = rows
    .map(
      (row) => `
        <div class="row">
          <strong title="${escapeHtml(row.name)}">${escapeHtml(row.name)}</strong>
          <span>${escapeHtml(String(row.value))}</span>
        </div>
      `,
    )
    .join("");
}

function renderProviders(providers) {
  const root = $("providersList");
  if (!root) return;

  if (!providers.length) {
    root.innerHTML = `<div class="empty">No providers</div>`;
    return;
  }

  root.innerHTML = providers
    .map((provider) => {
      const title = `${provider.provider}/${provider.name || provider.id}`;
      const status = provider.is_active ? "Active" : "Disabled";
      return `
        <div class="row provider-row" data-provider-id="${escapeHtml(provider.id)}">
          <div>
            <strong title="${escapeHtml(title)}">${escapeHtml(title)}</strong>
            <div class="row-meta">
              <span>${escapeHtml(provider.auth_type)}</span>
              <span>${escapeHtml(provider.data?.format || "")}</span>
              <span>${escapeHtml(status)}</span>
              <span>${providerModelIds(provider).length} models</span>
            </div>
          </div>
          <div class="row-actions">
            <button class="row-button" type="button" data-action="edit" data-id="${escapeHtml(provider.id)}">Edit</button>
            <button class="row-button" type="button" data-action="test" data-id="${escapeHtml(provider.id)}">Check</button>
            <button class="row-button" type="button" data-action="models" data-id="${escapeHtml(provider.id)}">Models</button>
            <button class="row-button" type="button" data-action="fetch-models" data-id="${escapeHtml(provider.id)}">Fetch</button>
            <button class="row-button" type="button" data-action="toggle" data-active="${provider.is_active ? "false" : "true"}" data-id="${escapeHtml(provider.id)}">${provider.is_active ? "Disable" : "Enable"}</button>
            <button class="danger-button row-button" type="button" data-action="delete" data-id="${escapeHtml(provider.id)}">Delete</button>
          </div>
        </div>
      `;
    })
    .join("");
}

function renderComboModelOptions(providers) {
  const select = $("comboModelSelect");
  if (!select) return;

  const options = [];
  providers.forEach((provider) => {
    providerModelIds(provider).forEach((model) => {
      options.push({
        value: `${provider.provider}/${model}`,
        label: `${provider.provider}/${model}`,
      });
    });
  });

  select.innerHTML = [
    `<option value="">Select fetched provider model</option>`,
    ...options
      .sort((a, b) => a.label.localeCompare(b.label))
      .map(
        (option) =>
          `<option value="${escapeHtml(option.value)}">${escapeHtml(option.label)}</option>`,
      ),
  ].join("");
}

function providerModelIds(provider) {
  const models = provider?.data?.models;
  if (!Array.isArray(models) || models.length === 0) {
    const providerId = String(provider?.provider || "").toLowerCase();
    const alias = String(provider?.data?.alias || "").toLowerCase();
    const format = String(provider?.data?.format || "").toLowerCase();
    if (providerId === "antigravity" || alias === "ag" || format === "antigravity") {
      return [...ANTIGRAVITY_MODELS];
    }
    return [];
  }
  return models
    .map((model) => {
      if (typeof model === "string") return model.trim();
      if (model && typeof model === "object")
        return String(model.id || model.name || model.model || "").trim();
      return "";
    })
    .filter(Boolean);
}

function renderCombos(combos) {
  const root = $("combosList");
  if (!root) return;

  if (!combos.length) {
    root.innerHTML = `<div class="empty">No combos</div>`;
    return;
  }

  root.innerHTML = combos
    .map((combo) => {
      const models = Array.isArray(combo.models) ? combo.models : [];
      return `
        <div class="row combo-row" data-combo-id="${escapeHtml(combo.id)}">
          <div>
            <strong title="${escapeHtml(combo.name)}">${escapeHtml(combo.name)}</strong>
            <div class="row-meta">
              <span>${escapeHtml(combo.kind || "llm")}</span>
              <span>${models.length} fallback models</span>
            </div>
            <div class="model-stack">
              ${models.map((model, index) => `<span>${index + 1}. ${escapeHtml(model)}</span>`).join("")}
            </div>
          </div>
          <div class="row-actions">
            <button class="row-button" type="button" data-action="edit" data-id="${escapeHtml(combo.id)}">Edit</button>
            <button class="danger-button row-button" type="button" data-action="delete" data-id="${escapeHtml(combo.id)}">Delete</button>
          </div>
        </div>
      `;
    })
    .join("");
}

function escapeHtml(value) {
  return String(value)
    .replaceAll("&", "&amp;")
    .replaceAll("<", "&lt;")
    .replaceAll(">", "&gt;")
    .replaceAll('"', "&quot;")
    .replaceAll("'", "&#039;");
}

function bindTabs() {
  document.querySelectorAll(".tab").forEach((tab) => {
    tab.addEventListener("click", () => {
      const panelId = tab.dataset.panel;
      document
        .querySelectorAll(".tab")
        .forEach((item) => item.classList.toggle("active", item === tab));
      document
        .querySelectorAll(".panel")
        .forEach((panel) =>
          panel.classList.toggle("active", panel.id === panelId),
        );
    });
  });
}

bindTabs();
$("refreshButton").addEventListener("click", () => {
  refresh().catch((error) => {
    setBadge("healthBadge", "Offline", "error");
    console.error(error);
  });
});

$("configForm").addEventListener("submit", async (event) => {
  event.preventDefault();
  await saveConfig();
});

$("saveTelegramButton").addEventListener("click", async () => {
  await saveConfig();
});

$("apiKeyForm").addEventListener("submit", async (event) => {
  event.preventDefault();
  const created = await sendJson("/api/setup/api-keys", "POST", {
    name: $("apiKeyNameInput").value,
    machine_id: $("apiKeyMachineInput").value,
  });
  $("apiKeyNameInput").value = "";
  $("apiKeyMachineInput").value = "";
  showNewApiKey(created.key);
  await refresh();
});

$("providerForm").addEventListener("submit", async (event) => {
  event.preventDefault();
  let advancedData = {};
  const rawData = $("providerDataInput").value.trim();
  if (rawData) advancedData = JSON.parse(rawData);

  const authType = $("authTypeInput").value;
  const data = {
    ...advancedData,
    baseUrl: $("providerBaseUrlInput").value.trim() || advancedData.baseUrl,
    format: $("providerFormatInput").value.trim() || advancedData.format,
  };

  if (authType === "api-key" || authType === "bearer" || authType === "oauth") {
    const apiKey = $("providerApiKeyInput").value.trim();
    if (apiKey) data.apiKey = apiKey;
  } else if (authType === "basic") {
    const username = $("providerBasicUserInput").value.trim();
    const password = $("providerBasicPassInput").value;
    if (username) data.username = username;
    if (password) data.password = password;
  } else if (authType === "header") {
    const headerName = $("providerHeaderNameInput").value.trim();
    const headerValue = $("providerHeaderValueInput").value.trim();
    if (headerName) data.headerName = headerName;
    if (headerValue) data.headerValue = headerValue;
  }

  const editId = $("providerEditIdInput").value;
  const method = editId ? "PUT" : "POST";
  const path = editId
    ? `/api/setup/providers/${encodeURIComponent(editId)}`
    : "/api/setup/providers";

  await sendJson(path, method, {
    provider: $("providerInput").value,
    auth_type: $("authTypeInput").value,
    name: $("providerNameInput").value,
    priority: numberOrNull($("providerPriorityInput").value),
    is_active: $("providerActiveInput").checked,
    data,
  });

  resetProviderForm();
  await refresh();
});

$("providerCancelEditButton").addEventListener("click", resetProviderForm);

$("providerInput").addEventListener("input", () => {
  showOauthButton($("providerInput").value);
});

$("comboForm").addEventListener("submit", async (event) => {
  event.preventDefault();
  let models = $("comboModelsInput")
    .value.split(/\r?\n/)
    .map((item) => item.trim())
    .filter(Boolean);
  const selectedModel = $("comboModelSelect").value;
  if (!models.length && selectedModel) models = [selectedModel];
  if (!models.length) {
    $("comboModelSelect").focus();
    alert("Select a fetched provider model or enter at least one model.");
    return;
  }
  const editId = $("comboEditIdInput").value;
  const method = editId ? "PUT" : "POST";
  const path = editId
    ? `/api/setup/combos/${encodeURIComponent(editId)}`
    : "/api/setup/combos";

  await sendJson(path, method, {
    name: $("comboNameInput").value,
    kind: $("comboKindInput").value || "llm",
    models,
  });

  resetComboForm();
  await refresh();
});

$("comboCancelEditButton").addEventListener("click", resetComboForm);

$("providersList").addEventListener("click", async (event) => {
  const button = event.target.closest("button[data-action]");
  if (!button) return;

  const id = button.dataset.id;
  const action = button.dataset.action;
  if (action === "edit") {
    editProvider(id);
    return;
  }

  if (action === "test") {
    await checkProvider(id);
    return;
  }

  if (action === "models") {
    showProviderModels(id);
    return;
  }

  if (action === "fetch-models") {
    await fetchProviderModels(id);
    return;
  }

  if (action === "toggle") {
    await sendJson(
      `/api/setup/providers/${encodeURIComponent(id)}/toggle`,
      "POST",
      {
        is_active: button.dataset.active === "true",
      },
    );
    await refresh();
    return;
  }

  if (action === "delete") {
    if (!confirm("Delete this provider connection?")) return;
    await sendJson(`/api/setup/providers/${encodeURIComponent(id)}`, "DELETE");
    await refresh();
  }
});

$("comboAddModelButton").addEventListener("click", () => {
  const selected = $("comboModelSelect").value;
  if (!selected) return;
  addComboModel(selected);
});

$("combosList").addEventListener("click", async (event) => {
  const button = event.target.closest("button[data-action]");
  if (!button) return;

  const id = button.dataset.id;
  const action = button.dataset.action;
  if (action === "edit") {
    editCombo(id);
    return;
  }

  if (action === "delete") {
    if (!confirm("Delete this combo?")) return;
    await sendJson(`/api/setup/combos/${encodeURIComponent(id)}`, "DELETE");
    await refresh();
  }
});

$("providerPresetInput").addEventListener("change", () => {
  const catalog = Array.isArray(state.providerCatalog)
    ? state.providerCatalog
    : [];
  const preset = catalog.find(
    (item) => item.id === $("providerPresetInput").value,
  );
  if (!preset) return;

  $("providerInput").value = preset.id;
  $("authTypeInput").value = preset.auth_type;
  showOauthButton(preset.id);
  updateAuthFields();
  $("providerBaseUrlInput").value = preset.base_url;
  $("providerFormatInput").value = preset.format;
  $("providerNameInput").value = preset.name;
  $("providerApiKeyInput").placeholder =
    preset.api_key_hint || "provider API key or access token";
  $("providerDataInput").value = JSON.stringify(
    defaultProviderData(preset),
    null,
    2,
  );
});

function defaultProviderData(preset) {
  const data = {
    baseUrl: preset.base_url,
    format: preset.format,
    alias: preset.alias,
    website: preset.website,
    models: [],
  };

  if (preset.id === "commandcode") {
    data.headers = {
      "x-command-code-version": "0.25.7",
      "x-cli-environment": "cli",
    };
  }

  if (preset.id === "cline") {
    data.headers = {
      "HTTP-Referer": "https://cline.bot",
      "X-Title": "Cline",
    };
    data.tokenUrl = "https://api.cline.bot/api/v1/auth/token";
    data.refreshUrl = "https://api.cline.bot/api/v1/auth/refresh";
  }

  if (preset.id === "antigravity") {
    data.models = [...ANTIGRAVITY_MODELS];
  }

  return data;
}

function editProvider(id) {
  const providers = Array.isArray(state.providers?.data)
    ? state.providers.data
    : [];
  const provider = providers.find((item) => item.id === id);
  if (!provider) return;

  $("providerEditIdInput").value = provider.id;
  $("providerPresetInput").value = provider.provider;
  $("providerInput").value = provider.provider;
  $("authTypeInput").value = provider.auth_type;
  updateAuthFields();
  $("providerBaseUrlInput").value = provider.data?.baseUrl || "";
  $("providerFormatInput").value = provider.data?.format || "";
  $("providerNameInput").value = provider.name || "";
  $("providerPriorityInput").value = provider.priority ?? "";
  $("providerApiKeyInput").value = "";
  $("providerBasicUserInput").value = provider.data?.username || "";
  $("providerBasicPassInput").value = "";
  $("providerHeaderNameInput").value = provider.data?.headerName || "";
  $("providerHeaderValueInput").value = "";
  $("providerActiveInput").checked = Boolean(provider.is_active);
  $("providerDataInput").value = JSON.stringify(provider.data || {}, null, 2);
  $("providerSubmitButton").textContent = "Save Provider";
  $("providerCancelEditButton").classList.remove("hidden");
  showOauthButton(provider.provider);
  setBadge("modelsBadge", "Editing provider", "warn");
}

function resetProviderForm() {
  $("providerEditIdInput").value = "";
  $("providerInput").value = "";
  $("authTypeInput").value = "api-key";
  $("providerBaseUrlInput").value = "";
  $("providerFormatInput").value = "";
  $("providerNameInput").value = "";
  $("providerPriorityInput").value = "";
  $("providerApiKeyInput").value = "";
  $("providerBasicUserInput").value = "";
  $("providerBasicPassInput").value = "";
  $("providerHeaderNameInput").value = "";
  $("providerHeaderValueInput").value = "";
  $("providerDataInput").value = "";
  $("providerActiveInput").checked = true;
  $("providerPresetInput").value = "";
  $("providerSubmitButton").textContent = "Add Provider";
  $("providerCancelEditButton").classList.add("hidden");
  showOauthButton("");
  updateAuthFields();
}

function updateAuthFields() {
  const authType = $("authTypeInput").value;
  const showApiKey =
    authType === "api-key" || authType === "bearer" || authType === "oauth";
  const showBasic = authType === "basic";
  const showHeader = authType === "header";

  $("apiKeyField").style.display = showApiKey ? "" : "none";
  $("basicUserField").style.display = showBasic ? "" : "none";
  $("basicPassField").style.display = showBasic ? "" : "none";
  $("headerNameField").style.display = showHeader ? "" : "none";
  $("headerValueField").style.display = showHeader ? "" : "none";
}

$("authTypeInput").addEventListener("change", updateAuthFields);

function editCombo(id) {
  const combos = Array.isArray(state.combos?.data) ? state.combos.data : [];
  const combo = combos.find((item) => item.id === id);
  if (!combo) return;

  $("comboEditIdInput").value = combo.id;
  $("comboNameInput").value = combo.name || "";
  $("comboKindInput").value = combo.kind || "llm";
  $("comboModelsInput").value = Array.isArray(combo.models)
    ? combo.models.join("\n")
    : "";
  $("comboSubmitButton").textContent = "Save Combo";
  $("comboCancelEditButton").classList.remove("hidden");
  setBadge("combosBadge", "Editing combo", "warn");
}

function resetComboForm() {
  $("comboEditIdInput").value = "";
  $("comboNameInput").value = "";
  $("comboKindInput").value = "llm";
  $("comboModelsInput").value = "";
  $("comboSubmitButton").textContent = "Add Combo";
  $("comboCancelEditButton").classList.add("hidden");
}

async function checkProvider(id) {
  const notice = $("providerTestNotice");
  notice.textContent = "Checking provider connection...";
  notice.classList.remove("hidden");
  const result = await sendJson(
    `/api/setup/providers/${encodeURIComponent(id)}/test`,
    "POST",
    {},
  );
  notice.textContent = `${result.ok ? "OK" : "Check failed"}: ${result.message}${result.status ? ` (HTTP ${result.status})` : ""}`;
  notice.classList.toggle("error", !result.ok);
}

async function startOAuth(providerId) {
  const notice = $("oauthNotice");
  notice.textContent = "Starting OAuth flow...";
  notice.classList.remove("hidden");
  notice.classList.remove("error");

  try {
    const resp = await fetch(
      `/api/oauth/${encodeURIComponent(providerId)}/start`,
      { method: "POST" },
    );
    const result = await resp.json();

    if (!resp.ok) {
      notice.textContent = "❌ " + (result.error || "Failed to start OAuth");
      notice.classList.add("error");
      return;
    }

    if (result.flow_type === "device_code") {
      // Show device code UI
      notice.innerHTML = [
        "<strong>Login with " + result.provider + "</strong>",
        "<p>1. Open this URL:</p>",
        `<a href="${escapeHtml(result.verification_uri)}" target="_blank" rel="noopener">${escapeHtml(result.verification_uri)}</a>`,
        "<p>2. Enter this code:</p>",
        `<div class="oauth-code">${escapeHtml(result.user_code)}</div>`,
        "<p>3. Waiting for authorization...</p>",
      ].join("");

      // Poll for token
      pollOAuthToken(providerId, result.state, 5000);
    } else {
      // Authorization code flow: open browser
      notice.textContent =
        "Opening browser for " + result.provider + " login...";
      window.open(result.url, "_blank", "width=600,height=700");
      notice.innerHTML =
        "<strong>Login with " +
        result.provider +
        "</strong><p>Complete authorization in the opened window.</p><p>Waiting for token...</p>";

      // Poll for token
      pollOAuthToken(providerId, result.state, 2000);
    }
  } catch (error) {
    notice.textContent = "❌ OAuth failed: " + error.message;
    notice.classList.add("error");
  }
}

async function pollOAuthToken(providerId, state, interval) {
  for (let i = 0; i < 60; i++) {
    await new Promise((r) => setTimeout(r, interval));
    try {
      const resp = await fetch(
        `/api/oauth/${encodeURIComponent(providerId)}/status?state=${encodeURIComponent(state)}`,
      );
      const data = await resp.json();

      if (data.status === "done" && data.access_token) {
        $("providerApiKeyInput").value = data.access_token;
        // Store project_id in provider data for Antigravity
        if (data.project_id) {
          try {
            const existing = JSON.parse($("providerDataInput").value || "{}");
            existing.projectId = data.project_id;
            $("providerDataInput").value = JSON.stringify(existing, null, 2);
          } catch (e) {}
        }
        const extra = data.project_id ? ` (Project: ${data.project_id})` : "";
        $("oauthNotice").innerHTML =
          `<strong>✅ Token received!</strong>${extra} Fill remaining fields and save.`;
        $("oauthNotice").classList.remove("error");
        return;
      }
      if (data.status === "error") {
        $("oauthNotice").textContent =
          "❌ OAuth error: " + (data.error || "Unknown");
        $("oauthNotice").classList.add("error");
        return;
      }
    } catch (e) {
      // Continue polling
    }
  }
  $("oauthNotice").innerHTML =
    "<strong>⏰ Login timed out.</strong> Please try again.";
  $("oauthNotice").classList.add("error");
}

function showOauthButton(providerId) {
  const btn = $("oauthGithubButton");
  if (!btn) return;

  const normalizedProviderId = (providerId || "").trim().toLowerCase();
  const oauthProviders = [
    "github",
    "codex",
    "openai",
    "google",
    "gemini",
    "antigravity",
  ];
  if (oauthProviders.includes(normalizedProviderId)) {
    btn.style.display = "";
    const labels = {
      github: "🔑 Login with GitHub",
      codex: "🔑 Login with OpenAI",
      openai: "🔑 Login with OpenAI",
      google: "🔑 Login with Google",
      gemini: "🔑 Login with Google",
      antigravity: "🔑 Login with Google",
    };
    btn.textContent = labels[normalizedProviderId] || "🔑 OAuth Login";
    btn.onclick = () => startOAuth(normalizedProviderId);
  } else {
    btn.style.display = "none";
    btn.onclick = null;
  }
}

async function fetchProviderModels(id) {
  const notice = $("providerTestNotice");
  notice.textContent = "Fetching latest provider models...";
  notice.classList.remove("hidden");
  notice.classList.remove("error");
  try {
    const result = await sendJson(
      `/api/setup/providers/${encodeURIComponent(id)}/models`,
      "POST",
      {},
    );
    notice.textContent = `OK: ${result.message} from ${result.models_url}`;
    notice.classList.remove("error");
    await refresh();
  } catch (error) {
    notice.textContent = `Fetch failed: ${error.message}`;
    notice.classList.add("error");
  }
}

function showProviderModels(id) {
  const providers = Array.isArray(state.providers?.data)
    ? state.providers.data
    : [];
  const provider = providers.find((item) => item.id === id);
  if (!provider) return;

  const models = providerModelIds(provider);
  const notice = $("providerTestNotice");
  notice.classList.remove("hidden");
  notice.classList.toggle("error", models.length === 0);
  notice.textContent = models.length
    ? `${provider.provider} models (${models.length}): ${models.join(", ")}`
    : `${provider.provider} has no saved models. Use Fetch or add data.models manually.`;
}

function addComboModel(model) {
  const current = $("comboModelsInput")
    .value.split(/\r?\n/)
    .map((item) => item.trim())
    .filter(Boolean);
  if (!current.includes(model)) current.push(model);
  $("comboModelsInput").value = current.join("\n");
}

async function saveConfig() {
  const payload = {
    require_api_key: $("configRequireApiKey").checked,
    telegram_enabled: $("telegramEnabledInput").checked,
    telegram_admin_ids: parseAdminIds($("telegramAdminInput").value),
    telegram_link_code_ttl_seconds: Number($("telegramTtlInput").value || 300),
    telegram_use_webhook: $("telegramWebhookInput").checked,
    telegram_webhook_url: $("telegramWebhookUrlInput").value.trim() || null,
  };
  await sendJson("/api/setup/config", "PUT", payload);
  setBadge("configSaveBadge", "Saved", "ok");
  await refresh();
}

function parseAdminIds(value) {
  return value
    .split(",")
    .map((item) => Number(item.trim()))
    .filter((item) => Number.isFinite(item));
}

function numberOrNull(value) {
  if (value === "") return null;
  const parsed = Number(value);
  return Number.isFinite(parsed) ? parsed : null;
}

function showNewApiKey(key) {
  const notice = $("newApiKeyNotice");
  notice.textContent = `New API key: ${key}`;
  notice.classList.remove("hidden");
}

refresh().catch((error) => {
  setBadge("healthBadge", "Offline", "error");
  console.error(error);
});

// Handle OAuth callback: if URL has ?token=..., fill it in
(function () {
  const params = new URLSearchParams(window.location.search);
  const token = params.get("token");
  const provider = params.get("provider");

  if (token && provider) {
    // Restore form state
    const saved = sessionStorage.getItem("oauthProviderForm");
    if (saved) {
      try {
        const data = JSON.parse(saved);
        $("providerInput").value = data.provider;
        $("providerNameInput").value = data.name;
        $("providerBaseUrlInput").value = data.baseUrl;
        $("providerFormatInput").value = data.format;
        $("providerPriorityInput").value = data.priority;
      } catch (e) {}
      sessionStorage.removeItem("oauthProviderForm");
    }

    // Fill the token
    $("providerApiKeyInput").value = token;
    $("oauthNotice").textContent =
      "✅ Token received from " +
      provider +
      "! Fill remaining fields and save.";
    $("oauthNotice").classList.remove("hidden");
    $("oauthNotice").classList.remove("error");

    // Clean URL
    window.history.replaceState({}, document.title, window.location.pathname);
  }
})();
