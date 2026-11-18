/** One key and one encoded body per logical write, including multipart uploads. */
export interface WriteIdentity {
  key?: string;
  keyChanged?: (key: string) => void | Promise<void>;
  wait?: (milliseconds: number) => Promise<void>;
}
export async function idempotentWrite(send: (key: string) => Promise<Response>, options: WriteIdentity = {}): Promise<Response> {
  let key = options.key ?? crypto.randomUUID();
  let renewed = false;
  let waits = 0;
  const wait = options.wait ?? (ms => new Promise(resolve => setTimeout(resolve, ms)));
  for (;;) {
    const reply = await send(key);
    if (reply.status !== 409 && reply.status !== 422) return reply;
    const payload = await reply.clone().json().catch(() => null);
    if (reply.status === 409 && payload?.code === 'idempotency_in_progress') {
      if (waits++ >= 30) return reply;
      const seconds = Number(reply.headers.get('Retry-After') ?? 2);
      await wait((Number.isFinite(seconds) && seconds >= 0 ? seconds : 2) * 1000);
      continue;
    }
    if (reply.status === 422 && payload?.code === 'idempotency_key_reused' && !renewed) {
      renewed = true;
      key = crypto.randomUUID();
      console.warn('Idempotency key was already used; retrying once with a fresh key.');
      await options.keyChanged?.(key);
      continue;
    }
    return reply;
  }
}
