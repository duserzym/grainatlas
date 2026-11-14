import { readFileSync } from 'node:fs';

const paths = process.argv.slice(2);
if (!paths.length) throw new Error('Usage: node scripts/playwright-times.mjs <Playwright JSON report> [...]');
for (const path of paths) {
  const report = JSON.parse(readFileSync(path, 'utf8'));
  console.log(`${path}: ${(report.stats.duration / 1000).toFixed(2)} s, ${report.stats.expected} passed, ${report.stats.unexpected} failed, ${report.stats.flaky} flaky, ${report.stats.skipped} skipped`);
  if (report.errors.length || report.stats.unexpected || report.stats.skipped || report.stats.flaky) {
    console.log('No comparison: the suite did not pass cleanly.');
    process.exitCode = 1;
    continue;
  }
  const times = new Map();
  const visit = suites => {
    for (const suite of suites) {
      for (const spec of suite.specs ?? []) for (const test of spec.tests)
        for (const result of test.results) times.set(spec.file, (times.get(spec.file) ?? 0) + result.duration);
      visit(suite.suites ?? []);
    }
  };
  visit(report.suites);
  console.log('Ten slowest files by summed test duration (all projects):');
  for (const [file, ms] of [...times].sort((a, b) => b[1] - a[1]).slice(0, 10))
    console.log(`${(ms / 1000).toFixed(2)} s  ${file}`);
}
