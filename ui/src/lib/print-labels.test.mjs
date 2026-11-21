import assert from 'node:assert/strict';
import test from 'node:test';
import { labelUI, readLabelOptions, pinLabel, spotDisplay, isSheetsStyle, withoutPrintLabelUI, parseLabelSize } from './print-labels.ts';
import { elementsOf } from './sync.ts';
import { elementHash, canonicalJson } from './sync-hash.ts';
import { printFingerprint } from './print-cache.ts';
import { storePrintEdits } from './strip-edit.ts';

const label = { spot_id: 'spot-1', occurrence: 'band/0', anchor_mm: [20, 30], box_mm: [22, 32, 30, 35] };
const options = () => readLabelOptions(null);
const project = () => ({ uuid: 'project-1', areas: [{ id: 'a', sample: 'MNT-A' }], grainatlas_ui: {
  grains: { a: [{ id: 'grain-1', number: 1, polygon_um: [[1, 1], [2, 2]], sample: 'MNT-A' }] },
  spots: [{ id: 'spot-1', name: 'MNT-A_12', area_id: 'a', x_um: 12, y_um: 18, diameter_um: 20 }], labels: [],
} });

test('add, move, clear and save pins preserve sync elements and their hashes byte for byte', async () => {
  const p = project();
  const bytes = canonicalJson(elementsOf(p));
  const hashes = await Promise.all(Object.values(elementsOf(p)).map(elementHash));
  let state = options();
  for (const origin of [[42, 53], [48.5, 56], null]) {
    state = pinLabel(state, label, origin);
    p.grainatlas_ui = { ...p.grainatlas_ui, ...labelUI(state) };
    const saved = JSON.parse(JSON.stringify(p));
    assert.equal(canonicalJson(elementsOf(saved)), bytes);
    assert.deepEqual(await Promise.all(Object.values(elementsOf(saved)).map(elementHash)), hashes);
    assert.deepEqual(readLabelOptions(saved.grainatlas_ui), state);
    assert.equal(saved.print_sheets, undefined);
  }
});

test('fingerprint changes with size and offsets, including moves in the same object', () => {
  const inputs = { identity: 'p', controls: { style: 'spots', spotLabels: options() }, areas: [], tone: {}, grains: {}, edits: [] };
  const before = printFingerprint(inputs);
  inputs.controls.spotLabels.size_mm = 3;
  assert.notEqual(printFingerprint(inputs), before);
  const size = printFingerprint(inputs);
  inputs.controls.spotLabels.pins['spot-1/band/0'] = [2, 4];
  assert.notEqual(printFingerprint(inputs), size);
  const pinned = printFingerprint(inputs);
  inputs.controls.spotLabels.pins['spot-1/band/0'][0]++;
  assert.notEqual(printFingerprint(inputs), pinned);
});

test('style switches share the same cut and saved edits', () => {
  const plan = { params: { packing: 'packed', um_per_mm: 8 }, um_per_mm: 8, strips: [{ key: 'band/0' }] };
  const app = { project: project(), readOnly: false, dirty: false };
  storePrintEdits(app, plan, [{ op: 'arrange' }]);
  const saved = JSON.stringify(app.project.print_sheets);
  const on = spotDisplay(plan, true, app.project.grainatlas_ui.spots, options());
  assert.strictEqual(on.params, plan.params); assert.strictEqual(on.strips, plan.strips);
  assert.deepEqual(spotDisplay(on, false, [], options()), plan);
  assert.equal(JSON.stringify(app.project.print_sheets), saved);
  assert.ok(isSheetsStyle('spots') && isSheetsStyle('sheets')); assert.ok(!isSheetsStyle('layout'));
});

test('malformed UI pins and sizes are bounded without changing source JSON', () => {
  const ui = { print_label_pins: { a: [NaN, 3], b: [2, 4], c: [1] }, print_label_size_mm: 20 };
  const state = readLabelOptions(ui); assert.deepEqual(state, { size_mm: 4, pins: { b: [2, 4] } });
  assert.ok(Number.isNaN(ui.print_label_pins.a[0]));
});

test('server bundle omits label preferences while the local snapshot keeps them', () => {
  const local = project();
  Object.assign(local.grainatlas_ui, { print_label_pins: { 'spot-1/band/0': [2, 3] }, print_label_size_mm: 3 });
  const before = JSON.stringify(local);
  const published = withoutPrintLabelUI(local);
  assert.equal(published.grainatlas_ui.print_label_pins, undefined);
  assert.equal(published.grainatlas_ui.print_label_size_mm, undefined);
  assert.deepEqual(published.grainatlas_ui.spots, local.grainatlas_ui.spots);
  assert.equal(JSON.stringify(local), before);
});

test('display refresh keeps a cut stale when scale or other cutting inputs changed', async () => {
  const { sheetGeometryControls } = await import('./print-labels.ts');
  const controls = { style: 'sheets', spotLabels: options(), sheet: { um_per_mm: 8, packing: 'lines' } };
  const original = JSON.stringify(sheetGeometryControls(controls));
  assert.equal(JSON.stringify(sheetGeometryControls({ ...controls, style: 'spots', spotLabels: { ...options(), size_mm: 3 } })), original);
  assert.notEqual(JSON.stringify(sheetGeometryControls({ ...controls, style: 'spots', sheet: { ...controls.sheet, um_per_mm: 12 } })), original);
});

test('Auto is the default and round-trips in the same UI field as fixed sizes', () => {
  assert.equal(options().size_mm, 'auto');
  for (const size_mm of ['auto', 1.6, 2.9, 4]) {
    const state = { ...options(), size_mm };
    assert.equal(labelUI(state).print_label_size_mm, size_mm);
    assert.deepEqual(readLabelOptions(labelUI(state)), state);
  }
  assert.equal(parseLabelSize('Auto'), 'auto');
  assert.equal(parseLabelSize('2.9'), 2.9);
  for (const value of ['', 'NaN', '1.5', '4.1', 'word']) assert.equal(parseLabelSize(value), null);
});
