import assert from 'node:assert/strict';
import test from 'node:test';
import { exportOptions, writeGrainExport, writeFolderBatch, writeDirectoryFile } from './grain-export.ts';
const choice = { scope: 'area', pixel_values: 'original', format: 'png', margin_um: 10, include_crops: true, include_masks: true };
test('dialog options preserve formats, masks, table-only scope and selection', () => {
  const tone = { a: { gamma: 1 } };
  const options = exportOptions(choice, 'a', ['g'], tone, 'now');
  assert.equal(options.selected, null); assert.equal(options.area_id, 'a'); assert.equal(options.exported, 'now');
  tone.a.gamma = 2; assert.equal(options.display_tone.a.gamma, 1);
  const selected = exportOptions({ ...choice, scope: 'selected', format: 'tiff', pixel_values: 'displayed' }, 'a', ['g'], {}, 'now');
  assert.deepEqual(selected.selected, ['g']); assert.equal(selected.format, 'tiff'); assert.equal(selected.pixel_values, 'displayed');
  const table = exportOptions({ ...choice, scope: 'mount', include_crops: false, include_masks: false }, 'a', [], {}, 'now');
  assert.equal(table.area_id, null); assert.equal(table.include_crops, false); assert.equal(table.include_masks, false);
  for (const margin_um of [-1, NaN, Infinity]) assert.throws(() => exportOptions({ ...choice, margin_um }, 'a', [], {}, 'now'));
  assert.throws(() => exportOptions({ ...choice, scope: 'selected' }, 'a', [], {}, 'now'));
  assert.throws(() => exportOptions(choice, null, [], {}, 'now'));
});
function coreHarness() {
  const calls = []; let n = 0;
  const core = {
    beginGrainExport: async () => calls.push('begin'),
    grainExportBatch: async zip => { calls.push(`batch:${zip}`); return { data: new Uint8Array([++n]), done: n === 3 }; },
    abortGrainExport: async () => calls.push('abort'),
  };
  return { core, calls };
}
test('browser drains each batch before requesting the next, including final metadata', async () => {
  const { core, calls } = coreHarness();
  await writeGrainExport(core, {}, {}, { zipStream: true, write: async b => { calls.push(`write:${b[0]}`); }, finish: async () => { calls.push('finish'); } }, () => {}, () => false);
  assert.deepEqual(calls, ['begin', 'batch:true', 'write:1', 'batch:true', 'write:2', 'batch:true', 'write:3', 'finish', 'abort']);
});
test('browser cancellation preserves source worker and discards pending batch', async () => {
  const { core, calls } = coreHarness(); let cancelled = false;
  await assert.rejects(writeGrainExport(core, {}, {}, { zipStream: false, write: async () => { cancelled = true; }, finish: async () => assert.fail('must not finish') }, () => {}, () => cancelled), /cancelled/);
  assert.deepEqual(calls, ['begin', 'batch:false', 'abort']);
});
test('browser write errors release the export and never finish the sink', async () => {
  const { core, calls } = coreHarness();
  await assert.rejects(writeGrainExport(core, {}, {}, { zipStream: false, write: async () => { throw new Error('disk full'); }, finish: async () => assert.fail('must not finish') }, () => {}, () => false), /disk full/);
  assert.equal(calls.at(-1), 'abort');
});
function directory() {
  const files = new Map();
  const dir = prefix => ({
    getDirectoryHandle: async name => dir(`${prefix}${name}/`),
    getFileHandle: async name => ({ createWritable: async () => ({ write: async bytes => files.set(`${prefix}${name}`, [...bytes]), close: async () => {} }) }),
  });
  return { root: dir(''), files };
}
function batch(name, bytes) {
  const n = new TextEncoder().encode(name), b = new Uint8Array(30 + n.length + bytes.length + 22), v = new DataView(b.buffer);
  v.setUint32(0, 0x04034b50, true); v.setUint32(18, bytes.length, true); v.setUint16(26, n.length, true);
  b.set(n, 30); b.set(bytes, 30 + n.length); v.setUint32(30 + n.length + bytes.length, 0x06054b50, true); return b;
}
test('folder writer handles nested crops, empty batches, invalid paths and short writes', async () => {
  const { root, files } = directory();
  await writeFolderBatch(root, batch('crops/a.png', new Uint8Array([1, 2, 3])));
  assert.deepEqual(files.get('crops/a.png'), [1, 2, 3]);
  const empty = new Uint8Array(22); new DataView(empty.buffer).setUint32(0, 0x06054b50, true); await writeFolderBatch(root, empty);
  await assert.rejects(writeFolderBatch(root, batch('../escape', new Uint8Array([1]))), /Invalid export file path/);
  await assert.rejects(writeFolderBatch(root, new Uint8Array([1, 2])), /Invalid grain export/);
  const truncated = batch('a', new Uint8Array([1])); new DataView(truncated.buffer).setUint32(18, 900, true);
  await assert.rejects(writeFolderBatch(root, truncated), /Truncated/);
  let aborted = false;
  await assert.rejects(writeDirectoryFile({ getFileHandle: async () => ({ createWritable: async () => ({ write: async () => { throw new Error('write failed'); }, close: async () => assert.fail(), abort: async () => { aborted = true; } }) }) }, 'a', new Uint8Array()), /write failed/);
  assert.equal(aborted, true);
});
