import { defineConfig } from '@playwright/test';

// Smoke tests against the static build (`pnpm build`), served by `vite preview`.
export default defineConfig({
  testDir: './tests',
  fullyParallel: true,
  // Measured on the 14-core Mac: 7 workers 194 s, 10 workers 142 s, 12 workers 141 s.
  // CI runners have few cores: Playwright's default there.
  workers: process.env.CI ? undefined : 10,
  timeout: 120_000,
  expect: { timeout: 60_000 },
  use: { baseURL: 'http://localhost:4173/', viewport: { width: 1440, height: 900 }, acceptDownloads: true },
  webServer: { command: 'pnpm preview', url: 'http://localhost:4173/', reuseExistingServer: true, timeout: 60_000 },
  projects: [
    { name: 'chromium', use: { browserName: 'chromium' } },
    { name: 'webkit', testMatch: /app-storage/, use: { browserName: 'webkit' } },
  ],
});
