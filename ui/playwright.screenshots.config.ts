import { defineConfig } from '@playwright/test';

// `make screenshots`: walks the workflow on the bundled fixture and writes docs/images/*.png.
// Not part of `make test`.
export default defineConfig({
  testDir: './scripts',
  testMatch: 'screenshots.spec.ts',
  timeout: 300_000,
  expect: { timeout: 60_000 },
  use: { baseURL: 'http://localhost:4173/', viewport: { width: 1440, height: 900 }, deviceScaleFactor: 1, colorScheme: 'light' },
  webServer: { command: 'pnpm preview', url: 'http://localhost:4173/', reuseExistingServer: true, timeout: 60_000 },
  projects: [{ name: 'chromium', use: { browserName: 'chromium' } }],
});
