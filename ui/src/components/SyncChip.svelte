<script lang="ts">
  import { app } from '../lib/store.svelte';
  import MountPicker from './MountPicker.svelte';
  import type { Resolution } from '../lib/sync';
  let dialogElement = $state<HTMLDialogElement>();
  let offerOpen = $state(false);
  let mount = $state('');
  let choices = $state<Record<string, Resolution>>({});
  let publishImages = $state(true);
  const session = $derived(app.sync);
  const action = $derived(session.state === 'unregistered' ? 'Register…' : session.state === 'server-newer' ? 'Update from database' : session.state === 'both-changed' ? 'Review differences' : session.state === 'offline' ? 'Retry' : session.state === 'checking' ? 'Check' : 'Sync to database');
  const complete = $derived(session.review?.merge.differences.filter(d => d.collision).every(d => choices[`${d.kind}:${d.id}`]));
  $effect(() => { if (session.dialog === 'review') choices = {}; });
  $effect(() => { if (session.dialog && dialogElement && !dialogElement.open) dialogElement.showModal(); });
</script>

{#if app.project}
  <div class="sync" title={session.error || session.text}>
    {#if session.candidates.length && session.state === 'unregistered'}
      <button data-testid="sync-chip" aria-expanded={offerOpen} onclick={() => offerOpen = !offerOpen}>{session.text}</button>
      {#if offerOpen}
        <div class="glass panel link-offer" data-testid="sync-link-offer">
          {#each session.candidates as candidate (candidate.project_uuid)}
            <div><span>Link to mount {candidate.mount_label}?</span> <button disabled={session.busy} onclick={() => { offerOpen = false; void session.link(candidate); }}>Link</button></div>
          {/each}
          <button onclick={() => { offerOpen = false; session.dismissLink(); }}>Not this one</button>
        </div>
      {/if}
    {:else}<span data-testid="sync-chip" title={session.text} aria-live="polite">{session.text}{app.draftHere === 'unknown' ? ' · Your draft here, base not checked' : app.draftHere !== null ? ` · Your draft here, based on r${app.draftHere}` : ''}</span>{/if}
    <button data-testid="sync-action" disabled={session.busy || app.loadingProject || (session.state === 'in-sync' && !session.pixelsChanged)} onclick={() => session.action()}>{action}</button>
  </div>
{/if}
{#if session.dialog}
    <dialog bind:this={dialogElement} class="glass panel review" oncancel={() => session.dialog = null} onkeydown={e => e.stopPropagation()} aria-modal="true" aria-label={session.dialog === 'review' ? 'Review differences' : session.dialog === 'register' ? 'Register project' : 'Sync to database'} tabindex="-1">
      {#if session.dialog === 'register'}
        <h2>Register project</h2>
        <p>Pick an existing mount. Creating mounts is available in the database.</p>
        <MountPicker value={mount} onchange={id => mount = id} existingOnly />
        {#if session.pixelsChanged}<label class="check"><input type="checkbox" bind:checked={publishImages} /> Publish a new atlas version with these images</label>{/if}
        <div class="actions"><button onclick={() => session.dialog = null}>Cancel</button><button class="primary" disabled={!mount || session.busy} onclick={() => session.register(mount, session.pixelsChanged && publishImages)}>Register and sync</button></div>
      {:else if session.dialog === 'pixels'}
        <h2>Sync to database</h2>
        <p>New images not published. Element changes and images are saved separately.</p>
        <label class="check"><input type="checkbox" bind:checked={publishImages} /> Publish a new atlas version with these images</label>
        <div class="actions"><button onclick={() => session.dialog = null}>Cancel</button><button class="primary" onclick={() => { session.dialog = null; void session.sync(publishImages); }}>Sync to database</button></div>
      {:else if session.review}
        <h2>Review differences</h2>
        <p>Choose each conflicting element. Other changes are combined automatically. Deleted elements are shown as deleted.</p>
        <div class="differences">
          {#each session.review.merge.differences as d (`${d.kind}:${d.id}`)}
            <section class="panel" data-testid={d.collision ? 'sync-collision' : 'sync-combined'}>
              <strong>{d.kind} · {d.id}</strong>
              <p class="faint small">{d.collision ? 'Both changed' : 'Combined automatically'}</p>
              <div class="versions"><div><b>Mine</b><pre>{d.mine ? JSON.stringify(d.mine, null, 2) : 'Deleted'}</pre></div><div><b>Database</b><pre>{d.theirs ? JSON.stringify(d.theirs, null, 2) : 'Deleted'}</pre></div></div>
              {#if d.collision}
                <div class="actions">
                  {#each ['mine', 'theirs', ...(d.bothAllowed ? ['both'] : [])] as option}
                    <label class="check"><input type="radio" name={`resolve-${d.kind}-${d.id}`} value={option} checked={choices[`${d.kind}:${d.id}`] === option} onchange={() => choices[`${d.kind}:${d.id}`] = option as Resolution} />{option === 'mine' ? 'Mine' : option === 'theirs' ? 'Theirs' : 'Both (keep a separate copy)'}</label>
                  {/each}
                </div>
              {/if}
            </section>
          {/each}
        </div>
        {#if session.pixelsChanged}<label class="check"><input type="checkbox" bind:checked={publishImages} /> Publish a new atlas version with these images</label>{/if}
        <div class="actions"><button onclick={() => session.dialog = null}>Cancel</button><button class="primary" disabled={!complete || session.busy} data-testid="sync-apply" onclick={() => session.apply(choices, session.pixelsChanged && publishImages)}>Apply</button></div>
      {/if}
    </dialog>
{/if}

<style>
  .sync { position:relative; display:flex; align-items:center; gap:var(--space-2); flex:1 1 0; min-width:0; max-width:34rem; font-size:0.8rem; }
  .sync > [data-testid="sync-chip"] { overflow:hidden; text-overflow:ellipsis; white-space:nowrap; min-width:0; }
  .sync button { flex:none; padding:var(--space-1) var(--space-2); font-size:inherit; }
  .sync > button[data-testid="sync-chip"] { flex:0 1 auto; }
  .link-offer { position:absolute; top:calc(100% + var(--space-2)); right:0; width:max-content; max-width:24rem; background:var(--glass-strong); z-index:30; }
  dialog::backdrop { background:color-mix(in srgb, var(--bg) 60%, transparent); }
  .review { width:min(52rem, calc(100vw - 2rem)); max-height:calc(100vh - 2rem); overflow:auto; background:var(--glass-strong); }
  h2 { margin-top:0; }
  .differences { display:grid; gap:var(--space-2); max-height:55vh; overflow:auto; }
  .versions { display:grid; grid-template-columns:minmax(0,1fr) minmax(0,1fr); gap:var(--space-2); }
  pre { white-space:pre-wrap; overflow-wrap:anywhere; max-height:12rem; overflow:auto; font-size:0.75rem; }
  .actions { display:flex; flex-wrap:wrap; justify-content:flex-end; gap:var(--space-2); margin-top:var(--space-3); }
</style>
