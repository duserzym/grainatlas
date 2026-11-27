<!-- Ported from upblab ui/src/components/ui/SegmentedControl.svelte at 72e91bc; keep in step. -->
<!-- Usage: <SegmentedControl {options} bind:value label="Views" />; view switches only. -->
<script lang="ts">
  import type { Snippet } from 'svelte';
  import { segmentedIndex, type SegmentedOption } from '../ui-controls';
  let { options, value = $bindable(), label, onchange, size = 'normal', disabled = false, wrap = false, testid, content }: {
    options: SegmentedOption[]; value: string; label: string; onchange?: (value: string) => void; size?: 'small' | 'normal'; disabled?: boolean; wrap?: boolean; testid?: string; content?: Snippet<[SegmentedOption]>;
  } = $props();
  let buttons: HTMLButtonElement[] = [];
  const enabled = $derived(options.map((option, index) => option.disabled ? -1 : index).filter(index => index >= 0));
  const active = $derived(enabled.find(index => options[index].value === value) ?? enabled[0]);
  function select(index: number) {
    if (disabled || options[index].disabled) return;
    // A controlled value belongs to the parent, which may reject the request.
    if (onchange) onchange(options[index].value);
    else value = options[index].value;
  }
  function keyboard(event: KeyboardEvent, index: number) {
    const next = segmentedIndex(event.key, enabled.indexOf(index), enabled.length);
    if (disabled || next === null || event.altKey || event.ctrlKey || event.metaKey) return;
    event.preventDefault(); event.stopPropagation(); select(enabled[next]); buttons[enabled[next]]?.focus();
  }
</script>
<div class="segments" class:small={size === 'small'} class:wrap data-testid={testid} role="radiogroup" aria-label={label}>
  {#each options as option, index (option.value)}
    <button type="button" value={option.value} role="radio" aria-checked={value === option.value} aria-label={option.ariaLabel ?? option.label}
      title={option.title ?? option.ariaLabel ?? option.label} tabindex={index === active ? 0 : -1}
      disabled={disabled || option.disabled} data-testid={option.testid} bind:this={buttons[index]} onclick={() => select(index)} onkeydown={event => keyboard(event, index)}>{#if content}{@render content(option)}{:else}{option.label}{/if}</button>
  {/each}
</div>
<style>
  .segments { display: inline-flex; flex: none; border: 1px solid var(--hairline); border-radius: var(--radius-sm); background: var(--glass-strong); }
  button { border: 0; border-radius: 0; padding: 5px 12px; background: transparent; cursor: pointer; white-space: nowrap; transition: none; }
  button + button { border-left: 1px solid var(--hairline); }
  button:first-child { border-radius: calc(var(--radius-sm) - 1px) 0 0 calc(var(--radius-sm) - 1px); }
  button:last-child { border-radius: 0 calc(var(--radius-sm) - 1px) calc(var(--radius-sm) - 1px) 0; }
  button[aria-checked=true] { background: var(--accent-soft); color: var(--accent); box-shadow: inset 0 0 0 1px var(--accent); }
  button:hover { background-color: var(--row-hover); }
  button:active { background-image: linear-gradient(var(--hairline), var(--hairline)); }
  button:focus-visible { position: relative; z-index: 1; outline: 2px solid var(--accent); outline-offset: 2px; }
  .small button { height: 28px; padding: 2px 8px; font-size: 12px; }
  .wrap { flex-wrap: wrap; min-width: 0; max-width: 100%; }
  .wrap button { min-width: 0; max-width: 100%; flex: 1 1 auto; white-space: normal; }
  button:disabled { opacity: .45; cursor: default; }
</style>
