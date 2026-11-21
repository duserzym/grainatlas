<script lang="ts">
  import type { SpotLabelReport } from '../lib/core/types';
  import { labelKey } from '../lib/print-labels';
  let { report, paper, pixelsPerMM, onpin }: {
    report: SpotLabelReport | undefined; paper: [number, number]; pixelsPerMM: number;
    onpin: (label: SpotLabelReport['labels'][number], origin: [number, number] | null) => void;
  } = $props();
  let svg: SVGSVGElement;
  let drag = $state<{ label: SpotLabelReport['labels'][number]; start: [number, number]; origin: [number, number]; live: [number, number] } | null>(null);
  function point(e: PointerEvent): [number, number] {
    const matrix = svg.getScreenCTM();
    if (!matrix) return [0, 0];
    const p = new DOMPoint(e.clientX, e.clientY).matrixTransform(matrix.inverse());
    return [p.x, p.y];
  }
  function start(e: PointerEvent, label: SpotLabelReport['labels'][number]) {
    if (e.button !== 0 || e.ctrlKey || e.metaKey || e.altKey) return;
    e.stopPropagation(); e.preventDefault();
    const origin: [number, number] = [label.box_mm[0], label.box_mm[1]];
    drag = { label, start: point(e), origin, live: origin };
    svg.setPointerCapture(e.pointerId);
  }
  function move(e: PointerEvent) {
    if (!drag) return;
    e.stopPropagation(); const p = point(e);
    drag.live = [drag.origin[0] + p[0] - drag.start[0], drag.origin[1] + p[1] - drag.start[1]];
  }
  function end(e: PointerEvent) {
    if (!drag) return;
    e.stopPropagation(); const current = drag; drag = null;
    if (svg.hasPointerCapture(e.pointerId)) svg.releasePointerCapture(e.pointerId);
    onpin(current.label, current.live);
  }
</script>

<svg bind:this={svg} class="labels" width={paper[0] * pixelsPerMM} height={paper[1] * pixelsPerMM} viewBox="0 0 {paper[0]} {paper[1]}" data-testid="spot-label-overlay" aria-label="Move spot labels" role="group" onpointermove={move} onpointerup={end} onpointercancel={() => drag = null} onlostpointercapture={() => drag = null}>
  {#each report?.labels ?? [] as label (labelKey(label))}
    {@const origin = drag && labelKey(drag.label) === labelKey(label) ? drag.live : [label.box_mm[0], label.box_mm[1]]}
    <g role="button" tabindex="0" aria-label="Move {label.name}" aria-pressed={label.pinned} data-testid="spot-label-handle" data-pin={label.pinned} onpointerdown={e => start(e, label)} ondblclick={e => { e.stopPropagation(); onpin(label, null); }} onkeydown={e => { if (e.key === 'Delete' || e.key === 'Backspace') { e.preventDefault(); onpin(label, null); } }}>
      <title>Drag to pin. Double-click or Delete to unpin.</title>
      <rect x={origin[0]} y={origin[1]} width={label.box_mm[2] - label.box_mm[0]} height={label.box_mm[3] - label.box_mm[1]} rx="0.4" fill="white" fill-opacity="0.25" stroke="var(--accent)" stroke-width={label.pinned ? 0.45 : 0.2} />
      {#if drag && labelKey(drag.label) === labelKey(label)}<text x={origin[0] + 0.4} y={origin[1] + 2.5} font-size="2.4" fill="black">{label.name}</text>{/if}
    </g>
  {/each}
</svg>

<style>
  .labels {position:absolute;inset:0;pointer-events:none;overflow:visible;}
  g {pointer-events:all;cursor:move;touch-action:none;}
  g:focus {outline:none;}
  g:focus rect {stroke-width:0.5;}
</style>
