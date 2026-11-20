import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import test from 'node:test';
import { checkButtonView, friendlyUpdateError, STABLE_ENDPOINT, UPDATE_APP } from './updates.ts';

const config = JSON.parse(readFileSync(new URL('../../../tauri/src-tauri/tauri.conf.json', import.meta.url), 'utf8'));
const release = JSON.parse(readFileSync(new URL('../../../tauri/src-tauri/tauri.release.conf.json', import.meta.url), 'utf8'));
const endpoint = 'https://github.com/duserzym/grainatlas/releases/latest/download/latest.json';

function base64(value) {
  assert.match(value, /^(?:[A-Za-z0-9+/]{4})*(?:[A-Za-z0-9+/]{2}==|[A-Za-z0-9+/]{3}=)?$/);
  const decoded = Buffer.from(value, 'base64');
  assert.equal(decoded.toString('base64'), value, 'base64 must decode without ignored or invalid characters');
  return decoded;
}

function assertPublicKey(pubkey) {
  assert.equal(typeof pubkey, 'string');
  if (pubkey === '') {
    const raw = 'Updater public key is not configured';
    const expected = 'Updates are not configured for this build yet.';
    assert.equal(friendlyUpdateError(raw), expected);
    assert.equal(checkButtonView({ kind: 'error', message: raw }, config.version).text, expected);
    return;
  }
  const lines = base64(pubkey).toString('utf8').trimEnd().split(/\r?\n/);
  assert.equal(lines.length, 2);
  assert.match(lines[0], /^untrusted comment: minisign public key: .+$/);
  assert.match(lines[1], /^RW[A-Za-z0-9+/]+={0,2}$/);
  const key = base64(lines[1]);
  assert.equal(key.length, 42);
  assert.equal(key.subarray(0, 2).toString('ascii'), 'Ed');
}

test('base and merged release config use the canonical stable feed and display name', () => {
  assert.equal(STABLE_ENDPOINT, endpoint);
  assert.equal(config.productName, UPDATE_APP);
  assert.deepEqual(config.plugins.updater.endpoints, [endpoint]);
  assert.deepEqual(release.plugins?.updater?.endpoints ?? config.plugins.updater.endpoints, [endpoint]);
});

test('release config creates signed updater artifacts', () => {
  assert.equal(release.bundle.createUpdaterArtifacts, true);
});

test('configured updater public keys are empty or wrapped minisign Ed keys', () => {
  assertPublicKey(config.plugins.updater.pubkey);
  assertPublicKey(release.plugins?.updater?.pubkey ?? config.plugins.updater.pubkey);
});

test('an empty updater public key displays the canonical unconfigured message', () => {
  assertPublicKey('');
});

test('key check rejects malformed base64, missing comments and invalid key bytes', () => {
  const wrap = value => Buffer.from(value).toString('base64');
  for (const key of [
    'invalid base64!',
    wrap('RW' + 'A'.repeat(54)),
    wrap('untrusted comment: minisign public key: test\nRWAA'),
    wrap('untrusted comment: minisign public key: test\n' + Buffer.alloc(42).toString('base64')),
    wrap('untrusted comment: minisign public key: test\n' + Buffer.from('Ed').toString('base64') + '!'),
  ]) assert.throws(() => assertPublicKey(key));
});
