import assert from 'node:assert/strict';
import test from 'node:test';
import { execFileSync } from 'node:child_process';
import { mosaicEstimate, mosaicName, mosaicFitFactor, mosaicFilename, mosaicRasterEstimate, mosaicBytes, mosaicCanStream, mosaicFolderSink, mosaicZipSink, writeMosaicExport, MOSAIC_BLOB_LIMIT } from './mosaic-export.ts';
test('mosaic estimates include padded tiles, arrays and BigTIFF headroom', () => {
  assert.deepEqual(mosaicEstimate(513, 517, 16), { bytes: 4 * 512 * 512 * 2 + 32 + 1048576, big: false });
  assert.equal(mosaicEstimate(100000, 100000, 16).big, true);
  assert.equal(mosaicEstimate(512, 512, 8, 3).bytes, 512 * 512 * 3 + 8 + 1048576);
  for (const args of [[0,1,8],[1,NaN,8],[1,1,32],[1.5,2,8],[1,1,8,4]]) assert.throws(() => mosaicEstimate(...args));
  assert.equal(mosaicBytes({ images: [{ estimated_bytes: 2 }, { estimated_bytes: 3 }] }), 5);
  assert.equal(MOSAIC_BLOB_LIMIT, 536870912);
});
test('mosaic names are portable and retain user text', () => {
  assert.equal(mosaicName('../Area:A? '), '_Area_A_');
  assert.equal(mosaicName('NUL'), '_NUL'); assert.equal(mosaicName('lpt9.txt'), '_lpt9.txt');
  assert.equal(mosaicName(' . '), 'mosaic'); assert.equal(mosaicName('Grain α'), 'Grain α');
  assert.equal([...mosaicName('α'.repeat(101))].length, 100);
});
function harness() {
  const calls = []; let n = 0;
  const core = {
    beginMosaicExport: async () => { calls.push('begin'); return { images: [] }; },
    mosaicExportNext: async () => { calls.push(`next${++n}`); return { name: 'a.tiff', data: new Uint8Array([n]), last: n === 3, done: n === 3, tiles_done: n, tiles_total: 3 }; },
    abortMosaicExport: async () => calls.push('release'),
  };
  const sink = { write: async c => { calls.push(`write${c.data[0]}`); }, finish: async () => { calls.push('finish'); }, abort: async () => { calls.push('cleanup'); } };
  return { core, sink, calls };
}
test('mosaic worker backpressure drains before requesting another chunk', async () => {
  const { core, sink, calls } = harness(); const progress = [];
  await writeMosaicExport(core, {}, {}, sink, p => progress.push(p.fraction), () => false);
  assert.deepEqual(calls, ['begin','next1','write1','next2','write2','next3','write3','finish','release']);
  assert.deepEqual(progress, [1/3,2/3,1]);
});
test('mosaic cancel and write errors clean files and preserve the worker', async () => {
  for (const fail of ['cancel','write','close']) {
    const { core, sink, calls } = harness(); let cancelled = false;
    if (fail === 'cancel') sink.write = async () => { cancelled = true; };
    if (fail === 'write') sink.write = async () => { throw new Error('disk full'); };
    if (fail === 'close') sink.finish = async () => { throw new Error('flush failed'); };
    await assert.rejects(writeMosaicExport(core, {}, {}, sink, () => {}, () => cancelled));
    assert.deepEqual(calls.slice(-2), ['cleanup','release']);
  }
});
test('mosaic stale plan fails before writing pixels', async () => {
  const { core, sink, calls } = harness();
  await assert.rejects(writeMosaicExport(core, {}, {}, sink, () => {}, () => false, { images: [{ width: 12 }] }), /mosaic changed/);
  assert.deepEqual(calls, ['begin','cleanup','release']);
});
test('mosaic folder sink closes files and removes a cancelled directory', async () => {
  const calls = [];
  const dir = { getFileHandle: async name => ({ createWritable: async () => ({ write: async b => calls.push([name, ...b]), close: async () => calls.push('close'), abort: async () => calls.push('abort') }) }) };
  const sink = mosaicFolderSink(dir, async () => { calls.push('remove'); });
  await sink.write({ name: 'a.tiff', data: new Uint8Array([1]) });
  await sink.write({ name: 'a.tiff', data: new Uint8Array([2]), last: true });
  await sink.write({ name: 'a.json', data: new Uint8Array([3]) }); await sink.abort();
  assert.deepEqual(calls, [['a.tiff',1],['a.tiff',2],'close',['a.json',3],'abort','remove']);
  await assert.rejects(sink.write({ name: '../a', data: new Uint8Array([1]) }), /Invalid/);
});
test('streamed mosaic ZIP64 has valid offsets, CRC and UTF-8 names', async () => {
  const parts = []; let closed = false;
  const sink = mosaicZipSink(async b => { parts.push(b); }, async () => { closed = true; }, async () => assert.fail());
  await sink.write({ name: 'Area α.tiff', data: new Uint8Array([1,2]) });
  await sink.write({ name: 'Area α.tiff', data: new Uint8Array([3]), last: true });
  await sink.write({ name: 'Area α.json', data: new TextEncoder().encode('{}'), last: true });
  await sink.finish(); assert.equal(closed, true);
  const result = JSON.parse(execFileSync('python3', ['-c', 'import io,json,sys,zipfile; z=zipfile.ZipFile(io.BytesIO(sys.stdin.buffer.read())); assert z.testzip() is None; assert all(i.date_time[:3]==(1980,1,1) for i in z.infolist()); print(json.dumps({n:list(z.read(n)) for n in z.namelist()}))'], { input: Buffer.concat(parts), encoding: 'utf8' }));
  assert.deepEqual(result, { 'Area α.tiff': [1,2,3], 'Area α.json': [123,125] });
});

test('mosaic streaming capability needs a callable picker', () => {
  assert.equal(mosaicCanStream({}), false);
  assert.equal(mosaicCanStream({ showSaveFilePicker: undefined }), false);
  assert.equal(mosaicCanStream({ showSaveFilePicker: async () => {} }), true);
  assert.equal(mosaicCanStream({ showDirectoryPicker: async () => {} }), true);
});

test('format limits use powers of two and automatic compound names', () => {
  assert.equal(mosaicFitFactor(65535, 65535), 1); assert.equal(mosaicFitFactor(65536, 10), 2); assert.equal(mosaicFitFactor(262141, 5), 8);
  assert.throws(() => mosaicFitFactor(0, 1));
  assert.equal(mosaicFilename('Mount:A', 'Area/1', 'png'), 'Mount_A_Area_1_mosaic.png');
  assert.equal(mosaicFilename('Mount', null, 'jpg'), 'Mount_mount_mosaic.jpg');
  assert.ok(new TextEncoder().encode(mosaicFilename('α'.repeat(200), 'β'.repeat(200), 'tiff')).length < 255);
  assert.equal(mosaicRasterEstimate(100, 50, 16, 'png'), 100 * 50 * 4 + 1048576);
  assert.equal(mosaicRasterEstimate(100, 50, 8, 'jpg'), 100 * 50 * 8 + 1048576);
  assert.throws(() => mosaicRasterEstimate(100, 50, 16, 'jpg'));
});
