import assert from 'node:assert/strict';
import test from 'node:test';
import { ImageCache } from './tile-cache.ts';

// A tiny transactional store: the cache's real hashing, accounting and LRU run
// against this fake, without depending on browser IndexedDB in node --test.
function memoryDatabase() {
  const bytes = new Map(), metadata = new Map();
  let size = 0, queue = Promise.resolve();
  return {
    bytes, metadata,
    transaction(_write, work) {
      const result = queue.then(() => work({
        get: async hash => bytes.get(hash)?.slice(0),
        metadata: async hash => metadata.get(hash),
        touch: async entry => { metadata.set(entry.hash, entry); },
        put: async (entry, value) => { metadata.set(entry.hash, entry); bytes.set(entry.hash, value.slice(0)); },
        remove: async hash => { metadata.delete(hash); bytes.delete(hash); },
        oldest: async () => [...metadata.values()].sort((a, b) => a.lastUsed - b.lastUsed),
        size: async () => size,
        setSize: async value => { size = value; },
        clear: async () => { bytes.clear(); metadata.clear(); size = 0; },
      }));
      queue = result.catch(() => {});
      return result;
    },
  };
}
const buffer = (...values) => Uint8Array.from(values).buffer;
async function hash(bytes) {
  return Buffer.from(await crypto.subtle.digest('SHA-256', bytes)).toString('hex');
}

test('miss fetches verified bytes; hit uses stored bytes and updates last use', async () => {
  const db = memoryDatabase();
  let clock = 1, requests = 0;
  const cache = new ImageCache(async () => db, 100, () => clock++);
  const bytes = buffer(1, 2, 3), key = await hash(bytes);
  assert.equal(await cache.get(key), undefined);
  const network = async () => { requests++; return bytes; };
  assert.deepEqual(await cache.load(key, network), bytes);
  assert.equal(await cache.size(), 3);
  const used = db.metadata.get(key).lastUsed;
  assert.deepEqual(await cache.load(key, network), bytes);
  assert.equal(requests, 1);
  assert.ok(db.metadata.get(key).lastUsed > used);
  // Reopening a cache wrapper simulates another session with the same database.
  assert.deepEqual(await new ImageCache(async () => db).load(key, network), bytes);
  assert.equal(requests, 1);
});

test('hash mismatch and missing hashes never enter the cache', async () => {
  const db = memoryDatabase(), cache = new ImageCache(async () => db);
  const bytes = buffer(1, 2), key = await hash(buffer(9));
  for (const id of [key, undefined]) assert.deepEqual(await cache.load(id, async () => bytes), bytes);
  assert.equal(await cache.size(), 0);
  assert.equal(db.bytes.size, 0);
});

test('over cap evicts least recently used down to 80 percent; hits protect entries', async () => {
  const db = memoryDatabase();
  let clock = 0;
  const cache = new ImageCache(async () => db, 10, () => ++clock);
  const values = [buffer(1, 1, 1), buffer(2, 2, 2), buffer(3, 3, 3), buffer(4, 4, 4)];
  const keys = await Promise.all(values.map(hash));
  for (let i = 0; i < 3; i++) await cache.put(keys[i], values[i]);
  assert.equal(await cache.size(), 9);
  await cache.get(keys[0]);
  await cache.put(keys[3], values[3]);
  assert.equal(await cache.size(), 6);
  assert.deepEqual([...db.bytes.keys()].sort(), [keys[0], keys[3]].sort());
  await cache.put(keys[0], values[0]);
  assert.equal(await cache.size(), 6, 'replacing the same hash must not double count');
  await cache.put(await hash(new ArrayBuffer(11)), new ArrayBuffer(11));
  assert.equal(await cache.size(), 6, 'oversized files bypass the cache');
});

test('unavailable IndexedDB, failed transactions and quota errors fall back to network', async () => {
  const bytes = buffer(4, 5), key = await hash(bytes);
  const unavailable = () => { throw new Error('No IndexedDB'); };
  const rejected = async () => { throw new Error('Private mode'); };
  const quota = async () => ({ transaction: async () => { throw new Error('Quota exceeded'); } });
  for (const open of [unavailable, rejected, quota]) {
    const cache = new ImageCache(open);
    assert.deepEqual(await cache.load(key, async () => bytes), bytes);
    assert.equal(await cache.size(), null);
    assert.equal(await cache.clear(), false);
  }
  const cache = new ImageCache(async () => memoryDatabase());
  await assert.rejects(cache.load(key, async () => { throw new Error('Network failed'); }), /Network failed/);
});

test('concurrent writes keep size exact; clear removes only this cache', async () => {
  const db = memoryDatabase(), cache = new ImageCache(async () => db, 100);
  const values = [buffer(1), buffer(2, 3), buffer(4, 5, 6)];
  await Promise.all(values.map(async bytes => cache.put(await hash(bytes), bytes)));
  assert.equal(await cache.size(), 6);
  assert.equal(await cache.clear(), true);
  assert.equal(await cache.size(), 0);
  assert.equal(db.bytes.size, 0);
  assert.equal(db.metadata.size, 0);
});
