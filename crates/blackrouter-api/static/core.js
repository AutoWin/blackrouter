/* ═══════════════════════════════════════════════════════════════════════════
   BlackRouter Control Panel — Core
   ═══════════════════════════════════════════════════════════════════════════ */

const state = {
  health: null, version: null, runtime: null,
  models: null, setupConfig: null, apiKeys: null,
  providers: null, providerCatalog: null, providerLimits: null,
  providerHealth: null, combos: null, aliases: null,
  errors: {},
};

let comboModelDraft = [];
let comboProviderFilter = "";
let oauthPending = null, oauthDone = false;
let limitsLoading = false;

const $ = (id) => document.getElementById(id);

/* ── Helpers ─────────────────────────────────────────────────────────────── */

function el(tag, attrs, ...children) {
  const node = document.createElement(tag);
  if (attrs) for (const k in attrs) {
    if (k === "className") node.className = attrs[k];
    else if (k === "textContent") node.textContent = attrs[k];
    else if (k.startsWith("on")) node.addEventListener(k.slice(2).toLowerCase(), attrs[k]);
    else if (k === "style" && typeof attrs[k] === "object") Object.assign(node.style, attrs[k]);
    else if (attrs[k] === false || attrs[k] == null) continue;
    else if (attrs[k] === true) node.setAttribute(k, "");
    else node.setAttribute(k, attrs[k]);
  }
  for (const child of children) {
    if (child == null || child === false) continue;
    if (typeof child === "string") node.appendChild(document.createTextNode(child));
    else if (child instanceof Node) node.appendChild(child);
  }
  return node;
}

function setValue(id, value) { const n = $(id); if (n) n.value = value ?? ""; }
function setText(id, text) { const n = $(id); if (n) n.textContent = text ?? ""; }

function setPill(id, label, tone) {
  const n = $(id); if (!n) return;
  n.textContent = label;
  n.className = `pill ${tone || "muted"}`;
}

function escapeHtml(s) { return String(s).replaceAll("&","&amp;").replaceAll("<","&lt;").replaceAll(">","&gt;").replaceAll('"',"&quot;").replaceAll("'","&#039;"); }

function fmtNum(v) { return Number(v || 0).toLocaleString(); }
function fmtUsd(v) { return "$" + Number(v || 0).toFixed(4); }
function fmtBool(v) { return v ? "On" : "Off"; }
function fmtSec(s) {
  const v = Number(s || 0);
  if (v < 60) return v + "s";
  if (v < 3600) return Math.floor(v/60) + "m " + (v%60) + "s";
  return Math.floor(v/3600) + "h " + Math.floor((v%3600)/60) + "m";
}
function fmtUnix(s) {
  const v = Number(s); if (!Number.isFinite(v) || v <= 0) return s || "-";
  return new Date(v * 1000).toLocaleString();
}
function numOrNull(v) { if (v === "") return null; const p = Number(v); return Number.isFinite(p) ? p : null; }
function csvValues(v) { return v.split(",").map(x => x.trim()).filter(Boolean); }
function parseAdminIds(v) { return v.split(",").map(x => Number(x.trim())).filter(x => Number.isFinite(x)); }

const ANTIGRAVITY_MODELS = [
  "gemini-3-flash-agent","gemini-3.5-flash-low","gemini-3.5-flash-extra-low",
  "gemini-pro-agent","gemini-3.1-pro-low","claude-sonnet-4-6","claude-opus-4-6-thinking",
  "gpt-oss-120b-medium","gemini-3-flash","gemini-2.0-flash-lite","gemini-2.0-flash",
  "gemini-2.5-flash","gemini-1.5-flash","gemini-1.5-pro","gemini-2.5-pro",
  "gemini-3-flash-preview","gemini-3-pro-preview",
];

/* ── API Client ──────────────────────────────────────────────────────────── */

async function getJson(path) {
  const r = await fetch(path, { cache: "no-store" });
  if (!r.ok) throw new Error(`${path} ${r.status}`);
  return r.json();
}

async function readPayload(r) {
  const ct = r.headers.get("content-type") || "";
  if (ct.includes("application/json")) return r.json().catch(() => ({}));
  const t = await r.text().catch(() => "");
  return t ? { error: t } : {};
}

function errMsg(p, fallback) {
  if (typeof p?.error === "string") return p.error;
  if (typeof p?.error?.message === "string") return p.error.message;
  if (typeof p?.message === "string") return p.message;
  return fallback;
}

async function sendJson(path, method, body) {
  const opts = { method, headers: { "content-type": "application/json" } };
  if (body !== undefined) opts.body = JSON.stringify(body);
  const r = await fetch(path, opts);
  const p = await readPayload(r);
  if (!r.ok) throw new Error(errMsg(p, `${path} ${r.status}`));
  return p;
}

/* ── Toast & Confirm ─────────────────────────────────────────────────────── */

