// Live browser control on a phone: the page image sits in a stage the person can zoom and pan
// with two fingers, while one finger taps (click) or drags (press, move, release) in the page.
// Pure math and a small gesture state machine, so both can be tested without a DOM.

export const PAGE_W = 1280;
export const PAGE_H = 800;

/** Page pixel (x, y) shows at stage pixel (x * scale + tx, y * scale + ty). */
export interface View {
  scale: number;
  tx: number;
  ty: number;
}

export interface Size {
  width: number;
  height: number;
}

export interface Box {
  x: number;
  y: number;
  width: number;
  height: number;
}

/** How far in a person can zoom: four stage pixels per page pixel. */
export const MAX_SCALE = 4;

/** The whole page, as large as fits, pinned to the top so no band sits above it. */
export function fitView(stage: Size, page: Size = { width: PAGE_W, height: PAGE_H }): View {
  if (stage.width <= 0 || stage.height <= 0) return { scale: 1, tx: 0, ty: 0 };
  const scale = Math.min(stage.width / page.width, stage.height / page.height);
  return { scale, tx: (stage.width - page.width * scale) / 2, ty: 0 };
}

/** Keeps the zoom between fit and MAX_SCALE and the page covering the stage where it can. */
export function clampView(view: View, stage: Size, page: Size = { width: PAGE_W, height: PAGE_H }): View {
  const min = fitView(stage, page).scale;
  const scale = Math.min(Math.max(view.scale, min), Math.max(min, MAX_SCALE));
  const axis = (t: number, stageLen: number, pageLen: number, centre: boolean) => {
    const len = pageLen * scale;
    if (len <= stageLen) return centre ? (stageLen - len) / 2 : 0;
    return Math.min(0, Math.max(stageLen - len, t));
  };
  return { scale, tx: axis(view.tx, stage.width, page.width, true), ty: axis(view.ty, stage.height, page.height, false) };
}

/** Zooms so `box` fills the stage with a margin, centred on it (a CAPTCHA widget, say). */
export function zoomToBox(box: Box, stage: Size, page: Size = { width: PAGE_W, height: PAGE_H }, margin = 16): View {
  if (box.width <= 0 || box.height <= 0) return fitView(stage, page);
  const room = { width: Math.max(1, stage.width - margin * 2), height: Math.max(1, stage.height - margin * 2) };
  const scale = Math.min(room.width / box.width, room.height / box.height, MAX_SCALE);
  const cx = box.x + box.width / 2;
  const cy = box.y + box.height / 2;
  return clampView({ scale, tx: stage.width / 2 - cx * scale, ty: stage.height / 2 - cy * scale }, stage, page);
}

/** The page pixel under a stage point, or null off the page. */
export function stageToPage(sx: number, sy: number, view: View, page: Size = { width: PAGE_W, height: PAGE_H }): { x: number; y: number } | null {
  if (view.scale <= 0) return null;
  const x = (sx - view.tx) / view.scale;
  const y = (sy - view.ty) / view.scale;
  if (x < 0 || y < 0 || x >= page.width || y >= page.height) return null;
  return { x: Math.floor(x), y: Math.floor(y) };
}

/** Like stageToPage, but holds the point on the page's edge (a drag that strays off it). */
export function stageToPageClamped(sx: number, sy: number, view: View, page: Size = { width: PAGE_W, height: PAGE_H }): { x: number; y: number } {
  const x = (sx - view.tx) / view.scale;
  const y = (sy - view.ty) / view.scale;
  return { x: Math.floor(Math.min(page.width - 1, Math.max(0, x))), y: Math.floor(Math.min(page.height - 1, Math.max(0, y))) };
}

