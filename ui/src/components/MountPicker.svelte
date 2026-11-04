<script lang="ts">
  // Links the project to a laicpmsdb mount: a list from GET /v1/mounts when the server answers
  // (hosted token, pasted token, or the same-origin sign-in), else a typed mount id.
  import { app } from '../lib/store.svelte';
  import { authToken } from '../lib/atlas';
  import { listMounts, type MountOut } from '../lib/laicpmsdb';

  let { value, onchange, disabled = false }: { value: string | null; onchange: (id: string) => void; disabled?: boolean } = $props();
  let mounts = $state<MountOut[] | null>(null);
  let note = $state('');
  let typed = $state(false);

  async function load(server: string | null) {
    mounts = null;
    if (!server) {
      note = 'No laicpmsdb server set (Settings): type the mount id.';
      return;
    }
    note = 'Loading mounts…';
    try {
      mounts = await listMounts(server, await authToken());
      note = mounts.length ? '' : 'The server lists no mounts for you: type the id.';
    } catch (e) {
      note = `${e instanceof Error ? e.message : e}. Type the mount id instead.`;
    }
  }
  $effect(() => {
    void load(app.server);
  });
</script>

<div class="picker">
  {#if mounts?.length && !typed}
    <label class="field">Mount
      <select value={value ?? ''} {disabled} onchange={(e) => onchange(e.currentTarget.value)} data-testid="mount-select">
        <option value="">not linked</option>
        {#each mounts as m (m.id)}
          <option value={m.id} disabled={!m.can_edit}>{m.label} · {m.mount_type} · {m.grain_count} grains, {m.spot_count} spots{m.can_edit ? '' : ' (read-only)'}</option>
        {/each}
        {#if value && !mounts.some((m) => m.id === value)}<option value={value}>{value}</option>{/if}
      </select>
    </label>
    <button class="linkish" onclick={() => (typed = true)}>Type an id instead</button>
  {:else}
    <label class="field">Mount id
      <input type="text" placeholder="laicpmsdb mount UUID" value={value ?? ''} {disabled} onchange={(e) => onchange(e.currentTarget.value)} data-testid="mount-id" />
    </label>
    {#if mounts?.length}<button class="linkish" onclick={() => (typed = false)}>Pick from the list</button>{/if}
  {/if}
  {#if note}<p class="faint small" data-testid="mount-note">{note}</p>{/if}
</div>

<style>
  .picker {
    display: grid;
    gap: var(--space-1);
  }
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
