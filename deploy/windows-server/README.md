# Windows Server runtime

The production layout is intentionally separate from the MT5 Review app:

- executable: `C:\BlackRouter\bin\blackrouter.exe`
- SQLite data: `C:\BlackRouter\data\blackrouter.db`
- ACL-protected environment: `C:\BlackRouter\config\blackrouter.env`
- logs: `C:\BlackRouter\logs\blackrouter.log`
- startup task: `BlackRouter` running as `SYSTEM`

BlackRouter listens only on `127.0.0.1:20129`. Caddy exposes the
OpenAI-compatible `/v1/*` API and health probes at
`https://llm.blackcat.io.vn`; setup and control-plane routes remain available
only through the loopback listener in an Administrator RDP session.

Remote administration uses `https://llm-admin.blackcat.io.vn` behind a
Cloudflare Access self-hosted application. The Caddy origin accepts this host
only from Cloudflare's published proxy networks and rejects requests that do
not carry both `Cf-Access-Jwt-Assertion` and the Access application session
cookie. BlackRouter still requires its independent `BLACKROUTER_CONTROL_TOKEN`
after Access authentication. Do not create an Access Bypass or Everyone policy
for this hostname.

Start or restart the runtime with:

```powershell
Stop-ScheduledTask -TaskName BlackRouter -ErrorAction SilentlyContinue
Start-ScheduledTask -TaskName BlackRouter
Invoke-WebRequest http://127.0.0.1:20129/readyz -UseBasicParsing
```
