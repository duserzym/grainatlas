<script lang="ts">
  import { isHosted } from '@lab/bridge';
  import { oversightSession } from '../lib/oversight';
  import { app } from '../lib/store.svelte';
  import { syncReminder } from '../lib/close-sync';
  import SyncAttention from './SyncAttention.svelte';
  import MountPicker from './MountPicker.svelte';
  import SyncLinkOffer from './SyncLinkOffer.svelte';
  import { describeChange, elementLabel, type Difference, type Resolution } from '../lib/sync';
  let dialogElement = $state<HTMLDialogElement>();
  let { detailsOpen = $bindable(false) }: { detailsOpen?: boolean } = $props();
  let mount = $state('');
  let choices = $state<Record<string, Resolution>>({});
  let publishImages = $state(true);
  const hosted = $derived(isHosted() || app.hosted);
  const session = $derived(app.sync);
  const action = $derived(session.staleOpen ? 'Review…' : session.imageDecision ? 'Review images' : session.confirmation ? 'Confirm…' : session.state === 'unregistered' ? 'Register…' : session.state === 'server-newer' ? 'Get latest changes' : session.state === 'both-changed' ? session.detailText === 'Database has more' ? 'Sync now' : 'Review differences' : session.state === 'offline' ? 'Retry' : session.state === 'checking' ? 'Check' : !hosted && !app.link.syncAutomatically ? 'Sync' : 'Sync now');
  const pillState = $derived(session.text === 'Needs your attention' ? 'needs-confirmation' : session.text.startsWith('Syncing') ? 'checking' : session.text === 'Up to date' ? 'in-sync' : session.text === 'Offline, will sync later' ? 'offline' : 'local-changes');
  const outbox = $derived(hosted ? 0 : syncReminder(app).outbox);
  const text = $derived(!hosted && !app.link.syncAutomatically && outbox && !session.busy && !session.imageBusy && session.text !== 'Needs your attention'
    ? `${outbox + session.counts.spots + session.counts.grains + session.counts.labels} ${outbox + session.counts.spots + session.counts.grains + session.counts.labels === 1 ? 'change' : 'changes'} not synced` : session.text);
  const revisionTitle = $derived(session.status ? `v${session.status.atlas_version ?? '—'} r${session.status.revision}` : '');
  const title = $derived([session.detailText, session.detailText.includes(revisionTitle) ? '' : revisionTitle, app.draftHere === 'unknown' ? 'Your draft here, base not checked' : app.draftHere !== null ? `Your draft here, based on r${app.draftHere}` : '', session.syncedAt ? `Last synced ${new Date(session.syncedAt).toLocaleString()}` : '', session.syncDisabledReason || session.error].filter(Boolean).join(' · '));
  const kindName = (d: Difference) => d.kind === 'labels' ? 'Annotation' : d.kind === 'grains' ? 'Grain' : 'Spot';
  const nameOf = (d: Difference) => elementLabel(d.kind, d.id, { [d.kind]: [d.mine, d.theirs, d.base].filter(e => !!e) }, session.draft?.base);
  const grainName = (id: string | null | undefined) => id ? session.label('grains', id) : 'none';
  function sideText(d: Difference, side: 'mine' | 'theirs'): string {
    const after = d[side];
    if (!after && d.base) return side === 'theirs' ? d.kind === 'grains' ? 'Retired in the database' : 'Deleted in the database' : d.kind === 'grains' ? 'Retired here' : 'Deleted here';
    const lines = describeChange(d.kind, d.base, after, grainName);
    return lines.length ? lines.join(', ') : 'No change';
  }
  const complete = $derived(session.review?.merge.differences.filter(d => d.collision).every(d => choices[`${d.kind}:${d.id}`]));
  $effect(() => { if (session.dialog === 'review') choices = {}; });
  $effect(() => { if (session.dialog && dialogElement && !dialogElement.open) dialogElement.showModal(); });
</script>

