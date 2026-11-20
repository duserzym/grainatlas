import assert from 'node:assert/strict';
import { createHash, generateKeyPairSync, randomBytes, sign } from 'node:crypto';
import { mkdtempSync, mkdirSync, readFileSync, writeFileSync, rmSync, existsSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import test from 'node:test';
import { generateFeed, parseArgs, verifyArtifact } from './release-feed.mjs';

// Test-only minisign writer; never reads or creates a release signing key.
function signer() {
  const { publicKey, privateKey } = generateKeyPairSync('ed25519');
  const id = randomBytes(8);
  const packet = Buffer.concat([Buffer.from('Ed'), id, publicKey.export({ type: 'spki', format: 'der' }).subarray(-32)]);
  const pubkey = Buffer.from(`untrusted comment: minisign public key: test\n${packet.toString('base64')}\n`).toString('base64');
  function signature(bytes, alg = 'ED', trusted = 'timestamp:123 file:test', keyId = id) {
    const message = alg === 'ED' ? createHash('blake2b512').update(bytes).digest() : bytes;
    const sig = sign(null, message, privateKey);
    const packet = Buffer.concat([Buffer.from(alg), keyId, sig]);
    const global = sign(null, Buffer.concat([sig, Buffer.from(trusted)]), privateKey);
    return Buffer.from(`untrusted comment: test\n${packet.toString('base64')}\ntrusted comment: ${trusted}\n${global.toString('base64')}\n`).toString('base64');
  }
  return { pubkey, signature };
}
function changeLine(signature, at, change) {
  const lines = Buffer.from(signature, 'base64').toString().trimEnd().split('\n');
  lines[at] = change(lines[at]);
  return Buffer.from(lines.join('\n') + '\n').toString('base64');
}
function flip(value) { const bytes = Buffer.from(value, 'base64'); bytes[bytes.length - 1] ^= 1; return bytes.toString('base64'); }

for (const alg of ['ED', 'Ed']) {
  test(`verifies minisign ${alg} artifact and global signatures`, () => {
    const key = signer(), bytes = Buffer.from('synthetic artifact');
    assert.doesNotThrow(() => verifyArtifact(bytes, key.signature(bytes, alg), key.pubkey));
  });
  test(`rejects changed artifact, signatures, trusted comment and key for ${alg}`, () => {
    const key = signer(), bytes = Buffer.from('synthetic artifact'), sig = key.signature(bytes, alg);
    assert.throws(() => verifyArtifact(Buffer.from('changed'), sig, key.pubkey), /Artifact signature/);
    assert.throws(() => verifyArtifact(bytes, changeLine(sig, 1, flip), key.pubkey), /Artifact signature/);
    assert.throws(() => verifyArtifact(bytes, changeLine(sig, 3, flip), key.pubkey), /Global signature/);
    assert.throws(() => verifyArtifact(bytes, changeLine(sig, 2, s => s + ' changed'), key.pubkey), /Global signature/);
    assert.throws(() => verifyArtifact(bytes, sig, signer().pubkey), /key id/);
    assert.throws(() => verifyArtifact(bytes, key.signature(bytes, alg, 'comment', randomBytes(8)), key.pubkey), /key id/);
  });
}

test('matching key ids still require the correct cryptographic key', () => {
  const key = signer(), wrong = signer(), bytes = Buffer.from('synthetic artifact');
  const originalLines = Buffer.from(key.pubkey, 'base64').toString().trimEnd().split('\n');
  const wrongLines = Buffer.from(wrong.pubkey, 'base64').toString().trimEnd().split('\n');
  const packet = Buffer.from(wrongLines[1], 'base64');
  Buffer.from(originalLines[1], 'base64').copy(packet, 2, 2, 10);
  wrongLines[1] = packet.toString('base64');
  assert.throws(() => verifyArtifact(bytes, key.signature(bytes), Buffer.from(wrongLines.join('\n')).toString('base64')), /Artifact signature/);
});

test('rejects empty keys and malformed minisign encodings, packets and algorithms', () => {
  const key = signer(), bytes = Buffer.from('synthetic'), sig = key.signature(bytes);
  for (const pubkey of ['', '!', Buffer.from('untrusted comment: key\nAAAA').toString('base64')]) {
    assert.throws(() => verifyArtifact(bytes, sig, pubkey));
  }
  for (const bad of ['', sig + '!', changeLine(sig, 0, () => 'missing comment'), changeLine(sig, 1, () => 'AAAA'), changeLine(sig, 2, () => 'missing trusted comment'), changeLine(sig, 3, () => 'AAAA'), key.signature(bytes, 'XX')]) {
    assert.throws(() => verifyArtifact(bytes, bad, key.pubkey));
  }
});

function fixture(t) {
  const root = mkdtempSync(join(tmpdir(), 'grainatlas-release-feed-'));
  t.after(() => rmSync(root, { recursive: true, force: true }));
  mkdirSync(join(root, 'tauri/src-tauri'), { recursive: true });
  mkdirSync(join(root, 'release-notes'));
  const key = signer();
  const config = join(root, 'tauri/src-tauri/tauri.conf.json');
  writeFileSync(config, JSON.stringify({ version: '1.2.3', plugins: { updater: { pubkey: key.pubkey } } }));
  const notes = join(root, 'release-notes/1.2.3.md');
  writeFileSync(notes, '- Print sheets fit A4 and Letter.\n');
  for (const file of ['grainatlas.app.tar.gz', 'grainatlas_x64-setup.exe']) {
    const bytes = Buffer.from(`synthetic ${file}`);
    writeFileSync(join(root, file), bytes);
    writeFileSync(join(root, file + '.sig'), key.signature(bytes));
  }
  return { root, key, config, notes, options: { root, darwinAarch64: 'grainatlas.app.tar.gz', out: 'latest.json', now: new Date('2026-11-20T12:00:00Z') } };
}

test('writes the default version, notes, ISO date and both verified platform URLs', t => {
  const { root, options } = fixture(t);
  const feed = generateFeed({ ...options, windowsX8664: 'grainatlas_x64-setup.exe' });
  assert.equal(feed.version, '1.2.3');
  assert.equal(feed.notes, '- Print sheets fit A4 and Letter.');
  assert.equal(feed.pub_date, '2026-11-20T12:00:00.000Z');
  assert.deepEqual(Object.keys(feed.platforms), ['darwin-aarch64', 'windows-x86_64']);
  for (const [platform, file] of [['darwin-aarch64', 'grainatlas.app.tar.gz'], ['windows-x86_64', 'grainatlas_x64-setup.exe']]) {
    assert.deepEqual(feed.platforms[platform], {
      url: `https://github.com/duserzym/grainatlas/releases/download/v1.2.3/${file}`,
      signature: readFileSync(join(root, file + '.sig'), 'utf8'),
    });
  }
  assert.deepEqual(JSON.parse(readFileSync(join(root, 'latest.json'))), feed);
});

test('explicit version and macOS-only feed work with 600 characters of notes', t => {
  const { root, options } = fixture(t);
  writeFileSync(join(root, 'release-notes/1.2.4.md'), 'a'.repeat(600));
  const feed = generateFeed({ ...options, version: '1.2.4' });
  assert.equal(feed.version, '1.2.4');
  assert.equal(feed.notes.length, 600);
  assert.deepEqual(Object.keys(feed.platforms), ['darwin-aarch64']);
});

for (const [name, change] of [
  ['missing notes', f => rmSync(f.notes)],
  ['empty notes', f => writeFileSync(f.notes, ' \n')],
  ['long notes', f => writeFileSync(f.notes, 'a'.repeat(601))],
  ['headings', f => writeFileSync(f.notes, '# Changes')],
  ['six bullets', f => writeFileSync(f.notes, '- Change\n'.repeat(6))],
  ['empty pubkey', f => writeFileSync(f.config, JSON.stringify({ version: '1.2.3', plugins: { updater: { pubkey: '' } } }))],
  ['missing signature', f => rmSync(join(f.root, 'grainatlas.app.tar.gz.sig'))],
  ['changed artifact', f => writeFileSync(join(f.root, 'grainatlas.app.tar.gz'), 'changed')],
  ['bad Windows signature', f => writeFileSync(join(f.root, 'grainatlas_x64-setup.exe.sig'), '!')],
]) {
  test(`refuses ${name} before touching the output`, t => {
    const f = fixture(t);
    change(f);
    const options = { ...f.options, windowsX8664: 'grainatlas_x64-setup.exe' };
    assert.throws(() => generateFeed(options));
    assert.equal(existsSync(join(f.root, 'latest.json')), false);
    writeFileSync(join(f.root, 'latest.json'), 'existing feed');
    assert.throws(() => generateFeed(options));
    assert.equal(readFileSync(join(f.root, 'latest.json'), 'utf8'), 'existing feed');
  });
}

test('CLI accepts only the named arguments with values', () => {
  assert.deepEqual(parseArgs(['--version', '1.2.3', '--darwin-aarch64', 'a.app.tar.gz', '--windows-x86_64', 'setup.exe', '--out', 'latest.json']), {
    version: '1.2.3', darwinAarch64: 'a.app.tar.gz', windowsX8664: 'setup.exe', out: 'latest.json',
  });
  for (const args of [['--wrong', 'x'], ['--version'], ['--out', '--version'], ['--out', 'a', '--out', 'b']]) assert.throws(() => parseArgs(args));
});
