import { untrack } from 'svelte';
import { isHosted } from '@lab/bridge';
import { authToken } from './atlas';
import { browserAccount, cacheKey, readCache, serverMirrorId, SERVER_SESSION_CHANGED, type BrowserLab } from './mount-browser';
import { mountMirror } from './mount-mirror';
import { oversightSession } from './oversight';
import { canWrite } from './presentation';
import type { AppState } from './store.svelte';
import { MINERALS, TimsApi, type Mineral } from './tims';
import { TimsOutbox } from './tims-outbox';

interface SharedStore { store: TimsOutbox; loading: Promise<void>; subscribers: Map<() => void, () => boolean> }
const stores = new Map<string, SharedStore>();
/** Tab-owned session. Sign-out detaches the view immediately, retaining its keyed mirror
 * and unsent writes. Tabs on the same mount share one writer and one badge projection. */
export class TimsSession {
  store = $state.raw<TimsOutbox | null>(null);
  version = $state(0);
  error = $state('');
  minerals = $state.raw<Mineral[]>([]);
  labs = $state.raw<BrowserLab[]>([]);
  private epoch = $state(0);
  private generation = 0;
  private detach?: () => void;
  private hidden = false;
  private badgeMap = $derived.by(() => { this.version; return this.store?.badges ?? new Map(); });
  get badges(): TimsOutbox['badges'] { return this.badgeMap; }
  private app: AppState;
  constructor(app: AppState) { this.app = app; }
  start() {
    const hide = () => {
      this.hidden = true; this.generation++; this.detach?.(); this.detach = undefined; this.store = null; this.error = ''; this.labs = [];
      this.epoch++;
    };
    const wake = () => { if (isHosted() || this.app.hosted || this.app.link.syncAutomatically) { this.store?.wake(); void this.store?.refresh(); } };
    if (typeof window !== 'undefined') {
      window.addEventListener(SERVER_SESSION_CHANGED, hide);
      window.addEventListener('online', wake); window.addEventListener('focus', wake);
    }
    $effect(() => {
      this.epoch;
      const app = this.app;
      const server = app.remote?.server ?? app.server, mount = app.project?.upbdb.mount_id ?? null;
      const token = app.link.token, project = app.project?.uuid, access = app.remote?.access ?? null;
      const viewAs = oversightSession.state?.viewAs?.username;
      const readOnly = app.readOnly || !canWrite() || oversightSession.readOnly;
      // The shell notifies SERVER_SESSION_CHANGED when credentials change. Standalone
      // credentials and project switches are reactive dependencies here.
      void token;
      const generation = ++this.generation;
      untrack(() => { this.detach?.(); this.detach = undefined; this.store = null; this.error = ''; this.labs = []; });
      if (!server || !mount || !project) return;
      void this.connect(server, mount, project, access, viewAs, readOnly, generation);
      return () => { if (generation === this.generation) { this.generation++; this.detach?.(); this.detach = undefined; this.store = null; } };
    });
    $effect(() => {
      const automatic = isHosted() || this.app.hosted || this.app.link.syncAutomatically;
      const store = this.store;
      untrack(() => store?.setAutomatic(automatic));
    });
    let lastSync: string | null = null, lastRevision: number | undefined, lastChange: string | null | undefined;
    $effect(() => {
      const synced = this.app.sync.syncedAt ?? this.app.sync.draft?.synced_at ?? null;
      const revision = this.app.sync.status?.revision;
      const serverChange = this.app.sync.status?.changed_at;
      const store = this.store;
      if (store && synced && synced !== lastSync) untrack(() => store.mountSynced());
      if (store && (isHosted() || this.app.hosted || this.app.link.syncAutomatically) && (revision !== lastRevision || serverChange !== lastChange)) untrack(() => { void store.refresh(); });
      lastSync = synced; lastRevision = revision; lastChange = serverChange;
    });
    return () => {
      this.generation++; this.detach?.(); this.store = null;
      if (typeof window !== 'undefined') {
        window.removeEventListener(SERVER_SESSION_CHANGED, hide); window.removeEventListener('online', wake); window.removeEventListener('focus', wake);
      }
    };
  }
  private async connect(server: string, mount: string, project: string, access: string | null, viewAs: string | undefined, readOnly: boolean, generation: number) {
    const current = () => generation === this.generation;
    try {
      const token = await authToken(this.app, server);
      if (!current()) return;
      if (!token && !access) { this.error = 'Sign in to pluck grains for TIMS'; return; }
      const storage = readOnly ? { getItem: mountMirror.getItem.bind(mountMirror), setItem: () => {} } : mountMirror;
      // A signed viewer access token can read fractions without resolving an account.
      // Its isolated memory-only view never opens another account's mirror.
      const account = token ? await browserAccount(server, token, storage) : { mirrorId: await serverMirrorId(server), id: `access-${await serverMirrorId(`https://access.invalid/${encodeURIComponent(access!)}`)}` };
      if (current()) this.labs = 'labs' in account ? account.labs : [];
      if (token && current()) {
        const cache = await readCache(storage, cacheKey(account.mirrorId, account.id));
        const mountRow = Object.values(cache.pages).flatMap(p => p.rows).find(m => m.id === mount);
        this.minerals = (mountRow?.mineral ?? []).filter((m): m is Mineral => MINERALS.includes(m as Mineral));
      }
      if (!current()) return;
      const serverKey = await serverMirrorId(server);
      const viewKey = viewAs ? `-as-${await serverMirrorId(`https://view.invalid/${encodeURIComponent(viewAs)}`)}` : '';
      if (!current()) return;
      const key = `mirror/${account.mirrorId}/${account.id}/tims/${serverKey}/${mount}${viewKey}.json`;
      const poolKey = `${key}:${readOnly ? 'read' : 'write'}`;
      let shared = stores.get(poolKey);
      if (!shared) {
        const subscribers: SharedStore['subscribers'] = new Map();
        const store = new TimsOutbox(key, mount, storage, new TimsApi(server, token, access, project), () => { for (const notify of subscribers.keys()) notify(); }, () => !readOnly && [...subscribers.values()].some(writable => writable()));
        shared = { store, subscribers, loading: token && !viewAs ? store.load() : Promise.resolve() };
        stores.set(poolKey, shared);
      }
      this.hidden = false;
      const notify = () => { if (current() && !this.hidden) this.version++; };
      shared.subscribers.set(notify, () => current() && !this.app.readOnly && canWrite() && !oversightSession.readOnly);
      const selected = shared;
      this.detach = () => {
        selected.subscribers.delete(notify);
        // Keep a single in-memory owner for a durable outbox, including a write whose
        // reply is still in flight. Reopening cannot race a second copy of that outbox.
        if (!selected.subscribers.size) { selected.store.dispose(); selected.store.api = new TimsApi(server, null, null, project); }
      };
      await selected.loading;
      if (!current()) return;
      selected.store.api = new TimsApi(server, token, access, project);
      selected.store.enabled = true;
      selected.store.setAutomatic(isHosted() || this.app.hosted || this.app.link.syncAutomatically);
      this.store = selected.store; this.version++;
      void selected.store.refresh(); selected.store.wake();
    } catch (e) { if (current()) { this.detach?.(); this.detach = undefined; this.error = e instanceof Error ? e.message : String(e); } }
  }
}
