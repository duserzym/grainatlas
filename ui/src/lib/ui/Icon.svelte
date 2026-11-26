<!-- Ported from upblab ui/src/components/ui/Icon.svelte at 72e91bc; keep in step. -->
<!-- Local changes: canvas action, zoom and menu symbols. -->
<!-- Usage: <Icon kind="refresh" />; decorative, inherits the control's colour. -->
<script module lang="ts">
  export type IconName = 'trash' | 'undo' | 'redo' | 'more' | 'minus' | 'plus' | 'fit' | 'rotate-left' | 'rotate-right' | 'refresh' | 'edit' | 'history' | 'person' | 'flask' | 'building' | 'info' | 'check' | 'chevron-left' | 'calendar' | 'rows' | 'lanes';
</script>
<script lang="ts">
  let { kind }: { kind: IconName } = $props();
</script>
<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
  {#if kind === 'refresh'}<path d="M20 7v5h-5M4 17v-5h5M5.5 7a7 7 0 0 1 12-2L20 8M4 16l2.5 3a7 7 0 0 0 12-2" />
  {:else if kind === 'edit'}<path d="m4 16 12-12 4 4L8 20H4ZM14 6l4 4" />
  {:else if kind === 'history'}<circle cx="12" cy="12" r="9" /><path d="M12 7v5l3 2" />
  {:else if kind === 'person'}<circle cx="12" cy="7" r="4" /><path d="M4 21v-2a8 8 0 0 1 16 0v2" />
  {:else if kind === 'flask'}<path d="M9 3h6M10 3v7L4 19q-1 2 2 2h12q3 0 2-2l-6-9V3M7 15h10" />
  {:else if kind === 'building'}<path d="M3 21h18M5 21V7l7-4 7 4v14M9 8h1m4 0h1M9 12h1m4 0h1M10 21v-5h4v5" />
  {:else if kind === 'info'}<circle cx="12" cy="12" r="9" /><path d="M12 11v6M12 7h.01" />
  {:else if kind === 'check'}<path d="m5 12 4 4L19 6" />
  {:else if kind === 'chevron-left'}<path d="m15 5-7 7 7 7" />
  {:else if kind === 'calendar'}<rect x="3" y="5" width="18" height="16" rx="2" /><path d="M7 3v4m10-4v4M3 11h18M7 15h2m4 0h2M7 18h2" />
  {:else if kind === 'rows'}<path d="M8 5h13M8 12h13M8 19h13M3 5h.01M3 12h.01M3 19h.01" />
  {:else if kind === 'trash'}<path d="M4 6h16M9 6V3h6v3M6 6l1 15h10l1-15M10 10v7m4-7v7" />
  {:else if kind === 'undo'}<path d="M4 10h10a6 6 0 0 1 0 12M4 10l5-5M4 10l5 5" />
  {:else if kind === 'redo'}<path d="M20 10H10a6 6 0 0 0 0 12M20 10l-5-5M20 10l-5 5" />
  {:else if kind === 'more'}<path d="M5 12h.01M12 12h.01M19 12h.01" />
  {:else if kind === 'minus'}<path d="M5 12h14" />
  {:else if kind === 'plus'}<path d="M5 12h14M12 5v14" />
  {:else if kind === 'fit'}<path d="M9 3H3v6m12-6h6v6M3 15v6h6m12-6v6h-6" />
  {:else if kind === 'rotate-left'}<path d="M3 4v6h6M3 10a9 9 0 1 1 1 8" />
  {:else if kind === 'rotate-right'}<path d="M21 4v6h-6M21 10a9 9 0 1 0-1 8" />
  {:else}<rect x="3" y="4" width="18" height="16" rx="2" /><path d="M9 4v16M15 4v16" />{/if}
</svg>
<style>svg { width: 18px; height: 18px; flex: none; display: block; }</style>
