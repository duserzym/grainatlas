import { defineConfig } from '@playwright/test';

export default defineConfig({
  testDir: './e2e',
  outputDir: '../target/e2e-results',
  workers: 1,
  retries: 0,
  timeout: 180_000,
  expect: { timeout: 30_000 },
  use: { browserName: 'chromium', baseURL: 'http://localhost:4173', viewport: { width: 1440, height: 900 },
    deviceScaleFactor: 1, locale: 'en-US', timezoneId: 'UTC', reducedMotion: 'reduce', trace: 'off', screenshot: 'off', video: 'off' },
  webServer: { command: 'pnpm preview', port: 4173, reuseExistingServer: true, timeout: 120_000 },
});
