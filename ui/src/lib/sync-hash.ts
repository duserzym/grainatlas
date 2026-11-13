/** Canonical JSON for the mock sync contract: object keys sorted by Unicode code point,
 * array order retained, undefined object properties omitted as in JSON.stringify. */
export function canonicalJson(value: unknown): string {
  return JSON.stringify(value, (_key, v: unknown) => v && typeof v === 'object' && !Array.isArray(v)
    ? Object.fromEntries(Object.entries(v).sort(([a], [b]) => a < b ? -1 : a > b ? 1 : 0)) : v);
}

/** Test/fake hash only. Production state compares opaque server hashes with saved server
 * hashes. JS numbers cannot reproduce Python float spelling (1 versus 1.0, exponents).
 * Python's default ensure_ascii also escapes non-ASCII UTF-16 code units. */
export async function elementHash(elements: readonly { id: string }[]): Promise<string> {
  const sha = async (value: string) => {
    const digest = await globalThis.crypto.subtle.digest('SHA-256', new TextEncoder().encode(value));
    return Array.from(new Uint8Array(digest), x => x.toString(16).padStart(2, '0')).join('');
  };
  const lines = await Promise.all([...elements].sort((a, b) => a.id < b.id ? -1 : a.id > b.id ? 1 : 0).map(async element =>
    `${element.id} ${await sha(canonicalJson(element).replace(/[\u007f-\uffff]/g, c => `\\u${c.charCodeAt(0).toString(16).padStart(4, '0')}`))}\n`));
  return sha(lines.join(''));
}
