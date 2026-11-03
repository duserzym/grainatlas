// Desktop implementation of Core (phase 7).
import type { Core } from './core';
export class TauriCore implements Partial<Core> {
  readonly kind = 'tauri' as const;
}
