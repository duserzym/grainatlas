<script lang="ts">
  import { onDestroy, untrack } from 'svelte';
  import FloatingPanel from './FloatingPanel.svelte';
  import { app } from '../lib/store.svelte';
  import { FRACTION_NAME, MINERALS, lastCapsuleSet, rememberCapsuleSet, orderedBatches, uniqueGrains, sampleConflict, type FractionFields, type FractionOut, type Mineral, type SampleRef, type SourceInput } from '../lib/tims';
  import type { OutboxEntry } from '../lib/tims-outbox';

  export interface PluckGrain { grain: string; label: string; spot?: string | null; spotName?: string; sample: SampleRef }
  let { grains = $bindable(), fraction = null, failed = null, onclose }: { grains: PluckGrain[]; fraction?: FractionOut | null; failed?: OutboxEntry | null; onclose: () => void } = $props();
  const store = $derived(app.tims.store);
  const revision = $derived(app.tims.version);
  const data = $derived.by(() => { revision; return store?.data; });
  const syncError = $derived.by(() => { revision; return store?.error ?? ''; });
  const batches = $derived.by(() => { revision; return store?.batches ?? []; });
  let name = $state(untrack(() => fraction?.name ?? ''));
  let mineral = $state<Mineral>(untrack(() => fraction?.mineral ?? app.tims.minerals[0] ?? 'zircon'));
  let notes = $state(untrack(() => fraction?.notes ?? ''));
  let setId = $state(untrack(() => store?.batches.find(b => b.id === fraction?.batch_id)?.capsule_set_id ?? (lastCapsuleSet() || store?.data.lastSet || '')));
  let batchId = $state(untrack(() => fraction?.batch_id ?? orderedBatches(store?.batches ?? [], setId)[0]?.id ?? (setId ? 'new' : '')));
  let capsule = $state<number | null>(untrack(() => fraction?.capsule_number ?? (batchId ? store?.nextCapsule(batchId, fraction?.id) : null) ?? null));
  let fragment = $state(untrack(() => failed?.op === 'source.add' ? failed.body.is_fragment === true : failed?.op === 'fraction.create' ? (failed.body.sources as SourceInput[]).some(s => s.is_fragment) : false));
  let fragmentNote = $state(untrack(() => failed?.op === 'source.add' ? String(failed.body.fragment_note ?? '') : failed?.op === 'fraction.create' ? (failed.body.sources as SourceInput[]).find(s => s.is_fragment)?.fragment_note ?? '' : ''));
  const today = new Date();
  let started = $state(untrack(() => failed?.op === 'batch.create' ? String(failed.body.started_on) : `${today.getFullYear()}-${String(today.getMonth() + 1).padStart(2, '0')}-${String(today.getDate()).padStart(2, '0')}`));
  let batchNotes = $state(untrack(() => failed?.op === 'batch.create' ? String(failed.body.notes ?? '') : ''));
  let sample = $state<SampleRef>(untrack(() => fraction?.sample ?? grains[0]?.sample ?? { id: '', name: '' }));
  let checked = $state(true);
  let busy = $state(false);
  let error = $state('');
  let saved = $state(false);
  let savedId = $state<string | null>(null);
  const currentFraction = $derived.by(() => { revision; const id = savedId; return id ? store?.fractions.find(f => f.id === store.resolve(id)) ?? fraction : fraction; });
  const edit = $derived(!!currentFraction && (!fragment || !!savedId));
  const set = $derived(data?.sets.find(s => s.id === setId));
  const choices = $derived(orderedBatches(batches, setId));
  const severalLabs = $derived(app.tims.labs.length > 1 || new Set(data?.sets.map(s => s.lab_id)).size > 1);
  const selectedGrains = $derived(uniqueGrains(grains));
  const conflict = $derived(sampleConflict(grains.map(g => ({ sample: data?.samples[g.grain] ?? g.sample }))));
  const occupied = $derived.by(() => { revision; return store?.occupancy(batchId, savedId ? store.resolve(savedId) : edit ? fraction?.id : undefined) ?? {}; });
  const fractionNames = $derived.by(() => { revision; return new Map([...(store?.fractions ?? []), ...Object.values(data?.batchFractions ?? {}).flat()].map(f => [f.id, f.name])); });
  const reason = $derived(!grains.length ? 'Choose at least one grain' : conflict ? conflict : fragment && !fragmentNote.trim() ? 'Add a fragment note' : !FRACTION_NAME.test(name) ? 'Use a name such as z1 or a2' : batchId === 'new' && !started ? 'Choose a start date' : batchId && !capsule ? 'Choose a free capsule' : batchId && batchId !== 'new' && capsule && occupied[String(capsule)] ? 'Choose a free capsule' : '');
  const liveErrors = $derived.by(() => {
    revision; const id = savedId ? store?.resolve(savedId) : fraction?.id;
    return data?.outbox.filter(e => e.state === 'failed' && (e.target === id || e.target === batchId)) ?? [];
  });
  const batchFailure = $derived(liveErrors.find(e => e.op === 'batch.create') ?? (failed?.op === 'batch.create' && failed.state === 'failed' ? failed : null));
  $effect(() => {
    revision;
    if (store && batchId && batchId !== 'new') { const real = store.resolve(batchId); if (real !== batchId) batchId = real; }
  });
  let loaded = false;
  let alive = true, choiceSequence = 0;
  onDestroy(() => { alive = false; suggestionSequence++; });
  $effect(() => {
    if (!store || loaded) return;
    loaded = true;
    if (!setId) setId = lastCapsuleSet() || store.data.lastSet || store.data.sets[0]?.id || '';
    if (!batchId && setId) batchId = orderedBatches(store.batches, setId)[0]?.id ?? 'new';
    if (!capsule && batchId) capsule = batchId === 'new' ? 1 : store.nextCapsule(batchId, fraction?.id);
    const choice = choiceSequence;
    void (async () => {
      await store.loadSets();
      if (!alive || choice !== choiceSequence) return;
      if (fraction?.batch_id) await store.loadBatch(fraction.batch_id);
      if (!alive || choice !== choiceSequence) return;
      const existingSet = store.batches.find(b => b.id === fraction?.batch_id)?.capsule_set_id;
      setId = existingSet ?? (lastCapsuleSet() || store.data.lastSet || '');
      if (!store.data.sets.some(s => s.id === setId)) setId = '';
      if (!setId && store.data.sets.length) setId = store.data.sets[0].id;
      if (setId) {
        await store.loadBatches(setId);
        if (!alive || choice !== choiceSequence) return;
        if (!fraction?.batch_id) batchId = orderedBatches(store.batches, setId)[0]?.id ?? 'new';
        await store.loadBatch(batchId);
        if (!alive || choice !== choiceSequence) return;
        if (!capsule && batchId) capsule = batchId === 'new' ? 1 : store.nextCapsule(batchId, fraction?.id);
      }
    })();
  });
  let previousFragment = untrack(() => fragment);
  let fragmentBase = untrack(() => fraction);
  $effect(() => {
    const value = fragment, current = store;
    if (current && value !== previousFragment) untrack(() => {
      const existing = value ? currentFraction : fragmentBase;
      if (!existing) return;
      if (value) { fragmentBase = existing; savedId = null; manualName = false; capsule = batchId ? current.nextCapsule(batchId) : null; }
      else { savedId = existing.id; name = existing.name; batchId = existing.batch_id ?? ''; capsule = existing.capsule_number; }
      saved = false;
    });
    previousFragment = value;
  });
  let suggestionSequence = 0;
  let manualName = false;
  $effect(() => {
    const first = grains[0], m = mineral, isEdit = edit;
    const sequence = ++suggestionSequence;
    if (!store || !first || isEdit) return;
    const cached = store.localName(first.grain, first.sample, m);
    if (!manualName) name = cached.name;
    sample = cached.sample; checked = false;
    void store.suggestion(first.grain, first.sample, m).then(result => {
      if (!alive || sequence !== suggestionSequence) return;
      if (!manualName) name = result.name;
      sample = result.sample; checked = result.checked;
    });
  });
  async function changeSet(id: string) {
    const choice = ++choiceSequence;
    setId = id; batchId = ''; capsule = null;
    if (!store) return;
    try {
      await store.loadBatches(id);
      if (!alive || choice !== choiceSequence) return;
      batchId = orderedBatches(store.batches, id)[0]?.id ?? (id ? 'new' : '');
      await store.loadBatch(batchId);
      if (!alive || choice !== choiceSequence) return;
      capsule = batchId === 'new' ? 1 : batchId ? store.nextCapsule(batchId, edit ? fraction?.id : undefined) : null;
      rememberCapsuleSet(id); await store.remember(id, batchId);
    } catch (e) { error = e instanceof Error ? e.message : String(e); }
  }
  async function changeBatch(id: string) {
    const choice = ++choiceSequence;
    batchId = id; if (!store) return;
    try {
      await store.loadBatch(id === 'new' ? '' : id);
      if (!alive || choice !== choiceSequence) return;
      capsule = id === 'new' ? 1 : store.nextCapsule(id, edit ? fraction?.id : undefined);
      await store.remember(setId, id === 'new' ? '' : id);
    } catch (e) { error = e instanceof Error ? e.message : String(e); }
  }
  function source(g: PluckGrain): SourceInput { return { atlas_grain_id: g.grain, ...(g.spot ? { atlas_spot_id: g.spot } : {}), is_fragment: fragment, fragment_note: fragment ? fragmentNote.trim() : '' }; }
  async function save() {
    if (!store || reason || app.readOnly) return;
    busy = true; error = ''; saved = false;
    try {
      let batch = batchId;
      if (batch === 'new') batch = await store.newBatch({ capsule_set_id: setId, started_on: started, notes: batchNotes }, mineral);
      const fields: FractionFields = { name: name.toLowerCase(), mineral, notes, batch_id: batch || null, capsule_number: batch ? capsule : null };
      const refused = liveErrors[0] ?? (failed?.state === 'failed' ? failed : null);
      if (refused) {
        if (refused.op === 'batch.create') await store.editFailed(refused.id, { capsule_set_id: setId, started_on: started, notes: batchNotes });
        else if (refused.op === 'fraction.retire' || refused.op === 'source.retire') await store.editFailed(refused.id, {});
        else if (refused.op === 'source.add') await store.editFailed(refused.id, { is_fragment: fragment, fragment_note: fragmentNote.trim() });
        else await store.editFailed(refused.id, { ...fields, ...(refused.op === 'fraction.create' ? { sources: selectedGrains.map(source) } : {}) });
        savedId = refused.op === 'batch.create' ? savedId : refused.target;
      } else if (edit && currentFraction) {
        savedId = currentFraction.id;
        await store.patchFraction(currentFraction.id, fields);
        const old = currentFraction.sources;
        for (const s of old) if (!grains.some(g => g.grain === s.atlas_grain_id && (g.spot ?? null) === (s.atlas_spot_id ?? null))) await store.retireSource(currentFraction.id, s.id);
        for (const g of selectedGrains) if (!old.some(s => s.atlas_grain_id === g.grain)) await store.addSource(currentFraction.id, source(g));
      } else savedId = await store.createFraction({ ...fields, sources: selectedGrains.map(source) }, sample);
      batchId = batch;
      rememberCapsuleSet(setId); await store.remember(setId, batch);
      saved = true;
      // Keep the panel open so a refusal stays next to the editable fields.
      await store.flush();
    } catch (e) { error = e instanceof Error ? e.message : String(e); }
    finally { busy = false; }
  }
  function removeGrain(index: number) { grains = grains.filter((_, i) => i !== index); saved = false; }
