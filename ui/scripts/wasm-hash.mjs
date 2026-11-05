// Wasm half of the parity check: stitch every area of a package with default
// parameters in the wasm core (under Node) and print `<area> <sha256>`, then the
// Auto + grains hash and the atlas bundle hashes (`grainatlas hash` prints the same).
import { readFileSync, readdirSync, statSync } from 'node:fs';
import { join, relative } from 'node:path';
import { createHash } from 'node:crypto';
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
// Same steps as `grainatlas hash` (auto_grains): defaults, grains, Auto, per-grain normalisation.
const EPOCH = '1970-01-01T00:00:00Z';
const PARITY_ID = '00000000-0000-7000-8000-000000000000';
s.pin_identity(PARITY_ID, EPOCH);
const defaults = default_params();
const segDefaults = JSON.stringify(JSON.parse(defaults).segment);
const grainsByArea = {};
for (const a of info.areas) {
  s.stitch(a.id, defaults, () => {}, () => false);
  const grains = s.segment(a.id, segDefaults, '', '');
  // Grain ids are new UUIDv7s on every run: number them from PARITY_ID + 0x1000 as the CLI does.
  const first = Object.values(grainsByArea).reduce((n, g) => n + g.length, 0);
  grainsByArea[a.id] = JSON.parse(grains).map((g, i) => ({ ...g, id: PARITY_ID.slice(0, 24) + (0x1000 + first + i).toString(16).padStart(12, '0') }));
  s.set_grains(a.id, grains);
  const p = JSON.parse(s.auto_params(a.id, defaults));
  p.tone.grain_norm.enabled = true;
  const r = JSON.parse(s.recompute(a.id, JSON.stringify(p), () => {}, () => false));
  console.log(`${a.id}+auto+grains ${r.mosaic.sha256}`);
}
// The atlas bundle of that state: sha256 of atlas.json (which lists every other file's hash)
// and of the zip.
const full = JSON.parse(s.project_json());
full.grainatlas_ui = { grains: grainsByArea, spots: [], labels: [] };
const zip = s.atlas_zip(JSON.stringify(full), JSON.stringify({ quality: 100, exported: EPOCH }));
const sha = (b) => createHash('sha256').update(b).digest('hex');
// atlas.json is the first entry of the (stored, uncompressed) zip.
const dv = new DataView(zip.buffer, zip.byteOffset);
const nameLen = dv.getUint16(26, true);
const size = dv.getUint32(18, true);
const start = 30 + nameLen + dv.getUint16(28, true);
console.log(`bundle ${sha(zip.subarray(start, start + size))}`);
console.log(`bundle.zip ${sha(zip)}`);
// ATLAS_OUT=<file.zip>: also write the zip (to diff it against `grainatlas atlas --project-id 00000000-0000-7000-8000-000000000000 --exported 1970-01-01T00:00:00Z`).
if (process.env.ATLAS_OUT) (await import('node:fs')).writeFileSync(process.env.ATLAS_OUT, zip);
