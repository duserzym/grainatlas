/** Raw IPC body: u32 registration byte length, UTF-8 JSON, then PDF bytes.
 * Avoid JSON number arrays or multi-megabyte request headers. */
export function printSavePacket(pdf: Uint8Array, registration: string | null): Uint8Array {
  const json = new TextEncoder().encode(registration ?? '');
  const body = new Uint8Array(4 + json.length + pdf.length);
  new DataView(body.buffer).setUint32(0, json.length, true);
  body.set(json, 4); body.set(pdf, 4 + json.length);
  return body;
}
