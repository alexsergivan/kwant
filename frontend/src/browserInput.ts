// Desktop takeover of a bot's browser: turns the person's mouse, wheel and keyboard into the
// stream's protocol v2 messages. Pure functions and small state machines, so all of it is tested
// without a DOM. Coordinates sent to the server are CSS px of the page's viewport.

export interface Size {
  width: number;
  height: number;
}

export interface Rect {
  left: number;
  top: number;
  width: number;
  height: number;
}

/** CDP modifier bits. */
export const MOD = { alt: 1, ctrl: 2, meta: 4, shift: 8 } as const;

/** The remote is Linux or headless Chrome, so a Mac's Cmd shortcuts are sent as Ctrl. */
export function isMacPlatform(nav: { platform?: string; userAgent?: string } | undefined = typeof navigator === 'undefined' ? undefined : navigator): boolean {
  const s = `${nav?.platform ?? ''} ${nav?.userAgent ?? ''}`;
  return /Mac|iPhone|iPad|iPod/i.test(s);
}

// ---------------------------------------------------------------------------------------------
// Viewport size and drawing

export const VIEWPORT_LIMITS = { minWidth: 360, maxWidth: 1920, minHeight: 360, maxHeight: 1200, minDpr: 1, maxDpr: 2 };

const clamp = (v: number, lo: number, hi: number) => Math.min(hi, Math.max(lo, v));

/** The `resize` the stage asks for: its own size in CSS px, inside the server's limits. */
export function resizeFor(stage: Size, devicePixelRatio = 1): { action: 'resize'; width: number; height: number; dpr: number } | null {
  if (!(stage.width > 0) || !(stage.height > 0)) return null;
  const L = VIEWPORT_LIMITS;
  const dpr = Math.round(clamp(Math.min(devicePixelRatio || 1, 2), L.minDpr, L.maxDpr) * 100) / 100;
  return {
    action: 'resize',
    width: clamp(Math.floor(stage.width), L.minWidth, L.maxWidth),
    height: clamp(Math.floor(stage.height), L.minHeight, L.maxHeight),
    dpr,
  };
}

/**
 * What the person sees for a live view error, or null for none. Running off either end of the
 * history is expected (the buttons don't know the history), so it's silent; the rest lose the
 * server's "Protocol error: " prefix.
 */
export function streamErrorText(action: string | undefined, raw: unknown): string | null {
  const text = String(raw ?? '').replace(/^\s*protocol error:\s*/i, '').trim();
  if (action === 'history' && /no page to go/i.test(text)) return null;
  return text || 'Something went wrong in the browser.';
}

/**
 * Whether the page has drifted from the size the stage asked for (a cross-site navigation can
 * drop the override), so the resize should be sent again.
 */
export function sizeDrifted(requested: { width: number; height: number } | null, page: Size): boolean {
  if (!requested || !(page.width > 0) || !(page.height > 0)) return false;
  return Math.abs(page.width - requested.width) > 1 || Math.abs(page.height - requested.height) > 1;
}

/**
 * Where the page image sits in the stage: 1:1 when it fits, scaled down to fit when it doesn't
 * (a resize still on its way, or a stage smaller than the smallest viewport), centred either way.
 */
export function stageLayout(stage: Size, page: Size): { scale: number; left: number; top: number; width: number; height: number } {
  if (!(page.width > 0) || !(page.height > 0) || !(stage.width > 0) || !(stage.height > 0)) {
    return { scale: 1, left: 0, top: 0, width: page.width || 0, height: page.height || 0 };
  }
  const scale = Math.min(1, stage.width / page.width, stage.height / page.height);
  const width = page.width * scale;
  const height = page.height * scale;
  return { scale, left: (stage.width - width) / 2, top: (stage.height - height) / 2, width, height };
}

/** The page point under a client point, given the drawn image's box; null off the page unless clamped. */
export function pagePoint(clientX: number, clientY: number, drawn: Rect, page: Size, clampToEdge = false): { x: number; y: number } | null {
  if (!(drawn.width > 0) || !(drawn.height > 0) || !(page.width > 0) || !(page.height > 0)) return null;
  let x = ((clientX - drawn.left) * page.width) / drawn.width;
  let y = ((clientY - drawn.top) * page.height) / drawn.height;
  if (clampToEdge) {
    x = clamp(x, 0, page.width - 1);
    y = clamp(y, 0, page.height - 1);
  } else if (x < 0 || y < 0 || x >= page.width || y >= page.height) return null;
  return { x: Math.floor(x), y: Math.floor(y) };
}

// ---------------------------------------------------------------------------------------------
// Mouse

