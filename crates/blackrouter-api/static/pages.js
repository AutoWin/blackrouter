/* ═══════════════════════════════════════════════════════════════════════════
   BlackRouter Control Panel — OAuth
   ═══════════════════════════════════════════════════════════════════════════ */

$("oauthGithubButton")?.addEventListener("click", async () => {
  const provider = $("providerInput").value.trim();
  if (!provider) { toast("Enter provider name first", "warn"); return; }
  try {
    oauthDone = false;
    const res = await sendJson("/api/setup/providers/" + encodeURIComponent(provider) + "/oauth-url", "POST", {});
    const url = res.url || res;
    if (!url || typeof url !== "string") { toast("No OAuth URL returned", "error"); return; }
    oauthPending = { providerId: provider, state: res.state || "" };
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
      if (e.data?.type === "oauth_callback" && e.data?.code && e.data?.state) {
        oauthDone = true;
        await exchangeOAuthCode(e.data.code, e.data.state);
      }
    });
  } catch (e) { toast("OAuth URL error: " + e.message, "error"); }
});

async function pollOAuth() {
  for (let i = 0; i < 60; i++) {
    await new Promise(r => setTimeout(r, 2000));
    if (oauthDone) return;
    try {
      const p = providerData().find(x => (x.id || x.provider) === oauthPending?.providerId);
      if (p?.data?.apiKey && $("providerApiKeyInput").value !== p.data.apiKey) {
        oauthDone = true;
        $("providerApiKeyInput").value = p.data.apiKey;
        toast("OAuth token received!", "success");
        return;
      }
    } catch (_) {}
  }
  if (!oauthDone) toast("OAuth timed out. Use manual fallback.", "warn");
}

async function exchangeOAuthCode(code, state) {
  if (!oauthPending) return;
  try {
    const res = await sendJson("/api/oauth/exchange", "POST", {
      provider: oauthPending.providerId, code, state,
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
  const code = $("oauthManualInput").value.trim();
  if (!code) { toast("Paste the code from the redirect URL", "warn"); return; }
  exchangeOAuthCode(code, oauthPending?.state || "");
});
