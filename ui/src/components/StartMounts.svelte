<script lang="ts">
  import { onMount, untrack } from 'svelte';
  import { app } from '../lib/store.svelte';
  import { mountMirror } from '../lib/mount-mirror';
  import { authToken } from '../lib/atlas';
  import { DRAFTS_CHANGED, listDrafts, type Draft } from '../lib/drafts';
  import { sameId, type RecentEntry } from '../lib/deeplink';
  import type { MountOut } from '../lib/upbdb';
  import { openFromServer } from '../lib/remote';
  import { oversightSession } from '../lib/oversight';
  import { SERVER_SESSION_CHANGED, mirrorServer, accountLabs, defaultScope, readResponse, checkedJson, browserAccount, recentRevisionIds, recentRevisions, cacheKey, hideMountCache, readCache, writeCache, mountPage, mountPath, localMatch, tileBadges, type BrowserAccount, type BrowserLab, type MountCache } from '../lib/mount-browser';
  import MountThumbnail from './MountThumbnail.svelte';
  let { recent, onopen, onlink, oncontent, onauth, onoffline, refresh = 0 }: { recent: RecentEntry[]; onopen: (r: RecentEntry) => void; onlink: (m: MountOut) => void; oncontent: (n: number) => void; onauth: (value: boolean) => void; onoffline: (value: boolean) => void; refresh?: number } = $props();
  let account = $state<BrowserAccount | null>(null);
  let token = $state('');
  let server = $state('');
  let rows = $state<MountOut[]>([]);
  let drafts = $state<Draft[]>([]);
  let next = $state<string | null>(null);
  let total = $state(0);
  let loading = $state(false);
  let offline = $state(false);
  let asOf = $state('');
  let error = $state('');
  let filters = $state<Record<string, string>>({ sort: '-last_changed' });
  let search = $state('');
  let advanced = $state(false);
  let loadingLabs = $state(false);
  let revisions = $state<Record<string, number>>({});
  let cache: MountCache = { pages: {} };
  let version = 0;
  let identity = '';
  let debounce: ReturnType<typeof setTimeout>;
  let alive = true;
  const selected = $derived(Object.entries(filters).filter(([k, v]) => v && k !== 'sort'));
  const size = $derived(app.layout.tileSize === 'S' ? 160 : app.layout.tileSize === 'L' ? 250 : 195);
  function save() { if (account) void writeCache(mountMirror, cacheKey(account.mirrorId, account.id), cache); }
  async function load(more = false) {
    if (!account || !token) return;
    const generation = ++version;
    const atServer = server, atToken = token;
    const path = mountPath({ ...filters, as: oversightSession.state?.viewAs?.username ?? '' }, more ? next : null);
    const previous = cache.pages[path];
    loading = true; error = '';
    if (!more) {
      asOf = previous?.at ?? '';
      rows = previous?.rows ?? []; next = previous?.next ?? null; total = previous?.total ?? 0;
      oncontent(rows.length);
    }
    const apply = (page: NonNullable<typeof previous>) => {
      rows = more ? [...rows, ...page.rows.filter(r => !rows.some(old => old.id === r.id))] : page.rows;
      next = page.next; total = page.total; asOf = page.at; oncontent(rows.length);
    };
    try {
      const page = await mountPage(atServer, atToken, path, previous);
      if (!alive || generation !== version) return;
      apply(page); offline = false;
      cache.pages[path] = page; save();
    } catch (e) {
      if (!alive || generation !== version) return;
      const message = e instanceof Error ? e.message : String(e);
      if (/Database (401|403)/.test(message)) {
        hideMountCache(false); account = null; rows = []; oncontent(0); identity = ''; onauth(false); error = message;
      } else {
        offline = !/^Database \d/.test(message);
        if (previous) apply(previous);
        error = message;
      }
    } finally { if (generation === version) loading = false; }
  }
  async function connect(force = false) {
    const base = app.server;
    const auth = base ? await authToken() : null;
    const key = JSON.stringify([base, auth]);
    if (!force && key === identity) return;
    identity = key; ++version;
    rows = []; account = null; cache = { pages: {} }; revisions = {}; offline = false; error = ''; asOf = ''; oncontent(0); onauth(false); token = auth ?? ''; server = base ?? '';
    if (!base || !auth) return;
    try {
      const show = (me: BrowserAccount, saved: MountCache) => {
        if (!alive || identity !== key) return;
        account = me; cache = saved; onauth(true); filters = defaultScope(me);
        const page = cache.pages[mountPath({ ...filters, as: oversightSession.state?.viewAs?.username ?? '' })];
        rows = page?.rows ?? []; next = page?.next ?? null; total = page?.total ?? 0; asOf = page?.at ?? '';
        oncontent(rows.length);
      };
      const me = await browserAccount(base, auth, mountMirror, show);
      if (!alive || identity !== key) return;
      show(me, await readCache(mountMirror, cacheKey(me.mirrorId, me.id)));
      if (!alive || identity !== key) return;
      await load();
    } catch (e) { if (alive && identity === key) error = e instanceof Error ? e.message : String(e); }
  }
  async function moreLabs() {
    if (!account?.labsNext || loadingLabs) return;
    const me = account, at = identity;
    loadingLabs = true;
    try {
      const query = new URLSearchParams({ limit: '100', after: me.labsNext! });
      const response = await readResponse(`${server}/v1/labs?${query}`, { headers: { Authorization: `Bearer ${token}` } });
      const labs = await checkedJson<BrowserLab[]>(response);
      if (identity !== at || !account) return;
      account.labs = [...account.labs, ...accountLabs(me, labs).filter(l => !account!.labs.some(old => old.handle === l.handle))];
      account.labsNext = response.headers.get('X-Next-Cursor');
      cache.account = account; save();
    } catch (e) { error = e instanceof Error ? e.message : String(e); }
    finally { loadingLabs = false; }
  }
  function change(key: string, value: string) { filters = { ...filters, [key]: value }; void load(); }
  function scope(key: string, value: string) { filters = { ...filters, mine: '', shared: '', lab: '', [key]: value }; void load(); }
  onMount(() => {
    alive = true;
    const update = () => { drafts = listDrafts(); };
    update(); void connect();
    const clear = () => { ++version; rows = []; account = null; cache = { pages: {} }; revisions = {}; offline = false; asOf = ''; error = ''; loading = false; identity = ''; onauth(false); oncontent(0); };
    window.addEventListener(SERVER_SESSION_CHANGED, clear);
    const timer = setInterval(() => void connect(), 5000);
    window.addEventListener(DRAFTS_CHANGED, update);
    const online = () => void connect(true);
    window.addEventListener('online', online);
    return () => { alive = false; ++version; clearInterval(timer); clearTimeout(debounce); window.removeEventListener(SERVER_SESSION_CHANGED, clear); window.removeEventListener(DRAFTS_CHANGED, update); window.removeEventListener('online', online); };
  });
  $effect(() => {
    const me = account, atServer = server, atToken = token;
    const ids = atServer ? recentRevisionIds(recent, drafts, atServer) : [];
    const as = oversightSession.state?.viewAs?.username;
    void as;
    let active = true;
    revisions = {};
    if (me && atToken && ids.length) {
      const atIdentity = identity;
      const visible = () => active && alive && app.screen === 'open' && !!app.server && mirrorServer(app.server) === mirrorServer(atServer) && oversightSession.state?.viewAs?.username === as && identity === atIdentity;
      void recentRevisions(atServer, atToken, me.id, ids, cache.revisions, visible).then(result => {
        if (!visible()) return;
        revisions = Object.fromEntries(result.batches.flatMap(b => b.rows.map(r => [r.project_uuid, r.revision])));
        cache.revisions = result; save();
      }).catch(e => { if (visible()) error = `Revision check: ${e instanceof Error ? e.message : e}`; });
    }
    return () => { active = false; };
  });
  $effect(() => onoffline(offline));
  $effect(() => {
    const changed = refresh; const as = oversightSession.state?.viewAs?.username;
    void changed; void as;
    untrack(() => { if (account) void load(); });
  });
