import { createSubscriber } from 'svelte/reactivity';
import type { OversightState } from '@lab/bridge';

/** Consent belongs to a project and signed-in session, never to a saved file. */
export class OversightSession {
  private value: OversightState | null = null;
  private grants = new Set<string>();
  private owners = new Map<string, string>();
  private pending = new Map<string, Promise<boolean>>();
  private listeners = new Set<() => void>();
  private observe = createSubscriber(update => { this.listeners.add(update); return () => { this.listeners.delete(update); }; });
  get state() { this.observe(); return this.value; }
  get readOnly() { return !!this.state?.viewAs; }
  set(value: OversightState | null) { this.value = value; for (const f of this.listeners) f(); }
  clear() { this.grants.clear(); this.owners.clear(); this.pending.clear(); for (const f of this.listeners) f(); }
  async consent(key: string, owner: string, ask: (message: string) => boolean | Promise<boolean> = message => globalThis.confirm(message)) {
    if (this.readOnly) throw new ServerReadOnlyError('Viewing as someone else is read-only.');
    if (this.grants.has(key)) return true;
    const waiting = this.pending.get(key);
    if (waiting) return waiting;
    const job = Promise.resolve(ask(`This mount is ${owner}'s. Sync on their behalf?`)).then(yes => {
      if (yes && !this.readOnly && this.pending.get(key) === job) { this.grants.add(key); this.owners.set(key, owner); for (const f of this.listeners) f(); }
      return yes && this.grants.has(key);
    }).finally(() => { if (this.pending.get(key) === job) this.pending.delete(key); });
    this.pending.set(key, job);
    return job;
  }
  allowed(key: string) { return this.grants.has(key); }
  owner(server: string | null, project: string | undefined): string | null {
    this.observe();
    if (!server || !project) return null;
    for (const [key, owner] of this.owners) { const [base, , uuid] = JSON.parse(key); if (base === server.replace(/\/+$/, '') && uuid === project) return owner; }
    return null;
  }
}
export const oversightSession = new OversightSession();
export class ServerReadOnlyError extends Error {}

/** Covers sync, link, publish and spot writes; read-only batch lookups are exempt. */
export async function serverFetch(input: string | URL, init: RequestInit = {}, project?: string,
  transport: (url: string, init: RequestInit) => Promise<Response> = (url, init) => fetch(url, init)): Promise<Response> {
  const url = new URL(input);
  const read = ['GET', 'HEAD'].includes((init.method ?? 'GET').toUpperCase()) || ((init.method ?? '').toUpperCase() === 'POST'
    && ['/v1/atlas/projects/revisions', '/v1/assets/check'].some(path => url.pathname.endsWith(path)));
  const viewAs = oversightSession.state?.viewAs;
  if (read && viewAs && !url.pathname.endsWith('/v1/accounts/me')) url.searchParams.set('as', viewAs.username);
  if (!read && (viewAs || url.searchParams.has('as'))) throw new ServerReadOnlyError('Viewing as someone else is read-only.');
  const headers = new Headers(init.headers);
  // The token stays in memory as part of this key; it is never persisted or logged.
  const key = JSON.stringify([url.href.split('/v1/')[0], headers.get('Authorization'), project]);
  if (!read && project && oversightSession.allowed(key)) headers.set('X-UPbDB-Oversight', 'on-behalf');
  const send = () => {
    if (!read && oversightSession.readOnly) throw new ServerReadOnlyError('Viewing as someone else is read-only.');
    const original = Object.fromEntries(new Headers(init.headers).entries());
    // Preserve existing header spelling for callers and diagnostics.
    const sent = init.headers && !(init.headers instanceof Headers) && !Array.isArray(init.headers) ? { ...init.headers } : original;
    if (headers.has('X-UPbDB-Oversight')) sent['X-UPbDB-Oversight'] = headers.get('X-UPbDB-Oversight')!;
    return transport(url.toString(), { ...init, headers: sent });
  };
  let response = await send();
  if (!read && project && response.status === 403) {
    const body = await response.clone().json().catch(() => null);
    if (body?.code === 'oversight_confirm') {
      const owner = typeof body.owner === 'string' ? body.owner : body.owner?.name ?? 'another researcher';
      if (await oversightSession.consent(key, owner)) {
        headers.set('X-UPbDB-Oversight', 'on-behalf');
        response = await send();
      }
    }
  }
  return response;
}
