const { defineConfig } = require('@playwright/test');

const port = process.env.BLACKROUTER_UI_PORT || '20139';
const baseURL = process.env.BLACKROUTER_UI_BASE_URL || `http://127.0.0.1:${port}`;

module.exports = defineConfig({
  testDir: './tests/ui',
  timeout: 30_000,
  webServer: process.env.BLACKROUTER_UI_BASE_URL ? undefined : {
    command: `cargo run -p blackrouter-bin`,
    url: `${baseURL}/health`,
    reuseExistingServer: true,
    timeout: 120_000,
    env: {
      ...process.env,
      BLACKROUTER_HOST: '127.0.0.1',
      BLACKROUTER_PORT: port,
      BLACKROUTER_DATA_DIR: 'target/playwright-data',
      BLACKROUTER_CONTROL_API_ENABLED: 'false',
    },
  },
  use: {
    baseURL,
    trace: 'retain-on-failure',
    screenshot: 'only-on-failure',
  },
  projects: [
    { name: 'chromium', use: { browserName: 'chromium' } },
  ],
});