/** Two fingers moved from (a0, b0) to (a1, b1): the page point under their midpoint stays under it. */
export function pinchView(start: View, a0: Pt, b0: Pt, a1: Pt, b1: Pt): View {
  const d0 = Math.hypot(b0.x - a0.x, b0.y - a0.y) || 1;
  const d1 = Math.hypot(b1.x - a1.x, b1.y - a1.y) || 1;
  const scale = start.scale * (d1 / d0);
  const m0 = { x: (a0.x + b0.x) / 2, y: (a0.y + b0.y) / 2 };
  const m1 = { x: (a1.x + b1.x) / 2, y: (a1.y + b1.y) / 2 };
  const px = (m0.x - start.tx) / start.scale;
  const py = (m0.y - start.ty) / start.scale;
  return { scale, tx: m1.x - px * scale, ty: m1.y - py * scale };
}

export interface Pt {
  x: number;
  y: number;
}

/** What a gesture sends to the page; coordinates are page pixels. */
export type PageInput =
  | { action: 'click'; x: number; y: number }
  | { action: 'pointer_down' | 'pointer_move' | 'pointer_up'; x: number; y: number }
  | { action: 'scroll'; x: number; y: number; delta_x: number; delta_y: number };

export interface GestureHost {
  /** Stage point → page point, null off the page. */
  toPage(p: Pt): Pt | null;
  /** Same, held on the page's edge. */
  toPageClamped(p: Pt): Pt;
  send(input: PageInput): void;
  /** Two-finger zoom or pan of the local view; omit where the view doesn't zoom (desktop). */
  view?: { get(): View; set(v: View): void; zoomed(): boolean };
  now(): number;
  /** Runs `fn` after `ms` (for the move throttle); returns a cancel. */
  later(fn: () => void, ms: number): () => void;
}

/** A finger or the mouse has to travel this far (stage px) before a tap becomes a drag. */
export const DRAG_SLOP = 6;
/** About one move per frame, which sliders need to follow the drag. */
export const MOVE_INTERVAL_MS = 16;
/** Stage pixels of two-finger swipe per wheel step, and how much a wheel step scrolls. */
const WHEEL_GAIN = 2.5;

type Mode =
  | { kind: 'idle' }
  | { kind: 'press'; id: number; start: Pt; last: Pt }
  | { kind: 'drag'; id: number; last: Pt }
  | { kind: 'two'; a0: Pt; b0: Pt; start: View; ids: [number, number]; pos: Map<number, Pt>; split: 'undecided' | 'view' | 'wheel'; wheelFrom: Pt }
  | { kind: 'spent' };

/**
 * One finger: a tap clicks, a drag presses, moves (about 60 a second) and releases.
 * Two fingers: pinch or pan the view when it can zoom; at the fitted view a vertical two-finger
 * swipe scrolls the page instead. A finger added mid-drag is ignored so a drag never breaks.
 */
