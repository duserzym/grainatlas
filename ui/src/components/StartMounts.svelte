<script lang="ts">
  import SegmentedControl from '../lib/ui/SegmentedControl.svelte';
  import ProgressBar from '../lib/ui/ProgressBar.svelte';
  import MoreMenu from '../lib/ui/MoreMenu.svelte';
  import Icon from '../lib/ui/Icon.svelte';
  import type { Snippet } from 'svelte';
  import { lastChangeTime, mountColumns, sortFromHeader, headerSort } from '../lib/open-presentation';
  import StatusChip from '../lib/ui/StatusChip.svelte';
  import IconButton from '../lib/ui/IconButton.svelte';
  import { onMount, untrack } from 'svelte';
  import { isHosted } from '@lab/bridge';
  import { startSyncPolling } from '../lib/sync-scheduling';
  import { app } from '../lib/store.svelte';
  import { mountMirror } from '../lib/mount-mirror';
  import { authToken } from '../lib/atlas';
  import { DRAFTS_CHANGED, listDrafts, storedDraftProject, type Draft } from '../lib/drafts';
  import type { RecentEntry } from '../lib/deeplink';
  import type { MountOut } from '../lib/upbdb';
  import { bundleProjects } from '../lib/offline-bundles';
  import { openOfflineCopy, openFromServer } from '../lib/remote';
  import { oversightSession } from '../lib/oversight';
  import { SERVER_SESSION_CHANGED, mirrorServer, accountLabs, defaultScope, readResponse, checkedJson, browserAccount, recentRevisionIds, recentRevisions, cacheKey, hideMountCache, readCache, writeCache, mountPage, mountPath, localMatch, startEntries, separateLocal, entryStatus, tileBadges, type BrowserAccount, type BrowserLab, type MountCache } from '../lib/mount-browser';
  import MountThumbnail from './MountThumbnail.svelte';
  let { recent, onopen, onlink, oncontent, onauth, onoffline, onrows, titleActions, orderControl, localColumn, splitter, databaseFirst, split = $bindable(), refresh = 0 }: { recent: RecentEntry[]; onopen: (r: RecentEntry) => Promise<boolean>; onlink: (m: MountOut) => void; oncontent: (n: number) => void; onauth: (value: boolean) => void; onoffline: (value: boolean) => void; onrows: (rows: MountOut[]) => void; refresh?: number; titleActions: Snippet; orderControl: Snippet; localColumn: Snippet; splitter: Snippet; databaseFirst: boolean; split: HTMLDivElement | undefined } = $props();
  let cachedBundles = $state<string[]>([]);
  let account = $state<BrowserAccount | null>(null);
  let token = $state('');
  let server = $state('');
  let rows = $state<MountOut[]>([]);
  let drafts = $state<Draft[]>([]);
  const entries = $derived(startEntries(rows, recent, drafts, server));
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
  let revisionRefresh = $state(0);
  let revisionRefreshSeen: string | null = null;
  function refreshDatabase() {
    revisionRefresh++;
    if (account) void load(); else void connect(true);
  }
  const selected = $derived(Object.entries(filters).filter(([k, v]) => v && k !== 'sort'));
  const scopeOptions = $derived(account ? [
    { value: 'mine', label: 'Mine' },
    ...account.labs.map(lab => ({ value: `lab:${lab.handle}`, label: lab.handle })),
    { value: 'shared', label: 'Shared with me' },
    ...(account.can_manage_people ? [{ value: 'all', label: 'All labs' }] : []),
  ] : []);
  // Advanced filters can combine scopes. Do not imply an exclusive selection then.
  const scopeValue = $derived([filters.mine, filters.shared, filters.lab].filter(Boolean).length > 1 ? '' : filters.mine === 'true' ? 'mine' : filters.shared === 'true' ? 'shared' : filters.lab ? `lab:${filters.lab}` : account?.can_manage_people ? 'all' : '');
  function chooseScope(value: string) {
    if (value === 'mine') scope('mine', 'true');
    else if (value === 'shared') scope('shared', 'true');
    else scope('lab', value === 'all' ? '' : value.slice(4));
  }
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

    window.addEventListener(DRAFTS_CHANGED, update);
    const online = () => { if (isHosted() || app.link.syncAutomatically) void connect(true); };
    const disconnected = () => { offline = true; };
    window.addEventListener('offline', disconnected);
    window.addEventListener('online', online);
    return () => { alive = false; ++version; clearTimeout(debounce); window.removeEventListener(SERVER_SESSION_CHANGED, clear); window.removeEventListener(DRAFTS_CHANGED, update); window.removeEventListener('online', online); window.removeEventListener('offline', disconnected); };
  });
  $effect(() => startSyncPolling(isHosted() || app.link.syncAutomatically, 5000, () => void connect()));
  // Credential changes are explicit in standalone Settings; polling is unnecessary.
  $effect(() => {
    const configured = app.link.server, credential = app.link.token;
    void configured; void credential;
    if (!isHosted()) untrack(() => { void connect(); });
  });
  const revisionIds = $derived(JSON.stringify(server ? recentRevisionIds(recent, drafts, server) : []));
  $effect(() => {
    const me = account, atServer = server, atToken = token;
    const ids: string[] = isHosted() || app.link.syncAutomatically
      ? atServer ? recentRevisionIds(recent, drafts, atServer) : []
      : JSON.parse(revisionIds);
    const refreshRequested = `${refresh}/${revisionRefresh}`;
    const force = revisionRefreshSeen !== null && revisionRefreshSeen !== refreshRequested;
    revisionRefreshSeen = refreshRequested;
    const as = oversightSession.state?.viewAs?.username;
    void as;
    let active = true;
    revisions = {};
    if (me && atToken && ids.length) {
      const atIdentity = identity;
      const visible = () => active && alive && app.screen === 'open' && !!app.server && mirrorServer(app.server) === mirrorServer(atServer) && oversightSession.state?.viewAs?.username === as && identity === atIdentity;
      void recentRevisions(atServer, atToken, me.id, ids, cache.revisions, visible, !isHosted() && force).then(result => {
        if (!visible()) return;
        revisions = Object.fromEntries(result.batches.flatMap(b => b.rows.map(r => [r.project_uuid, r.revision])));
        cache.revisions = result; save();
      }).catch(e => { if (visible()) error = `Revision check: ${e instanceof Error ? e.message : e}`; });
    }
    return () => { active = false; };
  });
  $effect(() => onrows(rows));
  $effect(() => {
    const me = account;
    let active = true;
    cachedBundles = [];
    if (me) void bundleProjects(mountMirror, me.mirrorId, me.id).then(ids => { if (active) cachedBundles = ids; }).catch(() => {});
    return () => { active = false; };
  });
  async function openDatabase(mount: MountOut, match: ReturnType<typeof localMatch>) {
    const published = mount.atlas?.project_id;
    const local = match?.recent ?? (match?.draft ? { id: match.draft.uuid, name: match.draft.name, when: match.draft.updated_at, areas: 0, spots: 0 } : null);
    try {
      if (local && !separateLocal(mount, match)) {
        if (await onopen(local)) return;
        if (!offline && published) {
          const location = local.path ?? local.package_path ?? (match?.draft ? storedDraftProject(match.draft)?.package.path : null) ?? 'the recorded location';
          if (await openFromServer(server, published, null, null, mount.id, undefined, true)) app.notice = `The local copy was not found at ${location}; showing the database version.`;
        }
      } else if (published) {
        if (offline) {
          if (!await openOfflineCopy(server, published, mount.id, mount.label)) app.notice = 'The offline copy is no longer available; connect to the database to open it.';
        } else if (await openFromServer(server, published, null, null, mount.id, undefined, true) && app.screen === 'open') app.screen = 'canvas';
      }
    } catch (e) { app.error = e instanceof Error ? e.message : String(e); }
  }
  $effect(() => onoffline(offline));
  let refreshSeen = -1;
  $effect(() => {
    const changed = refresh; const as = oversightSession.state?.viewAs?.username;
    void as;
    if ((refreshSeen === -1 || changed === refreshSeen) && !isHosted()) { refreshSeen = changed; return; }
    refreshSeen = changed;
    untrack(() => { if (account) void load(); else void connect(true); });
  });