export type MouseButtonName = 'left' | 'middle' | 'right' | 'none';

export function buttonName(button: number): MouseButtonName {
  return button === 0 ? 'left' : button === 1 ? 'middle' : button === 2 ? 'right' : 'none';
}

export function mouseModifiers(e: { altKey: boolean; ctrlKey: boolean; metaKey: boolean; shiftKey: boolean }, mac = false): number {
  let m = 0;
  if (e.altKey) m |= MOD.alt;
  if (e.ctrlKey) m |= MOD.ctrl;
  // Cmd-click (open in new tab on a Mac) is Ctrl-click over there
  if (e.metaKey) m |= mac ? MOD.ctrl : MOD.meta;
  if (e.shiftKey) m |= MOD.shift;
  return m;
}

/** Keeps only the newest value and hands it on once per animation frame; flush() sends it now. */
export function createFrameCoalescer<T>(schedule: (fn: () => void) => void, emit: (value: T) => void) {
  let pending: T | null = null;
  let scheduled = false;
  const run = () => {
    scheduled = false;
    if (pending === null) return;
    const v = pending;
    pending = null;
    emit(v);
  };
  return {
    push(value: T) {
      pending = value;
      if (!scheduled) {
        scheduled = true;
        schedule(run);
      }
    },
    /** Sends what's waiting before something that has to come after it (a press, a release). */
    flush() {
      if (pending !== null) run();
    },
    /** Forgets what's waiting (the stage closed). */
    drop() {
      pending = null;
    },
    get pending() {
      return pending;
    },
  };
}

// ---------------------------------------------------------------------------------------------
// Wheel

export const WHEEL_LINE_PX = 40;

/** A wheel event's deltas in CSS px: lines ×40, pages × the viewport's height. */
export function wheelPixels(e: { deltaX: number; deltaY: number; deltaMode: number }, viewportHeight: number): { dx: number; dy: number } {
  const unit = e.deltaMode === 1 ? WHEEL_LINE_PX : e.deltaMode === 2 ? viewportHeight || 800 : 1;
  return { dx: e.deltaX * unit, dy: e.deltaY * unit };
}

export interface WheelMessage {
  action: 'wheel';
  x: number;
  y: number;
  delta_x: number;
  delta_y: number;
  modifiers: number;
}

/**
 * Adds up every wheel step that lands in one animation frame and sends one `wheel` for it.
 * Nothing is dropped: whole pixels go out, the fraction carries over to the next frame.
 */
export function createWheelCoalescer(schedule: (fn: () => void) => void, emit: (msg: WheelMessage) => void) {
  let dx = 0;
  let dy = 0;
  let at = { x: 0, y: 0 };
  let modifiers = 0;
  let scheduled = false;
  const run = () => {
    scheduled = false;
    const sx = Math.trunc(dx);
    const sy = Math.trunc(dy);
    if (sx === 0 && sy === 0) return;
    dx -= sx;
    dy -= sy;
    emit({ action: 'wheel', x: at.x, y: at.y, delta_x: sx, delta_y: sy, modifiers });
  };
  return {
    add(point: { x: number; y: number }, delta: { dx: number; dy: number }, mods = 0) {
      at = point;
      modifiers = mods;
      dx += delta.dx;
      dy += delta.dy;
      if (!scheduled) {
        scheduled = true;
        schedule(run);
      }
    },
    flush() {
      run();
    },
  };
}

// ---------------------------------------------------------------------------------------------
// Keyboard

export interface KeyLike {
  type?: string;
  key: string;
  code: string;
  altKey: boolean;
  ctrlKey: boolean;
  metaKey: boolean;
  shiftKey: boolean;
  repeat?: boolean;
  location?: number;
  isComposing?: boolean;
  keyCode?: number;
}

export interface KeyMessage {
  action: 'key';
  type: 'down' | 'up';
  key: string;
  code: string;
  text?: string;
  modifiers: number;
  location: number;
  auto_repeat: boolean;
}

export type KeyPlan =
  /** Leave it to the browser (composition) and don't send anything. */
  | { kind: 'skip' }
  /** Let the local paste event fire; its text goes as `type`. */
  | { kind: 'paste' }
  | { kind: 'send'; messages: KeyMessage[]; copy?: boolean };

const MODIFIER_KEYS = new Set(['Shift', 'Control', 'Alt', 'Meta', 'OS', 'AltGraph', 'CapsLock', 'Fn', 'FnLock', 'Hyper', 'Super', 'Symbol', 'SymbolLock']);

/** One character (an emoji counts as one): a key that types. */
const printable = (key: string) => key.length > 0 && [...key].length === 1 && key !== '\u0000';

