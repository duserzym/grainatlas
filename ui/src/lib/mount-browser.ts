import type { MountOut } from './upbdb.ts';
import { sameId, type RecentEntry } from './deeplink.ts';
import type { Draft, DraftStorage } from './drafts.ts';
import { draftSyncStatus } from './sync-status.ts';
import type { MirrorStorage } from './mount-mirror.ts';
import { serverFetch, oversightSession } from './oversight.ts';

export const SERVER_SESSION_CHANGED = 'grainatlas:server-session-changed';
let sessionEpoch = 0;
export const mountSession = () => sessionEpoch;
export const PAGE_SIZE = 50;
export const CACHE_TILES = 500;
export const CACHE_BYTES = 20 * 1024 * 1024;
export interface BrowserLab { id: string; handle: string; display_name?: string; my_role?: 'owner' | 'member' | null }
export interface BrowserAccount { id: string; username: string; display_name: string; can_manage_people: boolean; mirrorId: string; labsNext?: string | null; labs: BrowserLab[] }
export interface MountPage { rows: MountOut[]; next: string | null; total: number; etag: string | null; at: string }
export interface MountCache { pages: Record<string, MountPage>; account?: BrowserAccount; meEtag?: string | null; labsEtag?: string | null; serverEtag?: string | null; revisions?: RevisionCache }
/** Matches UPb Lab: URL parser normalization, no query/fragment or trailing slash. */
export function mirrorServer(server: string) {
  const url = new URL(server.trim()); url.search = ''; url.hash = '';
  return url.href.replace(/\/+$/, '');
}
async function hash(value: string) {
  const bytes = await crypto.subtle.digest('SHA-256', new TextEncoder().encode(value));
  return Array.from(new Uint8Array(bytes), n => n.toString(16).padStart(2, '0')).join('');
}
export async function serverMirrorId(server: string) { return hash(mirrorServer(server)); }
export function cacheKey(mirrorId: string, account: string) { return `mirror/${mirrorId}/${account}/start-screen.json`; }
export async function readCache(storage: MirrorStorage, key: string): Promise<MountCache> {
  try { const v = JSON.parse(await storage.getItem(key) ?? 'null'); return v && v.pages && typeof v.pages === 'object' && !Array.isArray(v.pages) ? v : { pages: {} }; } catch { return { pages: {} }; }
}
export async function writeCache(storage: MirrorStorage, key: string, cache: MountCache) {
  // Newest pages first. This budget applies only to start-screen metadata and thumbnails.
  const pages: MountCache['pages'] = {};
  let count = 0;
  for (const [path, page] of Object.entries(cache.pages).sort((a, b) => b[1].at.localeCompare(a[1].at))) {
    const rows = page.rows.slice(0, CACHE_TILES - count).map(row => ({ ...row }));
    if (!rows.length && page.rows.length) continue;
    pages[path] = { ...page, rows };
    count += rows.length;
  }
  const bounded = { ...cache, pages };
  const bytes = () => new TextEncoder().encode(JSON.stringify(bounded)).length;
  if (bytes() > CACHE_BYTES) for (const page of Object.values(pages)) for (const row of page.rows) delete row.thumbnail_data;
  while (bytes() > CACHE_BYTES && Object.keys(pages).length) delete pages[Object.keys(pages).at(-1)!];
  try { await storage.setItem(key, JSON.stringify(bounded)); } catch { /* the in-memory list still works */ }
}
/** Hide server state and invalidate in-flight work; mirrors and drafts stay on disk.
 * A refusal hides the UI without retrying identity; sign-out also resets identity. */
