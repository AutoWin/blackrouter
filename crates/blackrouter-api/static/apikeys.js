/* ═══════════════════════════════════════════════════════════════════════════
   BlackRouter Control Panel — API Keys
   ═══════════════════════════════════════════════════════════════════════════ */

function apiKeyData() { return Array.isArray(state.apiKeys?.data) ? state.apiKeys.data : []; }

function renderApiKeys() {
  const list = $("apiKeysList");
  if (!list) return;
  list.innerHTML = "";
  const keys = apiKeyData();
  setText("navApiKeysBadge", keys.length);

  if (!keys.length) {
    list.appendChild(el("div", { className: "empty-state" },
      el("div", { className: "empty-state-icon", textContent: "🔑" }),
      el("h3", { textContent: "No API keys yet" }),
      el("p", { textContent: "Create API keys to access gateway routes." }),
      el("button", { className: "btn btn-primary", textContent: "+ Create API Key",
        onclick() { resetApiKeyForm(); openDrawer("drawer-apikey"); }
      })
    ));
    return;
  }

  keys.forEach(k => {
    const masked = "br-" + (k.id || "").toString().substring(0, 4) + "..." + (k.id || "").toString().substring((k.id || "").length - 4);
    const limits = Array.isArray(k.limits) ? k.limits : [];
    const dailyRpm = limits.find(l => l.period === "daily" && l.type === "rpm");
    const dailyTpm = limits.find(l => l.period === "daily" && l.type === "tpm");

    list.appendChild(el("div", { className: "data-row" },
      el("div", { className: "data-row-info" },
        el("strong", { textContent: k.name || k.id }),
        el("div", { className: "data-row-meta" },
          el("span", { className: "model-chip", textContent: masked }),
          k.tenant ? el("span", { textContent: "Tenant: " + k.tenant }) : null,
          dailyRpm?.quota ? el("span", { textContent: fmtNum(dailyRpm.quota) + " req/day" }) : null,
          dailyTpm?.quota ? el("span", { textContent: fmtNum(dailyTpm.quota) + " tok/day" }) : null,
        ),
      ),
      el("div", { className: "data-row-actions" },
        el("button", { className: "btn btn-sm btn-secondary", textContent: "Edit",
          onclick() { editApiKey(k); }
        }),
        el("button", { className: "btn btn-sm btn-ghost", textContent: "Rotate",
          onclick() { rotateApiKey(k); }
        }),
      ),
    ));
  });
}

function resetApiKeyForm() {
  $("apiKeyEditIdInput").value = "";
  $("apiKeyNameInput").value = "";
  $("apiKeyMachineInput").value = "";
  $("apiKeyTenantInput").value = "";
  $("apiKeyRequestsInput").value = "";
  $("apiKeyTokensInput").value = "";
  $("apiKeyCostInput").value = "";
  $("apiKeyProvidersInput").value = "";
  $("apiKeyModelsInput").value = "";
  $("apiKeySubmitButton").textContent = "Create API Key";
  $("apiKeyCancelEditButton").classList.add("hidden");
  setText("drawerApiKeyTitle", "Create API Key");
}

function editApiKey(k) {
  resetApiKeyForm();
  $("apiKeyEditIdInput").value = k.id || "";
  $("apiKeyNameInput").value = k.name || "";
  $("apiKeyMachineInput").value = k.machineId || k.machine_id || "";
  $("apiKeyTenantInput").value = k.tenant || "";
  const limits = Array.isArray(k.limits) ? k.limits : [];
  const rpm = limits.find(l => l.type === "rpm");
  const tpm = limits.find(l => l.type === "tpm");
  const cost = limits.find(l => l.type === "cost");
  if (rpm) $("apiKeyRequestsInput").value = rpm.quota || "";
  if (tpm) $("apiKeyTokensInput").value = tpm.quota || "";
  if (cost) $("apiKeyCostInput").value = cost.quota || "";
  $("apiKeyProvidersInput").value = Array.isArray(k.allowProviders) ? k.allowProviders.join(", ") : (k.allowProviders || "");
  $("apiKeyModelsInput").value = Array.isArray(k.allowModels) ? k.allowModels.join(", ") : (k.allowModels || "");
  $("apiKeySubmitButton").textContent = "Save API Key";
  $("apiKeyCancelEditButton").classList.remove("hidden");
  setText("drawerApiKeyTitle", "Edit API Key");
  openDrawer("drawer-apikey");
}

async function saveApiKey() {
  const name = $("apiKeyNameInput").value.trim();
  if (!name) { toast("API key name is required", "error"); return; }
  const id = $("apiKeyEditIdInput").value;
  const payload = {
    name,
    tenant: $("apiKeyTenantInput").value.trim() || null,
    machineId: $("apiKeyMachineInput").value.trim() || null,
    apiKey: null,
    allowProviders: csvValues($("apiKeyProvidersInput").value),
    allowModels: csvValues($("apiKeyModelsInput").value),
    limits: [],
  };
  const reqVal = numOrNull($("apiKeyRequestsInput").value);
  const tokVal = numOrNull($("apiKeyTokensInput").value);
  const costVal = numOrNull($("apiKeyCostInput").value);
  if (reqVal != null) payload.limits.push({ type: "rpm", quota: reqVal, period: "daily" });
  if (tokVal != null) payload.limits.push({ type: "tpm", quota: tokVal, period: "daily" });
  if (costVal != null) payload.limits.push({ type: "cost", quota: costVal });
  try {
    let result;
    if (id) {
      result = await sendJson("/api/setup/api-keys/" + encodeURIComponent(id), "PUT", payload);
      toast("API key updated", "success");
    } else {
      result = await sendJson("/api/setup/api-keys", "POST", payload);
      const rawKey = result.apiKey || result.token || result;
      if (rawKey && typeof rawKey === "string") {
        showSecretModal(rawKey);
      } else {
        toast("API key created", "success");
      }
    }
    closeDrawer("drawer-apikey");
    await refresh();
    renderApiKeys();
  } catch (e) { toast(e.message, "error"); }
}

function showSecretModal(rawKey) {
  setText("modalSecretText", rawKey);
  openModal("modal-secret");
}

$("modalSecretCopyBtn")?.addEventListener("click", async () => {
  try {
    await navigator.clipboard.writeText($("modalSecretText").textContent);
    toast("API key copied to clipboard", "success");
  } catch (_) { toast("Failed to copy", "error"); }
});

async function rotateApiKey(k) {
  showConfirm("Rotate API Key",
    "Rotating \"" + (k.name || k.id) + "\" will immediately invalidate the old key. Applications using the old key must be updated.",
    "Rotate", async () => {
      try {
        const result = await sendJson("/api/setup/api-keys/" + encodeURIComponent(k.id) + "/rotate", "POST", {});
        const rawKey = result.apiKey || result.token;
        if (rawKey && typeof rawKey === "string") showSecretModal(rawKey);
        toast("API key rotated", "success");
        await refresh();
        renderApiKeys();
      } catch (e) { toast(e.message, "error"); }
    });
}

$("apiKeySubmitButton").addEventListener("click", (e) => { e.preventDefault(); saveApiKey(); });
$("apiKeyCancelEditButton").addEventListener("click", () => { closeDrawer("drawer-apikey"); resetApiKeyForm(); });
$("addApiKeyBtn")?.addEventListener("click", () => { resetApiKeyForm(); openDrawer("drawer-apikey"); });
