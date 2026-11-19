import { test as base, expect, type ConsoleMessage, type Request, type Route } from '@playwright/test';
import { createHash } from 'node:crypto';

/** This one read-only POST is legitimate; every other endpoint stays guarded. */
export async function mockRevisionRead(route: Route, rows: { project_uuid: string; mount_id: string; revision: number }[] = []): Promise<boolean> {
  const request = route.request();
  if (request.method() !== 'POST' || new URL(request.url()).pathname !== '/v1/atlas/projects/revisions') return false;
  const { projects } = request.postDataJSON();
  expect(Array.isArray(projects)).toBe(true);
  expect(projects.length).toBeGreaterThan(0);
  expect(projects.length).toBeLessThanOrEqual(500);
  const answer = rows.filter(row => projects.includes(row.project_uuid));
  const etag = `"${createHash('sha256').update(JSON.stringify([projects, answer])).digest('hex')}"`;
  const headers = { 'access-control-allow-origin': '*', 'access-control-expose-headers': 'ETag', ETag: etag };
  if (request.headers()['if-none-match'] === etag) await route.fulfill({ status: 304, headers });
  else await route.fulfill({ json: answer, headers });
  return true;
}

/** Invented session metadata shared by the bundle-based server mocks.
 * Everything else still reaches each spec's unexpected-request guard. */
export function mockSessionRead(method: string, pathname: string): unknown | undefined {
  if (method !== 'GET') return undefined;
  if (pathname === '/v1/accounts/me') return { id: 'invented-account', username: 'casey', display_name: 'Casey Example', can_manage_people: false };
  if (pathname === '/v1/server') return { dataset_id: 'invented-dataset' };
  if (pathname === '/v1/labs') return [];
  return undefined;
}

export const test = base.extend<{ sessionRequestCounts: void; browserErrors: void }>({
  browserErrors: [async ({ page }, use, testInfo) => {
    const errors: string[] = [];
    const log = (message: string) => {
      errors.push(message);
      console.error(`[${testInfo.file.split('/').at(-1)}: ${testInfo.title}] ${message}`);
    };
    const consoleError = (message: ConsoleMessage) => {
      if (message.type() === 'error') log(`console.error: ${message.text()}\n${JSON.stringify(message.location())}`);
    };
    const pageError = (error: Error) => log(`pageerror: ${error.stack ?? error.message}`);
    page.on('console', consoleError);
    page.on('pageerror', pageError);
    try { await use(); }
    finally {
      page.off('console', consoleError);
      page.off('pageerror', pageError);
      if (errors.length) await testInfo.attach('browser-errors', { body: errors.join('\n\n'), contentType: 'text/plain' });
    }
  }, { auto: true }],
  sessionRequestCounts: [async ({ page }, use, testInfo) => {
    let document = 0;
    const counts = new Map<string, number>();
    const totals: Record<string, number> = {};
    const started = new Map<Request, number>();
    let readMs = 0;
    let revisionRequests = 0;
    let revisionMs = 0;
    const revisionStarted = new Map<Request, number>();
    page.on('request', request => {
      if (request.isNavigationRequest() && request.frame() === page.mainFrame()) document++;
      const path = new URL(request.url()).pathname;
      if (request.method() === 'POST' && path === '/v1/atlas/projects/revisions') { revisionRequests++; revisionStarted.set(request, performance.now()); }
      if (request.method() !== 'GET' || !['/v1/accounts/me', '/v1/server'].includes(path)) return;
      const key = JSON.stringify([document, new URL(request.url()).origin, request.headers().authorization, path]);
      counts.set(key, (counts.get(key) ?? 0) + 1);
      totals[path] = (totals[path] ?? 0) + 1;
      started.set(request, performance.now());
    });
    page.on('requestfinished', request => {
      const revisionStart = revisionStarted.get(request);
      if (revisionStart !== undefined) revisionMs += performance.now() - revisionStart;
      const start = started.get(request);
      if (start !== undefined) readMs += performance.now() - start;
    });
    await use();
    expect([...counts.values()].every(count => count <= 1), 'identity reads per sign-in/server/document').toBe(true);
    console.info(JSON.stringify({ file: testInfo.file.split('/').at(-1), spec: testInfo.title, identityRequests: totals, completedReadMs: Math.round(readMs), revisionRequests, revisionMs: Math.round(revisionMs) }));
  }, { auto: true }],
});
export { expect };
