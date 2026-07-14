/* ═══════════════════════════════════════════════════════════════════════════
   BlackRouter Control Panel — Limits & Cost
   ═══════════════════════════════════════════════════════════════════════════ */

let limitsLoading = false;

function renderLimits() {
  if (limitsLoading) return;
  const limits = state.providerLimits || {};
  const metrics = limits.metrics || {};
  const costGuard = limits.cost_guard || {};
  const data = Array.isArray(limits.data) ? limits.data : [];

  // Summary
  const sum = $("limitsSummary");
  if (sum) {
    sum.innerHTML = "";
    sum.appendChild(metricTile("Total Requests", fmtNum(metrics.total_requests || 0)));
    sum.appendChild(metricTile("Prompt Tokens", fmtNum(metrics.total_prompt_tokens || 0)));
    sum.appendChild(metricTile("Completion Tokens", fmtNum(metrics.total_completion_tokens || 0)));
    sum.appendChild(metricTile("Tracked Cost", fmtUsd(metrics.total_cost_usd || metrics.totalCostUsd || 0)));
  }

  // Cost guard summary
  const cg = $("costGuardSummary");
  if (cg) {
    cg.innerHTML = "";
    const enabled = costGuard.enabled !== false;
    cg.appendChild(metricTile("Cost Guard", enabled ? "Enabled" : "Disabled", enabled ? "ok" : "muted"));

    const dailyBudget = costGuard.daily_budget_usd || costGuard.dailyBudgetUsd || 0;
    if (dailyBudget > 0) {
      const dailyCost = Number(metrics.total_cost_usd || metrics.totalCostUsd || 0);
      const pct = Math.min(100, (dailyCost / dailyBudget) * 100);
      cg.appendChild(el("div", { className: "field" },
        el("span", { textContent: "Daily $" + dailyCost.toFixed(2) + " / $" + dailyBudget }),
        el("div", { className: "progress" },
          el("div", { className: "progress-bar " + (pct >= 90 ? "danger" : pct >= 70 ? "warn" : ""), style: { width: pct + "%" } }),
        ),
      ));
    }

    const monthlyBudget = costGuard.monthly_budget_usd || costGuard.monthlyBudgetUsd || 0;
    if (monthlyBudget > 0) {
      const monthlyCost = Number(metrics.monthly_cost_usd || metrics.monthlyCostUsd || 0);
      const mpct = Math.min(100, (monthlyCost / monthlyBudget) * 100);
      cg.appendChild(el("div", { className: "field" },
        el("span", { textContent: "Month $" + monthlyCost.toFixed(2) + " / $" + monthlyBudget }),
        el("div", { className: "progress" },
          el("div", { className: "progress-bar " + (mpct >= 90 ? "danger" : mpct >= 70 ? "warn" : ""), style: { width: mpct + "%" } }),
        ),
      ));
    }
  }

  // Provider limits
  const list = $("limitsList");
  if (!list) return;
  list.innerHTML = "";

  if (!data.length) {
    list.appendChild(el("p", { className: "text-muted text-sm", textContent: "No provider limit data available." }));
    return;
  }

  data.forEach(entry => {
    const fresh = entry.snapshot_freshness || "unknown";
    const freshTone = fresh === "fresh" ? "ok" : fresh === "stale" ? "warn" : "muted";
    list.appendChild(el("div", { className: "data-row" },
      el("div", { className: "data-row-info" },
        el("strong", { textContent: entry.provider || entry.id || "—" }),
        el("div", { className: "data-row-meta" },
          el("span", { className: "pill " + (entry.active !== false ? "ok" : "muted"), textContent: entry.active !== false ? "Active" : "Disabled" }),
          el("span", { className: "pill " + freshTone, textContent: "Snapshot: " + fresh }),
          entry.rpm_remaining != null ? el("span", { textContent: fmtNum(entry.rpm_remaining) + " / " + fmtNum(entry.rpm_limit) + " RPM" }) : null,
          entry.tpm_remaining != null ? el("span", { textContent: fmtNum(entry.tpm_remaining) + " / " + fmtNum(entry.tpm_limit) + " TPM" }) : null,
        ),
      ),
    ));
  });
}

async function refreshLimits() {
  limitsLoading = true;
  try {
    state.providerLimits = await getJson("/api/provider-limits");
    limitsLoading = false;
    renderLimits();
    setText("limitsUpdated", "Updated " + new Date().toLocaleTimeString());
  } catch (e) {
    limitsLoading = false;
    toast("Failed to load limits: " + e.message, "error");
  }
}

$("refreshLimitsBtn")?.addEventListener("click", refreshLimits);

$("costGuardForm")?.addEventListener("submit", async (e) => {
  e.preventDefault();
  const enabled = $("costGuardEnabledInput").checked;
  const daily = $("costGuardDailyInput").value ? parseFloat($("costGuardDailyInput").value) : null;
  const monthly = $("costGuardMonthlyInput").value ? parseFloat($("costGuardMonthlyInput").value) : null;
  try {
    await sendJson("/api/setup/config", "POST", {
      settings: {
        ...(state.setupConfig?.settings || {}),
        costGuard: { enabled, dailyBudgetUsd: daily, monthlyBudgetUsd: monthly },
      }
    });
    toast("Cost guard saved", "success");
    state.setupConfig = await getJson("/api/setup/config");
    renderLimits();
  } catch (e) { toast(e.message, "error"); }
});
