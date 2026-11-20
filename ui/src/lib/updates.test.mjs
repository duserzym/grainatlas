import assert from 'node:assert/strict';
import test from 'node:test';
import {
  checkButtonView, friendlyUpdateError, pickEndpoint, STABLE_ENDPOINT, transition,
  UPDATE_APP, UPDATE_HOST, UPDATE_UNSUPPORTED_REASON, updateErrorDetails, updateErrorKind,
} from './updates.ts';

const current = '0.1.7';
const info = { version: '0.2.0', currentVersion: current, notes: null, date: null, ready: false };
const configured = 'Updates are not configured for this build yet.';
const signature = "The update's signature did not match. Nothing was installed.";
const certificate = 'Could not verify the secure connection to github.com.';
const noRelease = 'No public release of grainatlas has been published.';
const offline = 'No internet connection, or github.com did not answer.';

test('update messages use the app display name and stable feed host', () => {
  assert.equal(UPDATE_APP, 'grainatlas');
  assert.equal(UPDATE_HOST, new URL(STABLE_ENDPOINT).host);
  assert.equal(UPDATE_HOST, 'github.com');
});

for (const [status, text, secondary, action, badge, busy] of [
  [{ kind: 'idle' }, 'Current version: v0.1.7', null, null, false, false],
  [{ kind: 'checking' }, 'Checking for updates…', null, null, false, true],
  [{ kind: 'none', checkedAt: 1 }, 'Up to date (v0.1.7)', null, null, false, false],
  [{ kind: 'available', info }, 'Update available: v0.2.0', 'Current version: v0.1.7', 'download', true, false],
  [{ kind: 'downloading', info, pct: 43 }, 'Downloading v0.2.0… 43%', null, null, false, true],
  [{ kind: 'downloading', info, pct: null }, 'Downloading v0.2.0…', null, null, false, true],
  [{ kind: 'downloading', info, pct: 0 }, 'Downloading v0.2.0… 0%', null, null, false, true],
  [{ kind: 'ready', info: { ...info, ready: true } }, 'v0.2.0 is ready', null, 'install', true, false],
  [{ kind: 'installing', info }, 'Installing v0.2.0 and restarting…', null, null, false, true],
  [{ kind: 'unsupported', reason: UPDATE_UNSUPPORTED_REASON }, 'grainatlas cannot replace itself in this folder. Download the new version from GitHub.', null, null, false, false],
  [{ kind: 'unsupported', reason: UPDATE_UNSUPPORTED_REASON, info }, 'grainatlas cannot replace itself in this folder. Download the new version from GitHub.', null, 'releases', true, false],
  [{ kind: 'unsupported', reason: 'The shell manages updates.' }, 'The shell manages updates.', null, null, false, false],
  [{ kind: 'error', message: 'HTTP 404' }, noRelease, null, null, false, false],
]) {
  test(`canonical ${status.kind} view: ${text}`, () => {
    assert.deepEqual(checkButtonView(status, current, false), {
      text, secondary, action, badge, busy,
      details: status.kind === 'error' ? status.message : null,
    });
  });
}

test('automatic download keeps the available badge and omits the manual download action', () => {
  assert.equal(checkButtonView({ kind: 'available', info }, current).action, null);
  assert.equal(checkButtonView({ kind: 'available', info }, current).badge, true);
});

for (const [kind, samples, text] of [
  ['not_configured', ['', 'invalid pubkey', 'empty public key', 'Updater public key is not configured', 'Updates are not configured for this build yet'], configured],
  ['certificate', ['invalid peer certificate: UnknownIssuer', 'certificate verify failed', 'CERTIFICATE_VERIFY_FAILED', 'UnknownIssuer', 'error sending request: invalid peer certificate: UnknownIssuer'], certificate],
  ['signature', ['the signature abc could not be verified', 'minisign verification failed', 'Verify', 'The signature could not be decoded, please check if it is a valid base64 string.'], signature],
  ['no_release', ['HTTP 404', 'release JSON', 'Could not fetch a valid release', 'not found', 'no assets'], noRelease],
  ['offline', ['DNS lookup failed', 'connect error', 'timed out', 'timeout', 'offline', 'network unavailable', 'error sending request', 'ENOTFOUND', 'URLError', 'Failed to fetch'], offline],
  ['not_writable', ['install folder not writable', 'Permission denied (os error 13)', 'Access is denied (os error 5)', 'Read-only file system', 'EACCES', 'EPERM'], UPDATE_UNSUPPORTED_REASON],
  ['other', ['unexpected updater response'], 'The update check failed: unexpected updater response'],
]) {
  for (const raw of samples) {
    test(`canonical ${kind} error: ${raw || '(empty key)'}`, () => {
      assert.equal(updateErrorKind(raw), kind);
      assert.equal(friendlyUpdateError(raw), text);
      const view = checkButtonView({ kind: 'error', message: raw }, current);
      assert.equal(view.text, text);
      assert.equal(view.details, raw);
    });
  }
}

test('error classification follows canonical precedence for overlapping diagnostics', () => {
  for (const [raw, kind] of [
    ['invalid public key; invalid peer certificate; signature; HTTP 404; network; permission denied', 'not_configured'],
    ['invalid peer certificate: UnknownIssuer; signature; HTTP 404; network; permission denied', 'certificate'],
    ['minisign signature not found; HTTP 404; network; permission denied', 'signature'],
    ['HTTP 404; error sending request; permission denied', 'no_release'],
    ['network; permission denied', 'offline'],
  ]) assert.equal(updateErrorKind(raw), kind);
});

test('Details preserves native diagnostics and caps the raw excerpt at 120 characters', () => {
  const raw = `unexpected updater response: ${'x'.repeat(200)}`;
  const state = transition({ kind: 'checking' }, { type: 'failed', message: raw });
  assert.deepEqual(state, { kind: 'error', message: raw });
  assert.equal(updateErrorDetails(raw), `${raw.slice(0, 119)}…`);
  assert.equal(updateErrorDetails(raw).length, 120);
  assert.equal(friendlyUpdateError(raw), `The update check failed: ${updateErrorDetails(raw)}`);
  assert.equal(checkButtonView(state, current).details, updateErrorDetails(raw));
  for (const length of [119, 120]) assert.equal(updateErrorDetails('x'.repeat(length)), 'x'.repeat(length));
});

test('stable feed and existing beta channel selection remain separate', () => {
  const beta = 'https://github.com/duserzym/grainatlas/releases/download/v0.2.0-beta.1/latest.json';
  const releases = [{ published_at: '2026-11-19', assets: [{ name: 'latest.json', browser_download_url: beta }] }];
  assert.equal(pickEndpoint('stable', releases), STABLE_ENDPOINT);
  assert.equal(pickEndpoint('beta', releases), beta);
  assert.equal(pickEndpoint('beta', []), STABLE_ENDPOINT);
});
