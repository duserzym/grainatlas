<script lang="ts">
  // Histogram of the base mosaic with black / white handles (drag) and the
  // current tone curve drawn on top. Opaque surface (data colours).
  let {
    bins,
    black = $bindable(0),
    white = $bindable(1),
    gamma = 1,
    onchange,
  }: { bins: Uint32Array | null; black?: number; white?: number; gamma?: number; onchange?: () => void } = $props();

  let el: SVGSVGElement;
  const W = 280, H = 90;
  const path = $derived.by(() => {
    if (!bins) return '';
    const logs = Array.from(bins, (v) => Math.log1p(v));
    const max = Math.max(...logs.slice(1, 255), 1e-9);
    let d = `M0 ${H}`;
    logs.forEach((v, i) => (d += ` L${(i / 255) * W} ${H - Math.min(1, v / max) * (H - 4)}`));
    return d + ` L${W} ${H} Z`;
  });
  const curve = $derived.by(() => {
    let d = '';
    for (let i = 0; i <= 64; i++) {
      const x = i / 64;
      const t = Math.min(1, Math.max(0, (x - black) / Math.max(1e-4, white - black)));
      const y = Math.pow(t, 1 / Math.max(0.01, gamma));
      d += `${i ? 'L' : 'M'}${x * W} ${H - y * (H - 2) - 1}`;
    }
    return d;
  });

  let drag: 'black' | 'white' | null = null;
  function at(e: PointerEvent) {
    const r = el.getBoundingClientRect();
    return Math.min(1, Math.max(0, (e.clientX - r.left) / r.width));
  }
  function down(e: PointerEvent) {
    const x = at(e);
    drag = Math.abs(x - black) < Math.abs(x - white) ? 'black' : 'white';
    el.setPointerCapture(e.pointerId);
    move(e);
  }
  function move(e: PointerEvent) {
    if (!drag) return;
    const x = at(e);
    if (drag === 'black') black = Math.min(x, white - 0.01);
    else white = Math.max(x, black + 0.01);
    onchange?.();
  }
</script>

<svg bind:this={el} class="surface-opaque hist" viewBox="0 0 {W} {H}" preserveAspectRatio="none" role="slider" aria-label="Black and white levels" aria-valuenow={black} tabindex="0"
  onpointerdown={down} onpointermove={move} onpointerup={() => (drag = null)}>
  <path d={path} fill="var(--plot-axis)" opacity="0.55" />
  <path d={curve} fill="none" stroke="var(--accent)" stroke-width="1.5" vector-effect="non-scaling-stroke" />
  <rect x={0} y={0} width={black * W} height={H} fill="black" opacity="0.35" />
  <rect x={white * W} y={0} width={(1 - white) * W} height={H} fill="white" opacity="0.15" />
  <line x1={black * W} x2={black * W} y1="0" y2={H} stroke="var(--text)" stroke-width="2" vector-effect="non-scaling-stroke" />
  <line x1={white * W} x2={white * W} y1="0" y2={H} stroke="var(--text)" stroke-width="2" vector-effect="non-scaling-stroke" />
</svg>

<style>
  .hist {
    width: 100%;
    height: 90px;
    display: block;
    cursor: ew-resize;
    touch-action: none;
  }
</style>
