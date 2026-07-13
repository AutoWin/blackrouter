const state = {
  health: null,
  version: null,
  runtime: null,
  models: null,
  setupConfig: null,
  apiKeys: null,
  providers: null,
  providerCatalog: null,
  providerLimits: null,
  combos: null,
  aliases: null,
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

// Ordered, de-duplicated list of models currently being composed in the combo form.
let comboModelDraft = [];

// When set, the available-models picker is filtered to this provider type.
let comboProviderFilter = "";

// When set, an OAuth flow is in progress. Used by the relay listeners and the
// manual-code fallback so they know which provider/state to resolve.
let oauthPending = null; // { providerId, state }
let oauthDone = false; // guards against double-exchanging (poll + relay)

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

function formatNumber(value) {
  return Number(value || 0).toLocaleString();
}

function formatUsd(value) {
  return `$${Number(value || 0).toFixed(4)}`;
}

function formatUnixSeconds(value) {
  const seconds = Number(value);
  if (!Number.isFinite(seconds) || seconds <= 0) return value || "-";
  return new Date(seconds * 1000).toLocaleString();
}

async function getJson(path) {
  const response = await fetch(path, { cache: "no-store" });
  if (!response.ok) throw new Error(`${path} ${response.status}`);
  return response.json();
}

async function readResponsePayload(response) {
  const contentType = response.headers.get("content-type") || "";
  if (contentType.includes("application/json")) {
    return response.json().catch(() => ({}));
  }
  const text = await response.text().catch(() => "");
  return text ? { error: text } : {};
}

function responseErrorMessage(payload, fallback) {
  if (typeof payload?.error === "string") return payload.error;
  if (typeof payload?.error?.message === "string") return payload.error.message;
  if (typeof payload?.message === "string") return payload.message;
  return fallback;
}

async function sendJson(path, method, body) {
  const options = {
    method,
    headers: { "content-type": "application/json" },
  };
  if (body !== undefined) options.body = JSON.stringify(body);
  const response = await fetch(path, options);
  const payload = await readResponsePayload(response);
  if (!response.ok) {
    throw new Error(
      responseErrorMessage(payload, `${path} ${response.status}`),
    );
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
    providerLimits,
    combos,
    aliases,
  ] = await Promise.all([
    getJson("/health"),
    getJson("/version"),
    getJson("/api/runtime/status"),
    getJson("/v1/models"),
    getJson("/api/setup/config"),
    getJson("/api/setup/api-keys"),
    getJson("/api/setup/providers"),
    getJson("/api/setup/provider-catalog"),
    getJson("/api/provider-limits"),
    getJson("/api/setup/combos"),
    getJson("/api/setup/aliases"),
  ]);

  state.health = health;
  state.version = version;
  state.runtime = runtime;
  state.models = models;
  state.setupConfig = setupConfig;
  state.apiKeys = apiKeys;
  state.providers = providers;
  state.providerCatalog = providerCatalog;
  state.providerLimits = providerLimits;
  state.combos = combos;
  state.aliases = aliases;

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
  const providerLimits = state.providerLimits || {};
  const combos = Array.isArray(state.combos?.data) ? state.combos.data : [];
  const aliases = Array.isArray(state.aliases?.data) ? state.aliases.data : [];
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
  const healthProbe = savedSettings.healthProbe || {};
  $("healthProbeEnabledInput").checked = healthProbe.enabled ?? true;
  setValue("healthProbeIntervalInput", healthProbe.intervalSeconds || 30);
  setValue("healthProbeTimeoutInput", healthProbe.timeoutSeconds || 5);
  setValue("healthProbeThresholdInput", healthProbe.failureThreshold || 3);
  const costGuard = savedSettings.costGuard || savedSettings.cost_guard || {};
  $("costGuardEnabledInput").checked = costGuard.enabled ?? true;
  setValue("costGuardDailyInput", costGuard.dailyBudgetUsd ?? "");
  setValue("costGuardMonthlyInput", costGuard.monthlyBudgetUsd ?? "");

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
  renderApiKeys(apiKeys);
  renderAliases(aliases);
  renderProviders(providers);
  renderLimits(providerLimits);
  renderRows(
    "modelsList",
    models.map((model) => ({
      name: model.id,
      value: model.owned_by || "blackrouter",
    })),
    "No models",
  );
  renderComboProviderOptions(providers);
  renderComboPicker(providers);
  renderCombos(combos);
}

function renderApiKeys(keys) {
  const root = $("apiKeysList");
  if (!keys.length) {
    root.innerHTML = '<div class="empty">No API keys</div>';
    return;
  }
  root.innerHTML = keys.map((key) => {
    const policy = key.policy || {};
    const quota = [
      policy.requests_per_day ? `${formatNumber(policy.requests_per_day)} req/day` : null,
      policy.tokens_per_day ? `${formatNumber(policy.tokens_per_day)} tokens/day` : null,
      policy.cost_per_month_usd != null ? `${formatUsd(policy.cost_per_month_usd)}/month` : null,
    ].filter(Boolean).join(" · ") || "unlimited";
    return `<div class="row"><div><strong>${escapeHtml(key.name || key.id)}</strong><div class="row-meta"><span>${escapeHtml(key.key_masked)}</span><span>${escapeHtml(key.tenant_id || "default tenant")}</span><span>${escapeHtml(quota)}</span></div></div><div class="row-actions"><button class="secondary-button" data-key-edit="${escapeHtml(key.id)}" type="button">Edit</button><button class="secondary-button" data-key-rotate="${escapeHtml(key.id)}" type="button">Rotate</button></div></div>`;
  }).join("");
}

function renderAliases(aliases) {
  setBadge("aliasesBadge", `${aliases.length} aliases`, aliases.length ? "ok" : "muted");
  const root = $("aliasesList");
  if (!aliases.length) {
    root.innerHTML = '<div class="empty">No model aliases</div>';
    return;
  }
  root.innerHTML = aliases.map((alias) => `<div class="row"><div><strong>${escapeHtml(alias.alias)}</strong><div class="row-meta"><span>${escapeHtml(alias.target)}</span></div></div><button class="secondary-button" data-alias-delete="${escapeHtml(alias.id)}" type="button">Delete</button></div>`).join("");
}

function renderLimits(payload) {
  const rows = Array.isArray(payload?.data) ? payload.data : [];
  const snapshots = rows.filter((row) => row.upstream_rate_limit);
  const nowSeconds = Math.floor(Date.now() / 1000);
  const freshSnapshots = snapshots.filter(
    (row) => snapshotFreshness(row.upstream_rate_limit, nowSeconds) === "fresh",
  );
  const staleSnapshots = snapshots.filter((row) => {
    const f = snapshotFreshness(row.upstream_rate_limit, nowSeconds);
    return f === "stale" || f === "expired";
  });

  if (snapshots.length) {
    setBadge(
      "limitsBadge",
      freshSnapshots.length + " fresh / " + staleSnapshots.length + " stale",
      staleSnapshots.length && !freshSnapshots.length ? "warn" : "ok",
    );
  } else {
    setBadge("limitsBadge", "No upstream data", "muted");
  }

  const metrics = payload?.metrics || {};
  $("limitsSummary").innerHTML =
    '<div class="metric-tile"><span>Total Requests</span><strong>' + formatNumber(metrics.total_requests) + '</strong></div>' +
    '<div class="metric-tile"><span>Prompt Tokens</span><strong>' + formatNumber(metrics.total_prompt_tokens) + '</strong></div>' +
    '<div class="metric-tile"><span>Completion Tokens</span><strong>' + formatNumber(metrics.total_completion_tokens) + '</strong></div>' +
    '<div class="metric-tile"><span>Tracked Cost</span><strong>' + formatUsd(metrics.total_cost) + '</strong></div>';

  const guard = payload?.cost_guard || {};
  $("costGuardSummary").innerHTML =
    '<div class="metric-tile"><span>Cost Guard</span><strong>' + (guard.enabled ? "Enabled" : "Disabled") + '</strong></div>' +
    budgetTile("Daily", guard.daily_spend_usd, guard.daily_budget_usd, guard.daily_exceeded) +
    budgetTile("Monthly", guard.monthly_spend_usd, guard.monthly_budget_usd, guard.monthly_exceeded);

  const root = $("limitsList");
  if (!root) return;
  if (!rows.length) {
    root.innerHTML =
      '<div class="empty">No provider connections yet.</div>' +
      '<p class="empty-hint">Add a provider connection, then send a request through it to collect upstream rate-limit snapshots.</p>';
    return;
  }

  root.innerHTML = rows
    .map((row) => renderLimitRow(row, nowSeconds))
    .join("");
}

function snapshotFreshness(snapshot, nowSeconds) {
  const observed = snapshot?.observedAt;
  const seconds = Number(observed);
  if (!observed || !Number.isFinite(seconds)) return "unknown";
  const ageSeconds = nowSeconds - seconds;
  if (ageSeconds < 0) return "fresh";
  if (ageSeconds <= 5 * 60) return "fresh";
  if (ageSeconds <= 30 * 60) return "stale";
  return "expired";
}

const FRESHNESS_LABEL = {
  fresh: "Fresh",
  stale: "Stale",
  expired: "Expired",
  unknown: "Unknown age",
};

function budgetTile(label, spent, budget, exceeded) {
  const hasBudget = typeof budget === "number" && budget > 0;
  const pct = hasBudget && spent > 0 ? Math.min(100, (spent / budget) * 100) : 0;
  const tone = exceeded ? "danger" : hasBudget && pct >= 80 ? "warn" : "";
  let body;
  if (hasBudget) {
    body =
      '<div class="progress"><div class="progress-bar ' + tone + '" style="width:' + pct.toFixed(0) + '%"></div></div>' +
      '<span class="progress-note">' + pct.toFixed(0) + '% used' + (exceeded ? ' — exceeded' : '') + '</span>';
  } else {
    body = '<span class="progress-note">no budget set</span>';
  }
  return (
    '<div class="metric-tile ' + tone + '">' +
    '<span>' + escapeHtml(label) + ' Spend</span>' +
    '<strong>' + formatUsd(spent) + (hasBudget ? ' / ' + formatUsd(budget) : '') + '</strong>' +
    body +
    '</div>'
  );
}

function renderLimitRow(row, nowSeconds) {
  const snapshot = row.upstream_rate_limit || {};
  const headers = snapshot.headers || {};
  const rtk = row.rtk || {};
  const usage = row.usage || {};
  const title = row.provider + "/" + (row.name || row.id);
  const emailSuffix = row.email
    ? '<span class="provider-email">' + escapeHtml(row.email) + '</span>'
    : "";

  const freshness = snapshotFreshness(snapshot, nowSeconds);
  const freshBadge =
    snapshot && freshness !== "unknown"
      ? '<span class="badge ' + (freshness === "fresh" ? "ok" : freshness === "expired" ? "error" : "warn") + ' limit-fresh">' + FRESHNESS_LABEL[freshness] + '</span>'
      : "";

  const statusTone =
    row.status === "healthy"
      ? "ok"
      : row.status === "cooldown" || row.status === "error"
        ? "error"
        : "muted";

  const limitGroup =
    '<div class="limit-group"><span class="limit-group-title">Upstream</span><div class="limit-grid">' +
    limitCell("RPM Remaining", headers["x-ratelimit-remaining-requests"], headers["x-ratelimit-limit-requests"]) +
    limitCell("TPM Remaining", headers["x-ratelimit-remaining-tokens"], headers["x-ratelimit-limit-tokens"]) +
    limitCell("Request Reset", formatReset(headers["x-ratelimit-reset-requests"])) +
    limitCell("Token Reset", formatReset(headers["x-ratelimit-reset-tokens"])) +
    '</div></div>';

  const rtkGroup = rtk && rtk.requests_remaining !== undefined
    ? '<div class="limit-group"><span class="limit-group-title">BlackRouter RTK (provider-wide)</span><div class="limit-grid">' +
      limitCell("Requests Remaining", rtk.requests_remaining) +
      limitCell("Tokens Remaining", rtk.tokens_remaining) +
      limitCell("Concurrent Remaining", rtk.concurrent_remaining) +
      limitCell("Retry After", rtk.retry_after_seconds != null ? rtk.retry_after_seconds + "s" : null) +
      (rtk.limited ? limitCell("State", "Limited") : "") +
      '</div></div>'
    : "";

  const usageGroup =
    '<div class="limit-group"><span class="limit-group-title">Usage</span><div class="limit-grid">' +
    limitCell("Requests", usage.requests) +
    limitCell("Prompt Tokens", usage.prompt_tokens) +
    limitCell("Completion Tokens", usage.completion_tokens) +
    limitCell("Cost", formatUsd(usage.cost)) +
    '</div></div>';

  const seen = snapshot
    ? '<span>seen ' + escapeHtml(formatUnixSeconds(snapshot.observedAt)) + '</span>'
    : '<span>no upstream snapshot</span>';

  return (
    '<div class="row limit-row">' +
    '<div class="limit-row-head"><div>' +
    '<strong title="' + escapeHtml(title) + '">' + escapeHtml(title) + '</strong>' +
    emailSuffix +
    '<div class="row-meta">' +
    '<span class="badge ' + statusTone + '">' + escapeHtml(row.status || "unknown") + '</span>' +
    '<span>' + (row.is_active ? "active" : "disabled") + '</span>' +
    '<span>' + escapeHtml(row.model || "no model") + '</span>' +
    freshBadge + seen +
    '</div></div></div>' +
    limitGroup + rtkGroup + usageGroup +
    '</div>'
  );
}

function formatReset(value) {
  if (value === undefined || value === null || value === "") return null;
  const seconds = Number(value);
  if (!Number.isFinite(seconds)) return String(value);
  if (seconds <= 0) return "now";
  if (seconds < 60) return seconds + "s";
  const minutes = Math.floor(seconds / 60);
  const rem = seconds % 60;
  return rem ? minutes + "m " + rem + "s" : minutes + "m";
}

function limitCell(label, value, limit) {
  const displayValue =
    value === undefined || value === null || value === "" ? "-" : String(value);
  const displayLimit =
    limit === undefined || limit === null || limit === "" ? "" : " / " + limit;
  return (
    '<div class="limit-cell">' +
    '<span>' + escapeHtml(label) + '</span>' +
    '<strong>' + escapeHtml(displayValue) + escapeHtml(displayLimit) + '</strong>' +
    '</div>'
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
      const emailSuffix = provider.email
        ? `<span class="provider-email">${escapeHtml(provider.email)}</span>`
        : "";
      return `
        <div class="row provider-row" data-provider-id="${escapeHtml(provider.id)}">
          <div>
            <strong title="${escapeHtml(title)}">${escapeHtml(title)}</strong>
            ${emailSuffix}
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

function comboPickerOptions(providers) {
  const list = Array.isArray(providers) ? providers : [];
  const sameProviderCount = {};
  list.forEach((provider) => {
    sameProviderCount[provider.provider] =
      (sameProviderCount[provider.provider] || 0) + 1;
  });

  const options = [];
  list.forEach((provider) => {
    if (comboProviderFilter && provider.provider !== comboProviderFilter) return;
    providerModelIds(provider).forEach((model) => {
      const value = `${provider.provider}/${model}`;
      let label = `${provider.provider}/${model}`;
      const needsSuffix = (sameProviderCount[provider.provider] || 1) > 1;
      if (needsSuffix) {
        const suffix = provider.email || provider.name || "";
        if (suffix) label += ` (${suffix})`;
      }
      options.push({ value, label });
    });
  });

  return options.sort((a, b) => a.label.localeCompare(b.label));
}

// Populates the Provider <select> from the available provider connections and
// restores the active filter so a background refresh doesn't lose the selection.
function renderComboProviderOptions(providers) {
  const select = $("comboProviderInput");
  if (!select) return;

  const list = Array.isArray(providers) ? providers : [];
  const providerTypes = [
    ...new Set(list.map((provider) => provider.provider).filter(Boolean)),
  ].sort((a, b) => a.localeCompare(b));

  select.innerHTML = [
    `<option value="">All providers</option>`,
    ...providerTypes.map(
      (type) => `<option value="${escapeHtml(type)}">${escapeHtml(type)}</option>`,
    ),
  ].join("");
  select.value = comboProviderFilter;
}

// Renders the reference list of available provider models with an "Add" button
// per row. Buttons for models already in the draft are disabled and labelled
// "Added" so the user gets immediate feedback.
function renderComboPicker(providersSource) {
  const providers = Array.isArray(providersSource)
    ? providersSource
    : Array.isArray(providersSource?.data)
      ? providersSource.data
      : [];
  const root = $("comboModelsList");
  if (!root) return;

  const options = comboPickerOptions(providers);
  if (!options.length) {
    root.innerHTML = `<div class="empty">No provider models available</div>`;
    return;
  }

  root.innerHTML = options
    .map((option) => {
      const added = comboModelDraft.includes(option.value);
      return `
        <div class="row model-pick-row">
          <div>
            <strong title="${escapeHtml(option.label)}">${escapeHtml(option.label)}</strong>
            <span>${escapeHtml(option.value)}</span>
          </div>
          <div class="row-actions">
            <button
              class="row-button"
              type="button"
              data-action="add-model"
              data-model="${escapeHtml(option.value)}"
              ${added ? "disabled" : ""}
            >${added ? "Added" : "Add"}</button>
          </div>
        </div>
      `;
    })
    .join("");
}

// Renders the ordered list of models in the combo draft with move/remove controls.
function renderComboModelDraft() {
  const root = $("comboModelsInput");
  if (!root) return;

  const hint = $("comboEmptyHint");
  if (!comboModelDraft.length) {
    root.innerHTML = "";
    if (hint) hint.classList.remove("hidden");
    return;
  }
  if (hint) hint.classList.add("hidden");

  root.innerHTML = comboModelDraft
    .map((model, index) => {
      const canUp = index > 0;
      const canDown = index < comboModelDraft.length - 1;
      return `
        <div class="model-edit-row" data-index="${index}">
          <span class="model-edit-index">${index + 1}</span>
          <span class="model-edit-name" title="${escapeHtml(model)}">${escapeHtml(model)}</span>
          <div class="row-actions">
            <button
              class="row-button icon-move"
              type="button"
              data-action="move-up"
              data-index="${index}"
              ${canUp ? "" : "disabled"}
              aria-label="Move up"
            >▲</button>
            <button
              class="row-button icon-move"
              type="button"
              data-action="move-down"
              data-index="${index}"
              ${canDown ? "" : "disabled"}
              aria-label="Move down"
            >▼</button>
            <button
              class="danger-button row-button"
              type="button"
              data-action="remove-model"
              data-index="${index}"
              aria-label="Remove"
            >✕</button>
          </div>
        </div>
      `;
    })
    .join("");
}

function getComboModels() {
  return comboModelDraft.slice();
}

function showComboNotice(message, isError) {
  const node = $("comboNotice");
  if (!node) return;
  node.textContent = message;
  node.classList.toggle("error", Boolean(isError));
  node.classList.remove("hidden");
}

function hideComboNotice() {
  const node = $("comboNotice");
  if (!node) return;
  node.classList.add("hidden");
  node.textContent = "";
}

function markFieldError(id) {
  const node = $(id);
  if (node) node.classList.add("field-error");
}

function clearFieldError(id) {
  const node = $(id);
  if (node) node.classList.remove("field-error");
}

function providerModelIds(provider) {
  const models = provider?.data?.models;
  if (!Array.isArray(models) || models.length === 0) {
    const providerId = String(provider?.provider || "").toLowerCase();
    const alias = String(provider?.data?.alias || "").toLowerCase();
    const format = String(provider?.data?.format || "").toLowerCase();
    if (
      providerId === "antigravity" ||
      alias === "ag" ||
      format === "antigravity"
    ) {
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
  const policy = {
    requests_per_day: numberOrNull($("apiKeyRequestsInput").value),
    tokens_per_day: numberOrNull($("apiKeyTokensInput").value),
    cost_per_month_usd: numberOrNull($("apiKeyCostInput").value),
    provider_allowlist: csvValues($("apiKeyProvidersInput").value),
    model_allowlist: csvValues($("apiKeyModelsInput").value),
  };
  const editId = $("apiKeyEditIdInput").value;
  if (editId) {
    await sendJson(`/api/setup/api-keys/${encodeURIComponent(editId)}`, "PUT", {
      tenant_id: $("apiKeyTenantInput").value.trim() || null,
      policy,
    });
    resetApiKeyForm();
    await refresh();
    return;
  }
  const created = await sendJson("/api/setup/api-keys", "POST", {
    name: $("apiKeyNameInput").value,
    machine_id: $("apiKeyMachineInput").value,
    tenant_id: $("apiKeyTenantInput").value.trim() || null,
    policy,
  });
  resetApiKeyForm();
  showNewApiKey(created.key);
  await refresh();
});

$("apiKeyCancelEditButton").addEventListener("click", resetApiKeyForm);

$("apiKeysList").addEventListener("click", async (event) => {
  const editId = event.target.closest("[data-key-edit]")?.dataset.keyEdit;
  if (editId) {
    const key = state.apiKeys?.data?.find((item) => item.id === editId);
    if (!key) return;
    $("apiKeyEditIdInput").value = key.id;
    setValue("apiKeyNameInput", key.name || "");
    setValue("apiKeyMachineInput", key.machine_id || "");
    setValue("apiKeyTenantInput", key.tenant_id || "");
    setValue("apiKeyRequestsInput", key.policy?.requests_per_day ?? "");
    setValue("apiKeyTokensInput", key.policy?.tokens_per_day ?? "");
    setValue("apiKeyCostInput", key.policy?.cost_per_month_usd ?? "");
    setValue("apiKeyProvidersInput", (key.policy?.provider_allowlist || []).join(", "));
    setValue("apiKeyModelsInput", (key.policy?.model_allowlist || []).join(", "));
    $("apiKeyNameInput").disabled = true;
    $("apiKeyMachineInput").disabled = true;
    setText("apiKeySubmitLabel", "Save Policy");
    $("apiKeyCancelEditButton").classList.remove("hidden");
    return;
  }
  const rotateId = event.target.closest("[data-key-rotate]")?.dataset.keyRotate;
  if (rotateId) {
    const created = await sendJson(`/api/setup/api-keys/${encodeURIComponent(rotateId)}/rotate`, "POST");
    showNewApiKey(created.key);
    await refresh();
  }
});

$("aliasForm").addEventListener("submit", async (event) => {
  event.preventDefault();
  await sendJson("/api/setup/aliases", "POST", {
    alias: $("aliasNameInput").value.trim(),
    target: $("aliasTargetInput").value.trim(),
  });
  $("aliasForm").reset();
  await refresh();
});

$("aliasesList").addEventListener("click", async (event) => {
  const id = event.target.closest("[data-alias-delete]")?.dataset.aliasDelete;
  if (!id) return;
  await sendJson(`/api/setup/aliases/${encodeURIComponent(id)}`, "DELETE");
  await refresh();
});

$("costGuardForm").addEventListener("submit", async (event) => {
  event.preventDefault();
  await saveConfig();
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
    email: $("providerEmailInput").value || null,
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
  hideComboNotice();

  const name = $("comboNameInput").value.trim();
  if (!name) {
    showComboNotice("Combo name is required.", "error");
    markFieldError("comboNameInput");
    $("comboNameInput").focus();
    return;
  }

  const models = getComboModels();
  if (!models.length) {
    showComboNotice(
      "Add at least one model to the combo (pick from the list or type one above).",
      "error",
    );
    markFieldError("comboModelsInput");
    return;
  }

  const editId = $("comboEditIdInput").value;
  const method = editId ? "PUT" : "POST";
  const path = editId
    ? `/api/setup/combos/${encodeURIComponent(editId)}`
    : "/api/setup/combos";

  try {
    await sendJson(path, method, {
      name,
      kind: $("comboKindInput").value.trim() || "llm",
      models,
    });
    showComboNotice(editId ? "Combo updated." : "Combo created.", false);
    resetComboForm();
    await refresh();
  } catch (error) {
    showComboNotice(`Failed: ${error.message}`, "error");
  }
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

$("comboAddManualButton").addEventListener("click", () => {
  const input = $("comboModelManualInput");
  addComboModel(input.value, $("comboProviderInput").value);
});

$("comboModelManualInput").addEventListener("keydown", (event) => {
  if (event.key === "Enter") {
    event.preventDefault();
    addComboModel(event.target.value, $("comboProviderInput").value);
  }
});

$("comboProviderInput").addEventListener("change", () => {
  comboProviderFilter = $("comboProviderInput").value;
  renderComboPicker(state.providers);
});

// Delegated handler for the available-models picker (Add buttons).
$("comboModelsList").addEventListener("click", (event) => {
  const button = event.target.closest('button[data-action="add-model"]');
  if (!button || button.disabled) return;
  addComboModel(button.dataset.model);
});

// Delegated handler for the ordered combo draft (move up/down, remove).
$("comboModelsInput").addEventListener("click", (event) => {
  const button = event.target.closest("button[data-action]");
  if (!button) return;

  const index = Number(button.dataset.index);
  const action = button.dataset.action;
  if (action === "move-up" && index > 0) {
    [comboModelDraft[index - 1], comboModelDraft[index]] = [
      comboModelDraft[index],
      comboModelDraft[index - 1],
    ];
  } else if (action === "move-down" && index < comboModelDraft.length - 1) {
    [comboModelDraft[index + 1], comboModelDraft[index]] = [
      comboModelDraft[index],
      comboModelDraft[index + 1],
    ];
  } else if (action === "remove-model") {
    comboModelDraft.splice(index, 1);
  } else {
    return;
  }
  renderComboModelDraft();
  renderComboPicker(state.providers);
});

$("comboNameInput").addEventListener("input", () =>
  clearFieldError("comboNameInput"),
);

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
  $("providerEmailInput").value = provider.email || "";
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
  $("providerEmailInput").value = "";
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
  setValue("comboKindInput", combo.kind || "llm");
  comboModelDraft = Array.isArray(combo.models) ? combo.models.slice() : [];
  comboProviderFilter = "";
  renderComboModelDraft();
  renderComboPicker(state.providers);
  const providerSelect = $("comboProviderInput");
  if (providerSelect) providerSelect.value = "";
  clearFieldError("comboNameInput");
  clearFieldError("comboModelsInput");
  $("comboSubmitButton").textContent = "Save Combo";
  $("comboCancelEditButton").classList.remove("hidden");
  setBadge("combosBadge", "Editing combo", "warn");
}

function resetComboForm() {
  $("comboEditIdInput").value = "";
  $("comboNameInput").value = "";
  setValue("comboKindInput", "llm");
  comboModelDraft = [];
  comboProviderFilter = "";
  renderComboModelDraft();
  renderComboPicker(state.providers);
  const providerSelect = $("comboProviderInput");
  if (providerSelect) providerSelect.value = "";
  $("comboModelManualInput").value = "";
  $("comboSubmitButton").textContent = "Add Combo";
  $("comboCancelEditButton").classList.add("hidden");
  hideComboNotice();
  clearFieldError("comboNameInput");
  clearFieldError("comboModelsInput");
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
  if (notice) {
    notice.textContent = "Starting OAuth flow...";
    notice.classList.remove("hidden");
    notice.classList.remove("error");
  }
  // Hide the manual-code fallback from any previous attempt.
  const manual = $("oauthManual");
  if (manual) manual.classList.add("hidden");

  // Reset in-progress tracking.
  oauthDone = false;
  oauthPending = { providerId, state: null };

  // Open the popup synchronously, inside the click gesture, so the browser
  // does not block it. We only learn the auth URL after the /start call,
  // so start with about:blank and navigate it once we have the URL.
  let popup = null;
  try {
    popup = window.open(
      "about:blank",
      "blackrouter-oauth",
      "width=600,height=720,left=" +
        (window.screenX + window.outerWidth / 2 - 300) +
        ",top=" +
        (window.screenY + window.outerHeight / 2 - 360),
    );
  } catch (e) {
    popup = null;
  }

  try {
    // For browser-based providers (Google/Antigravity) we compute the
    // redirect_uri from the page's own origin so the callback always lands on
    // a host the browser can actually reach, independent of BLACKROUTER_BASE_URL
    // or whether BlackRouter runs in Docker / behind a proxy.
    const normalizedProvider = (providerId || "").trim().toLowerCase();
    const usesOriginRedirect =
      normalizedProvider === "google" ||
      normalizedProvider === "gemini" ||
      normalizedProvider === "antigravity";
    const body = usesOriginRedirect
      ? { redirect_uri: window.location.origin + "/oauth/callback" }
      : {};

    const resp = await fetch(
      `/api/oauth/${encodeURIComponent(providerId)}/start`,
      {
        method: "POST",
        headers: { "content-type": "application/json" },
        body: JSON.stringify(body),
      },
    );
    const result = await readResponsePayload(resp);

    if (!resp.ok) {
      if (popup) popup.close();
      notice.textContent =
        "❌ " + responseErrorMessage(result, "Failed to start OAuth");
      notice.classList.add("error");
      oauthPending = null;
      return;
    }

    oauthPending = { providerId, state: result.state };

    if (result.flow_type === "device_code") {
      // Device-code flow has no popup — close it if we opened one.
      if (popup) popup.close();
      notice.innerHTML = [
        "<strong>Login with " + result.provider + "</strong>",
        "<p>1. Open this URL:</p>",
        `<a href="${escapeHtml(result.verification_uri)}" target="_blank" rel="noopener">${escapeHtml(result.verification_uri)}</a>`,
        "<p>2. Enter this code:</p>",
        `<div class="oauth-code">${escapeHtml(result.user_code)}</div>`,
        "<p>3. Waiting for authorization...</p>",
      ].join("");

      pollOAuthToken(providerId, result.state, 5000);
    } else {
      // authorization_code flow (OpenAI/Codex loopback, Google, etc.)
      // Keep this setup page open so it can receive the token. The popup
      // handles the provider login and is redirected back; a full-page
      // redirect would replace this page and the token could never flow back.
      sessionStorage.setItem(
        "blackrouter.oauth.pending",
        JSON.stringify({
          providerId,
          state: result.state,
          startedAt: Date.now(),
        }),
      );

      if (!popup) {
        notice.innerHTML =
          "<strong>Login with " +
          result.provider +
          "</strong><p>Popup was blocked. Redirecting to browser authorization...</p>";
        window.location.href = result.url;
      } else {
        popup.location.href = result.url;
        notice.innerHTML =
          "<strong>Login with " +
          result.provider +
          "</strong><p>A login window has opened. Complete authorization there — this page will update automatically.</p>";
      }

      // OpenAI/Codex use a localhost loopback the browser may not reach
      // (remote/Docker/port busy). Offer a manual paste fallback so login
      // can always complete.
      if (normalizedProvider === "codex" || normalizedProvider === "openai") {
        const manualBox = $("oauthManual");
        if (manualBox) manualBox.classList.remove("hidden");
      }

      pollOAuthToken(providerId, result.state, 3000);
    }
  } catch (error) {
    if (popup) popup.close();
    notice.textContent = "❌ OAuth failed: " + error.message;
    notice.classList.add("error");
    oauthPending = null;
  }
}

async function pollOAuthToken(providerId, state, interval) {
  for (let i = 0; i < 60; i++) {
    if (oauthDone) return;
    await new Promise((r) => setTimeout(r, interval));
    if (oauthDone) return;
    try {
      const resp = await fetch(
        `/api/oauth/${encodeURIComponent(providerId)}/status?state=${encodeURIComponent(state)}`,
      );
      const data = await resp.json();
      if (data.status === "done" && data.access_token) {
        applyOAuthResult(data);
        return;
      }
      if (data.status === "error") {
        applyOAuthResult(data);
        return;
      }
    } catch (e) {
      // Continue polling
    }
  }
  if (oauthDone) return;
  sessionStorage.removeItem("blackrouter.oauth.pending");
  $("oauthManual")?.classList.add("hidden");
  const notice = $("oauthNotice");
  notice.innerHTML = "<strong>⏰ Login timed out.</strong> Please try again.";
  notice.classList.add("error");
}

// Shared handler for a completed OAuth result (used by both the status poll and
// the relay/manual-code paths). Idempotent: once resolved, it is a no-op.
function applyOAuthResult(data) {
  if (oauthDone) return;
  // Capture the provider id before clearing in-progress state.
  const providerId = oauthPendingProviderId();
  oauthDone = true;
  oauthPending = null;
  sessionStorage.removeItem("blackrouter.oauth.pending");
  $("oauthManual")?.classList.add("hidden");
  const notice = $("oauthNotice");

  if (data.error && !data.access_token) {
    notice.textContent = "❌ OAuth error: " + (data.error || "Unknown");
    notice.classList.add("error");
    return;
  }
  if (!data.access_token) {
    notice.textContent = "❌ OAuth failed: no access token returned.";
    notice.classList.add("error");
    return;
  }

  $("providerApiKeyInput").value = data.access_token;
  if (data.email) {
    $("providerEmailInput").value = data.email;
    $("providerNameInput").value =
      $("providerNameInput").value || data.email.split("@")[0];
  }

  if (
    data.project_id ||
    data.refresh_token ||
    data.token_expires_at ||
    providerId === "antigravity"
  ) {
    try {
      const existing = JSON.parse($("providerDataInput").value || "{}");
      if (data.project_id) existing.projectId = data.project_id;
      if (data.refresh_token) existing.refreshToken = data.refresh_token;
      if (data.token_expires_at) existing.tokenExpiresAt = data.token_expires_at;
      $("providerDataInput").value = JSON.stringify(existing, null, 2);
    } catch (e) {}
  }

  const extra = data.email
    ? ` (${data.email})`
    : data.project_id
      ? ` (Project: ${data.project_id})`
      : "";
  notice.innerHTML =
    `<strong>✅ Token received!</strong>${extra} Fill remaining fields and save.`;
  notice.classList.remove("error");
}

function oauthPendingProviderId() {
  try {
    const raw = sessionStorage.getItem("blackrouter.oauth.pending");
    if (raw) return JSON.parse(raw).providerId;
  } catch (e) {}
  return oauthPending?.providerId || null;
}

// Relay listeners: receive the OAuth code/state from the popup (or another
// same-origin tab) and complete the exchange. Mirrors 9router's pattern using
// postMessage + BroadcastChannel + localStorage for maximum robustness.
function handleOAuthRelay(data) {
  if (oauthDone || !oauthPending) return;
  if (!data) return;
  if (data.error) {
    oauthDone = true;
    oauthPending = null;
    sessionStorage.removeItem("blackrouter.oauth.pending");
    $("oauthManual")?.classList.add("hidden");
    const notice = $("oauthNotice");
    notice.textContent =
      "❌ OAuth error: " + (data.errorDescription || data.error);
    notice.classList.add("error");
    return;
  }
  if (!data.code) return;
  const providerId = oauthPending.providerId;
  const state = data.state || oauthPending.state;
  sendJson(
    `/api/oauth/${encodeURIComponent(providerId)}/exchange`,
    "POST",
    { code: data.code, state },
  )
    .then(applyOAuthResult)
    .catch((err) => {
      const notice = $("oauthNotice");
      notice.textContent = "❌ OAuth exchange failed: " + err.message;
      notice.classList.add("error");
    });
}

function initOAuthRelay() {
  window.addEventListener("message", (event) => {
    const isLocal =
      event.origin.includes("localhost") || event.origin.includes("127.0.0.1");
    if (event.origin !== window.location.origin && !isLocal) return;
    if (event.data && event.data.type === "oauth_callback") {
      handleOAuthRelay(event.data.data);
    }
  });
  try {
    const channel = new BroadcastChannel("oauth_callback");
    channel.onmessage = (event) => handleOAuthRelay(event.data);
  } catch (e) {}
  window.addEventListener("storage", (event) => {
    if (event.key === "oauth_callback" && event.newValue) {
      try {
        handleOAuthRelay(JSON.parse(event.newValue));
        localStorage.removeItem("oauth_callback");
      } catch (e) {}
    }
  });
}

// Manual fallback for OpenAI/Codex when the localhost loopback can't be reached
// (remote/Docker). The user pastes the redirected callback URL (or raw code).
async function submitOAuthManual() {
  if (!oauthPending) return;
  const raw = ($("oauthManualInput").value || "").trim();
  if (!raw) return;
  let code = null;
  let state = oauthPending.state;
  if (raw.startsWith("eyJ") && raw.includes(".")) {
    code = raw; // raw JWT access token
  } else {
    try {
      const candidate = raw.includes("://")
        ? raw
        : "http://localhost/" + raw.replace(/^\?/, "");
      const url = new URL(candidate);
      code = url.searchParams.get("code");
      state = url.searchParams.get("state") || state;
      if (!code) code = raw; // treat as a raw code
    } catch (e) {
      code = raw;
    }
  }
  if (!code) {
    const notice = $("oauthNotice");
    notice.textContent = "❌ No code found in input.";
    notice.classList.add("error");
    return;
  }
  try {
    const data = await sendJson(
      `/api/oauth/${encodeURIComponent(oauthPending.providerId)}/exchange`,
      "POST",
      { code, state },
    );
    applyOAuthResult(data);
  } catch (err) {
    const notice = $("oauthNotice");
    notice.textContent = "❌ Exchange failed: " + err.message;
    notice.classList.add("error");
  }
}

function initOAuthManual() {
  const btn = $("oauthManualButton");
  if (btn) btn.addEventListener("click", submitOAuthManual);
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
      `/api/setup/providers/${encodeURIComponent(id)}/models?refresh=1`,
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
  const label = provider.email
    ? `${provider.provider} (${provider.email})`
    : provider.name
      ? `${provider.provider}/${provider.name}`
      : provider.provider;
  notice.textContent = models.length
    ? `${label} models (${models.length}): ${models.join(", ")}`
    : `${label} has no saved models. Use Fetch or add data.models manually.`;
}

function addComboModel(model, provider) {
  let value = String(model || "").trim();
  if (!value) return;
  // When a provider is selected and the entry has no provider prefix yet,
  // auto-qualify it as provider/model.
  if (!value.includes("/") && provider) value = `${provider}/${value}`;
  if (!comboModelDraft.includes(value)) {
    comboModelDraft.push(value);
    renderComboModelDraft();
    renderComboPicker(state.providers);
    clearFieldError("comboModelsInput");
  }
  const input = $("comboModelManualInput");
  if (input) input.value = "";
}

async function saveConfig() {
  const payload = {
    require_api_key: $("configRequireApiKey").checked,
    telegram_enabled: $("telegramEnabledInput").checked,
    telegram_admin_ids: parseAdminIds($("telegramAdminInput").value),
    telegram_link_code_ttl_seconds: Number($("telegramTtlInput").value || 300),
    telegram_use_webhook: $("telegramWebhookInput").checked,
    telegram_webhook_url: $("telegramWebhookUrlInput").value.trim() || null,
    health_probe_enabled: $("healthProbeEnabledInput").checked,
    health_probe_interval_seconds: Number($("healthProbeIntervalInput").value || 30),
    health_probe_timeout_seconds: Number($("healthProbeTimeoutInput").value || 5),
    health_probe_failure_threshold: Number($("healthProbeThresholdInput").value || 3),
    cost_guard_enabled: $("costGuardEnabledInput").checked,
    cost_guard_daily_budget_usd: numberOrNull($("costGuardDailyInput").value),
    cost_guard_monthly_budget_usd: numberOrNull($("costGuardMonthlyInput").value),
  };
  await sendJson("/api/setup/config", "PUT", payload);
  setBadge("configSaveBadge", "Saved", "ok");
  await refresh();
}

function csvValues(value) {
  return value.split(",").map((item) => item.trim()).filter(Boolean);
}

function resetApiKeyForm() {
  $("apiKeyForm").reset();
  $("apiKeyEditIdInput").value = "";
  $("apiKeyNameInput").disabled = false;
  $("apiKeyMachineInput").disabled = false;
  setText("apiKeySubmitLabel", "Create API Key");
  $("apiKeyCancelEditButton").classList.add("hidden");
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

async function resumePendingOAuth() {
  const raw = sessionStorage.getItem("blackrouter.oauth.pending");
  if (!raw) return;

  let pending;
  try {
    pending = JSON.parse(raw);
  } catch (error) {
    sessionStorage.removeItem("blackrouter.oauth.pending");
    return;
  }

  const providerId = pending?.providerId;
  const oauthState = pending?.state;
  const startedAt = Number(pending?.startedAt || 0);
  if (!providerId || !oauthState || Date.now() - startedAt > 10 * 60 * 1000) {
    sessionStorage.removeItem("blackrouter.oauth.pending");
    return;
  }

  const notice = $("oauthNotice");
  if (notice) {
    notice.innerHTML = `<strong>Waiting for ${escapeHtml(providerId)} authorization...</strong><p>Completing login and loading token.</p>`;
    notice.classList.remove("hidden");
    notice.classList.remove("error");
  }
  pollOAuthToken(providerId, oauthState, 1000);
}

let limitsLoading = false;

async function refreshLimits() {
  if (limitsLoading) return;
  limitsLoading = true;
  const button = $("refreshLimitsButton");
  const notice = $("limitsNotice");
  if (button) button.disabled = true;
  if (notice) {
    notice.textContent = "Refreshing limits...";
    notice.classList.remove("hidden", "error");
  }
  try {
    const payload = await getJson("/api/provider-limits");
    state.providerLimits = payload;
    renderLimits(payload);
    const updated = $("limitsUpdated");
    if (updated) {
      updated.textContent = "Updated " + new Date().toLocaleTimeString();
    }
  } catch (error) {
    if (notice) {
      notice.textContent = "Failed to load limits: " + error.message;
      notice.classList.add("error");
    }
  } finally {
    if (notice && notice.classList.contains("error") === false) {
      notice.classList.add("hidden");
    }
    if (button) button.disabled = false;
    limitsLoading = false;
  }
}

refresh()
  .then(resumePendingOAuth)
  .then(() => {
    initOAuthRelay();
    initOAuthManual();
  })
  .catch((error) => {
    setBadge("healthBadge", "Offline", "error");
    console.error(error);
  });

$("refreshLimitsButton")?.addEventListener("click", refreshLimits);

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
