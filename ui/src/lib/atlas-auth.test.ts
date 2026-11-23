import { afterEach, expect, it, vi } from 'vitest';
import { authToken } from './atlas';
import { SERVER_SESSION_CHANGED } from './mount-browser';

vi.mock('@lab/bridge', async importOriginal => ({
  ...await importOriginal<typeof import('@lab/bridge')>(),
  isHosted: () => false,
  token: async () => null,
}));
vi.mock('./store.svelte', () => ({ current: () => undefined }));

const server = 'https://handoff.example.test';
function tab(sessionToken: string | null, configured = '') {
  return { server, sessionServer: sessionToken ? server : null, sessionToken, link: { token: configured } } as NonNullable<Parameters<typeof authToken>[0]>;
}

afterEach(() => vi.unstubAllGlobals());

it('keeps separate project handoffs from triggering window-wide reconnects', async () => {
  const window = new EventTarget();
  vi.stubGlobal('window', window);
  let reconnects = 0;
  window.addEventListener(SERVER_SESSION_CHANGED, () => { reconnects++; });

  const first = tab('first-handoff');
  const second = tab('second-handoff');
  const standalone = tab(null, 'configured-token');
  expect(await authToken(standalone)).toBe('configured-token');
  for (let check = 0; check < 5; check++) {
    expect(await Promise.all([authToken(first), authToken(second)])).toEqual(['first-handoff', 'second-handoff']);
    expect(await authToken(standalone)).toBe('configured-token');
  }
  expect(reconnects).toBe(0);

  standalone.link.token = '';
  expect(await authToken(standalone)).toBeNull();
  expect(reconnects).toBe(1);
});
