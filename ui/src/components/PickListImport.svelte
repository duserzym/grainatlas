<script lang="ts">
  import StatusChip from '../lib/ui/StatusChip.svelte';
  import { onMount, tick } from 'svelte';
  import { current } from '../lib/store.svelte';
  import { authToken } from '../lib/atlas';
  import { TimsApi, type CapsuleOut, type TimsPages } from '../lib/tims';
  import { importable, pickNotesRequest, pickNotesRows, PickNotesIdentity, PickNotesRefused, type PickNotesPlan, type PickNotesRequest } from '../lib/tims-import';
  import { canWrite } from '../lib/presentation';
  import { oversightSession } from '../lib/oversight';
  import { browserAccount, SERVER_SESSION_CHANGED, type BrowserLab } from '../lib/mount-browser';
  import { mountMirror } from '../lib/mount-mirror';
  import { SyncApiError } from '../lib/sync-api';

  let { open = $bindable(false) }: { open?: boolean } = $props();
  const app = current();
  let text = $state(''), filename = $state(''), lab = $state(''), started = $state('');
  let first = $state<number | undefined>(), last = $state<number | undefined>();
  let labNames = $state.raw<BrowserLab[]>([]);
  let sets = $state.raw<CapsuleOut[]>([]), plan = $state.raw<PickNotesPlan | null>(null);
  let busy = $state(false), offline = $state(typeof navigator !== 'undefined' && !navigator.onLine);
  let error = $state(''), success = $state(''), showIgnored = $state(false), confirming = $state(false);
  let plannedRequest = $state('');
  let dialog = $state<HTMLDivElement>(null!), notes = $state<HTMLTextAreaElement>(null!), file = $state<HTMLInputElement>(null!);
  let generation = 0;
  let pages: TimsPages = {};
  let cacheScope = '';
  const identity = new PickNotesIdentity();
  const server = $derived(app.remote?.server ?? app.server);
  const labs = $derived([...new Set(sets.map(s => s.lab_id))]);
  const request = $derived.by(() => {
    try { return pickNotesRequest({ text, lab, first, last, filename, started }); } catch { return null; }
  });
  const currentPlan = $derived(plan && plannedRequest === JSON.stringify(request) ? plan : null);
  const rows = $derived(currentPlan ? pickNotesRows(currentPlan, app.project?.upbdb.mount_id ?? null, showIgnored) : []);
  const allowed = $derived(canWrite() && !oversightSession.readOnly);
  const ready = $derived(!!currentPlan && importable(currentPlan) && !offline && allowed);
  const count = $derived(currentPlan?.lines.filter(l => l.kind === 'pick' && l.status === 'ok' && l.number >= currentPlan.first_line && l.number <= currentPlan.last_line).length ?? 0);

  async function api() {
    if (!server) throw new Error('Choose a database in Settings');
    const token = await authToken(app, server);
    if (!token) throw new Error('Sign in to import a pick list');
    const scope = JSON.stringify([server, token, app.remote?.access, oversightSession.state?.viewAs?.username]);
    if (scope !== cacheScope) { pages = {}; plan = null; sets = []; labNames = []; lab = ''; cacheScope = scope; }
    return new TimsApi(server, token, app.remote?.access ?? null, app.project?.uuid);
  }
  // A read that started before an `offline` event must not clear it when its reply lands.
  let offlineEvents = 0;
  async function loadSets(sequence: number) {
    const address = server, events = offlineEvents;
    if (!address) { error = 'Choose a database in Settings'; return; }
    try {
      const client = await api();
      const values = await client.list<CapsuleOut>('/v1/tims/capsule-sets?active=true&limit=500', pages);
      if (sequence !== generation) return;
      sets = values;
      const token = await authToken(app, server);
      if (token) {
        const account = await browserAccount(address, token, { getItem: mountMirror.getItem.bind(mountMirror), setItem: () => {} });
        if (sequence !== generation) return;
        labNames = account.labs;
      }
      if (!labs.includes(lab)) lab = values[0]?.lab_id ?? '';
      if (events === offlineEvents) offline = false;
    } catch (e) { if (sequence === generation) failure(e); }
  }
  $effect(() => {
    if (!open) { generation++; return; }
    const scope = [server, app.link.token, app.remote?.access, oversightSession.state?.viewAs?.username]; void scope;
    const sequence = ++generation;
    const previous = document.activeElement as HTMLElement | null;
    void tick().then(() => notes?.focus());
    void loadSets(sequence);
    return () => { generation++; previous?.focus(); };
  });
  onMount(() => {
    const hide = () => { generation++; pages = {}; cacheScope = ''; sets = []; labNames = []; lab = ''; plan = null; confirming = false; success = ''; };
    window.addEventListener(SERVER_SESSION_CHANGED, hide);
    return () => { generation++; window.removeEventListener(SERVER_SESSION_CHANGED, hide); };
  });
  $effect(() => { if (!currentPlan) confirming = false; });
  function failure(e: unknown) {
    if (e instanceof PickNotesRefused) { plan = e.plan; error = e.message; offline = false; }
    else { offline = !(e instanceof SyncApiError) && (!navigator.onLine || /network|reach|fetch|connection/i.test(e instanceof Error ? e.message : String(e))); error = e instanceof Error ? e.message : String(e); }
  }
  async function choose(e: Event) {
    const selected = (e.currentTarget as HTMLInputElement).files?.[0];
    if (!selected) return;
    try {
      if (selected.size > 4000000) throw new Error('Use a notes file of at most 1000000 characters');
      text = await selected.text(); filename = selected.name; plan = null; success = ''; confirming = false;
    } catch (e) { failure(e); }
    file.value = '';
  }
  async function preview() {
    error = ''; success = ''; confirming = false;
    let body: PickNotesRequest;
    try { body = pickNotesRequest({ text, lab, first, last, filename, started }); } catch (e) { failure(e); return; }
    busy = true;
    const sequence = generation;
    try {
      const client = await api(), result = await client.previewPickNotes(body);
      if (sequence !== generation) return;
      plan = result; plannedRequest = JSON.stringify(body); offline = false;
    } catch (e) { if (sequence === generation) failure(e); }
    finally { busy = false; }
  }
  async function apply() {
    if (!ready || !request || !confirming || busy) return;
    const body = request, sequence = generation;
    confirming = false; busy = true; error = '';
    try {
      const client = await api(), result = await client.importPickNotes(body, identity.forRequest(body));
      if (sequence !== generation) return;
      plan = result.plan; offline = false;
      success = `Created ${result.created.fractions.length} fractions, ${result.created.batches.length} batches and ${result.created.sources.length} sources`;
      // Read the mount list once; badge projection remains local and does not edit the project.
      await app.tims.store?.refresh();
    } catch (e) { if (sequence === generation) failure(e); }
    finally { busy = false; }
  }
  function key(e: KeyboardEvent) {
    e.stopPropagation();
    if (e.key === 'Escape' && !busy) { e.preventDefault(); open = false; }
    if (e.key !== 'Tab') return;
    const fields = [...dialog.querySelectorAll<HTMLElement>('textarea, input:not(:disabled):not([type=hidden]), select:not(:disabled), button:not(:disabled)')].filter(el => el.getClientRects().length);
    const i = fields.indexOf(document.activeElement as HTMLElement);
    e.preventDefault(); fields[(i + (e.shiftKey ? -1 : 1) + fields.length) % fields.length]?.focus();
  }
