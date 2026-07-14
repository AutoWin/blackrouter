/* ═══════════════════════════════════════════════════════════════════════════
   BlackRouter Control Panel — Config Versioning UI
   Appends version list + restore to the Settings panel.
   ═══════════════════════════════════════════════════════════════════════════ */

let configVersions = [];

async function loadConfigVersions() {
  const list = $("configVersionsList");
  if (!list) return;
  list.innerHTML = el("span", { className: "text-muted text-sm", textContent: "Loading…" }).outerHTML;
  try {
    configVersions = await getJson("/api/setup/config/versions");
    renderConfigVersions();
  } catch (e) {
    list.innerHTML = el("span", { className: "text-muted text-sm", textContent: "Failed to load versions: " + e.message }).outerHTML;
  }
}

function renderConfigVersions() {
  const list = $("configVersionsList");
  if (!list) return;
  list.innerHTML = "";

  const versions = Array.isArray(configVersions) ? configVersions : (configVersions?.data || []);

  if (!versions.length) {
    list.innerHTML = el("p", { className: "text-muted text-sm", textContent: "No previous config versions found." }).outerHTML;
    return;
  }

  versions.forEach(v => {
    const ts = v.timestamp || v.created_at || v.saved_at || "";
    list.appendChild(el("div", { className: "data-row" },
      el("div", { className: "data-row-info" },
        el("strong", { textContent: "Version " + (v.version || v.id) }),
        el("div", { className: "data-row-meta" },
          ts ? el("span", { textContent: fmtUnix(ts) }) : null,
          v.saved_at ? el("span", { className: "pill muted", textContent: fmtUnix(v.saved_at) }) : null,
        ),
      ),
      el("div", { className: "data-row-actions" },
        el("button", { className: "btn btn-sm btn-secondary", textContent: "Preview",
          onclick() { previewVersionDiff(v); }
        }),
        el("button", { className: "btn btn-sm btn-ghost", textContent: "Restore",
          onclick() { confirmRestoreVersion(v); }
        }),
      ),
    ));
  });
}

function previewVersionDiff(version) {
  const dialog = $("modal-config-diff");
  if (!dialog) return;

  const settings = version.data || version.settings || {};
  const pretty = JSON.stringify(settings, null, 2);
  const diffEl = $("configDiffContent");
  if (diffEl) diffEl.textContent = pretty;

  setText("configDiffTitle", "Version " + (version.version || version.id));
  openModal("modal-config-diff");
}

function confirmRestoreVersion(version) {
  const verLabel = "Version " + (version.version || version.id);
  showConfirm(
    "Restore Config",
    "Restore " + verLabel + "? This will overwrite the current configuration and hot-reload.\n\nYou can revert by restoring a newer version.",
    "Restore " + verLabel,
    () => restoreVersion(version)
  );
}

async function restoreVersion(version) {
  try {
    const v = version.version || version.id;
    await sendJson("/api/setup/config/versions/" + encodeURIComponent(v) + "/restore", "POST", {});
    toast("Config restored to version " + v, "success");
    // Re-fetch to update settings UI
    state.setupConfig = await getJson("/api/setup/config");
    renderSettings();
    loadConfigVersions();
  } catch (e) { toast("Restore failed: " + e.message, "error"); }
}
