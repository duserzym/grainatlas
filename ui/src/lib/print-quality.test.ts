import { expect, it } from 'vitest';
import { PrintQuality, printPdfName } from './print-quality';
import type { Core } from './core/core';
it('pauses at a page boundary, resumes without repeated enhancement, and writes one PDF', async () => {
  const calls: string[] = [];
  let quality: PrintQuality;
  const core = { printTask: async (_id: number, action: string, page = 0) => {
    calls.push(`${action}:${page}`);
    if (action === 'quality' && page === 0) quality.stop();
    return action === 'pdf_finish' ? new Uint8Array([37,80,68,70]) : new TextEncoder().encode('{"resident_bytes":1234}');
  } } as Core;
  quality = new PrintQuality(core, 1, 3);
  await quality.start(); expect(quality.next).toBe(1); expect(quality.ready).toBe(false);
  expect(quality.residentBytes).toBe(1234);
  const bytes = await quality.pdf(() => {}, () => false);
  expect(quality.ready).toBe(true); expect(bytes).toEqual(new Uint8Array([37,80,68,70]));
  expect(calls).toEqual(['quality:0','quality:1','quality:2','summary:0','pdf_begin:0','pdf_prepare:0','pdf_prepare:1','pdf_prepare:2','pdf_page:0','pdf_page:1','pdf_page:2','pdf_finish:0']);
});
it('stops PDF work on cancellation and surfaces failures', async () => {
  let cancelled = false; const calls: string[] = [];
  const quality = new PrintQuality({ printTask: async (_id: number, action: string) => {
    calls.push(action); if (action === 'pdf_page') cancelled = true; return new Uint8Array();
  } } as Core, 1, 3);
  quality.ready = true; quality.next = 3;
  await expect(quality.pdf(() => {}, () => cancelled)).rejects.toThrow('cancelled');
  expect(calls).toEqual(['pdf_begin','pdf_prepare','pdf_prepare','pdf_prepare','pdf_page','pdf_abort']);
  const failed = new PrintQuality({ printTask: async () => { throw new Error('replaced'); } } as unknown as Core, 9, 1);
  await expect(failed.start()).rejects.toThrow('replaced'); expect(failed.error?.message).toBe('replaced');
});
it('uses one dated automatic PDF name that is safe on Windows and macOS', () => {
  const date = new Date('2026-11-18T12:00:00Z');
  expect(printPdfName('Mount A', date)).toBe('Mount A_print_2026-11-18.pdf');
  expect(printPdfName('a<>:"/\\|?* . ', date)).toBe('a__________print_2026-11-18.pdf');
  expect(printPdfName('CON', date)).toBe('_CON_print_2026-11-18.pdf');
  expect(printPdfName('CON.mount', date)).toBe('_CON.mount_print_2026-11-18.pdf');
  expect(printPdfName('...', date)).toBe('grainatlas_print_2026-11-18.pdf');
});
it('eviction releases its core job after the in-flight page without scheduling another', async () => {
  const calls: string[] = [];
  let finish!: () => void;
  let entered!: () => void;
  const started = new Promise<void>(resolve => { entered = resolve; });
  const core = { printTask: async (_id: number, action: string) => {
    calls.push(action);
    if (action === 'quality') { entered(); await new Promise<void>(resolve => { finish = resolve; }); }
    return new TextEncoder().encode('{}');
  } } as Core;
  const quality = new PrintQuality(core, 1, 3);
  const pending = quality.start();
  await started; quality.release(); finish(); await pending;
  await new Promise(resolve => setTimeout(resolve, 0));
  expect(calls).toEqual(['quality', 'cancel']);
  expect(quality.next).toBe(1);
});

it('shows each PDF phase and releases partial assembly after an error', async () => {
  const phases: string[] = [], calls: string[] = [];
  const quality = new PrintQuality({ printTask: async (_job: number, action: string) => {
    calls.push(action);
    if (action === 'pdf_finish') throw new Error('PDF failed');
    return new Uint8Array();
  } } as Core,1,1);
  quality.ready = true; quality.next = 1;
  await expect(quality.pdf(p => phases.push(p.stage), () => false)).rejects.toThrow('PDF failed');
  expect(phases).toContain('Starting PDF'); expect(phases).toContain('Preparing PDF fonts: page 1 of 1');
  expect(phases).toContain('Writing PDF: page 1 of 1'); expect(phases.at(-1)).toBe('Finishing PDF');
  expect(calls.at(-1)).toBe('pdf_abort');
});