{#if app.project}
  <div class="sync">
    <button class="sync-pill state-{pillState}" data-testid="sync-chip" {title} aria-busy={session.busy || session.imageBusy} aria-expanded={detailsOpen} aria-controls={detailsOpen ? 'sync-details' : undefined} onclick={() => detailsOpen = !detailsOpen}>
      <svg class:spinning={pillState === 'checking'} viewBox="0 0 16 16" width="14" height="14" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
        {#if pillState === 'in-sync'}<path d="m3 8 3 3 7-7" />
        {:else if pillState === 'checking'}<path d="M13.5 8A5.5 5.5 0 1 1 8 2.5" />
        {:else if pillState === 'needs-confirmation'}<path d="M8 2 1.5 14h13L8 2Zm0 4v4m0 2v.1" />
        {:else if pillState === 'offline'}<path d="m2 2 12 12M5 5a5 5 0 0 0-2 3m5-5a5 5 0 0 1 5 5M5 11a4 4 0 0 1 4-1m-1 3v.1" />
        {:else}<circle cx="8" cy="8" r="2" fill="currentColor" />{/if}
      </svg>
      <span aria-live="polite">{text}</span>
    </button>
    {#if session.staleOpen || session.imageDecision || session.state !== 'in-sync' || session.confirmation || session.pixelsChanged || outbox}
      <button class:primary={pillState !== 'checking' && pillState !== 'offline'} data-testid="sync-action" title={session.syncDisabledReason} disabled={!!session.syncDisabledReason || session.busy || session.imageBusy || app.loadingProject || (app.readOnly && session.state === 'local-changes')} onclick={() => { detailsOpen = false; void session.action(); }}>{action}</button>
    {/if}
  </div>
{/if}
{#if session.dialog}
    <dialog bind:this={dialogElement} class="glass panel review" oncancel={() => session.cancelConfirmation()} onkeydown={e => e.stopPropagation()} aria-modal="true" aria-label={session.dialog === 'bulk' ? 'Confirm sync' : session.dialog === 'review' ? 'Review differences' : session.dialog === 'register' ? 'Register project' : 'Sync to database'} tabindex="-1">
      {#if session.dialog === 'stale' && session.staleOpen}
        <SyncAttention title={session.staleBulk ? 'Keep these spots?' : 'This copy is older than the shared project'}
          message={session.staleBulk ? `${session.staleOpen.spots.length} spots would be removed from the shared project.` : `${session.staleOpen.spots.length} spots and ${session.staleOpen.grains.length} grains that are in the shared project are missing here.${session.staleOpen.labels.length ? ` ${session.staleOpen.labels.length} labels are also missing.` : ''}`}
          spots="Bring them back keeps every spot; Remove deletes them from the shared project for everyone.">
          {#snippet actions()}
            <button class="primary" disabled={session.busy || app.readOnly} onclick={() => session.bringBack()}>Bring them back</button>
            <button disabled={session.busy || app.readOnly} onclick={() => session.removeStale()}>Remove them</button>
          {/snippet}
        </SyncAttention>
      {:else if session.dialog === 'replace-images'}
        <SyncAttention title="Replace the current images?"
          message="This mount already shows images from another project. Replace them with this project's images?"
          spots="nothing; spots and grains are not changed by images.">
          {#snippet actions()}
            <button class="primary" onclick={() => session.keepImages()}>Keep the current images (recommended)</button>
            <button onclick={() => session.replaceImages()}>Replace</button>
          {/snippet}
        </SyncAttention>
      {:else if session.dialog === 'bulk' && session.confirmation}
        <SyncAttention title="Keep these grains and spots?"
          message={`This change removes ${session.confirmation.retireGrains} grains and ${session.confirmation.deleteSpots} spots from the shared project.`}
          spots={`${session.confirmation.deleteSpots} spots would be removed. Spots with measurements are kept.`}>
          {#snippet actions()}
            <button class="primary" disabled={session.busy} onclick={() => session.keepBulk()}>Keep them (recommended)</button>
            <button disabled={session.busy} onclick={() => session.confirmBulk()}>Remove and sync</button>
          {/snippet}
        </SyncAttention>
      {:else if session.dialog === 'register'}
        <h2>Register project</h2>
        <SyncLinkOffer />
        <p>Pick a mount or create one.</p>
        <MountPicker value={mount} onchange={id => mount = id} disabled={session.busy} existingOnly />
        {#if session.pixelsChanged}<label class="check"><input type="checkbox" bind:checked={publishImages} /> Publish a new atlas version with these images</label>{/if}
        <div class="actions"><button onclick={() => session.dialog = null}>Cancel</button><button class="primary" disabled={oversightSession.readOnly || !mount || session.busy} onclick={() => session.register(mount, session.pixelsChanged && publishImages)}>Register and sync</button></div>
      {:else if session.dialog === 'pixels'}
        <h2>Sync to database</h2>
        <p>New images not published. Element changes and images are saved separately.</p>
        <label class="check"><input type="checkbox" bind:checked={publishImages} /> Publish a new atlas version with these images</label>
        <div class="actions"><button onclick={() => session.dialog = null}>Cancel</button><button class="primary" onclick={() => { session.dialog = null; void session.sync(publishImages); }}>Sync to database</button></div>
      {:else if session.review}
        {#if session.registeredSpotConflict}
          <SyncAttention title="A saved spot changed in two places" message="Choose which version to keep. Other changes are combined automatically."
            spots="Nothing changes until you choose. Spots with measurements cannot be deleted." />
        {:else}
          <h2>Review differences</h2>
          <p>Choose which version to keep. Other changes are combined automatically.</p>
        {/if}
        <div class="differences">
          {#each session.review.merge.differences as d (`${d.kind}:${d.id}`)}
            <section class="panel" data-testid={d.collision ? 'sync-collision' : 'sync-combined'}>
              <strong data-testid="sync-difference-name">{kindName(d)} {nameOf(d)}</strong>
              <p class="faint small">{d.collision ? 'Both changed' : 'Combined automatically'}</p>
              <div class="versions"><div><b>Mine</b><p data-testid="sync-mine-change">{sideText(d, 'mine')}</p></div><div><b>Shared version</b><p data-testid="sync-theirs-change">{sideText(d, 'theirs')}</p></div></div>
              <details><summary class="faint small">Details</summary>
                <p class="faint small mono">{d.kind} {d.id}</p>
                <div class="versions"><pre>{d.mine ? JSON.stringify(d.mine, null, 2) : 'Deleted'}</pre><pre>{d.theirs ? JSON.stringify(d.theirs, null, 2) : 'Deleted'}</pre></div>
              </details>
              {#if d.collision}
                <div class="actions">
                  {#each ['theirs', 'mine', ...(d.bothAllowed && !(d.kind === 'spots' && d.base) ? ['both'] : [])] as option}
                    <label class="check"><input type="radio" name={`resolve-${d.kind}-${d.id}`} value={option} checked={choices[`${d.kind}:${d.id}`] === option} onchange={() => choices[`${d.kind}:${d.id}`] = option as Resolution} />{option === 'mine' ? 'Keep my version' : option === 'theirs' ? 'Keep shared version (recommended)' : 'Keep both as separate copies'}</label>
                  {/each}
                </div>
              {/if}
            </section>
          {/each}
        </div>
        {#if session.pixelsChanged}<label class="check"><input type="checkbox" bind:checked={publishImages} /> Publish a new atlas version with these images</label>{/if}
        <div class="actions"><button class="primary" onclick={() => session.dialog = null}>Keep working (recommended)</button><button disabled={!complete || session.busy} data-testid="sync-apply" onclick={() => session.apply(choices, session.pixelsChanged && publishImages)}>Apply</button></div>
      {/if}
    </dialog>
{/if}

<style>
  .sync { display:flex; align-items:center; gap:var(--space-2); flex:1 1 22rem; min-width:0; font-size:0.8rem; }
  .sync button { height:2rem; font-size:inherit; white-space:nowrap; }
  .sync > [data-testid="sync-action"] { flex:none; padding-inline:var(--space-2); }
  .sync .sync-pill { display:flex; align-items:center; gap:var(--space-2); border-radius:var(--radius-sm); min-width:3rem; max-width:max-content; padding-inline:var(--space-3); color:var(--text-muted); border-color:var(--hairline); background:var(--glass); }
  .sync-pill span { overflow:hidden; text-overflow:ellipsis; white-space:nowrap; min-width:0; }
  .sync-pill svg { flex:none; }
  .sync .state-in-sync { color:var(--ok); background:color-mix(in srgb, var(--ok) 7%, transparent); }
  .sync .state-local-changes, .sync .state-needs-confirmation { color:var(--warning); border-color:color-mix(in srgb, var(--warning) 35%, transparent); background:color-mix(in srgb, var(--warning) 12%, transparent); }
  .sync .state-server-newer { color:var(--accent); background:var(--accent-soft); border-color:color-mix(in srgb, var(--accent) 35%, transparent); }
  .sync .state-both-changed { color:var(--danger); background:color-mix(in srgb, var(--danger) 12%, transparent); border-color:color-mix(in srgb, var(--danger) 35%, transparent); }
  .sync .state-unregistered { background:transparent; }
  .sync .state-offline { background:color-mix(in srgb, var(--text-muted) 8%, transparent); }
  .spinning { animation:spin 1s linear infinite; }
  @keyframes spin { to { transform:rotate(360deg); } }
  @media (prefers-reduced-motion: reduce) { .spinning { animation:none; } }
  dialog::backdrop { background:color-mix(in srgb, var(--bg) 60%, transparent); }
  .review { width:min(52rem, calc(100vw - 2rem)); max-height:calc(100vh - 2rem); overflow:auto; background:var(--glass-strong); }
  h2 { margin-top:0; }
  .differences { display:grid; gap:var(--space-2); max-height:55vh; overflow:auto; }
  .versions { display:grid; grid-template-columns:minmax(0,1fr) minmax(0,1fr); gap:var(--space-2); }
  pre { white-space:pre-wrap; overflow-wrap:anywhere; max-height:12rem; overflow:auto; font-size:0.75rem; }
  .actions { display:flex; flex-wrap:wrap; justify-content:flex-end; gap:var(--space-2); margin-top:var(--space-3); }
</style>
