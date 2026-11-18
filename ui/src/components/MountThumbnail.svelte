<script lang="ts">
  import { onMount } from 'svelte';
  import { readResponse } from '../lib/mount-browser';
  let { url, cached, fallback, server, token, offline, onsave }: { url: string | null; cached?: string; fallback?: string; server: string; token: string; offline: boolean; onsave: (data: string) => void } = $props();
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
<div bind:this={element} class="thumbnail">
  {#if data || cached || fallback}<img src={data || cached || fallback} alt="Mount overview" />{:else}<span class="faint small">No overview</span>{/if}
</div>
<style>
  .thumbnail { height: 84px; background: var(--surface-2, var(--bg)); display: grid; place-items: center; overflow: hidden; border-radius: var(--radius); }
  img { width: 100%; height: 100%; object-fit: contain; }
</style>
