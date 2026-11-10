// G3 (0.9.0) Point at it: the person points at part of the page the bot is browsing, and the bot
// gets that exact element. Design: release-090/designs/g3-point.md (Mira). Wire: release-090/g3-contract.md
// rev 2 (Vega). Everything here is pure, so the tests can run it without a browser.

/** Every point error carries one of these; route on it, never on the error text. */
export type PointCode = 'nothing' | 'stale' | 'loading' | 'failed' | 'bad_request';
const CODES: ReadonlySet<string> = new Set(['nothing', 'stale', 'loading', 'failed', 'bad_request']);

export interface PointBox {
  x: number;
  y: number;
  width: number;
  height: number;
}

export interface PointFrame {
  host: string;
  cross_site: boolean;
}

/** One element under the pointer, from a `point_at` or `point_neighbour` ack. */
export interface PointEl {
  handle: string;
  label: string;
  selector: string | null;
  box: PointBox;
  viewport: { width: number; height: number };
  text: string;
  frame: PointFrame | null;
  /** `point_neighbour` only: nothing in that direction, so this is the same element. */
  edge: boolean;
}

/** A `point_pick` ack: the crop is already an unsent upload. */
export interface PointPick {
  attachment_id: string;
  picture: boolean;
  url: string;
  host: string;
  path: string;
  label: string;
  selector: string | null;
  box: PointBox;
  viewport: { width: number; height: number };
  frame: PointFrame | null;
  captured_at: string;
  thumb_url: string | null;
}

/** The bubble's `meta.attachments[i].point`. */
export interface PointMeta {
  n: number;
  of: number;
  url: string;
  host: string;
  path: string;
  label: string;
  selector: string | null;
  width: number;
  height: number;
  captured_at: string;
  picture: boolean;
  frame_host: string | null;
}

/** At most 5 picks per message (the server answers a 6th with `too_many_picks`). */
export const MAX_PICKS = 5;
/** Hover hit tests: one per 80 ms, only the newest is drawn (Mira's ENG note). */
export const HOVER_THROTTLE_MS = 80;
/** A selected element is re-read at most this often while frames arrive (`dir:"self"`). */
export const SELF_REFRESH_MS = 250;
/** A point request with no answer by then counts as failed. */
export const POINT_TIMEOUT_MS = 5000;
/** How long the teal "Added …" outline stays (none under reduced motion: it just stays teal). */
export const PICKED_FLASH_MS = 600;
export const FAIL_BAR_MS = 4000;
export const LIMIT_BAR_MS = 3000;

// Copy (Mira's list, all of it)
export const POINT_COPY = {
  button: 'Point',
  buttonName: 'Point at part of the page',
  barDesktop: 'Click the part you mean',
  barPhone: 'Tap the part you mean',
  keys: '↑ wider · Esc stop',
  waiting: 'Waiting for the page…',
  handBack: 'Hand back to point at something',
  failed: "Couldn't read that part of the page. Try again.",
  limit: 'Up to 5 parts per message',
  capturing: 'Taking a picture…',
  noPicture: 'No picture · the code was added',
  wider: 'Wider',
  add: 'Add to message',
  added: 'Added to your message',
  keep: 'Keep pointing',
  back: 'Back to chat',
  viewName: 'Page view. Arrows choose a part, Enter adds it, Escape stops',
} as const;

export const footerText = (n: number) => `Pointing · ${n} picked · Esc to stop`;
export const addedText = (label: string) => `Added ${label}`;
export const embeddedText = (host: string) => `Embedded page from ${host}`;
export const removeText = (name: string) => `Remove ${name}`;
export const pickedCountText = (n: number) =>
  n === 1 ? '1 part picked. Tap another to add it, or go back and write what to change.' : `${n} parts picked. Tap another to add it, or go back and write what to change.`;

/** An error the point actions answered, or `failed` for a timeout, a close or no socket. */
export class PointError extends Error {
  constructor(readonly code: PointCode | 'unsupported', message?: string) {
    super(message || code);
    this.name = 'PointError';
  }
}

/** The code on a point error; anything unknown (an older server) reads as failed. */
export function pointErrorCode(message: { code?: unknown; error?: unknown }): PointCode | 'unsupported' {
  if (typeof message.code === 'string' && CODES.has(message.code)) return message.code as PointCode;
  // A server from before G3 refuses the new actions as bad input: hide Point, don't log a bug
  if (message.code === undefined && /invalid browser input/i.test(String(message.error ?? ''))) return 'unsupported';
  return 'failed';
}

