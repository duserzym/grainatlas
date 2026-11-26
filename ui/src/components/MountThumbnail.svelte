<script lang="ts">
  import { onMount } from 'svelte';
  import { readResponse } from '../lib/mount-browser';
  let { url, cached, fallback, server, token, offline, onsave, compact = false }: { url: string | null; cached?: string; fallback?: string; server: string; token: string; offline: boolean; onsave: (data: string) => void; compact?: boolean } = $props();
  let element: HTMLDivElement;
  let data = $state('');
  onMount(() => {
    let gone = false;
    const observer = new IntersectionObserver(entries => {
      if (!entries.some(e => e.isIntersecting) || !url || offline || cached) return;
      observer.disconnect();
      void (async () => {
        try {
          const source = new URL(url, server);
          if (source.origin !== new URL(server).origin) return;
          const response = await readResponse(source.toString(), { headers: { Authorization: `Bearer ${token}` } });
          if (!response.ok) return;
          const blob = await response.blob();
          if (blob.size > 1024 * 1024 || !/^image\/(webp|png|jpeg)$/.test(blob.type)) return;
          const reader = new FileReader();
          reader.onload = () => { if (!gone) { data = String(reader.result); onsave(data); } };
          reader.readAsDataURL(blob);
        } catch { /* keep the neutral placeholder */ }
      })();
    }, { rootMargin: '100px' });
    observer.observe(element);
    return () => { gone = true; observer.disconnect(); };
  });
</script>
<div bind:this={element} class="thumbnail" class:compact title={data || cached || fallback ? undefined : 'No overview'}>
  {#if data || cached || fallback}<img src={data || cached || fallback} alt="Mount overview" />{:else}<span class="faint small" title="No overview">No overview</span>{/if}
</div>
<style>
  .thumbnail { position: relative; min-width: 0; height: var(--mount-thumbnail-height, 84px); background: var(--surface-2, var(--bg)); display: grid; place-items: center; overflow: hidden; border-radius: var(--radius); }
  span { max-width: 100%; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .compact span { position: absolute; width: 1px; height: 1px; padding: 0; clip-path: inset(50%); overflow: hidden; white-space: nowrap; }
  img { width: 100%; height: 100%; object-fit: contain; }
</style>