</script>
<div class="start-mounts">
  <div class="title-row">
    <div class="heading"><h2>Mounts</h2><span class="faint small">{account ? `${rows.length} of ${total}` : ''}</span></div>
    {#if account}
      <SegmentedControl label="Mount scope" size="small" wrap options={scopeOptions} value={scopeValue} onchange={chooseScope} />
      {#if account.labsNext}<button disabled={loadingLabs || offline} onclick={moreLabs}>More labs…</button>{/if}
      {#if !offline}<StatusChip text="Connected" />{/if}
    {/if}
    <div class="title-actions">{@render titleActions()}</div>
  </div>
  <div class="filter-row">
    {#if account}
      <input aria-label="Search mounts" placeholder="Search mounts, samples, people" bind:value={search} oninput={() => { clearTimeout(debounce); debounce = setTimeout(() => change('q', search), 300); }} />
      <select aria-label="Sort mounts" value={filters.sort.replace(/^-/, '')} onchange={e => change('sort', (filters.sort.startsWith('-') ? '-' : '') + e.currentTarget.value)}>
        {#each [['last_changed', 'Last change'], ['created', 'Created date'], ['imaged', 'Imaging date'], ['session', 'Session date'], ['label', 'Label'], ['creator', 'Creator'], ['lab', 'Lab'], ['mineral', 'Mineral']] as [key, label]}
          <option value={key}>{label}</option>
        {/each}
      </select>
      <select aria-label="Sort direction" value={filters.sort.startsWith('-') ? 'descending' : 'ascending'} onchange={e => change('sort', (e.currentTarget.value === 'descending' ? '-' : '') + filters.sort.replace(/^-/, ''))}>
        <option value="ascending">Ascending</option><option value="descending">Descending</option>
      </select>
    {/if}
    {@render orderControl()}
    {#if account}
      <IconButton icon="filter" label="Filters…" aria-expanded={advanced} onclick={() => advanced = !advanced} />
      <IconButton icon="refresh" label="Refresh" disabled={loading} onclick={refreshDatabase} />
    {/if}
    <div class="sizes"><SegmentedControl label="Tile size" size="small" options={['S', 'M', 'L'].map(value => ({ value, label: value }))} value={app.layout.tileSize}
      onchange={value => { app.layout.tileSize = value as 'S' | 'M' | 'L'; app.saveLayout(); }} /></div>
    {#if account && selected.length}<div class="active-filters" aria-label="Active filters">{#each selected as [key, value]}<button class="small" onclick={() => change(key, '')}>{key.replaceAll('_', ' ')}: {value} ×</button>{/each}</div>{/if}
  </div>
  {#if account && advanced}
    <form class="row filters" onsubmit={e => { e.preventDefault(); void load(); }}>
      {#each [['mineral', 'Mineral'], ['sample', 'Sample'], ['creator', 'Creator'], ['lab', 'Lab']] as [key, label]}
        <label class="field">{label}<input value={filters[key] ?? ''} onchange={e => change(key, e.currentTarget.value)} /></label>
      {/each}
      <label class="field">Date<select value={filters.date_field ?? 'last_changed'} onchange={e => change('date_field', e.currentTarget.value)}>{#each [['last_changed', 'Last change'], ['created', 'Created'], ['imaged', 'Imaging'], ['session', 'Session']] as [key, name]}<option value={key}>{name}</option>{/each}</select></label>
      <label class="field">From<input type="date" value={filters.date_from ?? ''} onchange={e => change('date_from', e.currentTarget.value)} /></label>
      <label class="field">To<input type="date" value={filters.date_to ?? ''} onchange={e => change('date_to', e.currentTarget.value)} /></label>
    </form>
  {/if}
  <div class="split" class:database-first={databaseFirst} bind:this={split} style={`--recent-width: ${app.layout.recentWidth}px`}>
    {#each databaseFirst ? ['mounts', 'splitter', 'local'] : ['local', 'splitter', 'mounts'] as column (column)}
      {#if column === 'local'}{@render localColumn()}
      {:else if column === 'splitter'}{@render splitter()}
      {:else}
      <section class="mounts" data-testid="mount-browser" aria-label="Mount list">
        {#if account}
          {#if offline}<p role="status" data-testid="offline-cache" title={error || undefined}><StatusChip text={`Offline copy${asOf ? ` · as of ${new Date(asOf).toLocaleString()}` : ''}`} /></p>{/if}
          {#if error && !offline}<p role="status" class="faint small">{error}</p>{/if}
          {#if loading}<p class="faint small" role="status">Loading mounts…</p>{/if}
          <div class="mount-table" class:small-rows={app.layout.tileSize === 'S'} class:medium-rows={app.layout.tileSize === 'M'} class:large-cards={app.layout.tileSize === 'L'} role="table" aria-label="Mounts">
            <div class="column-headers" role="row">
              <span role="columnheader" aria-label="Overview"></span>
              {#each mountColumns as column}
                <div role="columnheader" aria-sort={headerSort(filters.sort, column.sort)} class:number={column.label === 'Grains' || column.label === 'Spots'}>
                  {#if column.sort}<button onclick={() => change('sort', sortFromHeader(filters.sort, column.sort!))}>{column.label}<span aria-hidden="true">{headerSort(filters.sort, column.sort) === 'ascending' ? ' ↑' : headerSort(filters.sort, column.sort) === 'descending' ? ' ↓' : ''}</span></button>
                  {:else}{column.label}{/if}
                </div>
              {/each}
              <span role="columnheader" aria-label="Open"></span><span role="columnheader" aria-label="Actions"></span>
            </div>
            <div class="tiles" role="rowgroup">
              {#each entries.tiles as { mount, match } (mount.id)}
                {@const local = match?.recent ?? (match?.draft ? { id: match.draft.uuid, name: match.draft.name, when: match.draft.updated_at, areas: 0, spots: 0 } : null)}
                {@const published = mount.atlas?.project_id}
                {@const separateDraft = separateLocal(mount, match)}
                {@const hasLocal = !!local && !separateDraft}
                {@const hasOffline = !oversightSession.state?.viewAs && !!published && cachedBundles.includes(published.toLowerCase())}
                {@const disabledReason = hasLocal ? '' : offline ? hasOffline ? '' : 'Not on this computer; needs a connection' : published ? '' : 'No published database version'}
                {@const revision = match?.draft ? revisions[match.draft.linked_uuid ?? match.draft.uuid] : undefined}
                {@const samples = mount.sample_names ?? []}
                {@const sampleLimit = app.layout.tileSize === 'S' ? 1 : 3}
                {@const extraSamples = Math.max(0, (mount.sample_count ?? samples.length) - sampleLimit)}
                <div class="glass tile" role="row" data-testid="mount-tile">
                  <div class="preview" role="cell">{#key mount.atlas?.thumbnail ?? mount.id}
                  <MountThumbnail compact={app.layout.tileSize !== 'L'} url={mount.atlas?.thumbnail ?? null} cached={mount.thumbnail_data} fallback={match?.recent?.thumbnail} {server} {token} {offline} onsave={data => { mount.thumbnail_data = data; for (const page of Object.values(cache.pages)) { const row = page.rows.find(r => r.id === mount.id); if (row) row.thumbnail_data = data; } save(); }} />
                  {/key}</div>
                  <div class="mount-name" role="cell">
                    <button class="row-open" aria-label={mount.label} aria-describedby={`mount-owner-${mount.id} mount-samples-${mount.id} mount-facts-${mount.id} mount-spots-${mount.id} mount-changed-${mount.id} mount-state-${mount.id}`} disabled={!!disabledReason}
                      title={disabledReason || (offline && !hasLocal && hasOffline ? 'Open offline copy' : `${mount.label}\n${samples.join(', ')}\n${mount.last_changed_at ?? 'No changes yet'}`)} onclick={() => openDatabase(mount, match)}></button>
                    <strong title={mount.label}>{mount.label}</strong>
                    <span class="faint small owner" id={`mount-owner-${mount.id}`} title={`${mount.creator?.name ?? 'Unknown owner'}${app.layout.tileSize !== 'S' && mount.lab ? ` · ${mount.lab.handle}` : ''}`}>{mount.creator?.name ?? 'Unknown owner'}{app.layout.tileSize !== 'S' && mount.lab ? ` · ${mount.lab.handle}` : ''}</span>
                  </div>
                  <div class="samples small" role="cell" id={`mount-samples-${mount.id}`} title={samples.join(', ')}><span class="sample-names">{samples.slice(0, sampleLimit).join(', ')}</span>{#if extraSamples}<span class="sample-extra">{` +${extraSamples}`}</span>{/if}</div>
                  <div class="number small" role="cell" title={`${mount.grain_count} grains`} id={`mount-facts-${mount.id}`} aria-label={`${mount.grain_count} grains`}>{mount.grain_count}</div>
                  <div class="number small" role="cell" title={`${mount.spot_count} spots`} id={`mount-spots-${mount.id}`} aria-label={`${mount.spot_count} spots`}>{mount.spot_count}</div>
                  <div class="changed faint small" role="cell" id={`mount-changed-${mount.id}`} title={mount.last_changed_at ?? undefined}>{lastChangeTime(mount.last_changed_at)}</div>
                  <div class="mount-state" role="cell" id={`mount-state-${mount.id}`}>
                    <div data-testid="tile-status">
                      {#each entryStatus(mount, match, revision, { offline, hasOffline }) as part}<p class="small"><StatusChip text={part} /></p>{/each}
                    </div>
                    {#each tileBadges(mount, match, account.id, revision).filter(b => b.startsWith('Shared ·')) as badge}<StatusChip text={badge} />{/each}
                    {#if app.project?.uuid === published && app.sync.imageBusy}<ProgressBar value={app.sync.imageProgress?.done} total={app.sync.imageProgress?.total ?? 0} noun={`${mount.label} image files synced`} />{/if}
                  </div>
                  <span class="chevron" role="cell"><Icon kind="chevron-left" /></span>
                  <div class="mount-actions" role="cell">
                    {#if local && separateDraft || !match}
                      <MoreMenu label={`More actions for ${mount.label}`}>
                        {#if local && separateDraft}<button role="menuitem" onclick={() => onopen(local)}>Open other local copy</button>{/if}
                        {#if !match}<button role="menuitem" disabled={offline || !mount.can_edit || oversightSession.readOnly} title={offline ? 'Needs a connection' : oversightSession.readOnly ? 'Viewing as another account is read-only' : !mount.can_edit ? 'No permission to edit this mount' : undefined} onclick={() => onlink(mount)}>Link a local project…</button>{/if}
                      </MoreMenu>
                    {/if}
                  </div>
                </div>
              {/each}
            </div>
          </div>
          {#if !rows.length && !loading}<p class="muted">No mounts match these filters.</p>{/if}
          {#if next}<button data-testid="next-page" disabled={loading || offline} onclick={() => load(true)}>Show more</button>{/if}
        {:else}
          <p class="muted">Sign in to see your mounts</p>
          {#if error}<p class="faint small" role="status">{error}</p>{/if}
          <button onclick={() => app.screen = 'settings'}>Settings</button>
        {/if}
      </section>
      {/if}
    {/each}
  </div>
</div>
<style>
  .start-mounts, .mounts { min-width: 0; display: flex; flex-direction: column; gap: var(--space-2); }
  .title-row, .heading, .title-actions, .filter-row, .active-filters, .filters { display: flex; align-items: center; gap: var(--space-2); flex-wrap: wrap; min-width: 0; }
  .title-row { flex-wrap: wrap; }
  .heading { flex: none; }
  .title-actions { margin-left: auto; flex: none; }
  .title-actions :global(button) { white-space: nowrap; }
  .sizes { margin-left: auto; }
  .filter-row > input { flex: 1 1 180px; min-width: 140px; }
  .filter-row > select { max-width: 140px; }
  .active-filters { flex-basis: 100%; }
  .active-filters button { padding: 1px 6px; }
  .filters label { flex: 1 1 110px; min-width: 0; }
  .filters input, .filters select { width: 100%; min-width: 0; }
  .split { display: grid; grid-template-columns: minmax(180px, var(--recent-width)) 8px minmax(0, 1fr); gap: var(--space-2); align-items: start; min-width: 0; }
  .split.database-first { grid-template-columns: minmax(0, 1fr) 8px minmax(180px, var(--recent-width)); }
  h2, p { margin: 0; }
  .mount-table { --mount-columns: 40px minmax(120px, 1.2fr) minmax(70px, .8fr) 44px 44px 74px minmax(200px, 1.6fr) 16px 30px; min-width: 0; }
  .column-headers, .tile { display: grid; grid-template-columns: var(--mount-columns); gap: var(--space-2); align-items: center; padding-inline: var(--space-2); min-width: 0; }
  .column-headers { white-space: nowrap; height: 28px; font-size: 11px; color: var(--text-muted); }
  .column-headers button { display: inline-flex; align-items: center; gap: 2px; white-space: nowrap; padding: 0; border: 0; background: transparent; color: inherit; font-size: inherit; font-weight: 600; text-align: left; }
  .tiles { min-width: 0; }
  .tile { position: relative; min-height: 54px; border-radius: var(--radius-sm); box-shadow: none; border: 1px solid var(--hairline); }
  .tile + .tile { margin-top: 2px; }
  .tile:not(:focus-within):not(:hover) { content-visibility: auto; contain-intrinsic-size: auto 54px; }
  .tile:focus-within { z-index: 2; }
  .tile:hover { background: var(--row-hover); }
  .row-open { position: absolute; inset: 0 40px 0 0; z-index: 1; border: 0; background: transparent; border-radius: var(--radius-sm); padding: 0; }
  .row-open:hover:not(:disabled), .row-open:active:not(:disabled) { background: transparent; }
  .row-open:focus-visible { outline: 2px solid var(--accent); outline-offset: -2px; }
  .row-open:disabled { cursor: default; }
  .preview, .mount-name, .samples, .changed, .mount-state { min-width: 0; }
  .mount-name { display: grid; align-content: center; line-height: 1.15; }
  .mount-name strong, .owner, .samples, .changed { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .samples { display: flex; align-items: baseline; }
  .sample-names { min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .sample-extra { flex: none; white-space: pre; }
  .mount-name strong { font-size: 13px; }
  .owner { font-size: 11px; }
  .number { text-align: right; font-variant-numeric: tabular-nums; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .changed { font-variant-numeric: tabular-nums; font-size: 11px; }
  .mount-state { display: grid; justify-items: start; gap: 0; line-height: 12px; }
  .mount-state p { line-height: 12px; }
  .mount-state :global(.status) { padding: 0 4px; font-size: 10px; line-height: 11px; vertical-align: top; }
  .chevron { transform: rotate(180deg); color: var(--text-faint); }
  .mount-actions { position: relative; z-index: 2; }
  .small-rows { --mount-thumbnail-height: 30px; --mount-columns: 40px minmax(120px, 1.2fr) minmax(60px, .45fr) 44px 44px 92px minmax(240px, 2fr) 16px 30px; }
  .medium-rows { --mount-thumbnail-height: 52px; --mount-columns: 64px minmax(120px, 1.2fr) minmax(70px, .8fr) 44px 44px 74px minmax(200px, 1.6fr) 16px 30px; }
  .medium-rows .tile { min-height: 76px; }
  .medium-rows .tile:not(:focus-within):not(:hover) { contain-intrinsic-size: auto 76px; }
  .medium-rows .mount-name { gap: 3px; }
  .large-cards { --mount-thumbnail-height: 150px; }
  .large-cards .column-headers { display: none; }
  .large-cards .tiles { display: grid; grid-template-columns: repeat(auto-fill, minmax(min(100%, 260px), 1fr)); gap: var(--space-2); }
  .large-cards .tile { grid-template-columns: 1fr 1fr; gap: var(--space-2); padding: var(--space-3); margin: 0; align-content: start; }
  .large-cards .tile:not(:focus-within):not(:hover) { contain-intrinsic-size: auto 330px; }
  .large-cards .preview, .large-cards .mount-name, .large-cards .samples, .large-cards .mount-state, .large-cards .changed { grid-column: 1 / -1; }
  .large-cards .mount-name { gap: 3px; }
  .large-cards .number { text-align: left; }
  .large-cards .number::after { content: ' grains'; }
  .large-cards .number + .number::after { content: ' spots'; }
  .large-cards .mount-state { gap: 3px; padding-bottom: 24px; }
  .large-cards .mount-state :global(.status) { font-size: 12px; line-height: 1.3; padding: 1px 5px; }
  .large-cards .mount-state p { line-height: 1.4; }
  .large-cards .row-open { right: 0; }
  .large-cards .mount-actions { position: absolute; right: 6px; bottom: 6px; }
  .large-cards .chevron { position: absolute; left: 12px; bottom: 12px; }
  @media (max-width: 1100px) {
    .mount-table { --mount-columns: 32px minmax(100px, 1fr) minmax(50px, .4fr) 38px 38px 84px minmax(180px, 1.8fr) 14px 30px; }
    .column-headers, .tile { gap: var(--space-1); padding-inline: var(--space-1); }
  }
  @media (max-width: 850px) { .split, .split.database-first { grid-template-columns: minmax(0, 1fr); } }
</style>