export function hideMountCache(resetIdentity = true) {
  sessionEpoch++;
  revisionReads.clear();
  if (resetIdentity) sessionReads.clear();
  oversightSession.clear();
  if (typeof window !== 'undefined') window.dispatchEvent(new Event(SERVER_SESSION_CHANGED));
}
export function accountLabs(account: Pick<BrowserAccount, 'can_manage_people'>, labs: BrowserLab[]) {
  return labs.filter(lab => !account.can_manage_people || lab.my_role != null);
}
export function defaultScope(account: BrowserAccount): Record<string, string> {
  return { sort: '-last_changed', ...(account.can_manage_people ? account.labs[0] ? { lab: account.labs[0].handle } : { mine: 'true' } : {}) };
}
type LocalMatch = { recent?: RecentEntry; draft?: Draft } | null;
const idKey = (id: string | null | undefined) => id?.toLowerCase() ?? '';
/** Index once for a whole page; matching never scans the entire recent list per tile. */
function matchIndex(recent: RecentEntry[], drafts: Draft[], server: string) {
  const entries = new Map<string, RecentEntry>();
  for (const r of recent) if (r.id && !entries.has(idKey(r.id))) entries.set(idKey(r.id), r);
  const linked = new Map<string, Draft[]>();
  for (const d of drafts) if (server && d.server?.replace(/\/+$/, '') === server.replace(/\/+$/, '') && d.mount) {
    const key = idKey(d.mount), group = linked.get(key) ?? [];
    group.push(d); linked.set(key, group);
  }
  return (mount: MountOut): LocalMatch => {
    const group = linked.get(idKey(mount.id)) ?? [];
    const published = mount.atlas?.project_id;
    const draft = group.find(d => sameId(d.linked_uuid ?? d.uuid, published));
    const entry = entries.get(idKey(published)) ?? entries.get(idKey(draft?.uuid));
    if (entry || draft?.source_keys?.length) return { recent: entry, draft };
    for (const d of group) {
      const r = entries.get(idKey(d.uuid));
      if (r || d.source_keys?.length) return { recent: r, draft: d };
    }
    return null;
  };
}
export function localMatch(mount: MountOut, recent: RecentEntry[], drafts: Draft[], server: string): LocalMatch {
  return matchIndex(recent, drafts, server)(mount);
}
export function separateLocal(mount: MountOut, match: LocalMatch) {
  return !!match && !!mount.atlas?.project_id && !sameId(match.draft?.linked_uuid ?? match.recent?.id ?? match.draft?.uuid, mount.atlas.project_id);
}
export function startEntries(rows: MountOut[], recent: RecentEntry[], drafts: Draft[], server: string | null) {
  const match = matchIndex(recent, drafts, server ?? '');
  const hidden = new Set<RecentEntry>();
  const tiles = rows.map(mount => {
    const local = server ? match(mount) : null;
    if (local?.recent) hidden.add(local.recent);
    return { mount, match: local };
  });
  return { tiles, localOnly: recent.filter(r => !hidden.has(r)) };
}
export function relativeTime(at: string, now = Date.now()) {
  const time = Date.parse(at), age = Math.max(0, now - time);
  if (!Number.isFinite(time)) return 'unknown time';
  if (age < 60_000) return 'just now';
  if (age < 3_600_000) return `${Math.floor(age / 60_000)} min ago`;
  if (age < 86_400_000) return `${Math.floor(age / 3_600_000)} h ago`;
  if (age < 7 * 86_400_000) return `${Math.floor(age / 86_400_000)} d ago`;
  return new Date(time).toLocaleDateString();
}
export function entryStatus(mount: MountOut, match: LocalMatch, revision?: number, { offline = false, hasOffline = false, now = Date.now(), storage }: { offline?: boolean; hasOffline?: boolean; now?: number; storage?: DraftStorage } = {}): string[] {
  const parts: string[] = [];
  const d = match?.draft;
  if (match && !separateLocal(mount, match)) {
    const at = d?.synced_at ?? d?.checked_at;
    parts.push(offline ? 'Local copy on this computer' : `Database · local copy ${at ? `synced ${relativeTime(at, now)}` : 'not yet synced'}`);
    if (d && (d.incomplete || draftSyncStatus(d, storage).pending)) {
      const n = d.changes && d.changes.spots + d.changes.grains + d.changes.labels;
      parts.push(n ? `${n} ${n === 1 ? 'change' : 'changes'} not yet uploaded` : 'Local changes');
    }
    if (!offline && d?.base_revision != null && revision != null && revision > d.base_revision) parts.push('Newer in database');
  } else parts.push(offline ? hasOffline ? 'Offline copy available (read-only)' : 'Not on this computer' : 'Database only');
  return parts;
}
/** "Local changes" comes from the same status as the Open list and the sync chip: a stale
 * `pixels_changed` flag in an otherwise synced draft is not local work. */