export function createGestureTracker(host: GestureHost) {
  let mode: Mode = { kind: 'idle' };
  let lastMoveAt = -Infinity;
  let pendingMove: Pt | null = null;
  let cancelTimer: (() => void) | null = null;
  const fingers = new Map<number, Pt>();

  const flushMove = () => {
    cancelTimer?.();
    cancelTimer = null;
    if (!pendingMove) return;
    const p = host.toPageClamped(pendingMove);
    pendingMove = null;
    lastMoveAt = host.now();
    host.send({ action: 'pointer_move', ...p });
  };

  const queueMove = (p: Pt) => {
    pendingMove = p;
    const wait = MOVE_INTERVAL_MS - (host.now() - lastMoveAt);
    if (wait <= 0) flushMove();
    else if (!cancelTimer) cancelTimer = host.later(flushMove, wait);
  };

  const startTwo = () => {
    const [a, b] = [...fingers.entries()];
    const v = host.view?.get() ?? { scale: 1, tx: 0, ty: 0 };
    mode = { kind: 'two', a0: a[1], b0: b[1], start: v, ids: [a[0], b[0]], pos: fingers, split: 'undecided', wheelFrom: mid(a[1], b[1]) };
  };

  return {
    down(id: number, p: Pt, primary = true) {
      fingers.set(id, p);
      if (mode.kind === 'idle' && primary) {
        if (!host.toPage(p)) { mode = { kind: 'spent' }; return; }
        mode = { kind: 'press', id, start: p, last: p };
      } else if (mode.kind === 'press' && fingers.size === 2) {
        // A second finger before the first moved: a zoom, not a tap
        startTwo();
      } else if (mode.kind === 'spent' && fingers.size === 2) {
        startTwo();
      }
    },

    move(id: number, p: Pt) {
      if (!fingers.has(id)) return;
      fingers.set(id, p);
      if (mode.kind === 'press' && mode.id === id) {
        if (Math.hypot(p.x - mode.start.x, p.y - mode.start.y) < DRAG_SLOP) return;
        const from = host.toPageClamped(mode.start);
        host.send({ action: 'pointer_down', ...from });
        lastMoveAt = host.now();
        mode = { kind: 'drag', id, last: p };
        queueMove(p);
      } else if (mode.kind === 'drag' && mode.id === id) {
        mode.last = p;
        queueMove(p);
      } else if (mode.kind === 'two' && mode.ids.includes(id)) {
        const a1 = fingers.get(mode.ids[0])!;
        const b1 = fingers.get(mode.ids[1])!;
        if (mode.split === 'undecided') {
          const spread = Math.abs(Math.hypot(b1.x - a1.x, b1.y - a1.y) - Math.hypot(mode.b0.x - mode.a0.x, mode.b0.y - mode.a0.y));
          const m = mid(a1, b1);
          const travel = Math.hypot(m.x - mode.wheelFrom.x, m.y - mode.wheelFrom.y);
          if (spread < 10 && travel < 10) return;
          // Fingers spreading or pinching zoom; moving together scroll the page unless zoomed in
          mode.split = host.view && (spread >= 10 || host.view.zoomed()) ? 'view' : 'wheel';
        }
        if (mode.split === 'view' && host.view) {
          host.view.set(pinchView(mode.start, mode.a0, mode.b0, a1, b1));
        } else if (mode.split === 'wheel') {
          const m = mid(a1, b1);
          const dy = mode.wheelFrom.y - m.y;
          const dx = mode.wheelFrom.x - m.x;
          if (Math.abs(dy) < 12 && Math.abs(dx) < 12) return;
          mode.wheelFrom = m;
          const at = host.toPage(m) ?? { x: PAGE_W / 2, y: PAGE_H / 2 };
          const scale = host.view?.get().scale ?? 1;
          host.send({ action: 'scroll', ...at, delta_x: Math.round((dx * WHEEL_GAIN) / scale), delta_y: Math.round((dy * WHEEL_GAIN) / scale) });
        }
      }
    },

    up(id: number, p?: Pt) {
      if (p) fingers.set(id, p);
      const at = fingers.get(id);
      fingers.delete(id);
      if (mode.kind === 'press' && mode.id === id) {
        const target = host.toPage(at ?? mode.start);
        mode = fingers.size ? { kind: 'spent' } : { kind: 'idle' };
        if (target) host.send({ action: 'click', ...target });
        return;
      }
      if (mode.kind === 'drag' && mode.id === id) {
        if (at) pendingMove = at;
        flushMove();
        host.send({ action: 'pointer_up', ...host.toPageClamped(at ?? mode.last) });
        mode = fingers.size ? { kind: 'spent' } : { kind: 'idle' };
        return;
      }
      if (!fingers.size) mode = { kind: 'idle' };
      else if (mode.kind === 'two') mode = { kind: 'spent' };
    },

    /** The browser took the touch away (a system gesture, say): let go without clicking. */
    cancel(id: number) {
      const at = fingers.get(id);
      fingers.delete(id);
      if (mode.kind === 'drag' && mode.id === id) {
        flushMove();
        host.send({ action: 'pointer_up', ...host.toPageClamped(at ?? mode.last) });
      }
      mode = fingers.size ? { kind: 'spent' } : { kind: 'idle' };
    },

    /** Mid-drag right now (for the cursor and to keep the page from scrolling). */
    dragging: () => mode.kind === 'drag',
  };
}

function mid(a: Pt, b: Pt): Pt {
  return { x: (a.x + b.x) / 2, y: (a.y + b.y) / 2 };
}
