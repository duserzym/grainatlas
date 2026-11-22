<script module lang="ts">
  export interface WindowsPrintOptions {
    printer: string;
    copies: number;
    pageRanges: string;
  }
</script>

<script lang="ts">
  import { invoke } from '@tauri-apps/api/core';
  import { onMount } from 'svelte';

  let { paper, pages, submit, close }: {
    paper: readonly [number, number];
    pages: number;
    submit: (options: WindowsPrintOptions) => Promise<void>;
    close: () => void;
  } = $props();

  let printers = $state<string[]>([]);
  let printer = $state('');
  let copies = $state<number | undefined>(1);
  let pageRanges = $state('');
  let loading = $state(true);
  let printing = $state(false);
  let error = $state('');

  const paperSize = $derived.by(() => {
    const [width, height] = paper;
    const short = Math.min(width, height), long = Math.max(width, height);
    const inches = short === 279.4 && long === 431.8 || short === 215.9 && long === 279.4;
    return inches
      ? `${Number((width / 25.4).toFixed(2))} × ${Number((height / 25.4).toFixed(2))} in`
      : `${width} × ${height} mm`;
  });

  onMount(() => {
    invoke<{ names: string[]; defaultName: string | null }>('windows_print_printers')
      .then(({ names, defaultName }) => {
        printers = names;
        printer = defaultName && names.includes(defaultName) ? defaultName : names[0] ?? '';
      })
      .catch(cause => { error = `Could not list Windows printers. ${String(cause)}`; })
      .finally(() => { loading = false; });
  });

  function showModal(node: HTMLDialogElement) {
    node.showModal();
    return { destroy: () => node.close() };
  }

  async function print(event: SubmitEvent) {
    event.preventDefault();
    if (loading || printing || !printer || copies === undefined) return;
    printing = true;
    error = '';
    try {
      await submit({ printer, copies, pageRanges: pageRanges.trim() });
      close();
    } catch (cause) {
      error = cause instanceof Error ? cause.message : String(cause);
    } finally {
      printing = false;
    }
  }
</script>

<dialog use:showModal class="glass panel print-dialog" data-ui aria-label="Print sheets"
  oncancel={event => { event.preventDefault(); if (!printing) close(); }}>
  <form onsubmit={print}>
    <h2>Print sheets</h2>
    <p class="faint small">Paper {paperSize} · {pages} {pages === 1 ? 'page' : 'pages'}</p>
    {#if loading}
      <p role="status">Finding printers…</p>
    {:else if printers.length}
      <label class="field">Printer
        <select bind:value={printer} disabled={printing}>
          {#each printers as name (name)}<option value={name}>{name}</option>{/each}
        </select>
      </label>
      <label class="field">Copies
        <input type="number" min="1" max="999" step="1" bind:value={copies} required disabled={printing} />
      </label>
      {#if pages > 1}
        <label class="field">Pages
          <input type="text" bind:value={pageRanges} placeholder="All pages" disabled={printing} />
        </label>
        <p class="faint small">Use a range like 2-4, or leave blank for all pages.</p>
      {/if}
    {:else if !error}
      <p>No printers are installed on this computer.</p>
    {/if}
    {#if error}<p role="alert">{error}</p>{/if}
    <div class="dialog-actions">
      <button type="button" onclick={close} disabled={printing}>Cancel</button>
      <button type="submit" class="primary" disabled={loading || printing || !printer || copies === undefined}>
        {printing ? 'Sending to printer…' : 'Print'}
      </button>
    </div>
  </form>
</dialog>

<style>
  .print-dialog { width: min(28rem, calc(100% - 2rem)); max-height: calc(100% - 2rem); overflow: auto; }
  .print-dialog::backdrop { background: rgb(0 0 0 / 35%); }
  form { display: grid; gap: var(--space-3); }
  .dialog-actions { display: flex; justify-content: flex-end; gap: .5rem; }
  h2, p { margin: 0; }
  @media print { .print-dialog { display: none !important; } }
</style>
