const { test, expect } = require('@playwright/test');
const fs = require('node:fs');
const path = require('node:path');

const fixtures = path.resolve(__dirname, '../fixtures/api');
const readFixture = (name) => JSON.parse(fs.readFileSync(path.join(fixtures, name), 'utf8'));

async function mockApi(page, overrides = {}) {
  const defaults = {
    '/health': readFixture('health.json'),
    '/version': readFixture('version.json'),
    '/api/runtime/status': readFixture('runtime.json'),
    '/v1/models': readFixture('models.json'),
    '/api/setup/config': readFixture('setup-config.json'),
    '/api/setup/api-keys': readFixture('api-keys.json'),
    '/api/setup/providers': readFixture('providers.json'),
    '/api/setup/provider-catalog': readFixture('provider-catalog.json'),
    '/api/provider-limits': readFixture('provider-limits.json'),
    '/api/setup/combos': readFixture('combos.json'),
    '/api/setup/aliases': readFixture('aliases.json'),
    '/api/setup/config/versions': readFixture('config-versions.json'),
    '/api/doctor': { status: 'ok', issues: [] },
  };
  const routes = { ...defaults, ...overrides };

  await page.route('**/*', async (route) => {
    const url = new URL(route.request().url());
    if (url.pathname === '/setup' || url.pathname.endsWith('.js') || url.pathname.endsWith('.css')) {
      return route.continue();
    }
    const body = routes[url.pathname];
    if (body?.__status) {
      return route.fulfill({ status: body.__status, contentType: 'application/json', body: JSON.stringify(body.body || {}) });
    }
    if (body !== undefined) {
      return route.fulfill({ status: 200, contentType: 'application/json', body: JSON.stringify(body) });
    }
    return route.fulfill({ status: 200, contentType: 'application/json', body: '{}' });
  });
}

test.beforeEach(async ({ page }) => {
  await mockApi(page);
  await page.goto('/setup#overview');
});

