/* ═══════════════════════════════════════════════════════════════════════════
   BlackRouter Control Panel — Aliases
   ═══════════════════════════════════════════════════════════════════════════ */

function aliasData() { return Array.isArray(state.aliases?.data) ? state.aliases.data : []; }

function renderAliases() {
  const list = $("aliasesList");
  if (!list) return;
  list.innerHTML = "";
  const aliases = aliasData();
  setText("navAliasesBadge", aliases.length);

  if (!aliases.length) {
    list.appendChild(el("div", { className: "empty-state" },
      el("div", { className: "empty-state-icon", textContent: "→" }),
      el("h3", { textContent: "No aliases yet" }),
      el("p", { textContent: "Map friendly names like \"fast\" to provider/model targets." }),
      el("p", { className: "empty-hint", textContent: "Example: fast → openai/gpt-5-mini" }),
    ));
    return;
  }

  aliases.forEach(a => {
    list.appendChild(el("div", { className: "data-row" },
      el("div", { className: "data-row-info" },
        el("strong", { textContent: a.alias || a.name }),
        el("div", { className: "data-row-meta" },
          el("span", { textContent: "→ " + (a.target || a.model) }),
        ),
      ),
      el("div", { className: "data-row-actions" },
        el("button", { className: "btn btn-sm btn-danger", textContent: "Delete",
          onclick() { deleteAlias(a); }
        }),
      ),
    ));
  });
}

$("aliasForm").addEventListener("submit", async (e) => {
  e.preventDefault();
  const alias = $("aliasNameInput").value.trim();
  const target = $("aliasTargetInput").value.trim();
  if (!alias || !target) { toast("Both fields required", "warn"); return; }
  try {
    await sendJson("/api/setup/aliases", "POST", { alias, target });
    toast("Alias " + alias + " → " + target + " created", "success");
    $("aliasNameInput").value = "";
    $("aliasTargetInput").value = "";
    await refresh();
    renderAliases();
  } catch (e) { toast(e.message, "error"); }
});

async function deleteAlias(a) {
  const name = a.alias || a.name;
  showConfirm("Delete Alias", "Delete \"" + name + "\"?", "Delete", async () => {
    try {
      await sendJson("/api/setup/aliases/" + encodeURIComponent(name), "DELETE");
      toast(name + " deleted", "success");
      await refresh();
      renderAliases();
    } catch (e) { toast(e.message, "error"); }
  });
}
