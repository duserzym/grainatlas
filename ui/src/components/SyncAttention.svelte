<script lang="ts">
  import type { Snippet } from 'svelte';
  let { title, message, spots, actions }: { title: string; message: string; spots: string; actions?: Snippet } = $props();
</script>
<h2>{title}</h2>
<p>{message}</p>
<p>What happens to your spots: {spots}</p>
{#if actions}<div class="actions">{@render actions()}</div>{/if}
<style>
  h2 { margin-top:0; }
  .actions { display:flex; flex-wrap:wrap; gap:var(--space-2); margin-top:var(--space-3); }
</style>
