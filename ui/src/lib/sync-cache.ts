/** Bounded in-memory ETag cache, scoped to credentials and server; never stores credentials on disk. */
const scopes = new Map<string, { credentials: string; entries: Map<string, { etag: string; body: string; headers: Headers }> }>();
export function clearSyncCache() { scopes.clear(); }
export function syncCache(server: string, token: string | null, access: string | null) {
  const credentials = JSON.stringify([token, access]);
  let scope = scopes.get(server);
  if (!scope || scope.credentials !== credentials) {
    scope = { credentials, entries: new Map() };
    scopes.set(server, scope);
  }
  return scope.entries;
}