const num = (v: unknown): number | null => (typeof v === 'number' && Number.isFinite(v) ? v : null);
const str = (v: unknown, max = 2000): string | null => (typeof v === 'string' ? v.slice(0, max) : null);

function parseBox(v: unknown): PointBox | null {
  const b = v as Record<string, unknown> | null;
  if (!b || typeof b !== 'object') return null;
  const x = num(b.x), y = num(b.y), width = num(b.width), height = num(b.height);
  if (x === null || y === null || width === null || height === null || width < 0 || height < 0) return null;
  return { x, y, width, height };
}

function parseViewport(v: unknown): { width: number; height: number } | null {
  const b = v as Record<string, unknown> | null;
  const width = num(b?.width), height = num(b?.height);
  return width && height && width > 0 && height > 0 ? { width, height } : null;
}

function parseFrame(v: unknown): PointFrame | null {
  const f = v as Record<string, unknown> | null;
  if (!f || typeof f !== 'object' || typeof f.host !== 'string') return null;
  return { host: f.host.slice(0, 253), cross_site: f.cross_site === true };
}

/** An `el` from an ack, or null when it isn't one (then the reply is ignored, never drawn). */
export function parsePointEl(v: unknown, edge?: unknown): PointEl | null {
  const e = v as Record<string, unknown> | null;
  if (!e || typeof e !== 'object') return null;
  const handle = str(e.handle, 40);
  const label = str(e.label, 80);
  const box = parseBox(e.box);
  if (!handle || label === null || !box) return null;
  return {
    handle,
    label,
    selector: str(e.selector, 300) || null,
    box,
    viewport: parseViewport(e.viewport) ?? { width: 0, height: 0 },
    text: str(e.text, 80) ?? '',
    frame: parseFrame(e.frame),
    edge: edge === true,
  };
}

/** A `pick` from a `point_pick` ack, or null. */
export function parsePointPick(v: unknown): PointPick | null {
  const p = v as Record<string, unknown> | null;
  if (!p || typeof p !== 'object') return null;
  const attachment_id = str(p.attachment_id, 64);
  const box = parseBox(p.box);
  if (!attachment_id || !box) return null;
  const thumb = str(p.thumb_url, 300);
  return {
    attachment_id,
    picture: p.picture !== false,
    url: str(p.url) ?? '',
    host: str(p.host, 253) ?? '',
    path: str(p.path) ?? '',
    label: str(p.label, 80) ?? '',
    selector: str(p.selector, 300) || null,
    box,
    viewport: parseViewport(p.viewport) ?? { width: 0, height: 0 },
    frame: parseFrame(p.frame),
    captured_at: str(p.captured_at, 40) ?? '',
    // Only our own attachment route: a URL from the wire is never loaded from anywhere else
    thumb_url: thumb && thumb.startsWith('/api/attachments/') ? thumb : null,
  };
}

/** The bubble's `point` object, or null when malformed (then it's an ordinary file tile). */
export function parsePointMeta(v: unknown): PointMeta | null {
  const p = v as Record<string, unknown> | null;
  if (!p || typeof p !== 'object') return null;
  const n = num(p.n), of = num(p.of), width = num(p.width), height = num(p.height);
  if (n === null || of === null || width === null || height === null) return null;
  return {
    n,
    of,
    url: str(p.url) ?? '',
    host: str(p.host, 253) ?? '',
    path: str(p.path) ?? '',
    label: str(p.label, 80) ?? '',
    selector: str(p.selector, 300) || null,
    width,
    height,
    captured_at: str(p.captured_at, 40) ?? '',
    picture: p.picture !== false,
    frame_host: str(p.frame_host, 253) || null,
  };
}

/** The bubble's `point` for a pick, as the server will write it (n/of from the picks' order). */
export function pointMetaOf(pick: PointPick, n: number, of: number): PointMeta {
  return {
    n,
    of,
    url: pick.url,
    host: pick.host,
    path: pick.path,
    label: pick.label,
    selector: pick.selector,
    width: pick.box.width,
    height: pick.box.height,
    captured_at: pick.captured_at,
    picture: pick.picture,
    frame_host: pick.frame?.cross_site ? pick.frame.host : null,
  };
}

/** What the outline's tag says: `button.pay 328 × 36`, or `iframe pay.example.net` for a cross-site frame. */
export function tagText(el: Pick<PointEl, 'label' | 'box' | 'frame'>): string {
  if (el.frame?.cross_site) return `iframe ${el.frame.host}`;
  return `${el.label} ${Math.round(el.box.width)} × ${Math.round(el.box.height)}`;
}

