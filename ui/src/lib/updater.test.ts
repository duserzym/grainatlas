import bundledNotes from '../../../release-notes/0.1.8.md?raw';
import { afterEach, beforeEach, expect, test, vi } from 'vitest';

const native = vi.hoisted(() => ({ invoke: vi.fn(), getVersion: vi.fn(), hosted: false }));
vi.mock('@lab/bridge', () => ({ isHosted: () => native.hosted }));
vi.mock('./core/core', () => ({ isDesktop: () => true }));
vi.mock('@tauri-apps/api/core', () => ({ invoke: native.invoke }));
vi.mock('@tauri-apps/api/app', () => ({ getVersion: native.getVersion }));

let saved: string | null;
beforeEach(() => {
  vi.resetModules();
  vi.useFakeTimers();
  saved = null;
  native.hosted = false;
  native.invoke.mockReset().mockImplementation(async command => command === 'update_supported' ? true : null);
  native.getVersion.mockReset().mockResolvedValue('0.1.8');
  vi.stubGlobal('__APP_VERSION__', '0.1.8');
  vi.stubGlobal('$state', <T>(value: T) => value);
  vi.stubGlobal('localStorage', { getItem: () => saved, setItem: (_key: string, value: string) => { saved = value; } });
});
afterEach(() => { vi.useRealTimers(); vi.unstubAllGlobals(); });
async function start() {
  const module = await import('./updater.svelte');
  const stop = module.startScheduler();
  await vi.dynamicImportSettled();
  return { ...module, stop };
}

test('first desktop launch records version without a card', async () => {
  const { updater, stop } = await start();
  expect(updater.whatsNewOpen).toBe(false);
  expect(JSON.parse(saved!).lastSeenVersion).toBe('0.1.8');
  stop();
});

test('manual reinstall shows bundled notes once and can reopen them', async () => {
  saved = JSON.stringify({ lastSeenVersion: '0.1.7' });
  const { updater, openWhatsNew, stop } = await start();
  expect(updater.whatsNewOpen).toBe(true);
  expect(updater.whatsNewNotes).toBe(bundledNotes.trim());
  updater.whatsNewOpen = false;
  openWhatsNew();
  expect(updater.whatsNewOpen).toBe(true);
  stop();
  vi.resetModules();
  const next = await start();
  expect(next.updater.whatsNewOpen).toBe(false);
  next.stop();
});

test('unbundled version uses the saved manifest notes, even in an unsupported install folder', async () => {
  saved = JSON.stringify({ lastSeenVersion: '9.9.8', pendingNotes: { version: '9.9.9', notes: 'Saved manifest notes' } });
  native.getVersion.mockResolvedValue('9.9.9');
  native.invoke.mockResolvedValue(false);
  const { updater, stop } = await start();
  expect(updater.whatsNewOpen).toBe(true);
  expect(updater.whatsNewNotes).toBe('Saved manifest notes');
  expect(updater.status.kind).toBe('unsupported');
  stop();
});

test('installation persists the target notes before the native install call', async () => {
  const { updater, install, stop } = await start();
  updater.status = { kind: 'ready', info: { version: '0.1.9', currentVersion: '0.1.8', notes: 'New notes', ready: true, date: null } };
  native.invoke.mockImplementation(async command => {
    expect(command).toBe('update_install');
    expect(JSON.parse(saved!).pendingNotes).toEqual({ version: '0.1.9', notes: 'New notes' });
  });
  await install();
  expect(updater.status.kind).toBe('installing');
  stop();
});

test('legacy beta preferences check only the stable feed', async () => {
  saved = JSON.stringify({ channel: 'beta', autoDownload: false });
  const { check, stop } = await start();
  await check(true);
  expect(native.invoke).toHaveBeenCalledWith('update_check', { endpoint: 'https://github.com/duserzym/grainatlas/releases/latest/download/latest.json' });
  expect(JSON.parse(saved!)).not.toHaveProperty('channel');
  stop();
});

test('hosted module does not check, record versions or show its own card', async () => {
  native.hosted = true;
  saved = JSON.stringify({ lastSeenVersion: '0.1.7' });
  const { updater, check, stop } = await start();
  await check(true);
  expect(updater.whatsNewOpen).toBe(false);
  expect(native.invoke).not.toHaveBeenCalled();
  expect(JSON.parse(saved!).lastSeenVersion).toBe('0.1.7');
  stop();
});

test('release notes open the exact tag through the native browser command', async () => {
  const { openReleaseNotes } = await import('./updater.svelte');
  await openReleaseNotes('0.1.9');
  expect(native.invoke).toHaveBeenCalledWith('update_open_release_notes', { url: 'https://github.com/duserzym/grainatlas/releases/tag/v0.1.9' });
});