export function tileBadges(mount: MountOut, match: ReturnType<typeof localMatch>, account: string, revision?: number, storage?: DraftStorage): string[] {
  const d = match?.draft;
  const badges = [match ? 'On this computer' : 'Not on this computer'];
  if (d && (d.incomplete || draftSyncStatus(d, storage).pending)) badges.push('Local changes');
  if (d?.base_revision != null && revision != null && revision > d.base_revision) badges.push('Newer on server');
  if (mount.creator?.id !== account && mount.role) badges.push(`Shared · ${mount.role === 'owner' ? 'Admin' : mount.role[0].toUpperCase() + mount.role.slice(1)}`);
  return badges;
}
export const DEFAULT_START_LAYOUT = { recentWidth: 240, tileSize: 'M' as 'S' | 'M' | 'L', startView: 'database' as 'database' | 'local' };
export function startLayout(value: { recentWidth?: number; tileSize?: string; startView?: string }) {
  return { startView: value.startView === 'local' ? 'local' as const : DEFAULT_START_LAYOUT.startView, recentWidth: Math.max(180, Math.min(360, Number(value.recentWidth) || DEFAULT_START_LAYOUT.recentWidth)), tileSize: ['S', 'M', 'L'].includes(value.tileSize ?? '') ? value.tileSize as 'S' | 'M' | 'L' : DEFAULT_START_LAYOUT.tileSize };
}

/** Only the connect and idle intervals are bounded; a progressing transfer has no total deadline. */
export async function readResponse(url: string, init: RequestInit = {}): Promise<Response> {
  const controller = new AbortController();
  let timer = setTimeout(() => controller.abort(), 8000);
  const reset = () => { clearTimeout(timer); timer = setTimeout(() => controller.abort(), 15000); };
  const started = performance.now();
  try {
    const r = await serverFetch(url, { ...init, signal: controller.signal });
    reset();
    if (!r.body || r.status === 304) return r;
    const reader = r.body.getReader();
    const chunks: Uint8Array[] = [];
    while (true) { const part = await reader.read(); if (part.done) break; chunks.push(part.value); reset(); }
    return new Response(new Blob(chunks as BlobPart[]), { status: r.status, statusText: r.statusText, headers: r.headers });
  } finally { clearTimeout(timer); console.debug('Mount browser request', new URL(url).pathname, Math.round(performance.now() - started), 'ms'); }
}
export async function checkedJson<T>(response: Response): Promise<T> {
  if (!response.ok) throw new Error(`Database ${response.status}: ${(await response.text()).slice(0, 180)}`);
  return response.json();
}
async function identityKey(server: string, token: string) {
  return `mirror-bindings/${await serverMirrorId(server)}/${await hash(token)}.json`;
}
/** Resolve only this credential's established mirror binding, without a network read. */
export async function boundMirror(server: string, token: string, storage: MirrorStorage): Promise<BrowserAccount | null> {
  if (!server || !token) return null;
  try {
    const binding = JSON.parse(await storage.getItem(await identityKey(server, token)) ?? 'null');
    if (!binding) return null;
    const cache = await readCache(storage, cacheKey(binding.mirrorId, binding.accountId));
    return cache.account && cache.account.id === binding.accountId ? cache.account : null;
  } catch { return null; }
}
// Retain fulfilled and rejected reads until sign-out/credential change. Store the
// promise before any await so concurrent consumers share the same network work.
// Responses are cloned for each caller; consuming a body never consumes the cache.
const sessionReads = new Map<string, { token: string; result: Promise<[Response, Response | undefined]> }>();
function sessionIdentity(server: string, token: string, saved: MountCache) {
  const previous = sessionReads.get(server);
  if (previous?.token === token) return previous.result;
  const auth = { Authorization: `Bearer ${token}` };
  const result = Promise.all([
    readResponse(`${server}/v1/accounts/me`, { headers: { ...auth, ...(saved.meEtag ? { 'If-None-Match': saved.meEtag } : {}) } }),
    // Older servers need not provide /server. /me still checks authentication.
    readResponse(`${server}/v1/server`, { headers: { ...auth, ...(saved.serverEtag ? { 'If-None-Match': saved.serverEtag } : {}) } }).catch(() => undefined),
  ]);
  sessionReads.set(server, { token, result });
  return result;
}
/** A binding is looked up by this server and credential only; it contains no token.
 * A fresh credential must resolve /me before it can read any account's folder. */
