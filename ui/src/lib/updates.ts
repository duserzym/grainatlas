export const STABLE_ENDPOINT = 'https://github.com/duserzym/grainatlas/releases/latest/download/latest.json';
export const RELEASES_PAGE = 'https://github.com/duserzym/grainatlas/releases/latest';
export const UPDATE_APP = 'grainatlas';
export const UPDATE_HOST = new URL(STABLE_ENDPOINT).host;
export const UPDATE_UNSUPPORTED_REASON = `${UPDATE_APP} cannot replace itself in this folder. Download the new version from GitHub.`;
export interface UpdateInfo {
  version: string;
  currentVersion: string;
  notes: string | null;
  date: string | null;
  ready: boolean;
}
export type UpdateStatus =
  | { kind: 'idle' }
  | { kind: 'checking' }
  | { kind: 'none'; checkedAt: number }
  | { kind: 'available'; info: UpdateInfo }
  | { kind: 'downloading'; info: UpdateInfo; pct: number | null }
  | { kind: 'ready'; info: UpdateInfo }
  | { kind: 'installing'; info: UpdateInfo }
  | { kind: 'error'; message: string }
  | { kind: 'unsupported'; reason: string; info?: UpdateInfo };

export type UpdateErrorKind = 'not_configured' | 'certificate' | 'signature' | 'no_release' | 'offline' | 'not_writable' | 'other';

/** Order matters: secure-connection failures often also mention a request or network. */
export function updateErrorKind(raw: string): UpdateErrorKind {
  if (!raw.trim() || /pubkey|public key|not configured/i.test(raw)) return 'not_configured';
  if (/certificate|UnknownIssuer/i.test(raw)) return 'certificate';
  if (/signature|minisign|\bverify\b|\bverified\b/i.test(raw)) return 'signature';
  if (/\b404\b|release JSON|could not fetch a valid release|not found|no assets/i.test(raw)) return 'no_release';
  if (/error sending request|dns|connect|timed out|timeout|offline|network|failed to fetch|ENOTFOUND|URLError/i.test(raw)) return 'offline';
  if (/not writ(?:able|eable)|permission denied|access (?:is )?denied|read.only (?:file system|filesystem|folder)|EACCES|EPERM/i.test(raw)) return 'not_writable';
  return 'other';
}

export function updateErrorDetails(raw: string): string {
  const message = raw.trim();
  return message.length > 120 ? `${message.slice(0, 119)}…` : message;
}

/** Keep native updater diagnostics in Details, rather than the status line. */
export function friendlyUpdateError(raw: string): string {
  switch (updateErrorKind(raw)) {
    case 'not_configured': return 'Updates are not configured for this build yet.';
    case 'certificate': return `Could not verify the secure connection to ${UPDATE_HOST}.`;
    case 'signature': return "The update's signature did not match. Nothing was installed.";
    case 'no_release': return `No public release of ${UPDATE_APP} has been published.`;
    case 'offline': return `No internet connection, or ${UPDATE_HOST} did not answer.`;
    case 'not_writable': return UPDATE_UNSUPPORTED_REASON;
    case 'other': return `The update check failed: ${updateErrorDetails(raw)}`;
  }
}

/** Presentation only; checks and installation stay in the shared updater. */
export function checkButtonView(status: UpdateStatus, version: string, autoDownload = true) {
  const badge = status.kind === 'available' || status.kind === 'ready' || status.kind === 'unsupported' && !!status.info;
  const busy = ['checking', 'downloading', 'installing'].includes(status.kind);
  let text: string;
  let secondary: string | null = null;
  let action: 'download' | 'install' | 'releases' | null = null;
  switch (status.kind) {
    case 'idle': text = `Current version: v${version}`; break;
    case 'checking': text = 'Checking for updates…'; break;
    case 'none': text = `Up to date (v${version})`; break;
    case 'available':
      text = `Update available: v${status.info.version}`;
      secondary = `Current version: v${version}`;
      if (!autoDownload) action = 'download';
      break;
    case 'downloading': text = `Downloading v${status.info.version}…${status.pct === null ? '' : ` ${status.pct}%`}`; break;
    case 'ready': text = `v${status.info.version} is ready`; action = 'install'; break;
    case 'installing': text = `Installing v${status.info.version} and restarting…`; break;
    case 'unsupported':
      text = status.reason;
      if (status.info) action = 'releases';
      break;
    case 'error': text = friendlyUpdateError(status.message); break;
  }
  const details = status.kind === 'error' ? updateErrorDetails(status.message) : null;
  return { text, secondary, details, action, badge, busy };
}

