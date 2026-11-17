import assert from 'node:assert/strict';
import test from 'node:test';
import { comparablePixelKey, pixelSignature } from './sync-pixels.ts';
import { canonicalJson } from './sync-hash.ts';
import { neutralTone } from './bundle-tone.ts';

const project = () => ({ areas: [{ id: 'area', params: { tone: neutralTone(), stitch: {}, photometric: {}, blend: {} },
  mosaic: { sha256: 'synthetic-pixels', w: 32, h: 24, um_per_px: 1 } }] });

test('old and new pixel keys ignore processing controls, retaining published inputs', () => {
  const p = project(), key = pixelSignature(p);
  const old = canonicalJson(JSON.parse(key).map(a => ({ ...a, stitch: { mode: 'synthetic' }, photometric: { gain: 2 }, blend: { feather: 4 } })));
  assert.equal(comparablePixelKey(old), comparablePixelKey(key));
  for (const field of ['stitch', 'photometric', 'blend']) p.areas[0].params[field] = { changed: true };
  assert.equal(pixelSignature(p), key);
  assert.equal(comparablePixelKey(old), comparablePixelKey(pixelSignature(p)));
});

test('comparison normalises nested tone, bundle tone and scale numbers to f32', () => {
  const p = project();
  p.areas[0].mosaic.um_per_px = 0.8;
  p.grainatlas_ui = { bundle_tone: { area: { curve: neutralTone(), display: neutralTone() } } };
  const key = pixelSignature(p);
  const legacy = JSON.parse(key);
  legacy[0].um_per_px = 0.800000012;
  legacy[0].tone.grain_norm.strength = 0.800000012;
  legacy[0].bundle_tone.curve.grain_norm.strength = 0.800000012;
  legacy[0].bundle_tone.display.grain_norm.strength = 0.800000012;
  assert.equal(comparablePixelKey(JSON.stringify(legacy)), comparablePixelKey(key));
  const compared = JSON.parse(comparablePixelKey(key))[0];
  assert.equal(compared.um_per_px, Math.fround(0.8));
  assert.equal(compared.tone.grain_norm.strength, Math.fround(0.8));
  for (const field of ['curve', 'display']) {
    assert.equal(compared.bundle_tone[field].grain_norm.strength, Math.fround(0.8));
  }

  legacy[0].tone.extra = [0.8, { integer: 16777217, text: '0.800000012', enabled: false, reference: null }];
  const extra = JSON.parse(comparablePixelKey(JSON.stringify(legacy)))[0].tone.extra;
  assert.deepEqual(extra, [Math.fround(0.8), { integer: 16777217, text: '0.800000012', enabled: false, reference: null }]);
});

test('every published mosaic field, display tone and bundle overlay changes the key', () => {
  const key = pixelSignature(project());
  for (const [field, value] of Object.entries({ sha256: 'new-pixels', w: 64, h: 48, um_per_px: 2 })) {
    const p = project(); p.areas[0].mosaic[field] = value;
    assert.notEqual(pixelSignature(p), key, field);
  }
  const renamed = project(); renamed.areas[0].id = 'other-area';
  assert.notEqual(pixelSignature(renamed), key);
  const p = project(); p.areas[0].params.tone.gamma = 2;
  assert.notEqual(pixelSignature(p), key, 'parameter tone fallback');
  p.grainatlas_ui = { tone: { area: neutralTone() } };
  assert.equal(pixelSignature(p), key, 'display tone overrides parameter tone');
  p.grainatlas_ui.tone.area.gamma = 3;
  assert.notEqual(pixelSignature(p), key, 'display tone');
  for (const field of ['curve', 'display']) {
    const q = project();
    q.grainatlas_ui = { bundle_tone: { area: { curve: neutralTone(), display: neutralTone(), on_sha256: 'binding' } } };
    const overlay = pixelSignature(q);
    q.grainatlas_ui.bundle_tone.area[field].gamma = 2;
    assert.notEqual(pixelSignature(q), overlay, field);
  }
});

test('pixel keys canonicalise area order and ignore areas without mosaics', () => {
  const p = project();
  p.areas.push({ ...structuredClone(p.areas[0]), id: 'second' });
  const key = pixelSignature(p);
  p.areas.reverse();
  p.areas.push({ id: 'unstitched', params: {} });
  assert.equal(pixelSignature(p), key);
  assert.equal(comparablePixelKey(JSON.stringify(JSON.parse(key).reverse())), comparablePixelKey(key));
  assert.equal(pixelSignature({ areas: [] }), '[]');
});

test('missing and damaged signatures are unknown', () => {
  for (const value of [undefined, null, '', '{', 'null', '{}', '[null]', '[{}]', '[{"id":"area"}]']) {
    assert.equal(comparablePixelKey(value), null, String(value));
  }
});
