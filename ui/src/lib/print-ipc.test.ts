import { afterEach, expect, it, vi } from 'vitest';
const invoke = vi.hoisted(() => vi.fn());
vi.mock('@tauri-apps/api/core', () => ({ invoke }));
vi.mock('@tauri-apps/api/event', () => ({ listen: async () => () => {} }));
vi.mock('./deferred-writes', () => ({ flushWrites: vi.fn() }));
import { TauriCore, savePrintPdf, setSaveTab } from './core/tauri';
import type { SheetParams } from './core/types';

afterEach(() => { vi.unstubAllGlobals(); invoke.mockReset(); });
it('transfers raw print bodies to a parser worker and reads band bytes without number arrays', async () => {
  const transfers: unknown[] = [];
  const terminate = vi.fn();
  vi.stubGlobal('Worker', class {
    onmessage!: (event: { data: unknown }) => void;
    terminate = terminate;
    postMessage(message: { id: number; body: ArrayBuffer }, transfer: unknown[]) {
      transfers.push(transfer[0]);
      queueMicrotask(() => this.onmessage({ data: { id: message.id, value: JSON.parse(new TextDecoder().decode(message.body)) } }));
    }
  });
  const body = new TextEncoder().encode('{"pages":["<svg/>"]}').buffer;
  invoke.mockImplementation(async (command: string) => command === 'print_band_image' ? new Uint8Array([137, 80, 78, 71]).buffer : body);
  const core = new TauriCore(7);
  expect(await core.printSheets({} as SheetParams, {}, {})).toEqual({ pages: ['<svg/>'] });
  expect(transfers).toEqual([body]);
  expect(invoke).toHaveBeenCalledWith('print_sheets', expect.objectContaining({ tab: 7 }));
  expect(await core.printBandImage('a', [0, 0, 1, 1], {})).toEqual(new Uint8Array([137, 80, 78, 71]));
  await core.dispose(); expect(terminate).toHaveBeenCalledTimes(1);
  await expect(core.printSheets({} as SheetParams, {}, {})).rejects.toThrow('Project closed');
});

it('saves PDF and registration with one native invocation and no second dialog command', async () => {
  invoke.mockResolvedValue('/chosen/Mount.pdf'); setSaveTab(7);
  const registration = JSON.stringify({ spots: [{ id: 'registered', label: 'µ' }] });
  const pdf = new TextEncoder().encode('%PDF-1.7');
  expect(await savePrintPdf('Mount.pdf',pdf,registration)).toBe('/chosen/Mount.pdf');
  expect(invoke).toHaveBeenCalledTimes(1);
  const [command,body,options] = invoke.mock.calls[0];
  expect(command).toBe('save_print_pdf');
  const n = new DataView(body.buffer).getUint32(0,true);
  expect(new TextDecoder().decode(body.subarray(4,4+n))).toBe(registration);
  expect(body.subarray(4+n)).toEqual(pdf);
  expect(options.headers).toEqual({ 'x-name':'Mount.pdf','x-tab':'7' });
});