</script>
<section class="mounts" data-testid="mount-browser">
  <div class="row"><h2>Mounts</h2><span class="faint small">{account ? `${rows.length} of ${total}` : ''}</span>
    <span class="sizes" role="group" aria-label="Tile size">{#each ['S', 'M', 'L'] as s}<button class:primary={app.layout.tileSize === s} onclick={() => { app.layout.tileSize = s as 'S' | 'M' | 'L'; app.saveLayout(); }}>{s}</button>{/each}</span>
  </div>
  {#if account}
    <div class="row" aria-label="Mount scope">
      <button class:primary={filters.mine === 'true'} onclick={() => scope('mine', 'true')}>Mine</button>
      {#each account.labs as lab}<button class:primary={filters.lab === lab.handle} onclick={() => scope('lab', lab.handle)}>{lab.handle}</button>{/each}
      {#if account.labsNext}<button disabled={loadingLabs || offline} onclick={moreLabs}>More labs…</button>{/if}
      <button class:primary={filters.shared === 'true'} onclick={() => scope('shared', 'true')}>Shared with me</button>
      {#if account.can_manage_people}<button class:primary={!filters.mine && !filters.shared && !filters.lab} onclick={() => scope('lab', '')}>All labs</button>{/if}
    </div>
    <div class="row filters">
      <input aria-label="Search mounts" placeholder="Search mounts" bind:value={search} oninput={() => { clearTimeout(debounce); debounce = setTimeout(() => change('q', search), 300); }} />
      <select aria-label="Sort mounts" value={filters.sort.replace(/^-/, '')} onchange={e => change('sort', (filters.sort.startsWith('-') ? '-' : '') + e.currentTarget.value)}>
        {#each [['last_changed', 'Last change'], ['created', 'Created date'], ['imaged', 'Imaging date'], ['session', 'Session date'], ['label', 'Label'], ['creator', 'Creator'], ['lab', 'Lab'], ['mineral', 'Mineral']] as [key, label]}
          <option value={key}>{label}</option>
        {/each}
      </select>
      <select aria-label="Sort direction" value={filters.sort.startsWith('-') ? 'descending' : 'ascending'} onchange={e => change('sort', (e.currentTarget.value === 'descending' ? '-' : '') + filters.sort.replace(/^-/, ''))}>
        <option value="ascending">Ascending</option><option value="descending">Descending</option>
      </select>
      <button aria-expanded={advanced} onclick={() => advanced = !advanced}>Filters…</button>
      <button disabled={loading} onclick={() => load()}>Refresh</button>
    </div>
    {#if advanced}
      <form class="row filters" onsubmit={e => { e.preventDefault(); void load(); }}>
        {#each [['mineral', 'Mineral'], ['sample', 'Sample'], ['creator', 'Creator'], ['lab', 'Lab']] as [key, label]}
          <label class="field">{label}<input value={filters[key] ?? ''} onchange={e => change(key, e.currentTarget.value)} /></label>
        {/each}
        <label class="field">Date<select value={filters.date_field ?? 'last_changed'} onchange={e => change('date_field', e.currentTarget.value)}>{#each [['last_changed', 'Last change'], ['created', 'Created'], ['imaged', 'Imaging'], ['session', 'Session']] as [key, name]}<option value={key}>{name}</option>{/each}</select></label>
        <label class="field">From<input type="date" value={filters.date_from ?? ''} onchange={e => change('date_from', e.currentTarget.value)} /></label>
        <label class="field">To<input type="date" value={filters.date_to ?? ''} onchange={e => change('date_to', e.currentTarget.value)} /></label>
      </form>
    {/if}
    {#if selected.length}<div class="row" aria-label="Active filters">{#each selected as [key, value]}<button class="small" onclick={() => change(key, '')}>{key.replaceAll('_', ' ')}: {value} ×</button>{/each}</div>{/if}
    {#if offline}<p role="status" data-testid="offline-cache">Offline{asOf ? `, as of ${new Date(asOf).toLocaleString()}` : ''}</p>{/if}
    {#if error}<p role="status" class="faint small">{error}</p>{/if}
    {#if loading}<p class="faint small" role="status">Loading mounts…</p>{/if}
    <div class="tiles" style={`--tile-size: ${size}px`}>
      {#each rows as mount (mount.id)}
        {@const match = localMatch(mount, recent, drafts, server)}
        {@const local = match?.recent ?? (match?.draft ? { id: match.draft.uuid, name: match.draft.name, when: match.draft.updated_at, areas: 0, spots: 0 } : null)}
        {@const published = mount.atlas?.project_id}
        {@const localProject = match?.draft?.linked_uuid ?? local?.id}
        {@const separateDraft = !!local && !!published && !sameId(localProject, published)}
        {@const openPublished = published && !offline && (!local || separateDraft)}
        {@const revision = match?.draft ? revisions[match.draft.linked_uuid ?? match.draft.uuid] : undefined}
        <article class="glass panel tile" data-testid="mount-tile">
          {#key mount.atlas?.thumbnail ?? mount.id}
          <MountThumbnail url={mount.atlas?.thumbnail ?? null} cached={mount.thumbnail_data} fallback={match?.recent?.thumbnail} {server} {token} {offline} onsave={data => { mount.thumbnail_data = data; for (const page of Object.values(cache.pages)) { const row = page.rows.find(r => r.id === mount.id); if (row) row.thumbnail_data = data; } save(); }} />
          {/key}
          <button class="title" disabled={!local && !openPublished} onclick={() => openPublished ? openFromServer(server, published, null, null, mount.id) : local && onopen(local)}>{mount.label}</button>
          <p class="faint small">{mount.creator?.name ?? 'Unknown owner'}{mount.lab ? ` · ${mount.lab.handle}` : ''}</p>
          <p class="small">{(mount.sample_names ?? []).slice(0, 3).join(', ')}{Math.max(0, (mount.sample_count ?? mount.sample_names?.length ?? 0) - 3) ? ` +${(mount.sample_count ?? mount.sample_names?.length ?? 0) - 3}` : ''}</p>
          <p class="small">{mount.grain_count} grains · {mount.spot_count} spots</p>
          <p class="faint small">{mount.last_changed_at ? new Date(mount.last_changed_at).toLocaleString() : 'No changes yet'}</p>
          <div class="badges">{#each tileBadges(mount, match, account.id, revision) as badge}<span class="small">{badge}</span>{/each}</div>
          <div class="row">
            {#if local}<button onclick={() => onopen(local)}>{separateDraft ? 'Open other local copy' : 'Open local'}</button>{/if}
            {#if published && (!local || separateDraft)}<button disabled={offline} onclick={() => openFromServer(server, published, null, null, mount.id)}>Open from server</button>{/if}
            {#if !match}<button disabled={offline || !mount.can_edit || oversightSession.readOnly} onclick={() => onlink(mount)}>Link a local project…</button>{/if}
          </div>
        </article>
      {/each}
    </div>
    {#if !rows.length && !loading}<p class="muted">No mounts match these filters.</p>{/if}
    {#if next}<button disabled={loading || offline} onclick={() => load(true)}>Show more</button>{/if}
  {:else}
    <p class="muted">Sign in to see your mounts</p>
    {#if error}<p class="faint small" role="status">{error}</p>{/if}
    <button onclick={() => app.screen = 'settings'}>Settings</button>
  {/if}
</section>
<style>
  .mounts { min-width: 0; display: flex; flex-direction: column; gap: var(--space-2); }
  h2, p { margin: 0; }
  .row { display: flex; gap: var(--space-2); align-items: center; flex-wrap: wrap; }
  .sizes { margin-left: auto; display: flex; gap: var(--space-1); }
  .filters input { min-width: 0; width: 150px; }
  .filters label { min-width: 110px; }
  .filters select { max-width: 160px; }
  .tiles { display: grid; grid-template-columns: repeat(auto-fill, minmax(min(100%, var(--tile-size)), 1fr)); gap: var(--space-3); align-items: start; }
  .tile { min-width: 0; padding: var(--space-3); display: grid; gap: var(--space-2); }
  .title { text-align: left; font-weight: 600; overflow-wrap: anywhere; }
  .tile p { overflow-wrap: anywhere; }
  .badges { display: flex; flex-wrap: wrap; gap: var(--space-1); }
  .badges span { background: var(--accent-soft); border-radius: var(--radius); padding: 2px 5px; }
  .tile .row button { font-size: .8rem; }
</style>
