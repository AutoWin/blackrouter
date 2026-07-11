# BlackRouter Operations Runbook

## Probes

- `GET /healthz`: process liveness only.
- `GET /readyz`: SQLite, active providers, proactive probe state, RTK, and Redis when configured.
- `GET /api/provider-health`: latest proactive provider probe summary.

Docker Compose uses `/readyz`. A provider is marked `degraded` after a failed probe and `unhealthy` after the configured threshold. Successful user traffic or a successful probe restores `healthy`.

## Runtime Configuration

Settings saved through `/api/setup/config` take effect without restart. Every save creates a row in `settingsHistory`.

```bash
curl -H "X-Control-Token: $BLACKROUTER_CONTROL_TOKEN" \
  http://localhost:20129/api/setup/config/versions

curl -X POST -H "X-Control-Token: $BLACKROUTER_CONTROL_TOKEN" \
  http://localhost:20129/api/setup/config/versions/3/restore
```

The Setup UI manages API-key enforcement, provider probes, and cost guard budgets. `blackrouter-cli config apply config.json` applies the same JSON payload accepted by the API.

## Tenant Keys

Each key may have a `tenant_id` and a policy containing:

- `requests_per_day`
- `tokens_per_day`
- `cost_per_month_usd`
- `provider_allowlist`
- `model_allowlist`

Empty limits and allowlists preserve legacy unlimited access. Usage stores the API-key ID, never the raw key. Rotation deactivates the old credential and preserves its tenant policy.

## Multi-Replica Mode

Set the same values on every replica:

```env
BLACKROUTER_REDIS_URL=redis://redis:6379/
BLACKROUTER_SHARED_STATE_PREFIX=blackrouter-production
```

Redis coordinates request-per-minute limiting and shares RTK snapshots and deterministic response-cache entries. SQLite must also be placed on storage that supports the deployment's write model; for concurrent multi-host writes, migrate the persistence layer to a central database before increasing replicas.

If Redis becomes unavailable, `/readyz` fails and local RTK continues as a fail-open fallback. Remove the instance from traffic until Redis recovers.

## Backup And Recovery

1. Stop writes or remove the replica from traffic.
2. Copy the SQLite database together with its `-wal` and `-shm` files, or use SQLite's online backup command.
3. Persist Redis with AOF/RDB when shared state continuity is required.
4. Run `blackrouter-cli migrate /path/to/blackrouter.db` after restoring an older database.
5. Verify with `blackrouter-cli doctor` and `/readyz`.

## Incident Checks

```bash
blackrouter-cli doctor
blackrouter-cli usage export usage.json
curl -H "X-Control-Token: $BLACKROUTER_CONTROL_TOKEN" \
  http://localhost:20129/api/provider-health
curl http://localhost:20129/metrics
```

Check provider `status`, `cooldownUntil`, circuit-breaker metrics, and the correlated `x-request-id`/OTel trace before manually re-enabling a provider.
