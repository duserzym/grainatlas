import { defineConfig } from '@playwright/test';

// Documentation capture only. The e2e tests (synthetic data only) use playwright.config.ts.
export default defineConfig({
  testDir: './screenshots',
  testMatch: 'screenshots.spec.ts',
  outputDir: '../target/screenshots/playwright',
  workers: 1,
  fullyParallel: false,
  retries: 0,
  timeout: 20 * 60_000,
  expect: { timeout: 120_000 },
  reporter: 'list',
  use: {
    browserName: 'chromium',
    baseURL: 'http://localhost:4173',
    viewport: { width: 1440, height: 900 },
    deviceScaleFactor: 1,
    locale: 'en-US',
    timezoneId: 'UTC',
    reducedMotion: 'reduce',
    actionTimeout: 120_000,
    navigationTimeout: 120_000,
    // Only the deliberate, viewport-sized captures below contain package pixels.
    screenshot: 'off',
    video: 'off',
    trace: 'off',
  },
  webServer: {
    command: 'pnpm preview',
    port: 4173,
    reuseExistingServer: true,
    timeout: 120_000,
  },
});
