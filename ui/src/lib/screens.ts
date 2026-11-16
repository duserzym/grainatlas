/** Shared tab access rules, including before published pixels have loaded. */
export function screenEnabled(id: string, state: { viewer: boolean; readOnly: boolean; remote: boolean; pkg: boolean; mosaic: boolean }): boolean {
  if (id === 'area' || id === 'layout') {
    if (state.viewer || (state.remote && state.readOnly)) return false;
    if (state.remote) return true;
    return id === 'area' ? state.pkg : state.mosaic;
  }
  return id !== 'canvas' || state.mosaic;
}
