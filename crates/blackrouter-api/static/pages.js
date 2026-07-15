/* ═══════════════════════════════════════════════════════════════════════════
   BlackRouter Control Panel — OAuth
   ═══════════════════════════════════════════════════════════════════════════ */

function startOAuth() {
  const provider = $("providerInput").value.trim();
  if (!provider) { toast("Enter provider name first", "warn"); return; }
  startOAuthForProvider(provider);
}

async function startOAuthForProvider(provider) {
  try {
    oauthDone = false;
    const res = await sendJson("/api/oauth/" + encodeURIComponent(provider) + "/start", "POST", {
      redirect_uri: window.location.origin + "/oauth/callback",
    });
    const url = res.url;
    if (!url || typeof url !== "string") { toast("No OAuth URL returned", "error"); return; }
    oauthPending = { providerId: provider, state: res.state || "", flowType: res.flow_type || "" };

    // Device code flow (e.g. GitHub): show user_code + verification_uri
    if (res.flow_type === "device_code") {
      const userCode = res.user_code || "";
      const verifyUrl = res.verification_uri || url;
      $("oauthNotice").classList.remove("hidden");
      $("oauthNotice").innerHTML =
        "Device code flow: enter <strong>" + escapeHtml(userCode) + "</strong> at " +
        "<a href='" + escapeHtml(verifyUrl) + "' target='_blank'>" + escapeHtml(verifyUrl) + "</a>";
      window.open(verifyUrl, "oauth_login", "width=800,height=700");
      pollOAuth();
      return;
    }

    // Authorization code flow (e.g. Google/Gemini/Antigravity/Codex): open popup
    const w = window.open(url, "oauth_login", "width=800,height=700");
    if (w) {
      const ch = new BroadcastChannel("br_oauth");
      ch.onmessage = async (ev) => {
        if (oauthDone || !oauthPending) return;
        oauthDone = true; ch.close();
        if (ev.data?.code && ev.data?.state) await exchangeOAuthCode(ev.data.code, ev.data.state);
      };
      pollOAuth();
    } else {
      $("oauthNotice").classList.remove("hidden");
      $("oauthNotice").innerHTML = "Popup blocked. <a href='" + escapeHtml(url) + "' target='_blank'>Open link manually</a>";
      $("oauthManual").classList.remove("hidden");
    }
    window.addEventListener("message", async (e) => {
      if (oauthDone || !oauthPending) return;
      if (e.data?.type === "oauth_callback") {
        const payload = e.data?.data || e.data;
        if (payload?.code && payload?.state) {
          oauthDone = true;
          await exchangeOAuthCode(payload.code, payload.state);
        }
      }
    });
  } catch (e) { toast("OAuth start error: " + e.message, "error"); }
}

$("oauthGithubButton")?.addEventListener("click", startOAuth);

async function pollOAuth() {
  if (!oauthPending) return;
  const provider = oauthPending.providerId;
  const state = oauthPending.state;
  const interval = 2000;
  for (let i = 0; i < 60; i++) {
    await new Promise(r => setTimeout(r, interval));
    if (oauthDone) return;
    try {
      const res = await sendJson("/api/oauth/" + encodeURIComponent(provider) + "/status?state=" + encodeURIComponent(state), "GET");
      if (res.status === "done" && res.access_token) {
        oauthDone = true;
        $("providerApiKeyInput").value = res.access_token;
        toast("OAuth token received!", "success");
        $("oauthNotice").classList.add("hidden");
        return;
      }
      if (res.status === "error") {
        oauthDone = true;
        toast("OAuth error: " + (res.error || "unknown"), "error");
        return;
      }
    } catch (_) {}
  }
  if (!oauthDone) toast("OAuth timed out. Use manual fallback.", "warn");
}

async function exchangeOAuthCode(code, state) {
  if (!oauthPending) return;
  const provider = oauthPending.providerId;
  try {
    const res = await sendJson("/api/oauth/" + encodeURIComponent(provider) + "/exchange", "POST", {
      code, state,
    });
    if (res.token || res.access_token || res.data?.apiKey) {
      $("providerApiKeyInput").value = res.token || res.access_token || res.data?.apiKey || "";
      toast("OAuth token exchanged!", "success");
    }
  } catch (e) { toast("Token exchange failed: " + e.message, "error"); }
  oauthPending = null;
  oauthDone = true;
}

$("oauthManualButton")?.addEventListener("click", () => {
  const raw = $("oauthManualInput").value.trim();
  if (!raw) { toast("Paste the redirect URL or code from the login window", "warn"); return; }
  // Accept either a full redirect URL or a bare code parameter
  let code = raw, state = oauthPending?.state || "";
  try {
    const u = new URL(raw);
    code = u.searchParams.get("code") || code;
    if (u.searchParams.get("state")) state = u.searchParams.get("state");
  } catch {}
  exchangeOAuthCode(code, state);
});
