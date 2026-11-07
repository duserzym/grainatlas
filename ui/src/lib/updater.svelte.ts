import { isHosted } from '@lab/bridge';
import { isDesktop } from './core/core';
import { loadPrefs, pickEndpoint, savePrefs, STABLE_ENDPOINT, transition, type UpdateEvent, type UpdateInfo, type UpdatePrefs, type UpdateStatus } from './updates';

const HOSTED_REASON = 'UPb Lab updates grainatlas with the shell.';
const WEB_REASON = 'Updates come with the desktop app; the web page is always the version its server serves.';
const INSTALL_REASON = 'Install grainatlas to receive in-app updates on this computer.';
function storage() {
  try { return globalThis.localStorage; } catch { return undefined; }
}
const previewName = typeof window !== 'undefined' && !('__TAURI_INTERNALS__' in window)
  ? new URLSearchParams(window.location.search).get('update-preview') : null;
export const updatePreview = ['none', 'available', 'downloading', 'ready', 'error', 'unsupported'].includes(previewName ?? '');
const fake: UpdateInfo = { version: '0.2.0', currentVersion: '0.1.0', notes: 'Faster stitching, new checks', date: null, ready: false };
function previewStatus(): UpdateStatus {
  switch (previewName) {
    case 'none': return { kind: 'none', checkedAt: Date.now() };
    case 'available': return { kind: 'available', info: fake };
    case 'downloading': return { kind: 'downloading', info: fake, pct: 43 };
    case 'ready': return { kind: 'ready', info: { ...fake, ready: true } };
    case 'error': return { kind: 'error', message: 'Updates are not configured for this build yet' };
    default: return { kind: 'unsupported', reason: INSTALL_REASON, info: fake };
  }
}

export const updater = $state({
  status: (updatePreview ? previewStatus() : isHosted() ? { kind: 'unsupported', reason: HOSTED_REASON }
    : isDesktop() ? { kind: 'idle' } : { kind: 'unsupported', reason: WEB_REASON }) as UpdateStatus,
  prefs: loadPrefs(storage()),
  version: __APP_VERSION__,
  supported: false,
  manualNoneUntil: updatePreview && previewName === 'none' ? Date.now() + 5000 : 0,
});

export function setPrefs(patch: Partial<UpdatePrefs>) {
  updater.prefs = { ...updater.prefs, ...patch };
  savePrefs(updater.prefs, storage());
}
function send(event: UpdateEvent) { updater.status = transition(updater.status, event); }
function fail(error: unknown) {
  const raw = String(error);
  const message = /pubkey|public key|base64|decode/i.test(raw) ? 'Updates are not configured for this build yet' : raw;
  send({ type: 'failed', message });
  setPrefs({ lastResult: `Update check failed: ${message}` });
}
async function invoke<T>(command: string, args?: Record<string, unknown>): Promise<T> {
  return (await import('@tauri-apps/api/core')).invoke<T>(command, args);
}
async function endpoint() {
  if (updater.prefs.channel === 'stable') return STABLE_ENDPOINT;
  try {
    const res = await fetch('https://api.github.com/repos/duserzym/grainatlas/releases?per_page=10', {
      headers: { Accept: 'application/vnd.github+json' }, signal: AbortSignal.timeout(15_000),
    });
    if (!res.ok) return STABLE_ENDPOINT;
    return pickEndpoint('beta', await res.json());
  } catch { return STABLE_ENDPOINT; }
}

export async function check(manual = false): Promise<void> {
  if (isHosted() || !updatePreview && !isDesktop()) return;
  if (['checking', 'downloading', 'installing'].includes(updater.status.kind) || updater.status.kind === 'ready' && !manual) return;
  // Unsupported installed formats can still check and link to the download page.
  if (updater.status.kind === 'unsupported') updater.status = { kind: 'idle' };
  send({ type: 'check', manual });
  updater.manualNoneUntil = 0;
  setPrefs({ lastCheck: Date.now() });
  try {
    if (!updatePreview) updater.supported = await invoke<boolean>('update_supported');
    const info = updatePreview ? null : await invoke<UpdateInfo | null>('update_check', { endpoint: await endpoint() });
    const checkedAt = Date.now();
    setPrefs({ lastCheck: checkedAt, lastResult: info ? `Version ${info.version} ${info.ready ? 'is ready to install' : 'is available'}` : "You're up to date" });
    send({ type: 'checked', info, checkedAt });
    if (!updater.supported && !updatePreview) {
      updater.status = { kind: 'unsupported', reason: INSTALL_REASON, info: info ?? undefined };
      return;
    }
    if (!info && manual) updater.manualNoneUntil = checkedAt + 5000;
    if (info && !info.ready && updater.prefs.autoDownload) await download();
  } catch (error) { fail(error); }
}

export async function download(): Promise<void> {
  if (updater.status.kind !== 'available' || isHosted() || !updatePreview && !updater.supported) return;
  send({ type: 'download' });
  let off: (() => void) | undefined;
  try {
    if (updatePreview) {
      send({ type: 'downloaded', info: { ...fake, ready: true } });
      return;
    }
    const { listen } = await import('@tauri-apps/api/event');
    off = await listen<{ downloaded: number; total: number | null }>('update-progress', ({ payload }) => {
      send({ type: 'progress', pct: payload.total ? Math.round(payload.downloaded / payload.total * 100) : null });
    });
    const info = await invoke<UpdateInfo>('update_download');
    send({ type: 'downloaded', info });
    setPrefs({ lastResult: `Version ${info.version} is ready to install` });
  } catch (error) { fail(error); }
  finally { off?.(); }
}

export async function install(): Promise<void> {
  if (updater.status.kind !== 'ready' || isHosted() || !updatePreview && !updater.supported) return;
  send({ type: 'install' });
  if (updatePreview) return;
  try { await invoke('update_install'); } catch (error) { fail(error); }
}

/** Mount once; cleanup also handles unmounting while native support is being queried. */
export function startScheduler(): () => void {
  let stopped = false;
  let first: ReturnType<typeof setTimeout> | undefined;
  let every: ReturnType<typeof setInterval> | undefined;
  if (isHosted()) updater.status = { kind: 'unsupported', reason: HOSTED_REASON };
  else if (!updatePreview && isDesktop()) {
    void (async () => {
      try {
        updater.version = await (await import('@tauri-apps/api/app')).getVersion();
        updater.supported = await invoke<boolean>('update_supported');
        if (stopped) return;
        if (!updater.supported) {
          updater.status = { kind: 'unsupported', reason: INSTALL_REASON };
          return;
        }
        const pending = await invoke<UpdateInfo | null>('update_status');
        if (stopped) return;
        if (pending) updater.status = { kind: pending.ready ? 'ready' : 'available', info: pending };
        first = setTimeout(() => void check(), 8000);
        every = setInterval(() => void check(), 6 * 3600_000);
      } catch (error) {
        if (!stopped) { updater.status = { kind: 'checking' }; fail(error); }
      }
    })();
  }
  return () => { stopped = true; clearTimeout(first); clearInterval(every); };
}