/** Mac editing moves that the Linux remote spells differently. */
function macMove(key: string, meta: boolean, alt: boolean): { key: string; code: string; ctrl: boolean } | null {
  if (meta) {
    if (key === 'ArrowLeft') return { key: 'Home', code: 'Home', ctrl: false };
    if (key === 'ArrowRight') return { key: 'End', code: 'End', ctrl: false };
    if (key === 'ArrowUp') return { key: 'Home', code: 'Home', ctrl: true };
    if (key === 'ArrowDown') return { key: 'End', code: 'End', ctrl: true };
  }
  if (alt && (key === 'ArrowLeft' || key === 'ArrowRight' || key === 'Backspace' || key === 'Delete')) return { key, code: key, ctrl: true };
  return null;
}

/**
 * What a local keydown or keyup sends to the page. Printable keys carry `text` (so keydown,
 * keypress and input all fire there). On a Mac, Cmd shortcuts become Ctrl ones, Cmd+arrows
 * Home/End and Option+arrows word moves; Option's special characters are typed as they are.
 * Cmd or Ctrl+V lets the paste event through; Cmd or Ctrl+C and X also copy the page's selection.
 */
export function planKey(e: KeyLike, mac: boolean): KeyPlan {
  const down = e.type !== 'keyup';
  if (e.isComposing || e.keyCode === 229 || e.key === 'Dead' || e.key === 'Process' || e.key === 'Unidentified') return { kind: 'skip' };
  const shortcut = mac ? e.metaKey : e.ctrlKey;
  const lower = e.key.length === 1 ? e.key.toLowerCase() : '';
  if (down && shortcut && !e.altKey && (e.code === 'KeyV' || lower === 'v')) return { kind: 'paste' };

  let key = e.key;
  let code = e.code;
  let alt = e.altKey;
  let ctrl = e.ctrlKey;
  let meta = e.metaKey;
  let location = e.location ?? 0;

  if (mac) {
    const move = macMove(key, e.metaKey, e.altKey);
    if (move) {
      key = move.key;
      code = move.code;
      ctrl = move.ctrl;
      meta = false;
      alt = false;
    } else if (key === 'Meta') {
      // The Cmd key itself stands in for Control over there
      key = 'Control';
      code = code === 'MetaRight' ? 'ControlRight' : 'ControlLeft';
      ctrl = true;
      meta = false;
    } else if (meta) {
      ctrl = true;
      meta = false;
    }
    // Option+letter is how a Mac types é, ø, ß…: send the character, not an Alt shortcut
    if (alt && !ctrl && printable(key) && !MODIFIER_KEYS.has(key)) alt = false;
  }
  if (!code) {
    // A key with no physical code: only a typed character can still be sent, as text
    if (down && printable(key) && !ctrl && !meta) {
      return { kind: 'send', messages: [{ action: 'key', type: 'down', key, code: '', text: key, modifiers: e.shiftKey ? MOD.shift : 0, location, auto_repeat: !!e.repeat }] };
    }
    return { kind: 'skip' };
  }
  let modifiers = 0;
  if (alt) modifiers |= MOD.alt;
  if (ctrl) modifiers |= MOD.ctrl;
  if (meta) modifiers |= MOD.meta;
  if (e.shiftKey) modifiers |= MOD.shift;

  const msg: KeyMessage = { action: 'key', type: down ? 'down' : 'up', key, code, modifiers, location, auto_repeat: !!e.repeat };
  if (down && printable(key) && !ctrl && !meta && !alt) msg.text = key;
  if (down && key === 'Enter' && !ctrl && !meta && !alt) msg.text = '\r';
  const copy = down && (ctrl || meta) && !alt && (code === 'KeyC' || code === 'KeyX');
  return { kind: 'send', messages: [msg], copy: copy || undefined };
}

/**
 * Keeps the page's idea of which keys are down in step with ours. A Mac never sends keyup for a
 * key pressed while Cmd is down, so those go up straight after their down; anything still down
 * when focus leaves goes up too.
 */
