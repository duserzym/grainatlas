import { afterEach, expect, it, vi } from 'vitest';
import { createPrintDocument, prefixPrintSvgIds, printDocumentCss } from './print-document';

afterEach(() => vi.unstubAllGlobals());

// Only document creation and removal are needed to exercise print lifecycle
// ordering in the Node test environment. Browser layout is covered by Playwright.
function printEnvironment() {
  const root = { innerHTML: '', prepend: vi.fn(), remove: vi.fn(), replaceChildren: vi.fn() };
  const doc = { title: 'App title', body: { append: vi.fn() },
    createElement: (tag: string) => tag === 'div' ? root : { textContent: '' } };
  const win = Object.assign(new EventTarget(), { print: vi.fn<() => void | PromiseLike<void>>() });
  vi.stubGlobal('document', doc);
  vi.stubGlobal('window', win);
  return { root, doc, win };
}

it('cleans up when synchronous printing fires afterprint during the call', async () => {
  const { root, doc, win } = printEnvironment();
  const printing = createPrintDocument([], [210, 297]);
  win.print.mockImplementation(() => {
    expect(doc.title).toBe('Print title');
    win.dispatchEvent(new Event('afterprint'));
  });
  await printing.print('Print title');
  expect(root.remove).toHaveBeenCalledOnce();
  expect(doc.title).toBe('App title');
});

it('ignores native afterprint before and after its thenable resolves until explicit cleanup', async () => {
  const { root, doc, win } = printEnvironment();
  let resolve!: () => void;
  const native = new Promise<void>(done => { resolve = done; });
  // Use a thenable as well as a promise to cover Tauri shims without relying on instanceof.
  win.print.mockReturnValue({ then: native.then.bind(native) });
  const printing = createPrintDocument([], [210, 297]);
  const pending = printing.print('Print title');
  win.dispatchEvent(new Event('afterprint'));
  expect(root.remove).not.toHaveBeenCalled();
  expect(doc.title).toBe('Print title');
  resolve();
  await pending;
  win.dispatchEvent(new Event('afterprint'));
  expect(root.remove).not.toHaveBeenCalled();
  expect(doc.title).toBe('Print title');
  printing.cleanup();
  expect(root.remove).toHaveBeenCalledOnce();
  expect(doc.title).toBe('App title');
});

it('gives repeated core mask, pattern, clip and scale-bar IDs a page namespace', () => {
  const svg = `<svg><defs><mask id="repeat0"/><pattern id="hatch0"/><clipPath id="c0"/></defs>
    <rect fill="url(#hatch0)" mask="url(#repeat0)" clip-path="url(#c0)"/>
    <path id="paper-bar"/><path id="image-bar"/></svg>`;
  for (const page of [0, 1]) {
    const result = prefixPrintSvgIds(svg, page);
    for (const id of ['repeat0', 'hatch0', 'c0', 'paper-bar', 'image-bar']) {
      expect(result).toContain(`id="ga-print-${page}-${id}"`);
    }
    for (const id of ['repeat0', 'hatch0', 'c0']) {
      expect(result).toContain(`url(#ga-print-${page}-${id})`);
    }
    expect(result).not.toContain('url(#c0)');
  }
});

it('keeps prepared pages for a retry if Windows refuses the print job', async () => {
  const { root, doc, win } = printEnvironment();
  const printing = createPrintDocument([], [431.8, 279.4]);
  const submit = vi.fn(async () => {
    expect(doc.title).toBe('Tabloid print');
    win.dispatchEvent(new Event('afterprint'));
    throw new Error('This printer does not support the selected paper size');
  });
  await expect(printing.print('Tabloid print', submit)).rejects.toThrow('does not support');
  expect(win.print).not.toHaveBeenCalled();
  expect(root.remove).not.toHaveBeenCalled();
  expect(doc.title).toBe('App title');
  await printing.print('Tabloid print', async () => {});
  printing.cleanup();
  expect(root.remove).toHaveBeenCalledOnce();
});

it('rewrites fragment hrefs and quoted URLs without changing embedded PNGs or other attributes', () => {
  const svg = `<svg><clipPath id='arrange-spots-0'/><image id="whole-mosaic" xlink:href="data:image/png;base64,AAAA"/>
    <use href='#whole-mosaic'/><use xlink:href="#whole-mosaic"/>
    <g style="clip-path:url( '#arrange-spots-0' )" data-id="whole-mosaic"/>
    <text>#whole-mosaic</text></svg>`;
  const result = prefixPrintSvgIds(svg, 2);
  expect(result).toContain("href='#ga-print-2-whole-mosaic'");
  expect(result).toContain('xlink:href="#ga-print-2-whole-mosaic"');
  expect(result).toContain("url('#ga-print-2-arrange-spots-0')");
  expect(result).toContain('xlink:href="data:image/png;base64,AAAA"');
  expect(result).toContain('data-id="whole-mosaic"');
  expect(result).toContain('<text>#whole-mosaic</text>');
});

it('uses physical paper dimensions and releases viewport clipping only for print', () => {
  const css = printDocumentCss([431.8, 279.4]);
  expect(css).toContain('#ga-print-root { display: none !important; }');
  expect(css).toContain('@page { size: 431.8mm 279.4mm; margin: 0; }');
  const print = css.slice(css.indexOf('@media print'));
  for (const rule of ['height: auto !important', 'min-height: 0 !important', 'max-height: none !important',
    'overflow: visible !important', 'body > :not(#ga-print-root) { display: none !important; }',
    '#ga-print-root { display: block !important; }', 'width: 431.8mm', 'height: 279.4mm', 'break-after: page']) {
    expect(print).toContain(rule);
  }
});
