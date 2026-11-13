import { normalizeServer } from './laicpmsdb.ts';
export interface StandaloneLaunch { project: string; server: string; code: string; revision: number | null; spot?: string | null; grain?: string | null }
/** The host confirmation precedes even the exchange. No code or token is persisted. */
export async function exchangeLaunch(request: StandaloneLaunch, configured: string | null, ask: (message: string) => boolean | Promise<boolean>, fetcher: typeof fetch = fetch): Promise<{ server: string; token: string } | null> {
  const server = normalizeServer(request.server);
  if (!server) throw new Error('Invalid database server.');
  if (server !== normalizeServer(configured ?? '') && !(await ask(`Open project from ${new URL(server).host}? This is not your configured server.`))) return null;
  const response = await fetcher(`${server}/v1/auth/exchange`, { method: 'POST', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify({ code: request.code }) });
  if (!response.ok) throw new Error('Sign-in link expired or could not be exchanged. Open the project from UPb Lab again.');
  // accounts.api.LoginOut = { token: str, account: AccountOut }.
  const login = await response.json() as { token?: unknown };
  if (typeof login.token !== 'string' || !login.token) throw new Error('The database did not return a sign-in token.');
  return { server, token: login.token };
}
export async function openStandaloneLaunch(request: StandaloneLaunch) {
  const { current, tabs } = await import('./store.svelte');
  const { openFromServer, editCopy } = await import('./remote');
  const { listDrafts } = await import('./drafts');
  const app = tabs.list.find(t => t.project?.uuid === request.project) ?? (!current().project ? current() : tabs.add());
  const focus = async () => {
    if (!request.spot && !request.grain) return;
    const { applyShellFocus } = await import('./shellcmd');
    if (request.spot) await applyShellFocus(app, { type: 'focus-spot', project: request.project, spot: request.spot, flash: true });
    else if (request.grain) await applyShellFocus(app, { type: 'focus-grain', project: request.project, grain: request.grain, flash: true });
  };
  try {
    const accepted = await exchangeLaunch(request, app.link.server, message => confirm(message));
    // Erase our copy immediately after the exchange (also on cancellation).
    request.code = '';
    if (!accepted) return;
    tabs.select(app.id);
    app.sessionToken = accepted.token; app.sessionServer = accepted.server;
    app.serverOverride = accepted.server;
    if (app.project?.uuid === request.project && !app.readOnly && app.sync.unsynced && normalizeServer(app.remote?.server ?? app.server) === accepted.server) {
      if (app.remote) app.remote.token = accepted.token;
      app.draftHere = app.sync.draft?.base_revision ?? 'unknown';
      await app.sync.check(); await focus(); return;
    }
    const draft = listDrafts(accepted.server).find(d => d.uuid === request.project && (d.pending || d.changes === null || d.pixels_changed));
    if (!(await openFromServer(accepted.server, request.project, null, null, null, app))) return;
    if (draft) { await editCopy(app); app.draftHere = app.sync.draft?.base_revision ?? draft.base_revision ?? 'unknown'; }
    await focus();
  } catch { app.error = 'Could not open this sign-in link. Open the project from UPb Lab again.'; }
  finally { request.code = ''; }
}
