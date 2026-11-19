<script lang="ts">
  import type { AreaImageEdits } from '../lib/core/types';
  import { pointBounds, maskAppearance } from '../lib/image-edits';
  const id=$props.id();
  let { edits, width, height, handles = false, k = 1 }: { edits: AreaImageEdits; width: number; height: number; handles?: boolean; k?: number } = $props();
  const points = $derived(edits.crop?.kind === 'rect' ? (() => { const [x,y,r,b]=pointBounds(edits.crop.points_um); return [[x,y],[r,y],[r,b],[x,b]]; })() : edits.crop?.points_um ?? []);
  const path = $derived(points.length ? `M${points.map(p=>p.join(' ')).join('L')}Z` : '');
  const box = $derived(points.length ? pointBounds(points as [number,number][]) : [0,0,width,height]);
  const rotationY = $derived(box[1] < 36*k ? box[1]+30*k : box[1]-30*k);
  const vertices = $derived(edits.crop?.kind === 'rect' ? [...points,[(box[0]+box[2])/2,box[1]],[box[2],(box[1]+box[3])/2],[(box[0]+box[2])/2,box[3]],[box[0],(box[1]+box[3])/2]] : points);
  const strokes=$derived(edits.masks.filter(m=>m.points_um.length&&m.radius_um>0).map(m=>({
    d:`M${m.points_um.map(p=>p.join(' ')).join('L')}${m.points_um.length===1?'l0.001 0':''}`,radius:m.radius_um,
  })));
  const grey=`rgb(${maskAppearance.background} ${maskAppearance.background} ${maskAppearance.background})`;
  const ink=`rgb(${maskAppearance.ink} ${maskAppearance.ink} ${maskAppearance.ink})`;
  const pitch=maskAppearance.pitchUm;
</script>
{#if handles}
  {#if path}
    <path d={path} fill="none" stroke="var(--accent)" stroke-width="1.5" />
    {#each vertices as p, i}<rect data-image-handle={i} x={p[0]-4*k} y={p[1]-4*k} width={8*k} height={8*k} fill="white" stroke="var(--accent)" />{/each}
  {/if}
  <line x1={(box[0]+box[2])/2} y1={box[1]} x2={(box[0]+box[2])/2} y2={rotationY} stroke="var(--accent)" />
  <circle data-image-handle="rotate" cx={(box[0]+box[2])/2} cy={rotationY} r={6*k} fill="white" stroke="var(--accent)" />
{:else}
  {#if strokes.length}
    <defs>
      <clipPath id={`${id}-bounds`}><rect width={width} height={height} /></clipPath>
      <pattern id={`${id}-hatch`} patternUnits="userSpaceOnUse" width={pitch} height={pitch}>
        <path d={`M${-pitch} ${pitch}L${pitch} ${-pitch}M0 ${pitch}L${pitch} 0M0 ${2*pitch}L${2*pitch} 0`} fill="none" stroke={ink} stroke-width={maskAppearance.lineUm/Math.SQRT2} style="vector-effect:none" />
      </pattern>
      <mask id={`${id}-edge`} maskUnits="userSpaceOnUse" x="0" y="0" width={width} height={height} style="mask-type:luminance">
        {#each strokes as s}<path d={s.d} fill="none" stroke="white" stroke-width={2*s.radius} stroke-linecap="round" stroke-linejoin="round" style="vector-effect:none" />{/each}
        {#each strokes as s}{#if s.radius>maskAppearance.edgeUm}<path d={s.d} fill="none" stroke="black" stroke-width={2*(s.radius-maskAppearance.edgeUm)} stroke-linecap="round" stroke-linejoin="round" style="vector-effect:none" />{/if}{/each}
      </mask>
    </defs>
    <g clip-path={`url(#${id}-bounds)`} pointer-events="none">
      {#each strokes as s}<path data-testid="image-mask-stroke" d={s.d} fill="none" stroke={grey} stroke-width={2*s.radius} stroke-linecap="round" stroke-linejoin="round" style="vector-effect:none" />{/each}
      <rect data-testid="image-mask-hatch" width={width} height={height} fill={`url(#${id}-hatch)`} mask={`url(#${id}-edge)`} />
    </g>
  {/if}
  {#if path}<path data-testid="image-crop-outside" d={`M0 0H${width}V${height}H0Z${path}`} fill="black" fill-rule="evenodd" opacity="0.6" pointer-events="none" />{/if}
{/if}
