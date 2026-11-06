import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';

const read = (name) => readFileSync(new URL(`../${name}`, import.meta.url), 'utf8');
const workspace = read('Cargo.toml').match(/^\[workspace\.package\]\s*\n([\s\S]*?)(?=^\[|$(?![\s\S]))/m)?.[1];
const cargo = workspace?.match(/^version\s*=\s*"([^"]+)"/m)?.[1];
assert.ok(cargo, 'Cargo.toml needs a workspace.package version');
const tauri = JSON.parse(read('tauri/src-tauri/tauri.conf.json')).version;
const ui = JSON.parse(read('ui/package.json')).version;
assert.equal(tauri, cargo, `Tauri version ${tauri} differs from Cargo ${cargo}`);
assert.equal(ui, cargo, `UI version ${ui} differs from Cargo ${cargo}`);
console.log(cargo);
