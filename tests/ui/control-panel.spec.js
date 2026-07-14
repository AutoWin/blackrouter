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

test('shows control token prompt on protected endpoint', async ({ page }) => {
  await page.unroute('**/*');
  await mockApi(page, {
    '/api/setup/config': { __status: 401, body: { error: 'Invalid or missing control token' } },
  });
  await page.reload();
  await expect(page.getByRole('dialog', { name: 'Control Token Required' })).toBeVisible();
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

test('mobile navigation opens and closes', async ({ page }) => {
  await page.setViewportSize({ width: 390, height: 844 });
  await page.getByRole('button', { name: 'Open menu' }).click();
  await expect(page.locator('#sidebar')).toHaveClass(/open/);
  await page.getByRole('button', { name: 'Limits & Cost' }).click();
  await expect(page).toHaveURL(/#limits$/);
  await expect(page.locator('#sidebar')).not.toHaveClass(/open/);
});
