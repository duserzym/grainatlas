import { defineConfig } from '@playwright/test';

// Smoke tests against the static build (`pnpm build`), served by `vite preview`.
export default defineConfig({
  testDir: './tests',
  timeout: 120_000,
  expect: { timeout: 60_000 },
  use: { baseURL: 'http://localhost:4173/', viewport: { width: 1440, height: 900 }, acceptDownloads: true },
  webServer: { command: 'pnpm preview', url: 'http://localhost:4173/', reuseExistingServer: true, timeout: 60_000 },
  projects: [
    { name: 'chromium', use: { browserName: 'chromium' } },
    { name: 'webkit', testMatch: /app-storage/, use: { browserName: 'webkit' } },
  ],
});
