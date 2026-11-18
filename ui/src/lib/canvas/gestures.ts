// One gesture module for every image view (Area, Canvas, Layout preview), so
// they all pan and zoom the same way and with the same feel:
//
//   mouse wheel ........................ zoom about the cursor
//   trackpad two-finger scroll ......... pan
//   trackpad pinch ..................... zoom about the fingers
//       Chromium / Edge / WebView2: ctrl+wheel with small deltas
//       WebKit (Safari, Tauri on macOS): gesturestart / gesturechange / gestureend
//   ctrl / cmd + mouse wheel ........... zoom (same path as Chromium pinch)
//   alt + wheel, WebKit rotate gesture . rotate (views that allow it)
//   drag empty space ................... pan unless a tool takes the drag
//   middle drag, Space + drag .......... pan
//   two touch points ................... pinch zoom (+ rotate)
//   double-click empty space ........... fit
//   arrow keys ......................... nudge (Shift: larger), + / - zoom, 0 fit
//
// A wheel event is a mouse notch or a trackpad scroll depending on the
// engine: `wheelDeltaY` (WebKit and Chromium both have it) is exactly
// -3 × deltaY for trackpads and a multiple of 120 for mouse notches;
// Firefox has no wheelDeltaY and reports mouse notches in lines (deltaMode 1).

import { frameView } from './frame-view';
import { type View, panBy, rotateAt, toMosaic, zoomAt } from './view';

export interface ViewPoint {
  /** View (mosaic / content) coordinates. */
  x: number;
  y: number;
  /** Screen coordinates relative to the host. */
  sx: number;
  sy: number;
  ev: PointerEvent | MouseEvent;
}

export interface GestureOptions {
  /** The element that receives the gestures (positioned, `touch-action: none`). */
  host: HTMLElement;
  getView(): View;
  setView(v: View): void;
  /** Fit the content (double-click, key 0). */
  fit(): void;
  /** Rotation gestures (alt+wheel, two-finger rotate); off for the layout preview. */
  rotate?: boolean;
  /** A press on content: return true when a tool takes the drag (no pan). */
  onpress?(p: ViewPoint): boolean;
  ondrag?(p: ViewPoint): void;
  onrelease?(p: ViewPoint): void;
  /** A click without movement. */
  onclick?(p: ViewPoint): void;
  /** Double-click: return true when handled (else the view fits). */
  ondblclick?(p: ViewPoint): boolean | void;
  onhover?(p: ViewPoint | null): void;
  /** Arrow keys nudge the view unless this returns false (e.g. a selection moves instead). */
  canNudge?(): boolean;
}

/** Zoom speed for one pixel of wheel delta (mouse) and for pinch deltas. */
export const WHEEL_ZOOM = 0.0015;
export const PINCH_ZOOM = 0.01;
/** Pixels per wheel "line" (deltaMode 1) and the movement that turns a click into a pan. */
const LINE_PX = 33;
export const CLICK_SLOP = 4;
/** Arrow-key nudge in screen pixels (Shift: larger). */
export const NUDGE_PX = 60;
export const NUDGE_BIG_PX = 240;

export type WheelKind = 'pinch' | 'mouse' | 'trackpad';

/** What a wheel event is, in WebKit, Chromium and Firefox. */
export function classifyWheel(e: WheelEvent): WheelKind {
  if (e.ctrlKey || e.metaKey) return 'pinch';
  if (e.deltaMode === 1 || e.deltaMode === 2) return 'mouse';
  const wdy = (e as WheelEvent & { wheelDeltaY?: number }).wheelDeltaY;
  if (typeof wdy === 'number' && wdy !== 0) {
    if (Math.abs(wdy + 3 * e.deltaY) < 1e-6) return 'trackpad';
    if (wdy % 120 === 0) return 'mouse';
    return 'trackpad';
  }
  // No wheelDeltaY (Firefox pixel mode, synthetic events): notch-like whole deltas are a mouse.
  const mouseLike = e.deltaX === 0 && Math.abs(e.deltaY) >= 40 && Number.isInteger(e.deltaY) && !e.shiftKey;
  return mouseLike ? 'mouse' : 'trackpad';
}

/** Wheel delta in pixels whatever the delta mode. */
function px(d: number, mode: number, page: number) {
  return mode === 1 ? d * LINE_PX : mode === 2 ? d * page : d;
}