export function createKeyboard(mac: boolean, send: (msg: KeyMessage | { action: 'copy' }) => void) {
  const down = new Map<string, KeyMessage>();
  return {
    /** Returns whether the local default should be prevented (false only for paste and composition). */
    handle(e: KeyLike): boolean {
      const plan = planKey(e, mac);
      if (plan.kind === 'skip') return false;
      if (plan.kind === 'paste') return false;
      for (const m of plan.messages) {
        if (m.type === 'down') {
          send(m);
          const cmdHeld = mac && e.metaKey && e.key !== 'Meta';
          if (cmdHeld) send({ ...m, type: 'up', text: undefined, auto_repeat: false });
          else if (m.code) down.set(m.code, m);
        } else {
          // Up for a key we never sent down (it went up already, or was pressed before focus came)
          if (!down.has(m.code) && !MODIFIER_KEYS.has(m.key)) continue;
          down.delete(m.code);
          send(m);
        }
      }
      if (plan.copy) send({ action: 'copy' });
      return true;
    },
    /** Focus left the page: let go of everything still held. */
    releaseAll() {
      for (const m of down.values()) send({ ...m, type: 'up', text: undefined, auto_repeat: false, modifiers: 0 });
      down.clear();
    },
    held: () => [...down.keys()],
  };
}

/** Text from a paste or the end of an IME composition, in chunks the server accepts (16 KB). */
export function typeMessages(text: string, max = 16 * 1024): Array<{ action: 'type'; text: string }> {
  if (!text) return [];
  const out: Array<{ action: 'type'; text: string }> = [];
  let chunk = '';
  let bytes = 0;
  const enc = new TextEncoder();
  for (const ch of text) {
    const n = enc.encode(ch).length;
    if (bytes + n > max && chunk) {
      out.push({ action: 'type', text: chunk });
      chunk = '';
      bytes = 0;
    }
    chunk += ch;
    bytes += n;
  }
  if (chunk) out.push({ action: 'type', text: chunk });
  return out;
}

// ---------------------------------------------------------------------------------------------
// Popups the screencast can't show

export interface SelectOption {
  label: string;
  value: string;
  disabled: boolean;
  selected: boolean;
}

export interface SelectRequest {
  options: SelectOption[];
  multiple: boolean;
  rect: { x: number; y: number; width: number; height: number };
  /** The field's label on the page, when the server sends one (the phone sheet's title). */
  label?: string;
}

export interface PageDialog {
  kind: 'alert' | 'confirm' | 'prompt' | 'beforeunload';
  message: string;
  default_prompt: string;
}

/** The page's own file picker, intercepted instead of Chrome drawing it natively. */
export interface FileChooserRequest {
  /** The server's own per-connection counter, not the client's action-correlation id. */
  id: number;
  accept: string;
  multiple: boolean;
}

/** A `file_chooser_error`: `too_big`, `gone` or `failed` (the brief's codes); `error` is ready to show. */
export interface FileChooserFailure {
  code: string;
  error: string;
}

export function parseSelect(m: unknown): SelectRequest | null {
  const v = m as Record<string, unknown> | null;
  if (!v || !Array.isArray(v.options)) return null;
  const options = v.options.map((o: Record<string, unknown>) => ({
    label: String(o?.label ?? o?.value ?? ''),
    value: String(o?.value ?? ''),
    disabled: !!o?.disabled,
    selected: !!o?.selected,
  }));
  const r = (v.rect ?? {}) as Record<string, unknown>;
  const n = (x: unknown) => (typeof x === 'number' && Number.isFinite(x) ? x : 0);
  const label = typeof v.label === 'string' && v.label.trim() ? v.label.trim().slice(0, 120) : undefined;
  return { options, multiple: !!v.multiple, rect: { x: n(r.x), y: n(r.y), width: n(r.width), height: n(r.height) }, ...(label ? { label } : {}) };
}

export function parseDialog(m: unknown): PageDialog | null {
  const v = m as Record<string, unknown> | null;
  if (!v) return null;
  const kinds = ['alert', 'confirm', 'prompt', 'beforeunload'];
  const kind = (kinds.includes(String(v.kind)) ? v.kind : 'alert') as PageDialog['kind'];
  return { kind, message: String(v.message ?? ''), default_prompt: String(v.default_prompt ?? '') };
}

export function parseFileChooser(m: unknown): FileChooserRequest | null {
  const v = m as Record<string, unknown> | null;
  const id = typeof v?.id === 'number' && Number.isFinite(v.id) ? v.id : NaN;
  if (!Number.isFinite(id)) return null;
  return { id, accept: typeof v?.accept === 'string' ? v.accept : '', multiple: v?.multiple === true };
}

export function parseFileChooserFailure(m: unknown): FileChooserFailure | null {
  const v = m as Record<string, unknown> | null;
  if (typeof v?.id !== 'number') return null;
  return { code: typeof v.code === 'string' ? v.code : 'failed', error: typeof v.error === 'string' ? v.error : FILE_CHOOSER_FAILED_TEXT };
}

/** The one text shown for any server-side file-chooser failure that isn't `too_big` or `gone`. */
export const FILE_CHOOSER_FAILED_TEXT = 'Couldn’t send that file to the page.';

