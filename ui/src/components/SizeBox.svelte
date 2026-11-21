<script lang="ts">
  import { SizeEditor } from '../lib/size-editor';
  import { parseDiameter, stepDiameter } from '../lib/spots';

  let { value, target, onchange, label, testid, placeholder = '', disabled = false }: {
    value: number | null;
    target: string;
    placeholder?: string;
    onchange: (d: number) => void;
    label: string;
    testid?: string;
    disabled?: boolean;
  } = $props();
  let text = $state('');
  const editor = new SizeEditor();
  $effect.pre(() => { const reset = editor.sync(value, target); if (reset !== null) text = reset; });

  function focus() {
    const reset = editor.sync(value, target);
    if (reset !== null) text = reset;
    editor.focus(target, onchange);
  }
  function commit() {
    const d = editor.commit(text, value, target);
    text = String(d ?? value ?? '');
  }
  function step(direction: number, e: MouseEvent | KeyboardEvent) {
    const reset = editor.sync(value, target);
    if (reset !== null) text = reset;
    // Buttons are an explicit new edit of the currently displayed target.
    editor.focus(target, onchange);
    const base = parseDiameter(text) ?? value;
    if (base === null) return;
    text = String(stepDiameter(base, direction, e.shiftKey ? 5 : e.altKey ? 0.5 : 1));
    commit();
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
      else text = String(value ?? '');
    }
  }
</script>

<div class="size-box">
  <button type="button" class="icon" tabindex="-1" {disabled} title="1 µm smaller (Shift: 5 µm)" aria-label="1 µm smaller (Shift: 5 µm)"
    onpointerdown={(e) => e.preventDefault()} onclick={(e) => step(-1, e)}>−</button>
  <input type="text" inputmode="decimal" bind:value={text} {disabled} {placeholder} aria-label={label} title={label} data-testid={testid}
    onfocus={focus} onblur={() => { commit(); editor.blur(); }} onkeydown={key} />
  <button type="button" class="icon" tabindex="-1" {disabled} title="1 µm larger (Shift: 5 µm)" aria-label="1 µm larger (Shift: 5 µm)"
    onpointerdown={(e) => e.preventDefault()} onclick={(e) => step(1, e)}>+</button>
  <span>µm</span>
</div>

<style>
  .size-box { display: inline-flex; align-items: center; gap: var(--space-1); flex: none; width: max-content; }
  .size-box input { box-sizing: content-box; width: 6.5ch; font-variant-numeric: tabular-nums; text-align: right; flex: none; }
</style>
