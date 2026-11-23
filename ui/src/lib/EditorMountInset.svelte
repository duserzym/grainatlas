<script lang="ts">
  import { isHosted } from '@lab/bridge';
  import type { ComponentProps } from 'svelte';
  import MountInset from '../components/MountInset.svelte';
  import { defaultEditorInset } from './mount-inset';
  import { mountName } from './mount-inset-header';
  import { app } from './store.svelte';

  let props: Pick<ComponentProps<typeof MountInset>, 'current' | 'tiles' | 'lut' | 'view' | 'frame' | 'allAreasClickable' | 'currentPictureOnly' | 'onarea'> = $props();
  const mountLabel = $derived(mountName(isHosted() ? app.sync.status?.mount_label : null, app.remote?.atlas.name, app.project?.name));
  const mosaics = $derived(Object.fromEntries(Object.entries(app.stitched).flatMap(([id, s]) => s?.mosaic ? [[id, s.mosaic]] : [])));
  const selected = $derived.by(() => {
    const ids = new Set(app.selection);
    return ids.size ? app.spots.filter(s => ids.has(s.id)) : [];
  });
  // Initialise once per tab, not on resize or when switching editor screens.
  $effect(() => {
    if (!app.mountInset) app.mountInset = defaultEditorInset(window.innerWidth);
  });
</script>

{#if app.project?.areas.length}
  <MountInset {...props} areas={app.project.areas} {mosaics} {selected} {mountLabel} bind:editorInset={app.mountInset} />
{/if}
