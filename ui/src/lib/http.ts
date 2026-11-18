export const databaseOfflineMessage = 'The database did not answer. Your work is kept on this computer.';

/** Includes lost replies: a write may have reached the server and must keep its retry key. */
export class HttpNetworkError extends Error {}

interface Timeouts {
  responseTimeoutMs?: number;
  stallTimeoutMs?: number;
  signal?: AbortSignal;
}

/** Response timeout for a request: 8 s for reads; writes at least 60 s plus the body at
 * 64 KB/s, since the clock also runs while a body uploads and the server applies it. */
export function writeTimeout(init: RequestInit = {}): number {
  const method = (init.method ?? 'GET').toUpperCase();
  if (method === 'GET' || method === 'HEAD') return 8000;
  const body = init.body;
  const bytes = typeof body === 'string' ? body.length : body instanceof Blob ? body.size : body instanceof ArrayBuffer ? body.byteLength : ArrayBuffer.isView(body) ? body.byteLength : 0;
  return 60_000 + Math.ceil(bytes / 64_000) * 1000;
}

/** Bound the wait for response headers (connect, upload and server time), never the body:
 * a dead or unreachable host fails within the window instead of hanging until the engine's own
 * timeout. Reads get 8 s, writes the upload-scaled window of `writeTimeout`. The response is
 * returned untouched. (A stall timeout during body reads is a follow-up: wrapping the body
 * stream misbehaved in the browser.) `stallTimeoutMs` is accepted and ignored for now. */
export async function fetchWithTimeout(url: RequestInfo | URL, init: RequestInit = {},
  { responseTimeoutMs, signal }: Timeouts = {}): Promise<Response> {
  responseTimeoutMs ??= writeTimeout(init);
  const controller = new AbortController();
  const callers = [...new Set([signal, init.signal, typeof Request !== 'undefined' && url instanceof Request ? url.signal : null].filter((s): s is AbortSignal => !!s))];
  const callerAbort = () => controller.abort(callers.find(s => s.aborted)?.reason ?? new DOMException('Aborted', 'AbortError'));
  for (const s of callers) s.addEventListener('abort', callerAbort, { once: true });
  let timedOut = false;
  const timer = setTimeout(() => { timedOut = true; controller.abort(new HttpNetworkError(databaseOfflineMessage)); }, responseTimeoutMs);
  if (callers.some(s => s.aborted)) callerAbort();
  try {
    controller.signal.throwIfAborted();
    // Race the abort too: a fetch that ignores its signal must not hang the caller.
    const aborted = new Promise<never>((_, reject) => controller.signal.addEventListener('abort', () => reject(controller.signal.reason), { once: true }));
    return await Promise.race([fetch(url, { ...init, signal: controller.signal }), aborted]);
  } catch (e) {
    if (timedOut) throw new HttpNetworkError(databaseOfflineMessage, { cause: e });
    if (controller.signal.aborted) throw controller.signal.reason;
    throw new HttpNetworkError(databaseOfflineMessage, { cause: e });
  } finally {
    clearTimeout(timer);
    for (const s of callers) s.removeEventListener('abort', callerAbort);
  }
}

/** A whole-request deadline (headers and body) for small JSON calls: no stream wrapping.
 * Reads get 8 s, writes the upload-scaled window of `writeTimeout`. */
export async function fetchWithDeadline(url: RequestInfo | URL, init: RequestInit = {}, ms = writeTimeout(init)): Promise<Response> {
  const deadline = AbortSignal.timeout(ms);
  const signal = init.signal ? AbortSignal.any([init.signal, deadline]) : deadline;
  try { return await fetch(url, { ...init, signal }); }
  catch (e) {
    if (deadline.aborted && !init.signal?.aborted) throw new HttpNetworkError(databaseOfflineMessage, { cause: e });
    throw e;
  }
}

/** Read a whole reply as text, failing with a network error when no bytes arrive for
 * `stallTimeoutMs` (default 30 s); no cap on total size or duration. */
export async function readBodyText(response: Response, stallTimeoutMs = 30000): Promise<string> {
  if (!response.body) return '';
  const reader = response.body.getReader();
  const decoder = new TextDecoder();
  let text = '';
  for (;;) {
    let timer: ReturnType<typeof setTimeout> | undefined;
    const stalled = new Promise<never>((_, reject) => { timer = setTimeout(() => reject(new HttpNetworkError(databaseOfflineMessage)), stallTimeoutMs); });
    try {
      const next = await Promise.race([reader.read(), stalled]);
      if (next.done) return text + decoder.decode();
      text += decoder.decode(next.value, { stream: true });
    } catch (e) {
      void reader.cancel(e).catch(() => {});
      throw e instanceof HttpNetworkError ? e : new HttpNetworkError(databaseOfflineMessage, { cause: e });
    } finally { clearTimeout(timer); }
  }
}
