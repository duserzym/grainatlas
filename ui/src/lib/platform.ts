/** Use the same platform convention as the project tabs; safe without a browser. */
export function primaryModifier(platform = typeof navigator === 'undefined' ? '' : navigator.platform): 'Cmd' | 'Ctrl' {
  return /Mac/.test(platform) ? 'Cmd' : 'Ctrl';
}