</script>

<svelte:window onfocus={() => { if (store && batchId && batchId !== 'new') void store.loadBatch(batchId); }} />

<FloatingPanel title={edit ? 'Edit TIMS pluck' : 'Pluck for TIMS'} x={340} y={92} {onclose}>
  <div data-testid="pluck-panel" oninput={() => { saved = false; }} onchange={() => { saved = false; }}>
    <label class="field">Capsule set
      <select aria-label="Capsule set" value={setId} onchange={e => { void changeSet(e.currentTarget.value); }}><option value="">No capsule set</option>{#each data?.sets ?? [] as s (s.id)}<option value={s.id}>{s.name} ({s.code}){severalLabs ? ` · ${app.tims.labs.find(l => l.id === s.lab_id)?.handle ?? s.lab_name ?? s.lab_id}` : ''}</option>{/each}</select>
    </label>
    <label class="field">Batch
      <select aria-label="Batch" value={batchId} onchange={e => { void changeBatch(e.currentTarget.value); }}>
        {#if set}<option value="new">New batch (started {started})</option>{/if}
        <option value="">No batch</option>
        {#if batchFailure}<option value={batchId}>Batch needs attention</option>{/if}
        {#each choices as b (b.id)}<option value={b.id}>{set?.code} batch started {b.started_on}{b.operator_name ? ` · ${b.operator_name}` : ''}</option>{/each}
      </select>
    </label>
    {#if choices.some(b => !data?.details[b.id] && !b.id.startsWith('local:'))}<p class="faint small">Capsule use is checked when you choose a batch.</p>{/if}
    {#if batchId === 'new' || batchFailure}
      <label class="field">Start date <input type="date" bind:value={started} /></label>
      <label class="field">Batch notes <textarea rows="2" bind:value={batchNotes}></textarea></label>
    {/if}
    {#if batchId}
      <label class="field">Capsule number
        <select aria-label="Capsule number" bind:value={capsule}><option value={null}>Choose capsule</option>{#each Array.from({ length: set?.positions ?? 15 }, (_, i) => i + 1) as n (n)}<option value={n} disabled={batchId !== 'new' && !!occupied[String(n)]}>{set?.capsule_labels[n - 1] ?? n}{occupied[String(n)] ? ` · ${fractionNames.get(occupied[String(n)]!) ?? 'Taken'}` : ''}</option>{/each}</select>
      </label>
    {/if}
    {#if batchId}<p class="faint small">{capsule ? `${set?.code ?? ''}${capsule}` : 'Choose a capsule'}{Object.entries(occupied).some(([, id]) => id) ? ` · taken: ${Object.entries(occupied).filter(([, id]) => id).map(([n]) => Number(n)).sort((a, b) => a - b).join(', ')}` : ''}</p>{/if}
    <label class="field">Fraction name <input value={name} oninput={e => { manualName = true; name = e.currentTarget.value.toLowerCase(); saved = false; }} pattern={FRACTION_NAME.source} /></label>
    {#if !checked && !edit}<p class="faint small">Checked when online</p>{/if}
    <label class="field">Mineral <select aria-label="Mineral" bind:value={mineral}>{#each MINERALS as m (m)}<option value={m}>{m}</option>{/each}</select></label>
    <label class="check"><input type="checkbox" bind:checked={fragment} /> Fragment</label>
    {#if fragment}<label class="field">Fragment note <textarea rows="2" bind:value={fragmentNote}></textarea></label>{/if}
    <label class="field">Notes <textarea rows="2" bind:value={notes}></textarea></label>
    <strong>Grains</strong>
    <ul>{#each grains as g, i (`${g.grain}:${g.spot ?? ''}`)}<li>{g.label}{g.spotName ? ` · ${g.spotName}` : ''} <button class="icon" aria-label={`Remove ${g.label}`} onclick={() => removeGrain(i)}>Remove</button></li>{/each}</ul>
    <p class="faint small">Several grains can join one fraction. Shift-click to add grains.</p>
    {#if reason}<p class="faint small" data-testid="pluck-reason">{reason}</p>{/if}
    {#if error}<p class="danger" role="alert">{error}</p>{/if}
    {#each liveErrors as entry (entry.id)}<p class="danger" role="alert">{entry.error}</p><div class="row"><button onclick={() => { void save(); }}>Edit</button><button onclick={() => { if (store) void store.discard(entry.id).then(() => { saved = false; }); }}>Discard</button></div>{/each}
    {#if saved && !liveErrors.length}<p class="faint small" role="status">{store?.offline || data?.outbox.length ? 'Saved on this computer; sends when online' : 'Pluck saved'}</p>{/if}
    {#if app.tims.error || syncError}<p class="faint small">{app.tims.error || syncError}</p>{/if}
    <div class="row"><button class="primary" disabled={!!reason || busy || !store || app.readOnly || (saved && !liveErrors.length)} onclick={() => { void save(); }}>{busy ? 'Saving…' : edit ? 'Save' : 'Pluck'}</button><button onclick={onclose}>Cancel</button></div>
  </div>
</FloatingPanel>

<style>
  [data-testid=pluck-panel] {display:grid;gap:var(--space-2);min-width:0;}
  ul {margin:0;padding-left:1rem;}
  li {overflow-wrap:anywhere;}
  [role=alert] {color:var(--danger);margin:0;}
</style>
