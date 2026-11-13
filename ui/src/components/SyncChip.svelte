<script lang="ts">
  import { app } from '../lib/store.svelte';
  import MountPicker from './MountPicker.svelte';
  import type { Resolution } from '../lib/sync';
  let dialogElement = $state<HTMLDialogElement>();
  let { detailsOpen = $bindable(false) }: { detailsOpen?: boolean } = $props();
  let mount = $state('');
  let choices = $state<Record<string, Resolution>>({});
  let publishImages = $state(true);
  const session = $derived(app.sync);
  const action = $derived(session.confirmation ? 'Confirm…' : session.state === 'unregistered' ? 'Register…' : session.state === 'server-newer' ? 'Update from database' : session.state === 'both-changed' ? session.text === 'Database has more' ? 'Sync to database' : 'Review differences' : session.state === 'offline' ? 'Retry' : session.state === 'checking' ? 'Check' : 'Sync to database');
  const pillState = $derived(session.confirmation ? 'needs-confirmation' : session.checking ? 'checking' : session.state === 'in-sync' && session.pixelsChanged ? 'local-changes' : session.state);
  const text = $derived(session.text + (app.draftHere === 'unknown' ? ' · Your draft here, base not checked' : app.draftHere !== null ? ` · Your draft here, based on r${app.draftHere}` : ''));
  const title = $derived([text, session.syncedAt ? `Last synced ${new Date(session.syncedAt).toLocaleString()}` : '', session.syncDisabledReason || session.error].filter(Boolean).join(' · '));
  const complete = $derived(session.review?.merge.differences.filter(d => d.collision).every(d => choices[`${d.kind}:${d.id}`]));
  $effect(() => { if (session.dialog === 'review') choices = {}; });
  $effect(() => { if (session.dialog && dialogElement && !dialogElement.open) dialogElement.showModal(); });
</script>

{#if app.project}
  <div class="sync">
    <button class="sync-pill state-{pillState}" data-testid="sync-chip" {title} aria-busy={session.checking} aria-expanded={detailsOpen} aria-controls={detailsOpen ? 'sync-details' : undefined} onclick={() => detailsOpen = !detailsOpen}>
      <svg class:spinning={pillState === 'checking'} viewBox="0 0 16 16" width="14" height="14" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
        {#if pillState === 'in-sync'}<path d="m3 8 3 3 7-7" />
        {:else if pillState === 'checking'}<path d="M13.5 8A5.5 5.5 0 1 1 8 2.5" />
        {:else if pillState === 'server-newer'}<path d="M8 2v11m-4-4 4 4 4-4" />
        {:else if pillState === 'both-changed'}<path d="m3 5 3-3 3 3M6 2v9m1 0 3 3 3-3m-3 3V5" />
        {:else if pillState === 'needs-confirmation'}<path d="M8 2 1.5 14h13L8 2Zm0 4v4m0 2v.1" />
        {:else if pillState === 'offline'}<path d="m2 2 12 12M5 5a5 5 0 0 0-2 3m5-5a5 5 0 0 1 5 5M5 11a4 4 0 0 1 4-1m-1 3v.1" />
        {:else if pillState === 'unregistered'}<circle cx="8" cy="8" r="5" />
        {:else}<circle cx="8" cy="8" r="2" fill="currentColor" />{/if}
      </svg>
      <span aria-live="polite">{text}</span>
    </button>
    {#if session.state !== 'in-sync' || session.confirmation || session.pixelsChanged}
      <button class:primary={pillState !== 'checking' && pillState !== 'offline'} data-testid="sync-action" title={session.syncDisabledReason} disabled={!!session.syncDisabledReason || session.busy || app.loadingProject || (app.readOnly && session.state === 'local-changes')} onclick={() => { detailsOpen = false; void session.action(); }}>{action}</button>
    {/if}
  </div>
{/if}
{#if session.dialog}
    <dialog bind:this={dialogElement} class="glass panel review" oncancel={() => session.cancelConfirmation()} onkeydown={e => e.stopPropagation()} aria-modal="true" aria-label={session.dialog === 'bulk' ? 'Confirm sync' : session.dialog === 'review' ? 'Review differences' : session.dialog === 'register' ? 'Register project' : 'Sync to database'} tabindex="-1">
      {#if session.dialog === 'bulk' && session.confirmation}
        <h2>Confirm sync</h2>
        <p>This retires {session.confirmation.retireGrains} of {session.confirmation.grainsTotal} grains and deletes {session.confirmation.deleteSpots} spots. Continue?</p>
        <div class="actions"><button onclick={() => session.cancelConfirmation()}>Cancel</button><button class="primary" disabled={session.busy} onclick={() => session.confirmBulk()}>Continue</button></div>
      {:else if session.dialog === 'register'}
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
  .sync { display:flex; align-items:center; gap:var(--space-2); flex:1 1 22rem; min-width:0; font-size:0.8rem; }
  .sync button { height:2rem; font-size:inherit; white-space:nowrap; }
  .sync > [data-testid="sync-action"] { flex:none; padding-inline:var(--space-2); }
  .sync .sync-pill { display:flex; align-items:center; gap:var(--space-2); border-radius:999px; min-width:3rem; max-width:max-content; padding-inline:var(--space-3); color:var(--text-muted); border-color:var(--hairline); }
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
