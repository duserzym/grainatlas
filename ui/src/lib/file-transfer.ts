import { fetchWithTimeout } from './http.ts';

/** A transfer has a header deadline and an inactivity deadline, never a size deadline. */
export async function downloadFile(url: string, init: RequestInit, signal: AbortSignal, options: {
  connectMs?: number; stallMs?: number; wait?: (ms: number) => Promise<void>;
} = {}): Promise<ArrayBuffer> {
  const wait = options.wait ?? (ms => new Promise<void>((resolve, reject) => {
    const abort = () => { clearTimeout(timer); reject(signal.reason); };
    const timer = setTimeout(() => { signal.removeEventListener('abort', abort); resolve(); }, ms);
    signal.addEventListener('abort', abort, { once: true });
    if (signal.aborted) abort();
  }));
  for (let attempt = 0; ; attempt++) {
    signal.throwIfAborted();
    const controller = new AbortController();
    const abort = () => controller.abort(signal.reason);
    signal.addEventListener('abort', abort, { once: true });
    let timer: ReturnType<typeof setTimeout> | undefined;
    let transient = true;
    const arm = (ms: number) => {
      clearTimeout(timer);
      timer = setTimeout(() => controller.abort(new Error('Published file transfer stalled.')), ms);
    };
    try {
      const r = await fetchWithTimeout(url, { ...init, signal: controller.signal }, { responseTimeoutMs: options.connectMs ?? 10_000 });
      controller.signal.throwIfAborted();
      transient = r.ok || r.status >= 500 || r.status === 429;
      const reader = r.body?.getReader();
      if (!reader) {
        if (!r.ok) throw new Error(`Published file: ${r.status}`);
        return new ArrayBuffer(0);
      }
      const stop = () => { void reader.cancel(controller.signal.reason).catch(() => {}); };
      controller.signal.addEventListener('abort', stop, { once: true });
      const chunks: Uint8Array[] = [];
      let length = 0;
      try {
        arm(options.stallMs ?? 30_000);
        for (;;) {
          const { done, value } = await reader.read();
          controller.signal.throwIfAborted();
          if (done) break;
          if (value.length) { arm(options.stallMs ?? 30_000); chunks.push(value); length += value.length; }
        }
      } finally { controller.signal.removeEventListener('abort', stop); reader.releaseLock(); }
      const bytes = new Uint8Array(length);
      let offset = 0;
      for (const chunk of chunks) { bytes.set(chunk, offset); offset += chunk.length; }
      if (!r.ok) throw new Error(`Published file: ${r.status} · ${new TextDecoder().decode(bytes).slice(0, 300)}`);
      return bytes.buffer;
    } catch (error) {
      signal.throwIfAborted();
      if (!transient || attempt >= 3) throw error;
    } finally {
      clearTimeout(timer);
      signal.removeEventListener('abort', abort);
      controller.abort();
    }
    await wait(1000 * 2 ** attempt);
  }
}
