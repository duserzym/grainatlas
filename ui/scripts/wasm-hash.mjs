// Wasm half of the parity check: stitch every area of a package with default
// parameters in the wasm core (under Node) and print `<area> <sha256>`.
import { readFileSync, readdirSync, statSync } from 'node:fs';
import { join, relative } from 'node:path';
import init, { Session, default_params } from '../src/lib/core/pkg/grainatlas.js';

const root = process.argv[2];
await init({ module_or_path: readFileSync(new URL('../src/lib/core/pkg/grainatlas_bg.wasm', import.meta.url)) });
const s = new Session(root.split('/').filter(Boolean).pop());
const walk = (d) => {
  for (const n of readdirSync(d)) {
    const p = join(d, n);
    if (statSync(p).isDirectory()) walk(p);
    else s.add_file(relative(root, p).split('\\').join('/'), readFileSync(p));
  }
};
walk(root);
const opened = JSON.parse(s.open_package());
if (opened.status !== 'opened') throw new Error(`not a package: ${opened.status}`);
const info = opened.info;
for (const a of info.areas) {
  const r = JSON.parse(s.stitch(a.id, '', () => {}, () => false));
  console.log(`${a.id} ${r.mosaic.sha256}`);
}
// Same steps as `grainatlas hash` (auto_grains_hash): defaults, grains, Auto, per-grain normalisation.
const defaults = default_params();
const segDefaults = JSON.stringify(JSON.parse(defaults).segment);
for (const a of info.areas) {
  s.stitch(a.id, defaults, () => {}, () => false);
  const grains = s.segment(a.id, segDefaults, '');
  s.set_grains(a.id, grains);
  const p = JSON.parse(s.auto_params(a.id, defaults));
  p.tone.grain_norm.enabled = true;
  const r = JSON.parse(s.recompute(a.id, JSON.stringify(p), () => {}, () => false));
  console.log(`${a.id}+auto+grains ${r.mosaic.sha256}`);
}