</script>

<svelte:window onoffline={() => { offlineEvents++; offline = true; }} ononline={() => { offline = false; if (open) void loadSets(generation); }} />
{#if open}
<div class="backdrop" role="presentation">
  <div class="glass panel dialog" bind:this={dialog} role="dialog" tabindex="-1" aria-modal="true" aria-labelledby="pick-import-title" onkeydown={key} data-testid="pick-import-dialog">
    <h2 id="pick-import-title">Import pick list</h2>
    <label class="field">Notes <textarea bind:this={notes} rows="7" bind:value={text} disabled={busy} data-testid="pick-notes"></textarea></label>
    <div class="row"><button disabled={busy} onclick={() => file.click()}>Choose file…</button><span class="faint small">{filename || '.md or .txt'}</span></div>
    <input hidden type="file" bind:this={file} accept=".md,.txt,text/plain,text/markdown" onchange={choose} />
    <div class="row">
      <label class="field">First line <input type="number" min="1" bind:value={first} disabled={busy} /></label>
      <label class="field">Last line <input type="number" min="1" bind:value={last} disabled={busy} /></label>
      <label class="field">Start date <input type="date" bind:value={started} disabled={busy} /></label>
      {#if labs.length > 1}<label class="field">Lab <select aria-label="Lab" bind:value={lab} disabled={busy}>{#each labs as id}<option value={id}>{labNames.find(l => l.id === id)?.handle ?? sets.find(s => s.lab_id === id)?.lab_name ?? id}</option>{/each}</select></label>{/if}
    </div>
    <p class="faint small">Start date is used only for picks without a date heading. Preview does not write to the database.</p>
    {#if offline}<p role="status">Import needs the database; your notes stay here</p>{/if}
    {#if !allowed}<p class="faint small">Switch to an editable view to import a pick list.</p>{/if}
    {#if error}<p class="danger" role="alert">{error}</p>{/if}
    {#if success}<p role="status">{success}</p>{/if}
    <div class="row"><button onclick={preview} disabled={busy || !allowed || offline || !lab || !text.trim()} data-testid="pick-preview">{busy ? 'Working…' : 'Preview'}</button><button class="primary" disabled={!ready || busy || !!success} onclick={() => confirming = true} data-testid="pick-import">Import</button><button disabled={busy} onclick={() => open = false}>Close</button></div>
    {#if currentPlan}
      <p class="faint small">{currentPlan.summary.ok} ok · {currentPlan.summary.refused} refused · {currentPlan.summary.skipped} skipped · {currentPlan.summary.ignored} ignored</p>
      <label class="check"><input type="checkbox" bind:checked={showIgnored} /> Show ignored lines</label>
      <div class="table-scroll"><table class="data"><thead><tr>{#each ['Line', 'Text', 'Result', 'Sample', 'Mount', 'Spot', 'Grain', 'Capsule', 'Fraction'] as heading}<th>{heading}</th>{/each}</tr></thead><tbody>
        {#each rows as row (row.number)}<tr class:open-mount={row.openMount} data-testid="pick-import-row"><td>{row.number}</td><td>{row.text}</td><td><StatusChip text={row.result} /></td><td>{row.sample}</td><td>{row.mount}</td><td>{row.spot}</td><td>{row.grain}</td><td>{row.capsule}</td><td>{row.fraction}</td></tr>{/each}
      </tbody></table></div>
      <ul data-testid="pick-import-batches">{#each currentPlan.batches as batch}<li>{batch.action === 'create' ? 'Create' : 'Reuse'} {sets.find(s => s.id === batch.set_id)?.name ?? batch.set_id} batch started {batch.started_on}{batch.explanation ? ` · ${batch.explanation}` : ''}</li>{/each}</ul>
    {/if}
    {#if confirming && ready}<div class="panel" data-testid="pick-import-confirm"><p>Create {count} fractions in {currentPlan?.batches.length ?? 0} batches in the database?</p><div class="row"><button class="primary" onclick={apply}>Confirm import</button><button onclick={() => confirming = false}>Cancel</button></div></div>{/if}
  </div>
</div>
{/if}
<style>
  .backdrop {position:fixed;inset:0;z-index:80;display:grid;place-items:center;background:color-mix(in srgb,var(--bg) 55%,transparent);padding:var(--space-3);}
  .dialog {width:min(1100px,100%);max-height:calc(100dvh - 2rem);overflow:auto;display:grid;gap:var(--space-2);min-width:0;}
  h2,p {margin:0;}
  .row {display:flex;flex-wrap:wrap;gap:var(--space-2);align-items:end;}
  .row .field {max-width:12rem;}
  .table-scroll {overflow:auto;max-height:32vh;}
  td {min-width:4rem;overflow-wrap:anywhere;}
  td:nth-child(2) {min-width:15rem;white-space:pre-wrap;}
  .open-mount {background:var(--accent-soft);}
  .danger {color:var(--danger);}
</style>
