import { defineConfig } from '@playwright/test';
import config from './playwright.config';

// Manual Chromium measurements, separate from the default correctness suite.
// Generate the synthetic bundle and run all six cases with make perf-browser.
export default defineConfig({
  ...config,
  testIgnore: [],
  testMatch: '**/perf-*.spec.ts',
  outputDir: '../target/perf-results',
});
