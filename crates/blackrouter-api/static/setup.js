/* ═══════════════════════════════════════════════════════════════════════════
   BlackRouter Control Panel — setup.js (pages & interactions)
   Requires core.js loaded first.
   ═══════════════════════════════════════════════════════════════════════════ */

/* ── Hash Router ─────────────────────────────────────────────────────────── */

function readHash() { return window.location.hash.replace(/^#/, "") || "overview"; }

function navigateTo(panelId) {
  const panel = $(panelId);
  if (!panel) return;
  document.querySelectorAll(".panel").forEach(p => p.classList.remove("active"));
  panel.classList.add("active");

  document.querySelectorAll(".nav-item").forEach(btn => {
    btn.classList.toggle("active", btn.dataset.panel === panelId);
  });

  const titles = {
    overview: ["Overview", "System status at a glance"],
    providers: ["Provider Connections", "Manage upstream AI provider connections"],
    combos: ["Combos", "Fallback model chains"],
    aliases: ["Model Aliases", "Map friendly names to provider/model targets"],
    apikeys: ["API Keys", "Gateway API keys with per-tenant quota and access policies"],
    limits: ["Limits & Cost", "Gateway rate limits, upstream provider limits, and cost guard"],
    settings: ["Settings", "Runtime, database, configuration"],
  };

  const [title, subtitle] = titles[panelId] || [panelId, ""];
  setText("pageTitle", title);
  setText("pageSubtitle", subtitle);
  window.location.hash = panelId;

  // Close mobile nav
  $("sidebar").classList.remove("open");
  $("mobileNavOverlay").classList.add("hidden");

  // Render page-specific
  if (panelId === "providers") renderProviders();
  if (panelId === "combos") renderCombos();
  if (panelId === "aliases") renderAliases();
  if (panelId === "apikeys") renderApiKeys();
  if (panelId === "limits") renderLimits();
  if (panelId === "settings") renderSettings();
}

window.addEventListener("hashchange", () => navigateTo(readHash()));

document.querySelectorAll(".nav-item").forEach(btn => {
  btn.addEventListener("click", () => { if (btn.dataset.panel) navigateTo(btn.dataset.panel); });
});

/* ── Mobile Nav ──────────────────────────────────────────────────────────── */

$("mobileMenuBtn").addEventListener("click", () => {
  $("sidebar").classList.toggle("open");
  $("mobileNavOverlay").classList.toggle("hidden");
});

$("mobileNavOverlay").addEventListener("click", () => {
  $("sidebar").classList.remove("open");
  $("mobileNavOverlay").classList.add("hidden");
});

/* ── Quick Actions ───────────────────────────────────────────────────────── */

document.addEventListener("click", (e) => {
  const action = e.target.closest("[data-action]");
  if (!action) return;
  const act = action.dataset.action;
  if (act === "nav-to") navigateTo(action.dataset.panel);
  if (act === "add-provider") { navigateTo("providers"); resetProviderForm(); openDrawer("drawer-provider"); }
  if (act === "add-apikey") { navigateTo("apikeys"); resetApiKeyForm(); openDrawer("drawer-apikey"); }
  if (act === "add-combo") { navigateTo("combos"); resetComboForm(); openDrawer("drawer-combo"); }
  if (act === "run-doctor") runDoctor();
});

/* ── Refresh Data ────────────────────────────────────────────────────────── */

async function refresh() {
  setPill("healthPill", "Checking…", "muted");
  state.errors = {};

  const endpoints = [
    ["health", "/health"],
    ["version", "/version"],
    ["runtime", "/api/runtime/status"],
    ["models", "/v1/models"],
    ["setupConfig", "/api/setup/config"],
    ["apiKeys", "/api/setup/api-keys"],
    ["providers", "/api/setup/providers"],
    ["providerCatalog", "/api/setup/provider-catalog"],
    ["providerLimits", "/api/provider-limits"],
    ["combos", "/api/setup/combos"],
    ["aliases", "/api/setup/aliases"],
  ];

  const results = await Promise.allSettled(
    endpoints.map(([, path]) => getJson(path))
  );

  results.forEach((r, i) => {
    if (r.status === "fulfilled") {
      state[endpoints[i][0]] = r.value;
    } else {
      state.errors[endpoints[i][0]] = r.reason?.message || "Failed";
    }
  });

  render();
}

function render() {
  const runtime = state.runtime || {};
  const config = runtime.config || {};
  const storage = runtime.storage || {};
  const savedSettings = state.setupConfig?.settings || {};

  setText("versionLabel", state.version?.version || "0.1.0");
  setText("endpointLabel", `${config.host || "127.0.0.1"}:${config.port || ""}`);
  setText("uptimeLabel", fmtSec(runtime.uptime_seconds));

  // Health pill
  const healthOk = state.health?.status === "ok";
  setPill("healthPill", healthOk ? "Online" : "Degraded", healthOk ? "ok" : "warn");

  // Nav status dot
  const dot = $("navStatusDot");
  if (dot) {
    dot.textContent = "";
    dot.className = "nav-badge";
    dot.appendChild(el("span", { className: "status-dot " + (healthOk ? "ok" : "warn") }));
  }

  renderOverview();
}

/* ═══════════════════════════════════════════════════════════════════════════
   Overview Dashboard
   ═══════════════════════════════════════════════════════════════════════════ */

function renderOverview() {
  const runtime = state.runtime || {};
  const config = runtime.config || {};
  const providers = Array.isArray(state.providers?.data) ? state.providers.data : [];
  const limits = state.providerLimits || {};
  const metrics = limits.metrics || {};
  const costGuard = limits.cost_guard || {};

  // System Status tiles
  const ss = $("dashSystemStatus");
  if (!ss) return;

  const healthOk = state.health?.status === "ok";
  const dbOk = state.health?.database?.ok !== false;

  ss.innerHTML = "";
  ss.appendChild(metricTile("Gateway", healthOk ? "Online" : "Degraded", healthOk ? "ok" : "warn"));
  ss.appendChild(metricTile("Readiness", dbOk ? "Ready" : "Not Ready", dbOk ? "ok" : "warn"));
  ss.appendChild(metricTile("Uptime", fmtSec(runtime.uptime_seconds)));
  ss.appendChild(metricTile("Version", state.version?.version || "—"));

  // Alerts
  const alerts = $("dashAlerts");
  alerts.innerHTML = "";
  alerts.classList.add("hidden");
  const alertItems = [];

  if (providers.length === 0) {
    alertItems.push({ severity: "warn", title: "No providers configured", desc: "Add a provider connection to start routing AI requests.", action: { panel: "providers", label: "Add Provider" } });
  }
  if (providers.every(p => !p.is_active)) {
    alertItems.push({ severity: "error", title: "No active providers", desc: "All providers are disabled. Enable at least one.", action: { panel: "providers", label: "View Providers" } });
  }
  if (Object.keys(runtime).length && !(state.setupConfig?.settings?.requireApiKey)) {
    alertItems.push({ severity: "warn", title: "API key protection disabled", desc: "/v1 routes are open without authentication.", action: { panel: "settings", label: "Enable in Settings" } });
  }
  const costEnabled = costGuard.enabled ?? true;
  const dailyBudget = costGuard.daily_budget_usd || costGuard.dailyBudgetUsd || 0;
  if (costEnabled && dailyBudget > 0) {
    const dailyCost = Number(metrics.total_cost_usd || metrics.totalCostUsd || 0);
    if (dailyCost >= dailyBudget) {
      alertItems.push({ severity: "error", title: "Daily budget exceeded", desc: `$${dailyCost.toFixed(2)} / $${dailyBudget}`, action: { panel: "limits", label: "View Limits" } });
    } else if (dailyCost >= dailyBudget * 0.9) {
      alertItems.push({ severity: "warn", title: "Approaching daily budget", desc: `$${dailyCost.toFixed(2)} / $${dailyBudget}`, action: { panel: "limits", label: "View Limits" } });
    }
  }

  if (alertItems.length) {
    alerts.classList.remove("hidden");
    alertItems.forEach(a => {
      const card = el("div", { className: "alert-card " + a.severity },
        el("div", { className: "alert-card-body" },
          el("div", { className: "alert-card-title", textContent: a.title }),
          a.desc ? el("div", { className: "alert-card-desc", textContent: a.desc }) : null,
        ),
      );
      if (a.action) {
        card.appendChild(el("button", {
          className: "btn btn-sm btn-ghost",
          textContent: a.action.label,
          onclick() { navigateTo(a.action.panel); }
        }));
      }
      alerts.appendChild(card);
    });
  }

  // Provider overview
  const dp = $("dashProviders");
  dp.innerHTML = "";
  if (providers.length === 0) {
    dp.appendChild(el("div", { className: "empty-state" },
      el("div", { className: "empty-state-icon", textContent: "⬡" }),
      el("h3", { textContent: "No providers yet" }),
      el("p", { textContent: "Add an AI provider to begin routing requests." }),
      el("button", { className: "btn btn-primary", textContent: "+ Add Provider", onclick() { navigateTo("providers"); resetProviderForm(); openDrawer("drawer-provider"); } })
    ));
  } else {
    providers.slice(0, 6).forEach(p => {
      const active = p.is_active !== false;
      const row = el("div", { className: "data-row" },
        el("div", { className: "data-row-info" },
          el("strong", { textContent: p.name || p.provider }),
          el("div", { className: "data-row-meta" },
            el("span", { className: "pill " + (active ? "ok" : "muted"), textContent: active ? "Active" : "Disabled" }),
            el("span", { textContent: p.email || "" }),
          ),
        ),
        el("div", { className: "data-row-actions" },
          el("button", { className: "btn btn-sm btn-ghost", textContent: "Test", onclick() { testProvider(p.id || p.provider); } }),
        )
      );
      dp.appendChild(row);
    });
    setText("navProvidersBadge", providers.length);
  }

  // Traffic & Cost
  const traffic = $("dashTraffic");
  traffic.innerHTML = "";
  traffic.appendChild(metricTile("Requests", fmtNum(metrics.total_requests || 0)));
  traffic.appendChild(metricTile("Prompt Tokens", fmtNum(metrics.total_prompt_tokens || 0)));
  traffic.appendChild(metricTile("Completion Tokens", fmtNum(metrics.total_completion_tokens || 0)));

  const cost = $("dashCost");
  cost.innerHTML = "";
  const totalCost = Number(metrics.total_cost_usd || metrics.totalCostUsd || 0);
  cost.appendChild(metricTile("Today", fmtUsd(totalCost)));

  const monthlyCost = Number(metrics.monthly_cost_usd || metrics.monthlyCostUsd || 0);
  const monthlyBudget = costGuard.monthly_budget_usd || costGuard.monthlyBudgetUsd || 0;
  if (monthlyBudget > 0) {
    const pct = Math.min(100, (monthlyCost / monthlyBudget) * 100);
    cost.appendChild(el("div", { className: "field" },
      el("span", { textContent: "Month $" + monthlyCost.toFixed(2) + " / $" + monthlyBudget }),
      el("div", { className: "progress" },
        el("div", { className: "progress-bar " + (pct >= 90 ? "danger" : pct >= 70 ? "warn" : ""), style: { width: pct + "%" } })
      ),
    ));
  } else {
    cost.appendChild(metricTile("Month", fmtUsd(monthlyCost)));
  }
}

function metricTile(label, value, tone) {
  const cls = tone ? "metric-tile " + tone : "metric-tile";
  return el("div", { className: cls },
    el("span", { textContent: label }),
    el("strong", { textContent: String(value) }),
  );
}

/* ═══════════════════════════════════════════════════════════════════════════
   Providers
   ═══════════════════════════════════════════════════════════════════════════ */

function providerData() {
  return Array.isArray(state.providers?.data) ? state.providers.data : [];
}

function providerCatalog() {
  return Array.isArray(state.providerCatalog) ? state.providerCatalog : [];
}

function renderProviders() {
  const list = $("providersList");
  if (!list) return;
  list.innerHTML = "";

  const providers = providerData();
  setText("navProvidersBadge", providers.length);

  if (!providers.length) {
    list.appendChild(el("div", { className: "empty-state" },
      el("div", { className: "empty-state-icon", textContent: "⬡" }),
      el("h3", { textContent: "No providers configured" }),
      el("p", { textContent: "Add your first provider connection to begin routing AI requests." }),
      el("button", { className: "btn btn-primary", textContent: "+ Add Provider", onclick() { resetProviderForm(); openDrawer("drawer-provider"); } })
    ));
    return;
  }

  // Search filter
  const searchTerm = ($("providerSearchInput")?.value || "").toLowerCase();

  providers.filter(p => {
    if (!searchTerm) return true;
    const s = (p.name || p.provider || "").toLowerCase();
    return s.includes(searchTerm);
  }).forEach(p => {
    const active = p.is_active !== false;
    const auth = p.auth_type || p.data?.authType || "api-key";
    list.appendChild(providerRow(p, active, auth));
  });
}

function providerRow(p, active, auth) {
  const id = p.id || p.provider;
  const name = p.name || p.provider || "—";
  const email = p.email || "";

  return el("div", { className: "data-row" },
    el("div", { className: "data-row-info" },
      el("strong", { textContent: name }),
      el("div", { className: "data-row-meta" },
        el("span", { className: "pill " + (active ? "ok" : "muted"), textContent: active ? "Active" : "Disabled" }),
        email ? el("span", { textContent: email }) : null,
        el("span", { textContent: auth }),
        p.priority != null ? el("span", { textContent: "Priority " + p.priority }) : null,
        p.email ? el("span", { className: "provider-email", textContent: p.email }) : null,
      ),
    ),
    el("div", { className: "data-row-actions" },
      el("button", { className: "btn btn-sm btn-secondary", textContent: "Edit", onclick() { editProvider(p); } }),
      el("button", { className: "btn btn-sm btn-secondary", textContent: "Test", onclick() { testProvider(id); } }),
      el("button", { className: "btn btn-sm btn-ghost", textContent: active ? "Disable" : "Enable", onclick() { toggleProvider(id, !active); } }),
      el("button", { className: "btn btn-sm btn-danger", textContent: "Delete", onclick() { deleteProvider(id, name); } }),
      el("button", { className: "btn btn-sm btn-ghost", textContent: "Models", onclick() { fetchModelsFor(id); } }),
    ),
  );
}

function resetProviderForm() {
  $("providerEditIdInput").value = "";
  $("providerPresetInput").value = "";
  $("providerInput").value = "";
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
  $("providerActiveInput").checked = true;
  $("providerDataInput").value = "";
  $("authTypeInput").value = "api-key";
  $("providerSubmitButton").textContent = "Add Provider";
  $("providerCancelEditButton").classList.add("hidden");
  setText("drawerProviderTitle", "Add Provider");
  $("oauthNotice").classList.add("hidden");
  $("oauthManual").classList.add("hidden");
  $("providerTestNotice").classList.add("hidden");
  $("oauthGithubButton").style.display = "none";
  renderAuthFields();
}

function editProvider(p) {
  resetProviderForm();
  $("providerEditIdInput").value = p.id || p.provider;
  $("providerInput").value = p.provider || "";
  $("providerBaseUrlInput").value = p.data?.baseUrl || p.baseUrl || p.base_url || "";
  $("providerFormatInput").value = p.data?.format || p.format || "";
  $("providerNameInput").value = p.name || "";
  $("providerEmailInput").value = p.email || "";
  $("providerPriorityInput").value = p.priority ?? "";
  $("providerActiveInput").checked = p.is_active !== false;
  $("providerDataInput").value = JSON.stringify(p.data || {}, null, 2);
  $("authTypeInput").value = p.auth_type || p.data?.authType || "api-key";
  if (p.data?.apiKey) $("providerApiKeyInput").value = p.data.apiKey;
  if (p.data?.username) $("providerBasicUserInput").value = p.data.username;
  if (p.data?.password) $("providerBasicPassInput").value = p.data.password;
  if (p.data?.headerName) $("providerHeaderNameInput").value = p.data.headerName;
  if (p.data?.headerValue) $("providerHeaderValueInput").value = p.data.headerValue;
  $("providerSubmitButton").textContent = "Save Provider";
  $("providerCancelEditButton").classList.remove("hidden");
  setText("drawerProviderTitle", "Edit Provider");
  renderAuthFields();
  openDrawer("drawer-provider");
}

function renderAuthFields() {
  const authType = $("authTypeInput").value;
  const isOAuth = authType === "oauth";
  $("apiKeyField").style.display = ["api-key","bearer","oauth"].includes(authType) ? "" : "none";
  $("providerApiKeyInput").type = ["api-key","bearer","oauth"].includes(authType) ? "password" : "text";
  $("basicUserField").style.display = authType === "basic" ? "" : "none";
  $("basicPassField").style.display = authType === "basic" ? "" : "none";
  $("headerNameField").style.display = authType === "header" ? "" : "none";
  $("headerValueField").style.display = authType === "header" ? "" : "none";
  $("oauthGithubButton").style.display = isOAuth ? "" : "none";
  const apiKeyLabel = $("apiKeyField")?.querySelector("span");
  if (apiKeyLabel) apiKeyLabel.textContent = isOAuth ? "OAuth Token" : "API Key / Token";
}

$("authTypeInput").addEventListener("change", renderAuthFields);

$("providerSearchInput")?.addEventListener("input", () => renderProviders());

function buildProviderPayload() {
  const authType = $("authTypeInput").value;
  let data = {};
  try { data = JSON.parse($("providerDataInput").value || "{}"); } catch {}

  data.authType = authType;
  const baseUrl = $("providerBaseUrlInput").value.trim();
  const format = $("providerFormatInput").value.trim();
  if (baseUrl) data.baseUrl = baseUrl;
  if (format) data.format = format;
  if (["api-key","bearer"].includes(authType)) data.apiKey = $("providerApiKeyInput").value;
  if (authType === "oauth") data.apiKey = $("providerApiKeyInput").value;
  if (authType === "basic") { data.username = $("providerBasicUserInput").value; data.password = $("providerBasicPassInput").value; }
  if (authType === "header") { data.headerName = $("providerHeaderNameInput").value; data.headerValue = $("providerHeaderValueInput").value; }

  return {
    id: $("providerEditIdInput").value || null,
    provider: $("providerInput").value.trim(),
    name: $("providerNameInput").value.trim() || $("providerInput").value.trim(),
    email: $("providerEmailInput").value.trim() || null,
    auth_type: authType,
    is_active: $("providerActiveInput").checked,
    priority: parseInt($("providerPriorityInput").value) || null,
    data,
  };
}

async function saveProvider() {
  const payload = buildProviderPayload();
  if (!payload.provider) { toast("Provider name is required", "error"); return; }
  try {
    if (payload.id) {
      await sendJson(`/api/setup/providers/${payload.id}`, "PUT", payload);
      toast("Provider updated", "success");
    } else {
      await sendJson("/api/setup/providers", "POST", payload);
      toast("Provider added", "success");
    }
    closeDrawer("drawer-provider");
    await refresh();
    renderProviders();
  } catch (e) { toast(e.message, "error"); }
}

$("providerSubmitButton").addEventListener("click", (e) => { e.preventDefault(); saveProvider(); });

$("providerCancelEditButton").addEventListener("click", () => {
  closeDrawer("drawer-provider");
  resetProviderForm();
});

async function testProvider(id) {
  try {
    await sendJson(`/api/setup/providers/${encodeURIComponent(id)}/test`, "POST");
    toast("Connection test passed for " + id, "success");
  } catch (e) { toast("Test failed: " + e.message, "error"); }
}

async function toggleProvider(id, active) {
  try {
    await sendJson(`/api/setup/providers/${encodeURIComponent(id)}/toggle`, "PUT", { is_active: active });
    toast(active ? "Provider enabled" : "Provider disabled", "success");
    await refresh();
    renderProviders();
  } catch (e) { toast(e.message, "error"); }
}

async function deleteProvider(id, name) {
  showConfirm("Delete Provider", `Delete "${name}"? This cannot be undone.`, "Delete", async () => {
    try {
      await sendJson(`/api/setup/providers/${encodeURIComponent(id)}`, "DELETE");
      toast(`${name} deleted`, "success");
      await refresh();
      renderProviders();
    } catch (e) { toast(e.message, "error"); }
  });
}

async function fetchModelsFor(id) {
  try {
    toast("Fetching models for " + id + "…", "info");
    await sendJson(`/api/setup/providers/${encodeURIComponent(id)}/models`, "POST", {});
    toast("Models updated for " + id, "success");
    await refresh();
    renderProviders();
  } catch (e) { toast("Fetch failed: " + e.message, "error"); }
}

/* ── Provider Presets ────────────────────────────────────────────────────── */

function populatePresets() {
  const sel = $("providerPresetInput");
  if (!sel) return;
  sel.innerHTML = '<option value="">— Manual config —</option>';
  const catalog = providerCatalog();
  catalog.forEach(c => {
    sel.appendChild(el("option", { value: c.id, textContent: c.name || c.id }));
  });
}

$("providerPresetInput").addEventListener("change", function () {
  const v = this.value; if (!v) return;
  const catalog = providerCatalog();
  const c = catalog.find(x => x.id === v);
  if (!c) return;
  $("providerInput").value = c.id || "";
  $("providerBaseUrlInput").value = c.base_url || c.baseUrl || "";
  $("providerFormatInput").value = c.format || "";
  $("authTypeInput").value = c.auth_type || c.authType || "api-key";
  renderAuthFields();
});

/* ═══════════════════════════════════════════════════════════════════════════
   Combos
   ═══════════════════════════════════════════════════════════════════════════ */

function comboData() { return Array.isArray(state.combos?.data) ? state.combos.data : []; }

function renderCombos() {
  const list = $("combosList");
  if (!list) return;
  list.innerHTML = "";
  const combos = comboData();
  setText("navCombosBadge", combos.length);

  if (!combos.length) {
    list.appendChild(el("div", { className: "empty-state" },
      el("div", { className: "empty-state-icon", textContent: "⌘" }),
      el("h3", { textContent: "No combos yet" }),
      el("p", { textContent: "Create a combo to chain fallback models." }),
      el("button", { className: "btn btn-primary", textContent: "+ Create Combo", onclick() { resetComboForm(); openDrawer("drawer-combo"); } })
    ));
    return;
  }

  combos.forEach(c => {
    const models = Array.isArray(c.models) ? c.models : [];
    list.appendChild(el("div", { className: "data-row" },
      el("div", { className: "data-row-info" },
        el("strong", { textContent: c.name || c.id }),
        el("div", { className: "data-row-meta" },
          el("span", { textContent: models.length + " models" }),
          c.kind ? el("span", { textContent: c.kind }) : null,
        ),
        models.length ? el("div", { className: "model-stack" },
          ...models.map(m => el("span", { className: "model-chip", textContent: m }))
        ) : null,
      ),
      el("div", { className: "data-row-actions" },
        el("button", { className: "btn btn-sm btn-secondary", textContent: "Edit", onclick() { editCombo(c); } }),
        el("button", { className: "btn btn-sm btn-danger", textContent: "Delete", onclick() { deleteCombo(c); } }),
      ),
    ));
  });
}

function resetComboForm() {
  $("comboEditIdInput").value = "";
  $("comboNameInput").value = "";
  $("comboKindInput").value = "llm";
  $("comboProviderInput").innerHTML = '<option value="">All providers</option>';
  comboModelDraft = [];
  comboProviderFilter = "";
  comboSelectedModels.clear();
  $("comboSubmitButton").textContent = "Create Combo";
  $("comboCancelEditButton").classList.add("hidden");
  setText("drawerComboTitle", "Create Combo");
}

async function ensureModels() {
  if (providerData().some(provider => providerModelIds(provider).length)) return;
  try {
    state.providers = await getJson("/api/setup/providers");
  } catch (_) { state.providers = { data: [] }; }
}

function providerModelIds(provider) {
  const models = provider?.data?.models;
  if (!Array.isArray(models)) return [];
  return models.map(model => {
    if (typeof model === "string") return model.trim();
    if (model && typeof model === "object") {
      return String(model.id || model.name || model.model || "").trim();
    }
    return "";
  }).filter(Boolean);
}

function connectedProviderModels() {
  const connections = providerData();
  const selectedConnection = comboProviderFilter
    ? connections.filter(provider => (provider.id || provider.provider) === comboProviderFilter)
    : connections;
  const seen = new Set();
  const choices = [];

  selectedConnection.forEach(provider => {
    const providerType = provider.provider || "provider";
    const connectionId = provider.id || providerType;
    const connectionLabel = provider.name || provider.email || providerType;
    providerModelIds(provider).forEach(model => {
      // Provider model catalogs contain native upstream IDs. Some native IDs
      // include a vendor prefix (for example CommandCode's `tencent/Hy3`), so
      // they still need the BlackRouter provider prefix for routing.
      const value = providerType + "/" + model;
      if (seen.has(value)) return;
      seen.add(value);
      choices.push({
        id: value,
        owned_by: providerType,
        connection_id: connectionId,
        connection_label: connectionLabel,
      });
    });
  });

  return choices;
}

function openComboDrawer() {
  resetComboForm();
  populateComboProviders();
  renderComboDraft();
  renderComboAvailable();
  openDrawer("drawer-combo");
  ensureModels().then(() => { renderComboAvailable(); });
}

function editCombo(c) {
  resetComboForm();
  populateComboProviders();
  $("comboEditIdInput").value = c.id || c.name;
  $("comboNameInput").value = c.name || "";
  $("comboKindInput").value = c.kind || "llm";
  comboModelDraft = Array.isArray(c.models) ? [...c.models] : [];
  $("comboSubmitButton").textContent = "Save Combo";
  $("comboCancelEditButton").classList.remove("hidden");
  setText("drawerComboTitle", "Edit Combo");
  renderComboDraft();
  renderComboAvailable();
  openDrawer("drawer-combo");
  ensureModels().then(() => { renderComboAvailable(); });
}

function renderComboDraft() {
  const root = $("comboModelsInput");
  const hint = $("comboEmptyHint");
  if (!root) return;
  root.innerHTML = "";
  if (!comboModelDraft.length) {
    if (hint) hint.style.display = "";
    return;
  }
  if (hint) hint.style.display = "none";
  comboModelDraft.forEach((m, i) => {
    root.appendChild(el("div", { className: "model-draft-row" },
      el("span", { className: "model-draft-index", textContent: String(i + 1) }),
      el("span", { className: "model-draft-name", textContent: m }),
      el("div", { className: "btn-row", style: { gap: "2px" } },
        el("button", { className: "btn btn-sm btn-ghost", textContent: "↑", title: "Move up", "aria-label": "Move up",
          disabled: i === 0,
          onclick() { if (i > 0) { const t = comboModelDraft[i]; comboModelDraft[i] = comboModelDraft[i-1]; comboModelDraft[i-1] = t; renderComboDraft(); } }
        }),
        el("button", { className: "btn btn-sm btn-ghost", textContent: "↓", title: "Move down", "aria-label": "Move down",
          disabled: i === comboModelDraft.length - 1,
          onclick() { if (i < comboModelDraft.length - 1) { const t = comboModelDraft[i]; comboModelDraft[i] = comboModelDraft[i+1]; comboModelDraft[i+1] = t; renderComboDraft(); } }
        }),
        el("button", { className: "btn btn-sm btn-danger", textContent: "✕", title: "Remove", "aria-label": "Remove",
          onclick() { comboModelDraft.splice(i, 1); renderComboDraft(); renderComboAvailable(); }
        }),
      ),
    ));
  });
}

let comboSelectedModels = new Set();

function renderComboAvailable() {
  const list = $("comboModelsList");
  if (!list) return;
  list.innerHTML = "";

  const models = connectedProviderModels();
  const filtered = models;

  if (!models.length) {
    list.appendChild(el("p", {
      className: "text-muted text-sm",
      textContent: comboProviderFilter
        ? "This provider connection has no saved models. Use its Models button first."
        : "Connected providers have no saved models yet. Open Providers and fetch models first."
    }));
    return;
  }

  const remaining = filtered.filter(m => !comboModelDraft.includes(m.id));
  const selectedCount = remaining.filter(m => comboSelectedModels.has(m.id)).length;

  list.appendChild(el("div", { className: "combo-available-actions" },
    el("button", {
      className: "btn btn-sm btn-primary",
      textContent: selectedCount ? "Add selected (" + selectedCount + ")" : "Add all (" + remaining.length + ")",
      disabled: !remaining.length,
      onclick() {
        const toAdd = selectedCount ? remaining.filter(m => comboSelectedModels.has(m.id)) : remaining;
        toAdd.forEach(m => { if (!comboModelDraft.includes(m.id)) comboModelDraft.push(m.id); });
        comboSelectedModels.clear();
        renderComboDraft();
        renderComboAvailable();
      }
    }),
    el("span", {
      className: "text-muted text-sm",
      textContent: comboProviderFilter
        ? "Models from " + (filtered[0]?.connection_label || "selected connection")
        : "Models from all connected providers"
    }),
  ));

  filtered.slice(0, 60).forEach(m => {
    const added = comboModelDraft.includes(m.id);
    const checked = comboSelectedModels.has(m.id);
    const row = el("div", { className: "data-row selectable" + (added ? " added" : "") },
      el("label", { className: "combo-model-select" },
        el("input", { type: "checkbox", disabled: added, checked: checked && !added,
          onchange(e) {
            if (e.target.checked) comboSelectedModels.add(m.id);
            else comboSelectedModels.delete(m.id);
            renderComboAvailable();
          }
        }),
      ),
      el("div", { className: "data-row-info" },
        el("strong", { textContent: m.id }),
        el("div", { className: "data-row-meta" },
          el("span", { className: "pill muted", textContent: m.owned_by || "unknown" }),
          el("span", { textContent: m.connection_label || "" }),
        ),
      ),
      el("div", { className: "data-row-actions" },
        el("button", {
          className: "btn btn-sm " + (added ? "btn-ghost" : "btn-primary"),
          textContent: added ? "✓" : "+ Add",
          "aria-label": added ? "Model added" : "Add " + m.id,
          disabled: added,
          onclick() {
            if (!comboModelDraft.includes(m.id)) {
              comboModelDraft.push(m.id);
              comboSelectedModels.delete(m.id);
              renderComboDraft();
              renderComboAvailable();
            }
          }
        }),
      ),
    );
    list.appendChild(row);
  });
}

function populateComboProviders() {
  const sel = $("comboProviderInput");
  if (!sel) return;
  sel.innerHTML = '<option value="">All connected providers</option>';
  providerData().forEach(provider => {
    const id = provider.id || provider.provider;
    const account = provider.email || provider.name || id;
    const count = providerModelIds(provider).length;
    sel.appendChild(el("option", {
      value: id,
      textContent: (provider.provider || "provider") + " — " + account + " (" + count + " models)"
    }));
  });
}

$("comboProviderInput")?.addEventListener("change", function () {
  comboProviderFilter = this.value;
  comboSelectedModels.clear();
  renderComboAvailable();
  const list = $("comboModelsList");
  if (list) list.scrollIntoView({ behavior: "smooth", block: "nearest" });
});

$("comboAddManualButton")?.addEventListener("click", () => {
  const val = $("comboModelManualInput").value.trim();
  if (!val) return;
  if (comboModelDraft.includes(val)) { toast("Model already in combo", "warn"); return; }
  comboModelDraft.push(val);
  $("comboModelManualInput").value = "";
  renderComboDraft();
});

async function saveCombo() {
  const name = $("comboNameInput").value.trim();
  if (!name) { toast("Combo name is required", "error"); return; }
  if (!comboModelDraft.length) { toast("At least one model required", "error"); return; }

  const id = $("comboEditIdInput").value;
  const payload = {
    name,
    kind: $("comboKindInput").value,
    models: comboModelDraft,
  };

  try {
    if (id) {
      payload.id = id;
      await sendJson(`/api/setup/combos/${encodeURIComponent(id)}`, "PUT", payload);
      toast("Combo updated", "success");
    } else {
      await sendJson("/api/setup/combos", "POST", payload);
      toast("Combo created", "success");
    }
    closeDrawer("drawer-combo");
    await refresh();
    renderCombos();
  } catch (e) { toast(e.message, "error"); }
}

$("comboSubmitButton").addEventListener("click", (e) => { e.preventDefault(); saveCombo(); });
$("comboCancelEditButton").addEventListener("click", () => { closeDrawer("drawer-combo"); resetComboForm(); });

async function deleteCombo(c) {
  showConfirm("Delete Combo", `Delete "${c.name || c.id}"?`, "Delete", async () => {
    try {
      await sendJson(`/api/setup/combos/${encodeURIComponent(c.id || c.name)}`, "DELETE");
      toast(`${c.name || c.id} deleted`, "success");
      await refresh();
      renderCombos();
    } catch (e) { toast(e.message, "error"); }
  });
}

$("addProviderBtn")?.addEventListener("click", () => { resetProviderForm(); populatePresets(); openDrawer("drawer-provider"); });
$("addComboBtn")?.addEventListener("click", () => { resetComboForm(); populateComboProviders(); renderComboAvailable(); openDrawer("drawer-combo"); });
