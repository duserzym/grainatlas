import { DEFAULT_START_LAYOUT, startLayout } from './mount-browser.ts';
export interface LayoutPrefs {
  panel: { x: number; y: number };
  sideCollapsed: boolean;
  printCollapsed?: boolean;
  infoCollapsed: boolean;
  menuCollapsed: boolean;
  layersOpen: boolean;
  spotEntryOpen: boolean;
  recentWidth: number;
  tileSize: 'S' | 'M' | 'L';
}
export const LAYOUT_KEY = 'grainatlas.layout';
export const DEFAULT_LAYOUT: LayoutPrefs = { panel: { x: 16, y: 150 }, sideCollapsed: false, infoCollapsed: false, menuCollapsed: false, layersOpen: false, spotEntryOpen: true, ...DEFAULT_START_LAYOUT };
export function loadLayout(storage: Pick<Storage, 'getItem'> = localStorage): LayoutPrefs {
  try {
    const v = JSON.parse(storage.getItem(LAYOUT_KEY) || '{}');
    return { ...DEFAULT_LAYOUT, ...v, ...startLayout(v), panel: { ...DEFAULT_LAYOUT.panel, ...(v.panel ?? {}) } };
  } catch { return structuredClone(DEFAULT_LAYOUT); }
}
export function saveLayout(storage: Pick<Storage, 'setItem'>, layout: LayoutPrefs) {
  storage.setItem(LAYOUT_KEY, JSON.stringify({ ...layout, ...startLayout(layout) }));
}
export function resetLayoutStorage(storage: Pick<Storage, 'removeItem'>): LayoutPrefs {
  storage.removeItem(LAYOUT_KEY);
  return structuredClone(DEFAULT_LAYOUT);
}
