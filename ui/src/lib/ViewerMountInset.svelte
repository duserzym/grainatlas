<script lang="ts">
  // All exploration state is ephemeral and waits for the destination area's own view.
  import { isHosted } from '@lab/bridge';
  import { untrack, type ComponentProps } from 'svelte';
  import MountInset from '../components/MountInset.svelte';
  import { app } from './store.svelte';
  import { presentation } from './presentation';
  import { label, type Box, type GroupRegion } from './mount-inset';
  import { mountName } from './mount-inset-header';
  import { sendViewerRegion } from './viewer-region';
  import { regionBounds, regionFitRect, regionCamera } from './viewer-region-fit';

  type Camera = { fitAll: () => void; centerOn: (x: number, y: number, zoom?: number) => void };
  let { viewer, ...props }: ComponentProps<typeof MountInset> & { viewer: Camera | undefined } = $props();
  const mountLabel = $derived(mountName(isHosted() ? app.sync.status?.mount_label : null, app.remote?.atlas.name, app.project?.name));
  let pending = $state<{ project: string; area: string; rect: Box | null } | null>(null);

  function explore(kind: 'mount' | 'area' | 'sample', area: string | null = null, group?: GroupRegion) {
    if (presentation.current !== 'viewer') return;
    const project = app.project;
    if (!project) return;
    const record = project.areas.find(a => a.id === area);
    if (kind !== 'mount' && !record) return;
    const spots = kind === 'sample' ? group!.spots : kind === 'area' ? app.spots.filter(s => s.area_id === area).map(s => s.id) : [];
    sendViewerRegion({ kind, project: project.uuid, mount: app.sync.status?.mount_id ?? app.mountId,
      mountLabel, area, areaLabel: record ? label(record) : null, sample: group?.name ?? null, spots });
    if (kind === 'mount') return;
    const bounds = group ? regionBounds(app.spots, spots, area!) : null;
    pending = { project: project.uuid, area: area!, rect: bounds ? regionFitRect(bounds) : null };
    // Supersede a still-pending shell focus so it cannot move the camera after this click.
    app.focusSpot = null; app.focusGrain = null;
    props.onarea(area!);
  }

  $effect(() => {
    const target = pending;
    if (!target) return;
    if (presentation.current !== 'viewer' || app.project?.uuid !== target.project || app.areaId !== target.area) {
      pending = null; return;
    }
    const view = props.view, mosaic = props.mosaics[target.area], camera = viewer;
    if (props.current !== target.area || !view || view.w <= 0 || view.h <= 0 || !mosaic || !camera) return;
    pending = null;
    untrack(() => {
      if (!target.rect) camera.fitAll();
      else {
        const next = regionCamera(target.rect, view, mosaic.um_per_px);
        camera.centerOn(next.x, next.y, next.zoom);
      }
    });
  });
</script>

<MountInset {...props} {mountLabel} onmount={() => explore('mount')}
  onarea={id => explore('area', id)} onsample={(group, area) => explore('sample', area, group)} />