function isTyping(t: EventTarget | null) {
  const el = t as HTMLElement | null;
  return !!el && (el.tagName === 'INPUT' || el.tagName === 'TEXTAREA' || el.tagName === 'SELECT' || el.isContentEditable);
}

/** Attaches the gestures to `o.host`; returns the detach function. */
export function attachGestures(o: GestureOptions): () => void {
  const host = o.host;
  const view = frameView(o.getView, o.setView);
  if (!host.hasAttribute('tabindex')) host.tabIndex = 0;
  let spaceDown = false;
  let inside = false;
  let gesture: { scale: number; rot: number } | null = null;
  let mode: 'none' | 'tool' | 'maybe' | 'pan' = 'none';
  let start = { x: 0, y: 0 };
  let last = { x: 0, y: 0 };
  const touches = new Map<number, { x: number; y: number }>();
  let pinch: { d: number; a: number } | null = null;

  const rect = () => host.getBoundingClientRect();
  const pt = (e: PointerEvent | MouseEvent): ViewPoint => {
    const r = rect();
    const sx = e.clientX - r.left, sy = e.clientY - r.top;
    const [x, y] = toMosaic(view.get(), sx, sy);
    return { x, y, sx, sy, ev: e };
  };
  const onUi = (e: Event) => !!(e.target as Element | null)?.closest?.('[data-ui]');
  const set = (v: View) => view.set(v);

  function down(e: PointerEvent) {
    if (onUi(e)) return;
    host.focus({ preventScroll: true });
    host.setPointerCapture?.(e.pointerId);
    touches.set(e.pointerId, { x: e.clientX, y: e.clientY });
    if (touches.size === 2) {
      const [a, b] = [...touches.values()];
      pinch = { d: Math.hypot(b.x - a.x, b.y - a.y), a: Math.atan2(b.y - a.y, b.x - a.x) };
      mode = 'none';
      return;
    }
    start = last = { x: e.clientX, y: e.clientY };
    if (e.button === 1 || spaceDown) {
      mode = 'pan';
      e.preventDefault();
      return;
    }
    if (e.button !== 0) return;
    mode = o.onpress?.(pt(e)) ? 'tool' : 'maybe';
  }

  function move(e: PointerEvent) {
    if (touches.has(e.pointerId)) touches.set(e.pointerId, { x: e.clientX, y: e.clientY });
    if (pinch && touches.size === 2) {
      const [a, b] = [...touches.values()];
      const d = Math.hypot(b.x - a.x, b.y - a.y), ang = Math.atan2(b.y - a.y, b.x - a.x);
      const r = rect();
      const mx = (a.x + b.x) / 2 - r.left, my = (a.y + b.y) / 2 - r.top;
      let nv = zoomAt(view.get(), mx, my, d / pinch.d);
      if (o.rotate) nv = rotateAt(nv, mx, my, ang - pinch.a);
      pinch = { d, a: ang };
      set(nv);
      return;
    }
    if (mode === 'tool') o.ondrag?.(pt(e));
    else if (mode === 'maybe' && Math.hypot(e.clientX - start.x, e.clientY - start.y) > CLICK_SLOP) mode = 'pan';
    if (mode === 'pan') set(panBy(view.get(), e.clientX - last.x, e.clientY - last.y));
    last = { x: e.clientX, y: e.clientY };
    if (mode === 'none' && !onUi(e)) o.onhover?.(pt(e));
  }

  function up(e: PointerEvent) {
    touches.delete(e.pointerId);
    if (touches.size < 2) pinch = null;
    if (mode === 'tool') o.onrelease?.(pt(e));
    else if (mode === 'maybe') o.onclick?.(pt(e));
    mode = 'none';
  }

  function wheel(e: WheelEvent) {
    if (onUi(e)) return;
    e.preventDefault();
    const r = rect();
    const sx = e.clientX - r.left, sy = e.clientY - r.top;
    const kind = classifyWheel(e);
    const dy = px(e.deltaY, e.deltaMode, r.height);
    const dx = px(e.deltaX, e.deltaMode, r.width);
    if (kind === 'pinch') {
      if (gesture) return; // WebKit sends gesture events for the same pinch
      // Pinch deltas are small; a ctrl+mouse notch is large: clamp so both feel the same.
      set(zoomAt(view.get(), sx, sy, Math.exp(-Math.max(-50, Math.min(50, dy)) * PINCH_ZOOM)));
      return;
    }
    if (e.altKey && o.rotate) {
      set(rotateAt(view.get(), sx, sy, dy * 0.002));
      return;
    }
    if (kind === 'mouse') {
      if (e.shiftKey) set(panBy(view.get(), -(dx || dy), 0));
      else set(zoomAt(view.get(), sx, sy, Math.exp(-dy * WHEEL_ZOOM)));
      return;
    }
    set(panBy(view.get(), -dx, -dy));
  }

  // WebKit trackpad gestures (Safari, WKWebView in Tauri on macOS): pinch and rotate.
  function gstart(e: Event) {
    e.preventDefault();
    gesture = { scale: 1, rot: 0 };
  }
  function gchange(e: Event) {
    e.preventDefault();
    const g = e as unknown as { scale: number; rotation: number; clientX?: number; clientY?: number };
    if (!gesture) return;
    const r = rect();
    const sx = (g.clientX ?? r.left + r.width / 2) - r.left, sy = (g.clientY ?? r.top + r.height / 2) - r.top;
    let nv = zoomAt(view.get(), sx, sy, g.scale / gesture.scale);
    if (o.rotate) nv = rotateAt(nv, sx, sy, (((g.rotation ?? 0) - gesture.rot) * Math.PI) / 180);
    gesture = { scale: g.scale, rot: g.rotation ?? 0 };
    set(nv);
  }
  function gend(e: Event) {
    e.preventDefault();
    gesture = null;
  }

  function dbl(e: MouseEvent) {
    if (onUi(e)) return;
    if (o.ondblclick?.(pt(e))) return;
    view.clear();
    o.fit();
  }

  function key(e: KeyboardEvent) {
    if (e.code === 'Space' && !isTyping(e.target)) {
      spaceDown = e.type === 'keydown';
      return;
    }
    if (e.type !== 'keydown' || e.defaultPrevented || isTyping(e.target) || e.metaKey || e.ctrlKey || e.altKey) return;
    // Only the view under the pointer or holding the focus.
    if (!inside && !host.contains(document.activeElement)) return;
    const r = rect();
    const step = e.shiftKey ? NUDGE_BIG_PX : NUDGE_PX;
    const nudge: Record<string, [number, number]> = { ArrowLeft: [step, 0], ArrowRight: [-step, 0], ArrowUp: [0, step], ArrowDown: [0, -step] };
    if (nudge[e.key]) {
      if (o.canNudge && !o.canNudge()) return;
      e.preventDefault();
      set(panBy(view.get(), ...nudge[e.key]));
    } else if (e.key === '+' || e.key === '=') {
      e.preventDefault();
      set(zoomAt(view.get(), r.width / 2, r.height / 2, 1.5));
    } else if (e.key === '-' || e.key === '_') {
      e.preventDefault();
      set(zoomAt(view.get(), r.width / 2, r.height / 2, 1 / 1.5));
    } else if (e.key === '0') {
      e.preventDefault();
      view.clear();
    o.fit();
    }
  }

  const enter = () => (inside = true);
  const leave = () => {
    inside = false;
    o.onhover?.(null);
  };
  const opts = { passive: false } as AddEventListenerOptions;
  host.addEventListener('pointerdown', down);
  host.addEventListener('pointermove', move);
  host.addEventListener('pointerup', up);
  host.addEventListener('pointercancel', up);
  host.addEventListener('pointerenter', enter);
  host.addEventListener('pointerleave', leave);
  host.addEventListener('wheel', wheel, opts);
  host.addEventListener('dblclick', dbl);
  host.addEventListener('gesturestart', gstart, opts);
  host.addEventListener('gesturechange', gchange, opts);
  host.addEventListener('gestureend', gend, opts);
  window.addEventListener('keydown', key);
  window.addEventListener('keyup', key);
  return () => {
    view.clear();
    host.removeEventListener('pointerdown', down);
    host.removeEventListener('pointermove', move);
    host.removeEventListener('pointerup', up);
    host.removeEventListener('pointercancel', up);
    host.removeEventListener('pointerenter', enter);
    host.removeEventListener('pointerleave', leave);
    host.removeEventListener('wheel', wheel);
    host.removeEventListener('dblclick', dbl);
    host.removeEventListener('gesturestart', gstart);
    host.removeEventListener('gesturechange', gchange);
    host.removeEventListener('gestureend', gend);
    window.removeEventListener('keydown', key);
    window.removeEventListener('keyup', key);
  };
}
