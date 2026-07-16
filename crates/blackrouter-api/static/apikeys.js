/* ═══════════════════════════════════════════════════════════════════════════
   BlackRouter Control Panel — API Keys
   ═══════════════════════════════════════════════════════════════════════════ */

function apiKeyData() { return Array.isArray(state.apiKeys?.data) ? state.apiKeys.data : []; }

function apiKeyPolicy(k) { return k?.policy && typeof k.policy === "object" ? k.policy : {}; }

function apiKeySecret(result) {
  if (typeof result === "string") return result;
  return result?.key || result?.apiKey || result?.token || null;
}

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
    const masked = k.key_masked || k.keyMasked || ("br-" + (k.id || "").toString().substring(0, 4) + "..." + (k.id || "").toString().slice(-4));
    const policy = apiKeyPolicy(k);
    const limits = Array.isArray(k.limits) ? k.limits : [];
    const dailyRpm = limits.find(l => l.period === "daily" && l.type === "rpm");
    const dailyTpm = limits.find(l => l.period === "daily" && l.type === "tpm");
    const requestsPerDay = policy.requests_per_day ?? dailyRpm?.quota;
    const tokensPerDay = policy.tokens_per_day ?? dailyTpm?.quota;
    const tenant = k.tenant_id || k.tenantId || k.tenant;
    const isActive = k.is_active ?? k.isActive ?? true;

    list.appendChild(el("div", { className: "data-row" },
      el("div", { className: "data-row-info" },
        el("strong", { textContent: k.name || k.id }),
        el("div", { className: "data-row-meta" },
          el("span", { className: "model-chip", textContent: masked }),
          tenant ? el("span", { textContent: "Tenant: " + tenant }) : null,
          requestsPerDay != null ? el("span", { textContent: fmtNum(requestsPerDay) + " req/day" }) : null,
          tokensPerDay != null ? el("span", { textContent: fmtNum(tokensPerDay) + " tok/day" }) : null,
          !isActive ? el("span", { className: "pill muted", textContent: "Inactive" }) : null,
        ),
      ),
      el("div", { className: "data-row-actions" },
        isActive ? el("button", { className: "btn btn-sm btn-secondary", textContent: "Edit",
          onclick() { editApiKey(k); }
        }) : null,
        isActive ? el("button", { className: "btn btn-sm btn-ghost", textContent: "Rotate",
          onclick() { rotateApiKey(k); }
        }) : null,
        el("button", { className: "btn btn-sm btn-danger", textContent: "Delete",
          onclick() { deleteApiKey(k); }
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
  $("apiKeyTenantInput").value = k.tenant_id || k.tenantId || k.tenant || "";
  const policy = apiKeyPolicy(k);
  const limits = Array.isArray(k.limits) ? k.limits : [];
  const rpm = limits.find(l => l.type === "rpm");
  const tpm = limits.find(l => l.type === "tpm");
  const cost = limits.find(l => l.type === "cost");
  $("apiKeyRequestsInput").value = policy.requests_per_day ?? rpm?.quota ?? "";
  $("apiKeyTokensInput").value = policy.tokens_per_day ?? tpm?.quota ?? "";
  $("apiKeyCostInput").value = policy.cost_per_month_usd ?? cost?.quota ?? "";
  const providers = policy.provider_allowlist || k.allowProviders || [];
  const models = policy.model_allowlist || k.allowModels || [];
  $("apiKeyProvidersInput").value = Array.isArray(providers) ? providers.join(", ") : providers;
  $("apiKeyModelsInput").value = Array.isArray(models) ? models.join(", ") : models;
  $("apiKeySubmitButton").textContent = "Save API Key";
  $("apiKeyCancelEditButton").classList.remove("hidden");
  setText("drawerApiKeyTitle", "Edit API Key");
  openDrawer("drawer-apikey");
}

async function saveApiKey() {
  const name = $("apiKeyNameInput").value.trim();
  if (!name) { toast("API key name is required", "error"); return; }
  const id = $("apiKeyEditIdInput").value;
  const requestsPerDay = numOrNull($("apiKeyRequestsInput").value);
  const tokensPerDay = numOrNull($("apiKeyTokensInput").value);
  const costPerMonth = numOrNull($("apiKeyCostInput").value);
  const payload = {
    name,
    tenant_id: $("apiKeyTenantInput").value.trim() || null,
    machine_id: $("apiKeyMachineInput").value.trim() || null,
    policy: {
      requests_per_day: requestsPerDay,
      tokens_per_day: tokensPerDay,
      cost_per_month_usd: costPerMonth,
      provider_allowlist: csvValues($("apiKeyProvidersInput").value),
      model_allowlist: csvValues($("apiKeyModelsInput").value),
    },
  };
  try {
    let result;
    if (id) {
      result = await sendJson("/api/setup/api-keys/" + encodeURIComponent(id), "PUT", payload);
      toast("API key updated", "success");
    } else {
      result = await sendJson("/api/setup/api-keys", "POST", payload);
      const rawKey = apiKeySecret(result);
      if (rawKey) {
        showSecretModal(rawKey, "API Key Created");
      } else {
        toast("API key created", "success");
      }
    }
    closeDrawer("drawer-apikey");
    await refresh();
    renderApiKeys();
  } catch (e) { toast(e.message, "error"); }
}

function showSecretModal(rawKey, title) {
  setText("modalSecretTitle", title || "API Key Created");
  setText("modalSecretText", rawKey);
  openModal("modal-secret");
}

$("modalSecretCopyBtn")?.addEventListener("click", async () => {
  try {
    await copyText($("modalSecretText").textContent);
    toast("API key copied to clipboard", "success");
  } catch (_) { toast("Failed to copy", "error"); }
});

async function copyText(value) {
  if (navigator.clipboard?.writeText) {
    try {
      await navigator.clipboard.writeText(value);
      return;
    } catch (_) {
      // Fall through for browsers/contexts where Clipboard API permission is denied.
    }
  }
  const input = el("textarea", { readonly: true, style: { position: "fixed", opacity: "0" } });
  input.value = value;
  document.body.appendChild(input);
  input.select();
  const copied = typeof document.execCommand === "function" && document.execCommand("copy");
  input.remove();
  if (!copied) throw new Error("Copy failed");
}

async function rotateApiKey(k) {
  showConfirm("Rotate API Key",
    "Rotating \"" + (k.name || k.id) + "\" will immediately invalidate the old key. Applications using the old key must be updated.",
    "Rotate", async () => {
      try {
        const result = await sendJson("/api/setup/api-keys/" + encodeURIComponent(k.id) + "/rotate", "POST", {});
        const rawKey = apiKeySecret(result);
        if (!rawKey) throw new Error("Rotate succeeded but the new API key was not returned");
        showSecretModal(rawKey, "API Key Rotated");
        toast("API key rotated", "success");
        await refresh();
        renderApiKeys();
      } catch (e) { toast(e.message, "error"); }
    });
}

async function deleteApiKey(k) {
  const name = k.name || k.id;
  const activeWarning = (k.is_active ?? k.isActive ?? true)
    ? " This will immediately revoke access for applications using it."
    : "";
  showConfirm("Delete API Key",
    "Delete \"" + name + "\"?" + activeWarning + " This cannot be undone.",
    "Delete", async () => {
      try {
        await sendJson("/api/setup/api-keys/" + encodeURIComponent(k.id), "DELETE");
        toast(name + " deleted", "success");
        await refresh();
        renderApiKeys();
      } catch (e) { toast(e.message, "error"); }
    });
}

$("apiKeySubmitButton").addEventListener("click", (e) => { e.preventDefault(); saveApiKey(); });
$("apiKeyCancelEditButton").addEventListener("click", () => { closeDrawer("drawer-apikey"); resetApiKeyForm(); });
$("addApiKeyBtn")?.addEventListener("click", () => { resetApiKeyForm(); openDrawer("drawer-apikey"); });
