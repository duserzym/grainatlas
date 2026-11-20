#!/usr/bin/env node
import { createHash, createPublicKey, verify } from 'node:crypto';
import { readFileSync, writeFileSync } from 'node:fs';
import { basename, resolve } from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';

function base64(value, name) {
  if (typeof value !== 'string' || !value || !/^(?:[A-Za-z0-9+/]{4})*(?:[A-Za-z0-9+/]{2}==|[A-Za-z0-9+/]{3}=)?$/.test(value)) {
    throw new Error(`Invalid ${name} base64`);
  }
  const bytes = Buffer.from(value, 'base64');
  if (bytes.toString('base64') !== value) throw new Error(`Invalid ${name} base64`);
  return bytes;
}

/** Tauri wraps the public key and the four-line minisign signature in base64. */
export function verifyArtifact(file, signature, pubkey) {
  if (!pubkey?.trim()) throw new Error('Updater public key is empty');
  const keyLines = base64(pubkey.trim(), 'public key').toString('utf8').trimEnd().split(/\r?\n/);
  if (keyLines.length !== 2 || !keyLines[0].startsWith('untrusted comment: ')) throw new Error('Invalid public key format');
  const key = base64(keyLines[1], 'public key packet');
  if (key.length !== 42 || key.subarray(0, 2).toString() !== 'Ed') throw new Error('Invalid public key packet');
  const lines = base64(signature.trim(), 'signature').toString('utf8').trimEnd().split(/\r?\n/);
  if (lines.length !== 4 || !lines[0].startsWith('untrusted comment: ') || !lines[2].startsWith('trusted comment: ')) {
    throw new Error('Invalid signature format');
  }
  const packet = base64(lines[1], 'signature packet');
  const global = base64(lines[3], 'global signature');
  const alg = packet.subarray(0, 2).toString();
  if (packet.length !== 74 || !['ED', 'Ed'].includes(alg) || global.length !== 64) throw new Error('Invalid signature packet');
  if (!key.subarray(2, 10).equals(packet.subarray(2, 10))) throw new Error('Signature key id does not match');
  const publicKey = createPublicKey({
    key: Buffer.concat([Buffer.from('302a300506032b6570032100', 'hex'), key.subarray(10)]),
    format: 'der', type: 'spki',
  });
  const sig = packet.subarray(10);
  const message = alg === 'ED' ? createHash('blake2b512').update(file).digest() : file;
  if (!verify(null, message, publicKey, sig)) throw new Error('Artifact signature verification failed');
  const trusted = Buffer.from(lines[2].slice('trusted comment: '.length));
  if (!verify(null, Buffer.concat([sig, trusted]), publicKey, global)) throw new Error('Global signature verification failed');
}

export function generateFeed({ root = fileURLToPath(new URL('../', import.meta.url)), version, darwinAarch64, windowsX8664, out = 'latest.json', now = new Date() }) {
  const config = JSON.parse(readFileSync(resolve(root, 'tauri/src-tauri/tauri.conf.json'), 'utf8'));
  version ??= config.version;
  if (typeof version !== 'string' || !/^\d+\.\d+\.\d+(?:\+[0-9A-Za-z.-]+)?$/.test(version)) throw new Error('Version must be a stable semantic version without a v prefix');
  // Check the original text length, including whitespace, before trimming for display.
  const text = readFileSync(resolve(root, 'release-notes', `${version}.md`), 'utf8');
  if (!text.trim() || Array.from(text).length > 600) throw new Error('Release notes must be nonempty and at most 600 characters');
  if (/^\s*#/m.test(text) || (text.match(/^- /gm)?.length ?? 0) > 5) throw new Error('Release notes must have no headings and at most five bullets');
  if (!darwinAarch64) throw new Error('--darwin-aarch64 is required');
  if (!config.plugins?.updater?.pubkey?.trim()) throw new Error('Updater public key is empty');
  const platforms = {};
  for (const [platform, file] of [['darwin-aarch64', darwinAarch64], ['windows-x86_64', windowsX8664]]) {
    if (!file) continue;
    if (platform === 'darwin-aarch64' ? !file.endsWith('.app.tar.gz') : !file.endsWith('setup.exe')) throw new Error(`Invalid ${platform} artifact filename`);
    const artifact = resolve(root, file);
    const signature = readFileSync(`${artifact}.sig`, 'utf8');
    verifyArtifact(readFileSync(artifact), signature, config.plugins.updater.pubkey);
    platforms[platform] = {
      url: `https://github.com/duserzym/grainatlas/releases/download/v${version}/${encodeURIComponent(basename(file))}`,
      signature,
    };
  }
  const feed = { version, notes: text.trim(), pub_date: now.toISOString(), platforms };
  writeFileSync(resolve(root, out), `${JSON.stringify(feed, null, 2)}\n`);
  return feed;
}

export function parseArgs(args) {
  const names = { '--version': 'version', '--darwin-aarch64': 'darwinAarch64', '--windows-x86_64': 'windowsX8664', '--out': 'out' };
  const options = {};
  for (let i = 0; i < args.length; i += 2) {
    const key = names[args[i]];
    if (!key || key in options || !args[i + 1] || args[i + 1].startsWith('--')) throw new Error(`Invalid argument: ${args[i]}`);
    options[key] = args[i + 1];
  }
  return options;
}

if (process.argv[1] && import.meta.url === pathToFileURL(resolve(process.argv[1])).href) {
  try { generateFeed(parseArgs(process.argv.slice(2))); }
  catch (error) { console.error(`release-feed: ${error.message}`); process.exitCode = 1; }
}
