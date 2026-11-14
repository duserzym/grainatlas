import { execFileSync, spawn } from 'node:child_process';
import { existsSync, readFileSync } from 'node:fs';
import { matchesGlob, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import affected from '../ui/tests/affected.mjs';

const fullFiles = new Set(['Makefile', 'Cargo.toml', 'Cargo.lock', 'ui/package.json',
  'pnpm-lock.yaml', 'ui/pnpm-lock.yaml', 'ui/vite.config.ts', 'ui/playwright.config.ts']);
const documentation = file => file.startsWith('docs/') || file.startsWith('site/') || file.endsWith('.md');

export function choose(files) {
  const plan = { full: false, ui: false, core: false, cli: false, wasm: false, desktop: false,
    browser: false, specs: new Set(), reasons: [] };
  for (const file of files) {
    if (documentation(file)) continue;
    if (fullFiles.has(file)) {
      plan.full = true; plan.reasons.push(`${file}: this change needs the full check`);
    } else if (file.startsWith('ui/src/') || file.startsWith('ui/unit/')) {
      plan.ui = plan.browser = true;
      plan.reasons.push(`${file}: UI type check, unit tests, web build and affected browser specs`);
      let matched = false;
      for (const [glob, specs] of Object.entries(affected)) if (matchesGlob(file, glob)) {
        matched = true;
        for (const spec of specs) plan.specs.add(`${spec}.spec.ts`);
        plan.reasons.push(`  ${glob} -> ${specs.join(', ')}`);
      }
      if (!matched) plan.reasons.push(`  map has no entry for ${file}; smoke and imported-spec detection still run`);
    } else if (file.startsWith('ui/tests/')) {
      plan.browser = true;
      if (file.endsWith('.spec.ts')) plan.specs.add(file.slice('ui/tests/'.length));
      plan.reasons.push(`${file}: changed specs and their importers, plus smoke`);
      if (file === 'ui/tests/affected.mjs') {
        plan.full = true; plan.reasons.push(`${file}: this change needs the full check`);
      }
    } else if (file.startsWith('crates/grainatlas-core/')) {
      plan.core = true; plan.reasons.push(`${file}: grainatlas-core tests`);
    } else if (file.startsWith('crates/grainatlas-cli/')) {
      plan.cli = true; plan.reasons.push(`${file}: grainatlas-cli tests`);
    } else if (file.startsWith('crates/grainatlas-wasm/')) {
      plan.wasm = true; plan.reasons.push(`${file}: wasm build and native/wasm parity`);
    } else if (file.startsWith('tauri/')) {
      plan.desktop = true; plan.reasons.push(`${file}: desktop-check (including generated icons)`);
    } else {
      plan.full = true; plan.reasons.push(`${file}: no quick rule; this change needs the full check`);
    }
  }
  if (plan.browser) plan.specs.add('smoke.spec.ts');
  return plan;
}

export function changedFiles(base) {
  // diff BASE includes staged and unstaged changes; ls-files adds untracked files.
  const git = args => execFileSync('git', args, { encoding: 'utf8' }).split('\0').filter(Boolean);
  execFileSync('git', ['rev-parse', '--verify', `${base}^{commit}`], { stdio: 'pipe' });
  return [...new Set([...git(['diff', '--name-only', '--no-renames', '-z', base, '--']),
    ...git(['ls-files', '--others', '--exclude-standard', '-z'])])].sort();
}

function run(command, args, cwd = '.') {
  console.log(`quick: ${cwd === '.' ? '' : `${cwd}: `}${command} ${args.join(' ')}`);
  return new Promise((resolveRun, reject) => {
    const child = spawn(command, args, { cwd, stdio: 'inherit' });
    child.on('error', reject);
    child.on('exit', (code, signal) => code === 0 ? resolveRun() : reject(new Error(`${command} failed (${signal ?? code})`)));
  });
}
async function parallel(jobs) {
  const results = await Promise.allSettled(jobs);
  const failures = results.filter(result => result.status === 'rejected');
  if (failures.length) throw new Error(failures.map(result => result.reason.message).join('\n'));
}

async function main() {
  const base = process.env.BASE || 'HEAD';
  // File lists are for reproducible selection benchmarks; normal quick uses git.
  const filesArg = process.argv.indexOf('--files-from');
  const files = filesArg < 0 ? changedFiles(base)
    : readFileSync(process.argv[filesArg + 1], 'utf8').split('\n').filter(Boolean);
  const plan = choose(files);
  console.log(`quick: base ${base}; ${files.length} changed file(s)`);
  for (const reason of plan.reasons) console.log(reason);
  if (!plan.reasons.length) { console.log('quick: documentation/site only or no changes; no checks needed'); return; }
  if (process.argv.includes('--dry-run')) {
    console.log(JSON.stringify({ ...plan, specs: [...plan.specs].sort() }, null, 2)); return;
  }
  if (plan.full) { await run('make', ['full']); return; }

  const native = [];
  if (plan.core || plan.cli) {
    const args = ['test'];
    if (plan.core) args.push('-p', 'grainatlas-core');
    if (plan.cli) args.push('-p', 'grainatlas-cli');
    native.push(run('cargo', args));
  }
  if (plan.desktop) native.push(run('make', ['desktop-check']));
  const ui = async () => {
    const prep = [];
    if (plan.wasm) prep.push('parity');
    else if ((plan.browser && (plan.core || plan.cli))
      || ((plan.browser || plan.ui) && !existsSync('ui/src/lib/core/pkg/grainatlas.js'))) prep.push('wasm');
    // UI validation's make unit already prepares the fixture. Browser-only
    // selections need it here, before starting the preview server.
    if (plan.browser && !plan.ui) prep.push('atlas-fixture');
    if (prep.length) await run('make', ['-j2', ...prep]);
    if (plan.ui) await parallel([run('pnpm', ['check'], 'ui'), run('make', ['unit'])]);
    if (!plan.browser) return;
    // --only-changed includes helper/spec importers. Union it with the explicit
    // source map: filtering the mapped set with --only-changed would lose specs.
    const listing = execFileSync('pnpm', ['exec', 'playwright', 'test', `--only-changed=${base}`, '--list', '--reporter=json', '--pass-with-no-tests'],
      { cwd: 'ui', encoding: 'utf8', maxBuffer: 16 * 1024 * 1024 });
    const collect = suites => {
      for (const suite of suites) {
        if (suite.file?.endsWith('.spec.ts')) plan.specs.add(suite.file.replace(/^.*[\\/]tests[\\/]/, ''));
        collect(suite.suites ?? []);
      }
    };
    collect(JSON.parse(listing).suites);
    const specs = [...plan.specs].sort();
    console.log(`quick: browser union (map, changed specs/importers, always smoke): ${specs.join(', ')}`);
    await run('pnpm', ['build'], 'ui');
    await run('pnpm', ['exec', 'playwright', 'test', ...specs], 'ui');
  };
  await parallel([...native, ui()]);
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  main().catch(error => { console.error(`quick: ${error.message}`); process.exitCode = 1; });
}
