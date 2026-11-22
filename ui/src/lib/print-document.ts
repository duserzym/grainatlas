/** Core SVGs reuse clip, mask, pattern and scale-bar IDs across pages. */
export function prefixPrintSvgIds(svg: string, page: number): string {
  const ids = new Map<string, string>();
  const renamed = svg.replace(/(\s)id=(["'])(.*?)\2/g, (_match, space, quote, id) => {
    const unique = `ga-print-${page}-${id}`;
    ids.set(id, unique);
    return `${space}id=${quote}${unique}${quote}`;
  });
  return renamed
    .replace(/url\(\s*(["']?)#([^\s)"']+)\1\s*\)/g,
      (match, quote, id) => ids.has(id) ? `url(${quote}#${ids.get(id)}${quote})` : match)
    .replace(/(\s(?:xlink:)?href=)(["'])#(.*?)\2/g,
      (match, attr, quote, id) => ids.has(id) ? `${attr}${quote}#${ids.get(id)}${quote}` : match);
}

export function printDocumentCss([w, h]: readonly number[]): string {
  return `
#ga-print-root { display: none !important; }
@page { size: ${w}mm ${h}mm; margin: 0; }
@media print {
  html, body {
    display: block !important;
    height: auto !important;
    min-height: 0 !important;
    max-height: none !important;
    overflow: visible !important;
    margin: 0 !important;
    padding: 0 !important;
    background: white !important;
    color-scheme: light;
  }
  body > :not(#ga-print-root) { display: none !important; }
  #ga-print-root { display: block !important; }
  #ga-print-root > svg {
    display: block;
    width: ${w}mm;
    height: ${h}mm;
    margin: 0;
    break-inside: avoid;
    break-after: page;
    page-break-after: always;
  }
  #ga-print-root > svg:last-child { break-after: auto; page-break-after: auto; }
}`;
}

export type PrintDocument = ReturnType<typeof createPrintDocument>;

/** Keep the document alive until the next print or owner teardown; synchronous
 * browser printing can also clean up on afterprint.
 */
export function createPrintDocument(pages: string[], paper: readonly number[]) {
  const root = document.createElement('div');
  root.id = 'ga-print-root';
  const style = document.createElement('style');
  style.textContent = printDocumentCss(paper);
  root.innerHTML = pages.map(prefixPrintSvgIds).join('');
  root.prepend(style);
  document.body.append(root);
  let previousTitle: string | undefined;
  let printTitle: string | undefined;
  function restoreTitle() {
    // App.svelte owns title changes; do not overwrite a newer app title.
    if (previousTitle !== undefined && document.title === printTitle) document.title = previousTitle;
    previousTitle = undefined;
  }
  function cleanup() {
    window.removeEventListener('afterprint', cleanup);
    root.remove();
    root.replaceChildren();
    restoreTitle();
  }
  window.addEventListener('afterprint', cleanup);
  return {
    cleanup,
    async decodeImages() {
      await Promise.all(Array.from(root.querySelectorAll('svg image'), async node => {
        const img = document.createElement('img');
        const loaded = new Promise<void>((resolve, reject) => {
          img.onload = () => resolve();
          img.onerror = () => reject(new Error('A print image could not be loaded'));
        });
        void loaded.catch(() => {});
        img.src = node.getAttribute('href') ?? node.getAttributeNS('http://www.w3.org/1999/xlink', 'href') ?? '';
        await (img.decode ? img.decode() : loaded);
      }));
    },
    async print(title: string, submit?: () => Promise<void>) {
      previousTitle = document.title;
      printTitle = title;
      document.title = title;
      if (submit) {
        window.removeEventListener('afterprint', cleanup);
        try { await submit(); }
        finally { restoreTitle(); }
        return;
      }
      const printing = window.print() as void | PromiseLike<void>;
      // Chromium fires afterprint during the call, so listen before calling.
      // WebKit's native sheet can fire it while still paginating, even after
      // its promise resolves; that path must wait for the next print or teardown.
      if (printing && typeof printing.then === 'function') window.removeEventListener('afterprint', cleanup);
      await printing;
    },
  };
}