/** The live region's line: "button.pay, 328 by 36, Pay €42.00". */
export function spokenText(el: Pick<PointEl, 'label' | 'box' | 'text' | 'frame'>): string {
  const name = el.frame?.cross_site ? `iframe ${el.frame.host}` : el.label;
  const parts = [name, `${Math.round(el.box.width)} by ${Math.round(el.box.height)}`];
  if (el.text) parts.push(el.text);
  return parts.join(', ');
}

/** The chip's and the row's first line: the selector, or the label when nothing unique was built. */
export const pickName = (p: { selector: string | null; label: string }) => p.selector || p.label;

/** The chip's second line. */
export function chipLine(p: { picture: boolean; host: string; path: string; frame_host?: string | null; frame?: PointFrame | null }): string {
  const frameHost = p.frame_host ?? (p.frame?.cross_site ? p.frame.host : null);
  if (frameHost) return embeddedText(frameHost);
  if (!p.picture) return POINT_COPY.noPicture;
  return `${p.host}${p.path}`;
}

/** HH:MM in the viewer's local time, or '' for a bad time. */
export function clockText(at: string, locale?: string): string {
  const t = at ? new Date(at) : null;
  if (!t || Number.isNaN(t.getTime())) return '';
  return t.toLocaleTimeString(locale ?? [], { hour: '2-digit', minute: '2-digit', hour12: false });
}

/** The bubble row's second line: `{host}{path} · W × H · HH:MM`. */
export function rowLine(p: PointMeta, locale?: string): string {
  const where = p.frame_host ? embeddedText(p.frame_host) : `${p.host}${p.path}`;
  return [where, `${Math.round(p.width)} × ${Math.round(p.height)}`, clockText(p.captured_at, locale)].filter(Boolean).join(' · ');
}

/** Where the page image sits inside the view (object-contain): the scale and the letterbox offsets. */
export interface Fit {
  scale: number;
  left: number;
  top: number;
}

export function containFit(view: { width: number; height: number }, page: { width: number; height: number }): Fit | null {
  if (!(view.width > 0) || !(view.height > 0) || !(page.width > 0) || !(page.height > 0)) return null;
  const scale = Math.min(view.width / page.width, view.height / page.height);
  return { scale, left: (view.width - page.width * scale) / 2, top: (view.height - page.height * scale) / 2 };
}

/** A point in the view (relative to its top left) in page CSS px, or null in the letterbox. */
export function toPage(fit: Fit, page: { width: number; height: number }, x: number, y: number): { x: number; y: number } | null {
  const px = (x - fit.left) / fit.scale;
  const py = (y - fit.top) / fit.scale;
  if (px < 0 || py < 0 || px > page.width || py > page.height) return null;
  return { x: Math.round(px * 10) / 10, y: Math.round(py * 10) / 10 };
}

/** A page box drawn in the view, clipped to the page picture. */
export function toView(fit: Fit, page: { width: number; height: number }, box: PointBox): PointBox {
  const x0 = Math.max(0, box.x), y0 = Math.max(0, box.y);
  const x1 = Math.min(page.width, box.x + box.width), y1 = Math.min(page.height, box.y + box.height);
  return {
    x: fit.left + x0 * fit.scale,
    y: fit.top + y0 * fit.scale,
    width: Math.max(0, x1 - x0) * fit.scale,
    height: Math.max(0, y1 - y0) * fit.scale,
  };
}

/** The tag goes above the outline, or below it when there's no room above (inside it as a last resort). */
export function tagPlacement(outline: PointBox, viewHeight: number, tagHeight = 18): 'above' | 'below' | 'inside' {
  if (outline.y >= tagHeight + 2) return 'above';
  if (outline.y + outline.height + tagHeight + 2 <= viewHeight) return 'below';
  return 'inside';
}

/** The arrow keys' directions. */
export const ARROW_DIRS: Record<string, 'parent' | 'child' | 'prev' | 'next'> = {
  ArrowUp: 'parent',
  ArrowDown: 'child',
  ArrowLeft: 'prev',
  ArrowRight: 'next',
};

/** Why another pick can't be added now, or null. */
export function pickRefusal(picks: number, filesLeft: number): string | null {
  if (picks >= MAX_PICKS) return POINT_COPY.limit;
  if (filesLeft <= 0) return 'Up to 10 files per message';
  return null;
}
