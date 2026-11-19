// Raw native IPC avoids JSON-string escaping; parse the large body off the UI thread.
self.onmessage = (event: MessageEvent<{ id: number; body: ArrayBuffer }>) => {
  try {
    postMessage({ id: event.data.id, value: JSON.parse(new TextDecoder().decode(event.data.body)) });
  } catch (error) {
    postMessage({ id: event.data.id, error: String(error) });
  }
};
export {};