export async function browserAccount(server: string, token: string, storage: MirrorStorage, showCached?: (account: BrowserAccount, cache: MountCache) => void): Promise<BrowserAccount> {
  const epoch = sessionEpoch;
  const current = () => { if (epoch !== sessionEpoch) throw new Error('The signed-in account changed.'); };
  if (!token) throw new Error('Sign in to see your mounts');
  if (!server.trim()) throw new Error('Set the UPbDB server first.');
  server = mirrorServer(server);
  const key = await identityKey(server, token);
  const fallback = await serverMirrorId(server);
  let binding: { accountId: string; mirrorId: string } | null = null;
  try { binding = JSON.parse(await storage.getItem(key) ?? 'null'); } catch { /* unavailable storage */ }
  let saved = binding ? await readCache(storage, cacheKey(binding.mirrorId, binding.accountId)) : { pages: {} } as MountCache;
  if (saved.account?.id !== binding?.accountId) saved = { pages: {} };
  current();
  if (saved.account) showCached?.(saved.account, saved);
  try {
    const auth = { Authorization: `Bearer ${token}` };
    const [meResponse, datasetResponse] = await sessionIdentity(server, token, saved);
    const response = meResponse.clone();
    const serverResponse = datasetResponse?.clone();
    const serverInfo = serverResponse?.status === 304
      ? { dataset: saved.account?.mirrorId, etag: serverResponse.headers.get('ETag') ?? saved.serverEtag }
      : serverResponse?.ok
        ? { dataset: (await serverResponse.json().catch(() => null))?.dataset_id as unknown, etag: serverResponse.headers.get('ETag') }
        : undefined;
    current();
    if (response.status === 401 || response.status === 403) { hideMountCache(false); throw new Error(`Database ${response.status}: Sign in again.`); }
    const me = response.status === 304 && saved.account ? saved.account : await checkedJson<BrowserAccount>(response);
    const mirrorId = (typeof serverInfo?.dataset === 'string' && serverInfo.dataset) || binding?.mirrorId || fallback;
    // Only the same account's URL-keyed mirror can migrate when dataset_id first appears.
    if (binding?.accountId !== me.id || (mirrorId !== binding.mirrorId && binding.mirrorId !== fallback)) saved = { pages: {} };
    const destination = await readCache(storage, cacheKey(mirrorId, me.id));
    current();
    if (destination.account?.id === me.id) saved = destination;
    else if (!saved.account && mirrorId !== fallback && (!binding || binding.mirrorId === fallback)) {
      // A new credential may be the first to discover dataset_id for this account.
      const urlCopy = await readCache(storage, cacheKey(fallback, me.id));
      if (urlCopy.account?.id === me.id) saved = urlCopy;
    }
    current();
    const cachedAccount = saved.account;
    if (cachedAccount) showCached?.({ ...cachedAccount, mirrorId }, saved);
    const labsResponse = await readResponse(`${server}/v1/labs?limit=100`, { headers: { ...auth, ...(saved.labsEtag ? { 'If-None-Match': saved.labsEtag } : {}) } });
    if (labsResponse.status === 401 || labsResponse.status === 403) { hideMountCache(false); throw new Error(`Database ${labsResponse.status}: Sign in again.`); }
    const labs = labsResponse.status === 304 && cachedAccount ? cachedAccount.labs : accountLabs(me, await checkedJson<BrowserLab[]>(labsResponse));
    const account: BrowserAccount = { id: me.id, username: me.username, display_name: me.display_name, can_manage_people: me.can_manage_people, mirrorId, labsNext: labsResponse.status === 304 ? cachedAccount?.labsNext : labsResponse.headers.get('X-Next-Cursor'), labs };
    current();
    await writeCache(storage, cacheKey(mirrorId, me.id), { ...saved, account, meEtag: response.headers.get('ETag') ?? saved.meEtag, labsEtag: labsResponse.headers.get('ETag') ?? saved.labsEtag, serverEtag: serverInfo?.etag ?? saved.serverEtag });
    current();
    try { await storage.setItem(key, JSON.stringify({ accountId: me.id, mirrorId })); } catch { /* unavailable quota */ }
    current();
    return account;
  } catch (e) {
    // A refusal never revives the mirror. Network failures can use this credential's binding.
    if (!(e instanceof Error && /^Database \d/.test(e.message)) && saved.account && epoch === sessionEpoch) return saved.account;
    throw e;
  }
}
const observedTokens = new Map<string, string | null>();
export function observeServerToken(server: string | null, token: string | null) {
  if (!server) return;
  const key = mirrorServer(server), previous = observedTokens.get(key);
  if (previous !== undefined && previous !== token) {
    sessionReads.delete(key);
    hideMountCache(false);
  }
  observedTokens.set(key, token);
}
export function mountPath(filters: Record<string, string>, after?: string | null) {
  const params = new URLSearchParams({ limit: String(PAGE_SIZE), ...Object.fromEntries(Object.entries(filters).filter(([, v]) => !!v)) });
  if (after) params.set('after', after);
  return `/v1/mounts?${params}`;
}
export async function mountPage(server: string, token: string, path: string, cached?: MountPage): Promise<MountPage> {
  const response = await readResponse(server + path, { headers: { Authorization: `Bearer ${token}`, ...(cached?.etag ? { 'If-None-Match': cached.etag } : {}) } });
  if (response.status === 304 && cached) return { ...cached, at: new Date().toISOString() };
  const rows = await checkedJson<MountOut[]>(response);
  return { rows, next: response.headers.get('X-Next-Cursor'), total: Number(response.headers.get('X-Total-Count') ?? rows.length), etag: response.headers.get('ETag'), at: new Date().toISOString() };
}
export interface RevisionRow { project_uuid: string; mount_id: string; revision: number }
export async function batchRevisions(server: string, token: string, projects: string[], cached?: { rows: RevisionRow[]; etag: string | null }) {
  const ids = [...new Set(projects)];
  if (ids.length > 500) throw new Error('Revision batches accept at most 500 project UUIDs.');
  if (!ids.length) return { rows: [], etag: null };
  const response = await readResponse(`${server}/v1/atlas/projects/revisions`, { method: 'POST', headers: { Authorization: `Bearer ${token}`, 'Content-Type': 'application/json', ...(cached?.etag ? { 'If-None-Match': cached.etag } : {}) }, body: JSON.stringify({ projects: ids }) });
  return response.status === 304 && cached ? cached : { rows: await checkedJson<RevisionRow[]>(response), etag: response.headers.get('ETag') };
}

