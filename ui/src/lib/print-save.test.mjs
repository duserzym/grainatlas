import assert from 'node:assert/strict';
import test from 'node:test';
import { printSavePacket } from './print-save.ts';
test('one raw save request contains the complete Unicode registration and PDF bytes', () => {
  const json = JSON.stringify({ name: 'Mount µ', points: [{ id: 'registered', x: 12 }] });
  const pdf = Uint8Array.of(37, 80, 68, 70, 0, 255);
  const packet = printSavePacket(pdf, json), n = new DataView(packet.buffer).getUint32(0, true);
  assert.equal(new TextDecoder().decode(packet.subarray(4, 4+n)), json);
  assert.deepEqual(packet.subarray(4+n), pdf);
  assert.deepEqual(printSavePacket(pdf, null), Uint8Array.from([0,0,0,0,...pdf]));
});
