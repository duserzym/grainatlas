<script lang="ts">
  import type { SnapTarget } from '../spots';
  let { position, snap, diameter, k, colour, halo, contrast }: {
    position:[number,number]|null; snap:SnapTarget|null; diameter:number; k:number;
    colour:string;halo:string;contrast:string;
  }=$props();
  const centre = $derived(snap ? [snap.x, snap.y] : position);
</script>
<!-- Isolated, small SVG layer: moving this cursor never rebuilds spot/grain geometry. -->
<g data-testid="spot-preview-layer" pointer-events="none">
  {#if snap}
    <g stroke="var(--accent)" fill="none" stroke-width="1.5">
      <circle cx={snap.x} cy={snap.y} r={7*k} />
      <path d={`M ${snap.x-10*k} ${snap.y} H ${snap.x+10*k} M ${snap.x} ${snap.y-10*k} V ${snap.y+10*k}`} />
    </g>
  {/if}
  {#if centre}
    <circle cx={centre[0]} cy={centre[1]} r={diameter/2} fill="none" stroke={halo} stroke-width="3.6" stroke-dasharray="4 3" />
    <circle cx={centre[0]} cy={centre[1]} r={diameter/2} fill="none" stroke={colour} stroke-width="1.2" stroke-dasharray="4 3"
      data-testid="spot-ghost" data-contrast={contrast} />
  {/if}
</g>