interface RevisionBatch { key: string; rows: RevisionRow[]; etag: string | null; at: number }
export interface RevisionCache { batches: RevisionBatch[] }
// In-flight and fulfilled batches survive component remounts. The mirror also
// retains the last lookup across document reloads, without storing credentials.
interface RevisionRead { value?: RevisionBatch; pending?: Promise<RevisionBatch>; viewers: Set<() => boolean> }
const revisionReads = new Map<string, RevisionRead>();
export function recentRevisionIds(recent: RecentEntry[], drafts: Draft[], server: string): string[] {
  const ids = new Set(recent.flatMap(r => r.id ? [r.id.toLowerCase()] : []));
  return [...new Set(drafts.filter(d => ids.has(d.uuid.toLowerCase()) && d.mount && d.server && mirrorServer(d.server) === mirrorServer(server))
    .map(d => (d.linked_uuid ?? d.uuid).toLowerCase()))].sort();
}
/** Called in the background by the visible start screen only. Each response has
 * its own ETag and freshness time; no UUIDs are silently dropped at the bound. */
export async function recentRevisions(server: string, token: string, account: string, projects: string[], cached: RevisionCache | undefined, visible: () => boolean): Promise<RevisionCache> {
  const epoch = sessionEpoch;
  const current = () => visible() && epoch === sessionEpoch;
  const batches: RevisionBatch[] = [];
  if (!server || !token || !account || !projects.length || !current()) return { batches };
  server = mirrorServer(server);
  const ids = [...new Set(projects)].sort();
  const scope = [server, account, oversightSession.state?.viewAs?.username ?? ''];
  for (let from = 0; from < ids.length; from += 500) {
    if (!current()) throw new Error('The start-screen session changed.');
    const projects = ids.slice(from, from + 500);
    const key = JSON.stringify([...scope, projects]);
    const previous = cached?.batches.find(b => b.key === key);
    let entry = revisionReads.get(key);
    if (!entry) {
      entry = { value: previous, viewers: new Set() };
      revisionReads.set(key, entry);
    }
    const state = entry;
    state.viewers.add(current);
    try {
      const saved = state.value;
      if (saved && Date.now() >= saved.at && Date.now() - saved.at < 60_000) {
        batches.push(saved);
        continue;
      }
      if (!state.pending) {
        state.pending = Promise.resolve().then(async () => {
          if (![...state.viewers].some(viewer => viewer())) throw new Error('The start-screen session changed.');
          const result = await batchRevisions(server, token, projects, saved);
          if (epoch !== sessionEpoch) throw new Error('The signed-in account changed.');
          return state.value = { ...result, key, at: Date.now() };
        }).finally(() => { state.pending = undefined; });
      }
      const result = await state.pending;
      if (!current()) throw new Error('The start-screen session changed.');
      batches.push(result);
    } finally { state.viewers.delete(current); }
  }
  // Bound retained metadata for changing scopes/filters. Pending work is shared.
  for (const [key, entry] of revisionReads) {
    if (revisionReads.size <= 64) break;
    if (!entry.pending && !entry.viewers.size) revisionReads.delete(key);
  }
  return { batches };
}
