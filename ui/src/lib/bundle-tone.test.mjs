import assert from 'node:assert/strict';
import test from 'node:test';
import { bundlePixels, curveTone, neutralTone, overlayFor, saveBundleTones } from './bundle-tone.ts';
import { pixelSignature } from './sync-pixels.ts';
import { screenEnabled } from './screens.ts';

const raw = { ...neutralTone(), low_pct: 2, gamma: 1.3 };
const project = () => ({ uuid: 'project', package: { name: 'Synthetic' }, upbdb: { mount_id: 'mount' },
  areas: [{ id: 'a', params: { tone: structuredClone(raw), stitch: {}, photometric: {}, blend: {} }, mosaic: { sha256: 'raw', w: 32, h: 24, um_per_px: 1 } }],
  grainatlas_ui: { tone: { a: structuredClone(raw) }, spots: [{ id: 's', name: 'S' }], grains: { a: [{ id: 'g', label: 'G' }] }, labels: [{ id: 'l', text: 'L' }] } });
const save = (adopted, sources, display) => {
  const p = structuredClone(adopted);
  saveBundleTones(p, sources, { a: { tone: display } }, { a: display }, adopted);
  return p;
};
const source = { a: { rawCurve: raw, rawDisplay: { ...raw, gamma: 1.6 }, sha256: 'published' } };

test('pixel signature includes overlay curve and display, excluding binding hash and annotations', () => {
  const p = project(), signature = pixelSignature(p);
  p.grainatlas_ui.spots[0].name = 'Changed';
  assert.equal(pixelSignature(p), signature);
  p.grainatlas_ui.bundle_tone = { a: { curve: neutralTone(), display: neutralTone(), on_sha256: 'published' } };
  const overlay = pixelSignature(p);
  assert.notEqual(overlay, signature);
  p.grainatlas_ui.bundle_tone.a.on_sha256 = 'other';
  assert.equal(pixelSignature(p), overlay);
  p.grainatlas_ui.bundle_tone.a.curve.low_pct = 10;
  assert.notEqual(pixelSignature(p), overlay);
  p.grainatlas_ui.bundle_tone.a.curve.low_pct = 0;
  p.grainatlas_ui.bundle_tone.a.display.gamma = 2;
  assert.notEqual(pixelSignature(p), overlay);
  delete p.grainatlas_ui.bundle_tone;
  assert.equal(pixelSignature(p), signature);
});

test('fullProject bundle serialization keeps raw tones and all identities, then adopts by source hash', () => {
  const p = project(), display = { ...neutralTone(), low_pct: 12, gamma: 1.2 };
  const saved = save(p, source, display);
  assert.deepEqual(saved.areas[0].params.tone, raw);
  assert.deepEqual(saved.grainatlas_ui.tone.a, source.a.rawDisplay);
  assert.deepEqual(saved.grainatlas_ui.bundle_tone.a, { curve: display, display, on_sha256: 'published' });
  for (const key of ['grains', 'spots', 'labels']) assert.deepEqual(saved.grainatlas_ui[key], p.grainatlas_ui[key]);
  assert.deepEqual(saved.upbdb, p.upbdb);
  const adopted = JSON.parse(JSON.stringify(saved));
  assert.deepEqual(overlayFor(adopted, 'a', 'published'), { ...saved.grainatlas_ui.bundle_tone.a });
  assert.deepEqual(overlayFor(adopted, 'a', 'different'), { curve: neutralTone(), display: neutralTone() });
  const unloaded = save(adopted, null, raw);
  assert.deepEqual(unloaded.grainatlas_ui.bundle_tone, saved.grainatlas_ui.bundle_tone);
  const reset = save(adopted, source, neutralTone());
  assert.equal(reset.grainatlas_ui.bundle_tone, undefined);
  assert.deepEqual(p, project(), 'serialization must not mutate adopted state');
});

test('Area and Layout access before loading, in edit, view and viewer modes', () => {
  const remote = { viewer: false, readOnly: true, remote: true, pkg: false, mosaic: true };
  for (const id of ['area', 'layout']) {
    assert.equal(screenEnabled(id, remote), false);
    assert.equal(screenEnabled(id, { ...remote, readOnly: false }), true);
    assert.equal(screenEnabled(id, { ...remote, readOnly: false, viewer: true }), false);
  }
  assert.equal(screenEnabled('canvas', remote), true);
  assert.equal(screenEnabled('area', { ...remote, remote: false, pkg: true }), true);
  assert.equal(screenEnabled('layout', { ...remote, remote: false, mosaic: false }), false);
  assert.equal(bundlePixels({ remote: { readOnly: false }, readOnly: false, bundle: {} }), true);
  assert.equal(bundlePixels({ remote: { readOnly: false }, readOnly: false, bundle: null }), false);
  assert.equal(bundlePixels({ remote: { readOnly: true }, readOnly: true, bundle: {} }), false);
});

test('display-only edits keep a neutral base curve across save and adopt', () => {
  const p = project(), display = { ...neutralTone(), gamma: 2 };
  const saved = structuredClone(p);
  saveBundleTones(saved, source, { a: { tone: curveTone(display) } }, { a: display }, p);
  const overlay = overlayFor(JSON.parse(JSON.stringify(saved)), 'a', 'published');
  assert.deepEqual(overlay.curve, neutralTone());
  assert.equal(overlay.display.gamma, 2);
});