const CHOOSER_VIDEO_EXT = /\.(mp4|mov|m4v|webm|avi|mkv)$/i;
const CHOOSER_IMAGE_EXT = /\.(jpg|jpeg|png|gif|webp|heic|avif)$/i;
const isVideoToken = (t: string) => /^video\//i.test(t) || CHOOSER_VIDEO_EXT.test(t);
const isImageToken = (t: string) => /^image\//i.test(t) || CHOOSER_IMAGE_EXT.test(t);

/** Which kind an `accept` string names, by the rule `fileChooserTitle` and the drop handler share: every comma-separated token must agree, or it's `'file'`. */
export function acceptKind(accept: string): 'video' | 'image' | 'file' {
  const tokens = accept.split(',').map((t) => t.trim()).filter(Boolean);
  if (tokens.length > 0 && tokens.every(isVideoToken)) return 'video';
  if (tokens.length > 0 && tokens.every(isImageToken)) return 'image';
  return 'file';
}

/** Does a dropped file match what this chooser's `accept` asks for (by mime type, or the name's extension when there's no mime)? `'file'` takes anything. */
export function fileMatchesAccept(file: { type?: string; name: string }, kind: 'video' | 'image' | 'file'): boolean {
  if (kind === 'file') return true;
  const test = kind === 'video' ? isVideoToken : isImageToken;
  return test(file.type || '') || test(file.name);
}

/** The sheet's title, from the input's own `accept` (every comma-separated token must agree on a kind, so `image/*,video/*` falls back to "file"). */
export function fileChooserTitle(accept: string, multiple = false): string {
  const kind = acceptKind(accept);
  if (kind === 'video') return multiple ? 'Choose videos' : 'Choose a video';
  if (kind === 'image') return multiple ? 'Choose photos' : 'Choose a photo';
  return multiple ? 'Choose files' : 'Choose a file';
}

/** Type-ahead: the first enabled option after `from` (wrapping) whose label starts with `typed`. */
export function matchOption(options: SelectOption[], typed: string, from: number): number {
  const q = typed.toLocaleLowerCase();
  if (!q) return from;
  // Repeating one letter cycles through the options that start with it
  const same = [...q].every((c) => c === q[0]);
  const start = same && q.length > 1 ? from + 1 : q.length === 1 ? from + 1 : from;
  for (let k = 0; k < options.length; k++) {
    const i = (start + k + options.length) % options.length;
    const o = options[i];
    if (o.disabled) continue;
    const label = o.label.trim().toLocaleLowerCase();
    if (same && q.length > 1 ? label.startsWith(q[0]) : label.startsWith(q)) return i;
  }
  return from;
}

/** The next enabled option from `from` going `step` (±1), staying put at the ends. */
export function nextEnabled(options: SelectOption[], from: number, step: 1 | -1): number {
  for (let i = from + step; i >= 0 && i < options.length; i += step) if (!options[i].disabled) return i;
  return from;
}

/** Where the select list goes: under the field, or above it when there's no room below. */
export const POPOVER = { gap: 4, inset: 8, minWidth: 180, maxWidth: 360, flipBelow: 200, maxHeight: 320 } as const;

/**
 * Where a select's list goes in the page area (Mira's spec §4): 4 px under the field, left-aligned,
 * at least 180 and at most 360 wide. It flips above when there's less than 200 px below, and
 * stays 8 px inside the page area.
 */
export function popoverPlacement(anchor: Rect, stage: Size, listHeight: number): { left: number; top: number; width: number; maxHeight: number } {
  const { gap, inset } = POPOVER;
  const width = Math.min(POPOVER.maxWidth, Math.max(anchor.width, POPOVER.minWidth), Math.max(0, stage.width - inset * 2));
  const left = clamp(anchor.left, inset, Math.max(inset, stage.width - inset - width));
  const below = stage.height - inset - (anchor.top + anchor.height + gap);
  const above = anchor.top - gap - inset;
  const want = Math.min(listHeight, POPOVER.maxHeight);
  if (below >= POPOVER.flipBelow || below >= want || below >= above) {
    const maxHeight = Math.max(0, Math.min(POPOVER.maxHeight, below));
    const top = clamp(anchor.top + anchor.height + gap, inset, Math.max(inset, stage.height - inset - Math.min(want, maxHeight)));
    return { left, top, width, maxHeight };
  }
  const maxHeight = Math.max(0, Math.min(POPOVER.maxHeight, above));
  const h = Math.min(want, maxHeight);
  return { left, top: Math.max(inset, anchor.top - gap - h), width, maxHeight };
}