test('renders overview and navigates by hash', async ({ page }) => {
  await expect(page.getByRole('heading', { name: 'Overview' })).toBeVisible();
  await expect(page.getByText('OpenAI Primary')).toBeVisible();
  await page.getByRole('button', { name: 'Providers' }).click();
  await expect(page).toHaveURL(/#providers$/);
  await expect(page.locator('#providers h2')).toHaveText('Provider Connections');
});

test('opens create combo drawer and reorders models', async ({ page }) => {
  await page.getByRole('button', { name: 'Combos' }).click();
  await page.getByRole('button', { name: '+ Create Combo' }).click();
  await expect(page.getByRole('dialog', { name: 'Create Combo' })).toBeVisible();

  // Filter to a provider and multi-select models via checkboxes.
  await page.locator('#comboProviderInput').selectOption('provider-anthropic-fallback');
  await expect(page.locator('#comboModelsList .data-row.selectable')).toHaveCount(1);
  const checkboxes = page.locator('#comboModelsList input[type=checkbox]');
  await checkboxes.first().check();
  await expect(page.getByRole('button', { name: /Add selected/ })).toBeVisible();
  await page.getByRole('button', { name: /Add selected/ }).click();
  await expect(page.locator('#comboModelsInput .model-draft-row')).toHaveCount(1);

  // Switch back to all providers and use Add all for the rest.
  await page.locator('#comboProviderInput').selectOption('');
  await page.getByRole('button', { name: /Add all/ }).click();
  await expect(page.locator('#comboModelsInput .model-draft-row').first()).toContainText('anthropic/claude-sonnet-4');
});

test('adds a vendor-prefixed CommandCode model to a combo', async ({ page }) => {
  await page.unroute('**/*');
  await mockApi(page, {
    '/api/setup/providers': {
      data: [{
        id: 'provider-commandcode',
        provider: 'commandcode',
        name: 'Command Code',
        auth_type: 'api-key',
        is_active: true,
        data: { format: 'commandcode', models: ['tencent/Hy3'] },
      }],
    },
  });
  await page.reload();

  let submitted;
  await page.route('**/api/setup/combos', async (route) => {
    if (route.request().method() !== 'POST') return route.fallback();
    submitted = route.request().postDataJSON();
    return route.fulfill({
      status: 200,
      contentType: 'application/json',
      body: JSON.stringify({
        id: 'combo-hy3',
        name: 'hy3-fallback',
        kind: 'llm',
        models: ['commandcode/tencent/Hy3'],
      }),
    });
  });

  await page.getByRole('button', { name: 'Combos' }).click();
  await page.getByRole('button', { name: '+ Create Combo' }).click();
  await expect(page.getByText('commandcode/tencent/Hy3')).toBeVisible();
  await page.getByRole('button', { name: 'Add commandcode/tencent/Hy3' }).click();
  await page.locator('#comboNameInput').fill('hy3-fallback');
  await page.getByRole('button', { name: 'Create Combo', exact: true }).click();

  await expect(page.getByText('Combo created')).toBeVisible();
  expect(submitted.models).toEqual(['commandcode/tencent/Hy3']);
});

test('shows control token prompt on protected endpoint', async ({ page }) => {
  await page.unroute('**/*');
  await mockApi(page, {
    '/api/setup/config': { __status: 401, body: { error: 'Invalid or missing control token' } },
  });
  await page.reload();
  await expect(page.getByRole('dialog', { name: 'Control Token Required' })).toBeVisible();
});

test('keeps control token when v1 models requires an API key', async ({ page }) => {
  await page.unroute('**/*');
  await mockApi(page, {
    '/v1/models': {
      __status: 401,
      body: { error: { message: 'Missing API key', type: 'authentication_error' } },
    },
  });
  await page.evaluate(() => sessionStorage.setItem('br-ct', 'br-ct-test'));
  await page.reload();

  await expect(page.getByRole('dialog', { name: 'Control Token Required' })).toBeHidden();
  expect(await page.evaluate(() => sessionStorage.getItem('br-ct'))).toBe('br-ct-test');
});

test('clears an invalid control token and prompts again', async ({ page }) => {
  await page.unroute('**/*');
  await mockApi(page, {
    '/api/setup/config': { __status: 401, body: { error: 'Invalid or missing control token' } },
  });
  await page.evaluate(() => sessionStorage.setItem('br-ct', 'br-ct-invalid'));
  await page.reload();

  await expect(page.getByRole('dialog', { name: 'Control Token Required' })).toBeVisible();
  expect(await page.evaluate(() => sessionStorage.getItem('br-ct'))).toBeNull();
});

test('loads config history and opens preview', async ({ page }) => {
  await page.getByRole('button', { name: 'Settings' }).click();
  await page.getByRole('button', { name: 'Load versions' }).click();
  await expect(page.getByText('Version 2')).toBeVisible();
  await page.getByRole('button', { name: 'Preview' }).first().click();
  await expect(page.getByRole('dialog', { name: 'Version 2' })).toBeVisible();
});

test('fetches provider models via the Models button', async ({ page }) => {
  await page.getByRole('button', { name: 'Providers' }).click();
  await page.getByRole('button', { name: 'Models' }).first().click();
  await expect(page.getByText(/Models updated for/)).toBeVisible();
});

test('creates an API key with the backend schema and copies the returned key', async ({ page }) => {
  let submitted;
  await page.route('**/api/setup/api-keys', async (route) => {
    if (route.request().method() !== 'POST') return route.fallback();
    submitted = route.request().postDataJSON();
    return route.fulfill({
      status: 200,
      contentType: 'application/json',
      body: JSON.stringify({
        record: { id: 'key-created', name: 'CI key', key_masked: 'brk_****ated' },
        key: 'brk_created_secret',
      }),
    });
  });
  await page.evaluate(() => {
    window.__copiedText = null;
    Object.defineProperty(navigator, 'clipboard', {
      configurable: true,
      value: { writeText: async (value) => { window.__copiedText = value; } },
    });
  });

  await page.getByRole('button', { name: 'API Keys' }).click();
  await page.getByRole('button', { name: '+ Create API Key' }).click();
  await page.locator('#apiKeyNameInput').fill('CI key');
  await page.locator('#apiKeyMachineInput').fill('runner-1');
  await page.locator('#apiKeyTenantInput').fill('tenant-ci');
  await page.locator('#apiKeyRequestsInput').fill('200');
  await page.locator('#apiKeyProvidersInput').fill('openai, anthropic');
  await page.getByRole('button', { name: 'Create API Key', exact: true }).click();

  await expect(page.getByRole('dialog', { name: 'API Key Created' })).toBeVisible();
  await expect(page.locator('#modalSecretText')).toHaveText('brk_created_secret');
  await expect(page.getByRole('button', { name: /Copy to clipboard/ })).toBeVisible();
  await page.getByRole('button', { name: /Copy to clipboard/ }).click();
  await expect(page.getByText('API key copied to clipboard')).toBeVisible();
  expect(await page.evaluate(() => window.__copiedText)).toBe('brk_created_secret');
  expect(submitted).toEqual({
    name: 'CI key',
    tenant_id: 'tenant-ci',
    machine_id: 'runner-1',
    policy: {
      requests_per_day: 200,
      tokens_per_day: null,
      cost_per_month_usd: null,
      provider_allowlist: ['openai', 'anthropic'],
      model_allowlist: [],
    },
  });
});

test('rotates an active API key and exposes the new key for copying', async ({ page }) => {
  await page.route('**/api/setup/api-keys/key-local-cli/rotate', async (route) => {
    expect(route.request().method()).toBe('POST');
    return route.fulfill({
      status: 200,
      contentType: 'application/json',
      body: JSON.stringify({
        record: { id: 'key-rotated', name: 'Local CLI', key_masked: 'brk_****ated' },
        key: 'brk_rotated_secret',
      }),
    });
  });

  await page.getByRole('button', { name: 'API Keys' }).click();
  await expect(page.getByText('br-****abcd')).toBeVisible();
  await expect(page.getByText('1,000 req/day')).toBeVisible();
  await page.getByRole('button', { name: 'Rotate' }).click();
  await expect(page.getByRole('alertdialog', { name: 'Rotate API Key' })).toBeVisible();
  await page.locator('#modalConfirmAction').click();

  await expect(page.getByRole('dialog', { name: 'API Key Rotated' })).toBeVisible();
  await expect(page.locator('#modalSecretText')).toHaveText('brk_rotated_secret');
  await expect(page.getByRole('button', { name: /Copy to clipboard/ })).toBeVisible();
});

test('deletes an API key after confirmation', async ({ page }) => {
  let deleteCalled = false;
  await page.route('**/api/setup/api-keys/key-local-cli', async (route) => {
    if (route.request().method() !== 'DELETE') return route.fallback();
    deleteCalled = true;
    return route.fulfill({
      status: 200,
      contentType: 'application/json',
      body: JSON.stringify({ ok: true }),
    });
  });

  await page.getByRole('button', { name: 'API Keys' }).click();
  await page.getByRole('button', { name: 'Delete' }).click();
  const confirm = page.getByRole('alertdialog', { name: 'Delete API Key' });
  await expect(confirm).toContainText('immediately revoke access');
  await confirm.getByRole('button', { name: 'Delete' }).click();

  await expect(page.getByText('Local CLI deleted')).toBeVisible();
  expect(deleteCalled).toBe(true);
});

test('mobile navigation opens and closes', async ({ page }) => {
  await page.setViewportSize({ width: 390, height: 844 });
  await page.getByRole('button', { name: 'Open menu' }).click();
  await expect(page.locator('#sidebar')).toHaveClass(/open/);
  await page.getByRole('button', { name: 'Limits & Cost' }).click();
  await expect(page).toHaveURL(/#limits$/);
  await expect(page.locator('#sidebar')).not.toHaveClass(/open/);
});
