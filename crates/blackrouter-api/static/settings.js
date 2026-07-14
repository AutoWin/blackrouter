/* ═══════════════════════════════════════════════════════════════════════════
   BlackRouter Control Panel — Settings
   ═══════════════════════════════════════════════════════════════════════════ */

function renderSettings() {
  const runtime = state.runtime || {};
  const config = runtime.config || {};
  const storage = runtime.storage || {};
  const savedSettings = state.setupConfig?.settings || {};

  setValue("hostValue", config.host);
  setValue("portValue", config.port);
  setValue("dataDirValue", config.data_dir);
  setValue("databaseUrlValue", config.database_url);
  setValue("databasePathValue", storage.database_path || state.health?.database?.path);
  setValue("compatValue", fmtBool(config.compat_9router_db));

  setPill("databaseBadge",
    storage.schema_compatible ? "Compatible" : "Missing Tables",
    storage.schema_compatible ? "ok" : "warn");

  $("configRequireApiKey").checked = Boolean(savedSettings.requireApiKey);

  // Table counts
  const tableCounts = $("tableCounts");
  if (tableCounts) {
    tableCounts.innerHTML = "";
    const counts = storage.table_counts || {};
    Object.entries(counts).forEach(([table, count]) => {
      tableCounts.appendChild(el("div", { className: "data-row" },
        el("div", { className: "data-row-info" },
          el("strong", { textContent: table }),
        ),
        el("div", { className: "data-row-actions" },
          el("span", { className: "pill muted", textContent: fmtNum(count) + " rows" }),
        ),
      ));
    });
  }

  // Health probe
  const healthProbe = savedSettings.healthProbe || {};
  $("healthProbeEnabledInput").checked = healthProbe.enabled ?? true;
  setValue("healthProbeIntervalInput", healthProbe.intervalSeconds || 30);
  setValue("healthProbeTimeoutInput", healthProbe.timeoutSeconds || 5);
  setValue("healthProbeThresholdInput", healthProbe.failureThreshold || 3);

  // Cost guard
  const costGuard = savedSettings.costGuard || savedSettings.cost_guard || {};
  $("costGuardEnabledInput").checked = costGuard.enabled ?? true;
  setValue("costGuardDailyInput", costGuard.dailyBudgetUsd ?? "");
  setValue("costGuardMonthlyInput", costGuard.monthlyBudgetUsd ?? "");

  // Telegram
  const telegram = savedSettings.telegram || config.telegram || {};
  $("telegramEnabledInput").checked = Boolean(telegram.enabled);
  setValue("telegramAdminInput", Array.isArray(telegram.adminIds) ? telegram.adminIds.join(", ") : (telegram.adminIds || telegram.admin_ids || ""));
  $("telegramWebhookInput").checked = Boolean(telegram.useWebhook || telegram.webhook);
  setValue("telegramTtlInput", telegram.linkCodeTTL || telegram.link_code_ttl || 300);
  setValue("telegramWebhookUrlInput", telegram.webhookUrl || telegram.webhook_url || "");
}

$("configForm")?.addEventListener("submit", async (e) => {
  e.preventDefault();
  try {
    const settings = {
      ...(state.setupConfig?.settings || {}),
      requireApiKey: $("configRequireApiKey").checked,
      healthProbe: {
        enabled: $("healthProbeEnabledInput").checked,
        intervalSeconds: parseInt($("healthProbeIntervalInput").value) || 30,
        timeoutSeconds: parseInt($("healthProbeTimeoutInput").value) || 5,
        failureThreshold: parseInt($("healthProbeThresholdInput").value) || 3,
      },
      telegram: {
        enabled: $("telegramEnabledInput").checked,
        adminIds: parseAdminIds($("telegramAdminInput").value),
        useWebhook: $("telegramWebhookInput").checked,
        linkCodeTTL: parseInt($("telegramTtlInput").value) || 300,
        webhookUrl: $("telegramWebhookUrlInput").value.trim() || null,
      },
    };
    await sendJson("/api/setup/config", "POST", { settings });
    toast("Configuration saved", "success");
    state.setupConfig = await getJson("/api/setup/config");
    renderSettings();
  } catch (e) { toast(e.message, "error"); }
});

/* ── Config Versions ────────────────────────────────────────────────────── */

$("loadVersionsBtn")?.addEventListener("click", () => {
  if (typeof loadConfigVersions === "function") loadConfigVersions();
});

/* ── Doctor ──────────────────────────────────────────────────────────────── */

async function runDoctor() {
  try {
    toast("Running diagnostics…", "info");
    const result = await getJson("/api/doctor");
    const ok = result.status === "ok";
    if (ok) {
      toast("All checks passed", "success");
    } else {
      const issues = Array.isArray(result.issues) ? result.issues : [];
      toast(issues.length + " issue(s) found", "warn");
    }
  } catch (e) { toast("Doctor failed: " + e.message, "error"); }
}

/* ── Global & sidebar refresh buttons ────────────────────────────────────── */

$("globalRefreshBtn")?.addEventListener("click", refresh);
$("sidebarRefresh")?.addEventListener("click", refresh);

/* ── Boot ─────────────────────────────────────────────────────────────────── */

(async function boot() {
  try {
    await refresh();
    navigateTo(readHash());
  } catch (e) {
    console.error("Boot failed:", e);
    setPill("healthPill", "Offline", "error");
  }
})();
