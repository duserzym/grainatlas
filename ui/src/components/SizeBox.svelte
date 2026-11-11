<script lang="ts">
  import { parseDiameter, stepDiameter } from '../lib/spots';

  let { value, onchange, label, testid, disabled = false }: {
    value: number;
    onchange: (d: number) => void;
    label: string;
    testid?: string;
    disabled?: boolean;
  } = $props();
  let text = $state('');
  let typing = $state(false);
  $effect(() => { if (!typing) text = String(value); });

  function commit() {
    const d = parseDiameter(text);
    text = String(d ?? value);
    if (d !== null && d !== value) onchange(d);
  }
  function step(direction: number, e: MouseEvent | KeyboardEvent) {
    const d = stepDiameter(parseDiameter(text) ?? value, direction, e.shiftKey ? 5 : e.altKey ? 0.5 : 1);
    text = String(d);
    if (d !== value) onchange(d);
  }
  function key(e: KeyboardEvent) {
    if (e.key === 'ArrowUp' || e.key === 'ArrowDown') {
      e.preventDefault();
      e.stopPropagation();
      step(e.key === 'ArrowUp' ? 1 : -1, e);
    } else if (e.key === 'Enter' || e.key === 'Escape') {
      e.preventDefault();
      e.stopPropagation();
      if (e.key === 'Enter') commit();
      else text = String(value);
    }
  }
</script>

<div class="size-box">
  <button type="button" class="icon" tabindex="-1" {disabled} title="1 µm smaller (Shift: 5 µm)" aria-label="1 µm smaller (Shift: 5 µm)"
    onpointerdown={(e) => e.preventDefault()} onclick={(e) => step(-1, e)}>−</button>
  <input type="text" inputmode="decimal" bind:value={text} {disabled} aria-label={label} title={label} data-testid={testid}
    onfocus={() => (typing = true)} onblur={() => { commit(); typing = false; }} onkeydown={key} />
  <button type="button" class="icon" tabindex="-1" {disabled} title="1 µm larger (Shift: 5 µm)" aria-label="1 µm larger (Shift: 5 µm)"
    onpointerdown={(e) => e.preventDefault()} onclick={(e) => step(1, e)}>+</button>
  <span>µm</span>
</div>

<style>
  .size-box { display: inline-flex; align-items: center; gap: var(--space-1); flex: none; width: max-content; }
  .size-box input { box-sizing: content-box; width: 6.5ch; font-variant-numeric: tabular-nums; text-align: right; flex: none; }
</style>