/** Semver precedence; build metadata does not affect ordering. */
export function compareVersions(a: string, b: string): number {
  const parse = (v: string) => {
    const [core, ...pre] = v.replace(/^v/, '').split('+')[0].split('-');
    return { core: core.split('.').map(BigInt), pre: pre.join('-').split('.').filter(Boolean) };
  };
  const x = parse(a), y = parse(b);
  for (let i = 0; i < 3; i++) {
    const n = x.core[i] ?? 0n, m = y.core[i] ?? 0n;
    if (n !== m) return n > m ? 1 : -1;
  }
  if (!x.pre.length || !y.pre.length) return Number(!x.pre.length) - Number(!y.pre.length);
  for (let i = 0; i < Math.max(x.pre.length, y.pre.length); i++) {
    const n = x.pre[i], m = y.pre[i];
    if (n === undefined) return -1;
    if (m === undefined) return 1;
    if (n === m) continue;
    const nn = /^\d+$/.test(n), mn = /^\d+$/.test(m);
    if (nn && mn) return BigInt(n) > BigInt(m) ? 1 : -1;
    if (nn !== mn) return nn ? -1 : 1;
    return n > m ? 1 : -1;
  }
  return 0;
}

export type UpdateEvent =
  | { type: 'check'; manual?: boolean }
  | { type: 'checked'; info: UpdateInfo | null; checkedAt: number }
  | { type: 'download' }
  | { type: 'progress'; pct: number | null }
  | { type: 'downloaded'; info: UpdateInfo }
  | { type: 'install' }
  | { type: 'failed'; message: string };

export function transition(status: UpdateStatus, event: UpdateEvent): UpdateStatus {
  switch (event.type) {
    case 'check':
      return ['checking', 'downloading', 'installing', 'unsupported'].includes(status.kind)
        || status.kind === 'ready' && !event.manual ? status : { kind: 'checking' };
    case 'checked':
      return status.kind !== 'checking' ? status : event.info
        ? { kind: event.info.ready ? 'ready' : 'available', info: event.info }
        : { kind: 'none', checkedAt: event.checkedAt };
    case 'download':
      return status.kind === 'available' ? { kind: 'downloading', info: status.info, pct: 0 } : status;
    case 'progress':
      return status.kind === 'downloading' ? { ...status, pct: event.pct === null ? null : Math.max(0, Math.min(100, event.pct)) } : status;
    case 'downloaded':
      return status.kind === 'downloading' ? { kind: 'ready', info: event.info } : status;
    case 'install':
      return status.kind === 'ready' ? { kind: 'installing', info: status.info } : status;
    case 'failed':
      return ['checking', 'downloading', 'installing'].includes(status.kind) ? { kind: 'error', message: event.message } : status;
  }
}

export interface UpdatePrefs {
  autoDownload: boolean;
  lastCheck: number | null;
  lastResult: string;
  lastSeenVersion: string | null;
  pendingNotes: { version: string; notes: string } | null;
}
interface PrefStorage { getItem(key: string): string | null; setItem(key: string, value: string): void }
const PREF_KEY = 'grainatlas.updates';
export function loadPrefs(storage?: PrefStorage): UpdatePrefs {
  const defaults: UpdatePrefs = { autoDownload: true, lastCheck: null, lastResult: '', lastSeenVersion: null, pendingNotes: null };
  try {
    const p = JSON.parse(storage?.getItem(PREF_KEY) ?? 'null');
    if (!p || typeof p !== 'object') return defaults;
    const prefs: UpdatePrefs = {
      autoDownload: typeof p.autoDownload === 'boolean' ? p.autoDownload : true,
      lastCheck: typeof p.lastCheck === 'number' && Number.isFinite(p.lastCheck) ? p.lastCheck : null,
      lastResult: typeof p.lastResult === 'string' ? p.lastResult : '',
      lastSeenVersion: validVersion(p.lastSeenVersion) ? p.lastSeenVersion : null,
      pendingNotes: validVersion(p.pendingNotes?.version) && typeof p.pendingNotes?.notes === 'string'
        ? { version: p.pendingNotes.version, notes: p.pendingNotes.notes } : null,
    };
    if ('channel' in p) savePrefs(prefs, storage);
    return prefs;
  } catch { return defaults; }
}
export function savePrefs(prefs: UpdatePrefs, storage?: PrefStorage): void {
  try { storage?.setItem(PREF_KEY, JSON.stringify(prefs)); } catch { /* Storage can be unavailable. */ }
}

function validVersion(value: unknown): value is string {
  return typeof value === 'string' && /^v?\d+\.\d+\.\d+(?:-[0-9A-Za-z.-]+)?(?:\+[0-9A-Za-z.-]+)?$/.test(value);
}

export function releaseNotesUrl(version: string): string {
  return `https://github.com/duserzym/grainatlas/releases/tag/v${encodeURIComponent(version.replace(/^v/, ''))}`;
}

export function currentNotes(version: string, bundled: string | undefined, prefs: UpdatePrefs): string {
  return bundled?.trim() || (prefs.pendingNotes?.version === version ? prefs.pendingNotes.notes.trim() : '') || `Updated to v${version}.`;
}

/** Record every launch, including first installs and manual reinstalls. */
export function recordVersion(version: string, bundled: string | undefined, prefs: UpdatePrefs) {
  const show = prefs.lastSeenVersion !== null && compareVersions(version, prefs.lastSeenVersion) > 0;
  const notes = currentNotes(version, bundled, prefs);
  return { show, notes, prefs: { ...prefs, lastSeenVersion: version } };
}
