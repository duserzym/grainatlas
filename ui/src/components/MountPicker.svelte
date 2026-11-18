<script lang="ts">
  // Links the project to a UPbDB mount: a list from GET /v1/mounts when the server answers
  // (hosted token, pasted token, or the same-origin sign-in), else a typed mount id.
  import { app } from '../lib/store.svelte';
  import { authToken } from '../lib/atlas';
  import { oversightSession } from '../lib/oversight';
  import { mountPage, mountPath, type MountPage } from '../lib/mount-browser';
  import { createMount, type MountOut, type MountType } from '../lib/upbdb';

  let { value, onchange, disabled = false, existingOnly = false, startNew = false }: { value: string | null; onchange: (id: string) => void; disabled?: boolean; existingOnly?: boolean; startNew?: boolean } = $props();
  let mounts = $state<MountOut[] | null>(null);
  let note = $state('');
  let typed = $state(false);
  let creating = $state(false);
  let label = $state('');
  let mountType = $state<MountType>('epoxy_1in');
  let busy = $state(false);
  let error = $state('');
  const locked = $derived(disabled || app.readOnly || oversightSession.readOnly);
  let loadVersion = 0;
  let listing = $state(false);
  let next = $state<string | null>(null);
  let query = $state('');
  const pages = new Map<string, MountPage>();

  async function load(server: string | null, more = false) {
    if (more && listing) return;
    const version = ++loadVersion;
    if (!more) mounts = null;
    creating = false;
    error = '';
    if (!server) {
      note = existingOnly ? 'Set the database server in Settings to pick a mount.' : 'No UPbDB server set (Settings): type the mount id.';
      return;
    }
    listing = true;
    note = 'Loading mounts…';
    try {
      const token = await authToken();
      const path = mountPath({ q: query }, more ? next : null);
      const key = JSON.stringify([server, token, oversightSession.state?.viewAs, path]);
      const page = await mountPage(server, token ?? '', path, pages.get(key));
      pages.set(key, page);
      const listed = page.rows;
      if (version !== loadVersion) return;
      mounts = more ? [...(mounts ?? []), ...listed.filter(m => !mounts?.some(old => old.id === m.id))] : listed;
      next = page.next;
      note = mounts.length ? '' : 'No mounts yet: create one.';
      if (startNew && !more) newMount();
    } catch (e) {
      if (version !== loadVersion) return;
      note = `${e instanceof Error ? e.message : e}${existingOnly ? '' : '. Type the mount id instead.'}`;
    } finally { if (version === loadVersion) listing = false; }
  }
  function newMount() {
    label = (app.project?.name ?? '').trim().slice(0, 100);
    mountType = 'epoxy_1in';
    error = '';
    creating = true;
  }
  async function create(event: SubmitEvent) {
    event.preventDefault();
    const server = app.server;
    const project = app.project;
    if (!server || locked || busy || !label.trim()) return;
    busy = true;
    error = '';
    try {
      const token = await authToken();
      if (app.server !== server || app.project !== project || locked) return;
      const mount = await createMount(server, token, { label: label.trim(), mount_type: mountType });
      if (app.server !== server || app.project !== project || locked) return;
      ++loadVersion; // A pending list request must not replace the newly created mount.
      mounts = [...(mounts ?? []).filter(m => m.id !== mount.id), mount];
      typed = false;
      note = '';
      creating = false;
      onchange(mount.id);
    } catch (e) {
      if (app.server === server && app.project === project) error = e instanceof Error ? e.message : String(e);
    } finally {
      busy = false;
    }
  }
  $effect(() => {
    const server = app.server;
    const viewAs = oversightSession.state?.viewAs;
    void viewAs;
    void load(server);
  });
</script>

<div class="picker">
  <form onsubmit={(e) => { e.preventDefault(); void load(app.server); }}>
    <label class="field">Find a mount<input bind:value={query} placeholder="Label or sample" /></label>
    <button type="submit" disabled={busy || listing}>Search</button>
  </form>
  {#if mounts?.length && !typed}
    <label class="field">Mount
      <select value={value ?? ''} disabled={locked || busy} onchange={(e) => onchange(e.currentTarget.value)} data-testid="mount-select">
        <option value="">not linked</option>
        {#each mounts as m (m.id)}
          <option value={m.id} disabled={!m.can_edit}>{m.label} · {m.creator?.name ?? 'Unknown owner'} · {m.mount_type} · {m.grain_count} grains, {m.spot_count} spots{m.can_edit ? '' : ' (read-only)'}</option>
        {/each}
        {#if !existingOnly && value && !mounts.some((m) => m.id === value)}<option value={value}>{value}</option>{/if}
      </select>
    </label>
    {#if !existingOnly}<button class="linkish" disabled={locked || busy} onclick={() => (typed = true)}>Type an id instead</button>{/if}
  {:else if !existingOnly}
    <label class="field">Mount id
      <input type="text" placeholder="UPbDB mount UUID" value={value ?? ''} disabled={locked || busy} onchange={(e) => onchange(e.currentTarget.value)} data-testid="mount-id" />
    </label>
    {#if mounts?.length}<button class="linkish" disabled={locked || busy} onclick={() => (typed = false)}>Pick from the list</button>{/if}
  {/if}
  {#if next}<button disabled={busy || listing} onclick={() => load(app.server, true)}>Show more mounts</button>{/if}
  {#if note}<p class="faint small" data-testid="mount-note">{note}</p>{/if}
  {#if app.server}
    {#if creating}
      <form onsubmit={create} aria-label="New mount" aria-busy={busy}>
        <label class="field">Label
          <input type="text" bind:value={label} required maxlength="100" disabled={locked || busy} data-testid="mount-label" />
        </label>
        <label class="field">Type
          <select bind:value={mountType} disabled={locked || busy} data-testid="mount-type">
            <option value="epoxy_1in">Epoxy mount, 1 inch</option>
            <option value="epoxy_1in_half">Epoxy mount, half inch</option>
            <option value="thin_section">Thin section</option>
            <option value="grain_tape">Grain tape</option>
          </select>
        </label>
        {#if error}<p class="faint small" role="alert">{error}</p>{/if}
        <div class="actions">
          <button type="submit" class="primary" disabled={locked || busy || !label.trim()}>{busy ? 'Creating…' : 'Create'}</button>
          <button type="button" disabled={busy} onclick={() => { creating = false; error = ''; }}>Cancel</button>
        </div>
      </form>
    {:else}
      <button class="linkish" disabled={locked || busy} onclick={newMount}>New mount…</button>
    {/if}
  {/if}
</div>

<style>
  .picker {
    display: grid;
    gap: var(--space-1);
  }
  form { display: grid; gap: var(--space-2); min-width: 0; }
  .actions { display: flex; flex-wrap: wrap; gap: var(--space-2); }
  .linkish {
    justify-self: start;
    border: none;
    background: none;
    padding: 0;
    color: var(--accent);
    font-size: 0.8rem;
  }
  .small {
    font-size: 0.8rem;
    margin: 0;
  }
</style>
