// Fetches the linked mount's results (GET /v1/mounts/{id}/results) on demand; memory only.

import { authToken } from './atlas';
import { getResults } from './laicpmsdb';
import { normalize, type MountResults } from './results';
import { current } from './store.svelte';

/** Fetches the linked mount's results again. Errors stay in `app.results.error` (the tab shows them). */
export async function refreshResults(): Promise<void> {
  const app = current();
  const server = app.server;
  const mount = app.mountId;
  if (!server || !mount) {
    app.results = null;
    return;
  }
  const key = `${server}|${mount}`;
  app.results = { key, data: app.results?.key === key ? app.results.data : null, at: app.results?.at ?? null, error: null, loading: true };
  try {
    const raw = await getResults(server, await authToken(), mount, app.remote?.access ?? null);
    const data: MountResults = { ...raw, spots: (raw.spots ?? []).map((e) => normalize(e as unknown as Record<string, unknown>)) };
    if (app.results?.key === key) app.results = { key, data, at: new Date().toISOString(), error: null, loading: false };
  } catch (e) {
    if (app.results?.key === key) app.results = { ...app.results, error: e instanceof Error ? e.message : String(e), loading: false };
  }
}