function toast(msg, type) {
  const c = $("toastContainer"); if (!c) return;
  const t = el("div", { className: "toast " + (type || "info") },
    el("span", { className: "toast-body", textContent: msg }),
    el("button", { className: "toast-close", textContent: "✕", onclick() { t.remove(); } })
  );
  c.appendChild(t);
  setTimeout(() => { if (t.parentNode) t.remove(); }, 5000);
}

function showConfirm(title, desc, actionLabel, action) {
  setText("modalConfirmTitle", title);
  setText("modalConfirmDesc", desc);
  const btn = $("modalConfirmAction");
  btn.textContent = actionLabel;
  btn.className = "btn btn-danger";
  btn.onclick = async () => { await action(); closeModal("modal-confirm"); };
  openModal("modal-confirm");
  btn.focus();
}

/* ── Modal / Drawer ──────────────────────────────────────────────────────── */

function openModal(id) {
  const el = $(id); if (!el) return;
  el.classList.remove("hidden"); el.setAttribute("aria-hidden", "false");
  const first = el.querySelector("button, input");
  if (first) setTimeout(() => first.focus(), 50);
}

function closeModal(id) {
  const el = $(id); if (!el) return;
  el.classList.add("hidden"); el.setAttribute("aria-hidden", "true");
}

function openDrawer(id) { openModal(id); }
function closeDrawer(id) { closeModal(id); }

document.addEventListener("click", (e) => {
  const closeBtn = e.target.closest("[data-close]");
  if (closeBtn) closeModal(closeBtn.dataset.close);
});

document.addEventListener("keydown", (e) => {
  if (e.key === "Escape") {
    const modals = document.querySelectorAll(".modal-overlay:not(.hidden), .drawer-overlay:not(.hidden)");
    if (modals.length) closeModal(modals[modals.length-1].id);
  }
});

/* ── Theme ───────────────────────────────────────────────────────────────── */

function getTheme() { return localStorage.getItem("br-theme") || "system"; }
function applyTheme(t) {
  if (t === "dark") document.documentElement.setAttribute("data-theme", "dark");
  else if (t === "light") document.documentElement.setAttribute("data-theme", "light");
  else document.documentElement.removeAttribute("data-theme");
  localStorage.setItem("br-theme", t);
}
applyTheme(getTheme());

$("themeToggle").addEventListener("click", () => {
  const cur = getTheme();
  const next = cur === "dark" ? "light" : cur === "light" ? "system" : "dark";
  applyTheme(next);
});

/* --- Control Token --- */

let controlToken = sessionStorage.getItem("br-ct") || null;
let controlTokenPrompted = false;

function setControlToken(token) {
  controlToken = token;
  controlTokenPrompted = false;
  sessionStorage.setItem("br-ct", token);
  $("controlTokenInput").value = "";
  $("controlTokenError").classList.add("hidden");
  closeModal("modal-control-token");
  toast("Control token stored for this session", "success");
  if (typeof refresh === "function") refresh();
}

function clearControlToken() {
  controlToken = null;
  sessionStorage.removeItem("br-ct");
  controlTokenPrompted = false;
  toast("Control token cleared", "info");
}

$("controlTokenSubmit")?.addEventListener("click", function(e) {
  e.preventDefault();
  var token = $("controlTokenInput").value.trim();
  if (!token) { $("controlTokenError").classList.remove("hidden"); $("controlTokenError").textContent = "Token required"; return; }
  setControlToken(token);
});

$("controlTokenForm")?.addEventListener("submit", function(e) {
  e.preventDefault();
  $("controlTokenSubmit").click();
});

async function promptControlToken(reason) {
  if (controlTokenPrompted) return;
  controlTokenPrompted = true;
  setText("modalTokenTitle", "Control Token Required");
  $("controlTokenError").classList.add("hidden");
  if (reason) $("controlTokenError").textContent = reason;
  openModal("modal-control-token");
}

var _originalFetch = window.fetch;
window.fetch = async function(url, options) {
  options = options || {};
  var opts = Object.assign({}, options);
  opts.headers = Object.assign({}, opts.headers || {});
  if (controlToken) {
    opts.headers["X-Control-Token"] = controlToken;
  }
  var response = await _originalFetch(url, opts);
  if (response.status === 401 || response.status === 403) {
    if (controlToken) {
      controlToken = null;
      sessionStorage.removeItem("br-ct");
      controlTokenPrompted = false;
    }
    var ct = response.headers.get("Content-Type") || "";
    if (ct.includes("json")) {
      try {
        var cloned = response.clone();
        var body = await cloned.json();
        var msg = body.error || body.message || "";
        if (msg.toLowerCase().indexOf("token") !== -1 || msg.toLowerCase().indexOf("unauthorized") !== -1) {
          await promptControlToken(msg);
        }
      } catch(_) {}
    }
  }
  return response;
};
