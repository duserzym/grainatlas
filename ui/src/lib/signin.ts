// Browser sign-in for the standalone desktop app. The Rust side
// (tauri/src-tauri/src/signin.rs) listens once on 127.0.0.1 and opens
// <server>/app/#/connect; UPbDB's page sends a single-use code back, and
// this trades it at POST /v1/auth/exchange for the app's own token.

import { normalizeServer } from './upbdb.ts';

export interface SignedIn { token: string; name: string }

type Invoke = (cmd: string, args?: Record<string, unknown>) => Promise<unknown>;

/** True in the Tauri desktop app (not in a browser, not inside UPb Lab). */
export function canSignInWithBrowser(hosted: boolean): boolean {
  return !hosted && typeof window !== 'undefined' && '__TAURI_INTERNALS__' in window;
}

function randomState(): string {
  const bytes = new Uint8Array(24);
  crypto.getRandomValues(bytes);
  return btoa(String.fromCharCode(...bytes)).replace(/\+/g, '-').replace(/\//g, '_').replace(/=+$/, '');
}

async function tauriInvoke(cmd: string, args?: Record<string, unknown>): Promise<unknown> {
  const { invoke } = await import('@tauri-apps/api/core');
  return invoke(cmd, args);
}

export async function signInWithBrowser(server: string, invoke: Invoke = tauriInvoke, fetcher: typeof fetch = fetch): Promise<SignedIn> {
  const base = normalizeServer(server);
  if (!base) throw new Error('Set the UPbDB server first.');
  const code = await invoke('sign_in_browser', { server: base, state: randomState() });
  if (typeof code !== 'string' || !code) throw new Error('The browser returned no sign-in code.');
  let response: Response;
  try {
    response = await fetcher(`${base}/v1/auth/exchange`, {
      method: 'POST', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify({ code }),
    });
  } catch {
    throw new Error(`Cannot reach ${new URL(base).host}. Check the network (and Tailscale, if the server is on it).`);
  }
  if (!response.ok) throw new Error('The sign-in code expired or was already used; try again.');
  const login = await response.json() as { token?: unknown; account?: { display_name?: string; username?: string } };
  if (typeof login.token !== 'string' || !login.token) throw new Error('The database did not return a sign-in token.');
  return { token: login.token, name: login.account?.display_name || login.account?.username || 'this account' };
}

export async function cancelSignIn(invoke: Invoke = tauriInvoke): Promise<void> {
  await invoke('sign_in_cancel');
}
