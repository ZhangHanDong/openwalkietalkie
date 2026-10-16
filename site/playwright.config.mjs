import { defineConfig } from '@playwright/test';
export default defineConfig({
  testDir: './tests/browser',
  workers: 1,
  use: { baseURL: 'http://127.0.0.1:8790', channel: 'chrome', screenshot: 'only-on-failure' },
  webServer: {
    command: 'npm run db:init:local && npm run dev -- --ip 127.0.0.1 --port 8790 --inspector-port 9235',
    url: 'http://127.0.0.1:8790',
    timeout: 60000,
    reuseExistingServer: false
  }
});
