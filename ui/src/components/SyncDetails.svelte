<script lang="ts">
  import { app } from '../lib/store.svelte';
  import SyncLinkOffer from './SyncLinkOffer.svelte';
  import { changeCounts, KINDS, type Changes, type Element, type Kind, type ReadGrain } from '../lib/sync';

  const session = $derived(app.sync);
  function name(element: Element | ReadGrain | undefined, id: string) {
    if (!element) return id;
    if ('name' in element) return element.name;
    if ('text' in element) return element.text;
    return element.label || `${element.area_id}/${element.number}`;
  }
  function names(changes: Changes, kind: Kind) {
    const removed = [...changes[kind].delete, ...(kind === 'grains' ? changes.grains.retire : [])];
    const elements = [
      ...changes[kind].upsert.map(e => name(e, e.id)),
      ...removed.map(id => `${name(session.draft?.base?.[kind].find(e => e.id === id), id)} (removed)`),
    ];
    return elements.slice(0, 3).join(', ') + (elements.length > 3 ? `, and ${elements.length - 3} more` : '');
  }
</script>

<section id="sync-details" class="panel" data-testid="sync-details" aria-label="Sync details">
  <div class="changes">
    {#each [{ label: 'Local changes', changes: session.localChanges }, { label: 'Database changes', changes: session.databaseChanges }] as group}
      <div>
        <h3>{group.label}</h3>
        {#if group.changes}
          {@const counts = changeCounts(group.changes)}
          {#each KINDS as kind}
            <p><b>{counts[kind]} {counts[kind] === 1 ? kind.slice(0, -1) : kind}</b>{#if counts[kind]}: {names(group.changes, kind)}{/if}</p>
          {/each}
        {:else}<p class="faint small">Base not checked</p>{/if}
      </div>
    {/each}
    <dl>
      <dt>Base revision</dt><dd>{session.baseRevision === null ? 'Not recorded' : `r${session.baseRevision}`}</dd>
      <dt>Server revision</dt><dd>{session.status ? `r${session.status.revision}` : 'Not checked'}</dd>
      <dt>Atlas version</dt><dd>{session.status?.atlas_version == null ? 'Not published' : `v${session.status.atlas_version}`}</dd>
      <dt>Last sync</dt><dd>{session.syncedAt ? new Date(session.syncedAt).toLocaleString() : 'Not recorded'}</dd>
      <dt>Server changed by</dt><dd>{session.status?.changed_by ?? 'Not recorded'}</dd>
      <dt>Server changed at</dt><dd>{session.status?.changed_at ? new Date(session.status.changed_at).toLocaleString() : 'Not recorded'}</dd>
      <dt>Access</dt><dd>{session.status ? session.status.can_edit ? 'Can edit' : 'Read-only' : 'Not checked'}</dd>
      {#if session.notice}<dt>Last sync notes</dt><dd data-testid="sync-notes">{session.notice}</dd>{/if}
    </dl>
  </div>
  {#each KINDS as kind}
    {@const more = session.databaseUnresolved[kind]}
    {@const added = session.databaseAdded[kind].filter(e => !more.some(m => m.id === e.id))}
    {#if more.length || added.length}
      {#if more.length}<p>The database has {more.length} {kind} this copy does not.</p>{/if}
      {#if added.length}<p>{added.length} {kind} added from the database.</p>{/if}
      <p class="faint small">{[...more, ...added].map(e => name(e, e.id)).join(', ')}</p>
      {#if !app.readOnly}
        <!-- No bulk delete of elements the user has not looked at: they are kept and named here;
             one that should go is deleted like any other element, after it has been seen. -->
        {#if more.length}<button disabled={session.busy} onclick={() => session.keepDatabaseKind(kind)}>Keep them</button>{/if}
      {/if}
    {/if}
    {#if session.databaseContext[kind].length}
      <p>{kind} kept by the database (not part of this project's copy): {session.databaseContext[kind].join(', ')}.</p>
    {/if}
  {/each}
  {#if session.pixelsChanged}<p>Images changed; a new atlas version has not been published.</p>{/if}
  {#if session.syncDisabledReason || session.error}<p>{session.syncDisabledReason || session.error}</p>{/if}
  <SyncLinkOffer />
</section>

<style>
  section { border-top:1px solid var(--hairline); font-size:0.8rem; max-height:35vh; overflow:auto; }
  .changes { display:grid; grid-template-columns:minmax(0,1fr) minmax(0,1fr) minmax(0,1.5fr); gap:var(--space-4); }
  p { margin:var(--space-1) 0; overflow-wrap:anywhere; }
  dl { display:grid; grid-template-columns:auto minmax(0,1fr); gap:var(--space-1) var(--space-3); margin:0; }
  dt { color:var(--text-muted); }
  dd { margin:0; overflow-wrap:anywhere; }
  .small { font-size:0.8rem; }
</style>
