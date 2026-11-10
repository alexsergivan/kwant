// 0.8.0 drag and drop into chats (Mira's design, release-080/drag-drop/design.md). What a file drag
// carries, read while it is still in the air; the words the drop card shows; what a drop brought; and
// the window-level watch that knows a file drag is over the window at all.
import { MAX_ATTACHMENTS, isArchiveMime, previewRefusesMime } from './attachments';

/** One dragged item as the browser shows it before the drop: its kind and MIME type, never its name. */
export interface DragItemLike {
  kind: string;
  type: string;
}

export type DragKind = 'image' | 'video' | 'pdf' | 'text' | 'file' | 'refused';

const IMAGE = new Set(['image/png', 'image/jpeg', 'image/jpg', 'image/gif', 'image/webp']);
const VIDEO = new Set(['video/mp4', 'video/quicktime', 'video/webm']);

const mimeOf = (type: string) => (type || '').toLowerCase().split(';')[0].trim();

/**
 * The kind of a dragged file from its MIME alone (design §4). Anything that isn't surely refused and
 * isn't a known image, video, PDF or text type is a plain "file": an empty type, a folder, a `.ts`
 * file the browser calls `video/mp2t`, an SVG. The drop decides those by name.
 */
export function dragKind(type: string): DragKind {
  const mime = mimeOf(type);
  if (previewRefusesMime(mime)) return 'refused';
  if (IMAGE.has(mime)) return 'image';
  if (VIDEO.has(mime)) return 'video';
  if (mime === 'application/pdf') return 'pdf';
  if (mime.startsWith('text/') || mime === 'application/json') return 'text';
  return 'file';
}

/** True when a drag carries files (as opposed to text or a link), from `dataTransfer.types`. */
export function dragHasFiles(dt: { types?: ArrayLike<string> | readonly string[] | null } | null | undefined): boolean {
  if (!dt?.types) return false;
  return Array.from(dt.types as ArrayLike<string>).includes('Files');
}

export interface DragPreview {
  /** Files in the drag; null when the browser shows none yet (some WebKit builds during dragover). */
  count: number | null;
  /** Files that will land, by kind (never 'refused'). */
  kinds: Record<Exclude<DragKind, 'refused'>, number>;
  /** Files that will land: what `addFiles` keeps (the first `slotsLeft`, less the refused among them). */
  go: number;
  /** Files that won't fit in the tray (and aren't refused anyway). */
  overBy: number;
  /** MIME types of the files that surely can't go. */
  refused: string[];
  /** Files already in the tray. */
  inTray: number;
  slotsLeft: number;
}

/** Copies what a drag shows into plain objects; `DataTransferItemList` can't be read later. */
export const dragItemsOf = (items: ArrayLike<DragItemLike> | null | undefined): DragItemLike[] =>
  Array.from(items ?? [])
    .filter((i) => i?.kind === 'file')
    .map((i) => ({ kind: 'file', type: i.type || '' }));

/**
 * What a file drag holds, during dragenter or dragover. Mirrors `addFiles`: it keeps the first
 * `slotsLeft` files, and the refused ones among those still take a slot.
 */
export function previewDrag(items: ArrayLike<DragItemLike> | null | undefined, inTray = 0): DragPreview {
  const list = dragItemsOf(items);
  const slotsLeft = Math.max(0, MAX_ATTACHMENTS - inTray);
  const kinds = { image: 0, video: 0, pdf: 0, text: 0, file: 0 };
  const refused: string[] = [];
  let go = 0;
  let overBy = 0;
  list.forEach((item, i) => {
    const k = dragKind(item.type);
    if (k === 'refused') refused.push(mimeOf(item.type));
    else if (i >= slotsLeft) overBy += 1;
    else {
      go += 1;
      kinds[k] += 1;
    }
  });
  return { count: list.length || null, kinds, go, overBy, refused, inTray, slotsLeft };
}

const plural = (n: number, one: string, many = `${one}s`) => `${n} ${n === 1 ? one : many}`;
const KIND_WORDS: Record<Exclude<DragKind, 'refused'>, [string, string]> = {
  image: ['image', 'images'],
  video: ['video', 'videos'],
  pdf: ['PDF', 'PDFs'],
  text: ['text file', 'text files'],
  file: ['file', 'files'],
};

export interface KindPart {
  kind: Exclude<DragKind, 'refused'> | 'other';
  label: string;
}

/** The kinds for the what line: at most 3, biggest first; the rest fold into "{n} other". */
export function kindParts(p: DragPreview): KindPart[] {
  const order: Array<Exclude<DragKind, 'refused'>> = ['image', 'video', 'pdf', 'text', 'file'];
  const groups = order.filter((k) => p.kinds[k] > 0).sort((a, b) => p.kinds[b] - p.kinds[a]);
  const shown = groups.slice(0, 3).map((k) => ({ kind: k, label: plural(p.kinds[k], ...KIND_WORDS[k]) }));
  const rest = groups.slice(3).reduce((n, k) => n + p.kinds[k], 0);
  return rest ? [...shown, { kind: 'other', label: `${rest} other` }] : shown;
}

/** "3 files · 2 images, 1 PDF" (the what line as plain text). */
export function whatLine(p: DragPreview): string | null {
  if (p.count === null || !p.go) return null;
  return `${plural(p.go, 'file')} · ${kindParts(p)
    .map((k) => k.label)
    .join(', ')}`;
}

/** Who the files go to: from `answerTarget()`, so the card and the "Answers:" hint never disagree. */
export interface DropRecipient {
  /** bot: a bot's own chat or an @mention. general/team/channel: a room. */
  kind: 'bot' | 'general' | 'team' | 'channel';
  /** "Frontend", "General", "Kitchen" (team name without "team"), "#launch". */
  name: string;
}

/** The heading in three parts, so the name can be drawn in the accent. */
export interface Heading {
  pre: string;
  name: string;
  post: string;
}

export const headingText = (h: Heading) => `${h.pre}${h.name}${h.post}`;

/** "Drop to give Frontend these files" (B, C) or "Drop here to give…" (A). */
export function dropHeading(r: DropRecipient, count: number | null, armed = false): Heading {
  const here = armed ? 'Drop here to ' : 'Drop to ';
  if (r.kind === 'bot') return { pre: `${here}give `, name: r.name, post: count === null ? ' your files' : count === 1 ? ' this file' : ' these files' };
  if (r.kind === 'general') return { pre: `${here}share in `, name: r.name, post: '' };
  if (r.kind === 'team') return { pre: `${here}share with the `, name: r.name, post: ' team' };
  return { pre: `${here}share with `, name: r.name, post: '' };
}

/** The recipient inside a sentence: "Frontend", "General", "the Kitchen team". */
const nameIn = (r: DropRecipient) => (r.kind === 'team' ? `the ${r.name} team` : r.name);

const WORKS = 'Images, videos, PDFs and text or code files work.';

/** How a refused MIME is named when all refused files share it. */
function refusedWords(mime: string): { one: string; many: string; group: string } | null {
  if (mime === 'application/zip' || mime === 'application/x-zip-compressed') return { one: 'ZIP', many: 'ZIPs', group: 'ZIP files' };
  if (isArchiveMime(mime)) return { one: 'archive', many: 'archives', group: 'Archives' };
  if (mime.startsWith('audio/')) return { one: 'audio file', many: 'audio files', group: 'Audio files' };
  if (mime.startsWith('font/')) return { one: 'font', many: 'fonts', group: 'Fonts' };
  if (mime === 'application/msword' || /^application\/vnd\.(ms-|openxmlformats-officedocument\.|oasis\.opendocument\.)/.test(mime))
    return { one: 'Office file', many: 'Office files', group: 'Office files' };
  return null;
}

/** The kind's words when every refused file has the same one, else null. */
function sameRefused(mimes: string[]) {
  const words = mimes.map(refusedWords);
  const first = words[0];
  return first && words.every((w) => w?.one === first.one) ? first : null;
}

export interface Refusal {
  heading: Heading;
  line1: string;
  line2?: string;
}

export interface DropCardModel {
  /** over: at least one file will go (B, C). refused: nothing will (D). */
  state: 'over' | 'refused';
  heading: Heading;
  armedHeading: Heading;
  /** Files that will go, for the badge; null hides it. */
  count: number | null;
  kinds: KindPart[];
  what: string | null;
  /** The won't-go rows (C), at most 2. */
  wont: string[];
  refusal: Refusal | null;
  /** What a screen reader hears when this state starts. */
  announce: string;
}

export interface DropCardInput {
  preview: DragPreview;
  recipient: DropRecipient;
  /** The field holds `/idea …`: files can't go in the jar. */
  ideaMode?: boolean;
  /** The composer can't take files right now; its own reason. */
  disabledReason?: string | null;
}

/** Everything the drop card says, for one drag over one chat (design §2 B–D, §5). */
export function dropCard({ preview: p, recipient: r, ideaMode, disabledReason }: DropCardInput): DropCardModel {
  const refusal = refusalOf(p, r, ideaMode, disabledReason);
  if (refusal) {
    return {
      state: 'refused',
      heading: refusal.heading,
      armedHeading: refusal.heading,
      count: null,
      kinds: [],
      what: null,
      wont: [],
      refusal,
      announce: `${headingText(refusal.heading)}. ${refusal.line1}`,
    };
  }
  const count = p.count === null ? null : p.go;
  const wont: string[] = [];
  if (p.overBy) {
    const already = p.inTray ? `, ${p.inTray} ${p.inTray === 1 ? 'is' : 'are'} already in` : '';
    wont.push(`${p.overBy} more won't fit: ${MAX_ATTACHMENTS} files per message${already}.`);
  }
  if (p.refused.length) {
    const same = sameRefused(p.refused);
    const what = same ? (p.refused.length === 1 ? same.one : same.many) : p.refused.length === 1 ? 'file' : 'files';
    wont.push(`${p.refused.length} ${what} can't go. ${WORKS}`);
  }
  const kinds = count === null ? [] : kindParts(p);
  const what = whatLine(p);
  const heading = dropHeading(r, count);
  let announce: string;
  if (count === null) announce = `${headingText(heading)}.`;
  else {
    const files = plural(count, 'file');
    const lead = r.kind === 'bot' ? `Drop to give ${r.name} ${files}` : `Drop to share ${files} ${r.kind === 'general' ? 'in' : 'with'} ${nameIn(r)}`;
    announce = `${lead}: ${kinds.map((k) => k.label).join(', ')}.`;
  }
  const short: string[] = [];
  if (p.overBy) short.push(`${p.overBy} more won't fit.`);
  if (p.refused.length) {
    const same = sameRefused(p.refused);
    short.push(`${p.refused.length} ${same ? (p.refused.length === 1 ? same.one : same.many) : p.refused.length === 1 ? 'file' : 'files'} can't go.`);
  }
  if (short.length) announce += ` ${short.join(' ')}`;
  return { state: 'over', heading, armedHeading: dropHeading(r, count, true), count, kinds, what, wont: wont.slice(0, 2), refusal: null, announce };
}

/** Why nothing in this drag can go, in the design's order (§2 D), or null when something will. */
function refusalOf(p: DragPreview, r: DropRecipient, ideaMode?: boolean, disabledReason?: string | null): Refusal | null {
  const who = nameIn(r);
  if (ideaMode)
    return { heading: { pre: "Files can't go in the idea jar", name: '', post: '' }, line1: 'Ideas are text only.', line2: `Clear /idea from the field to send them to ${who}.` };
  if (disabledReason) return { heading: { pre: "Can't drop files here right now", name: '', post: '' }, line1: disabledReason };
  if (p.slotsLeft === 0) return { heading: { pre: 'The tray is full', name: '', post: '' }, line1: `${MAX_ATTACHMENTS} files per message.`, line2: 'Send these first, then drop the rest.' };
  if (p.count !== null && p.go === 0 && p.overBy === 0 && p.refused.length) {
    const n = p.refused.length;
    const same = sameRefused(p.refused);
    const archives = p.refused.every(isArchiveMime);
    return {
      heading: { pre: n === 1 ? "This can't go to " : "These can't go to ", name: who, post: '' },
      line1: same ? `${same.group} can't be attached.` : n === 1 ? "This file can't be attached." : "These files can't be attached.",
      line2: archives ? `${WORKS} ${n === 1 ? 'Unzip it' : 'Unzip them'} and drop the files inside.` : WORKS,
    };
  }
  return null;
}

/** One dropped item: a `DataTransferItem` whose entry tells a folder from a file. */
interface DropItemLike extends DragItemLike {
  getAsFile(): File | null;
  webkitGetAsEntry?: () => { isDirectory: boolean; name: string } | null;
}

export interface DroppedFiles {
  files: File[];
  /** Names of folders in the drop; kwant takes files only. */
  folders: string[];
}

/**
 * Splits a drop into files and folders. Call it inside the drop event: the items are empty after.
 * Without items (or without `webkitGetAsEntry`) every entry of `files` counts as a file.
 */
export function splitDrop(items: ArrayLike<DropItemLike> | null | undefined, files?: ArrayLike<File> | null): DroppedFiles {
  const list = Array.from(items ?? []).filter((i) => i?.kind === 'file');
  if (!list.length) return { files: Array.from(files ?? []), folders: [] };
  const out: DroppedFiles = { files: [], folders: [] };
  for (const item of list) {
    const entry = item.webkitGetAsEntry?.() ?? null;
    if (entry?.isDirectory) {
      out.folders.push(entry.name);
      continue;
    }
    const f = item.getAsFile();
    if (f) out.files.push(f);
  }
  return out;
}

/** The note shown (and read out) when a drop held folders (design §2 E5). */
export const FOLDER_NOTE = "Folders can't go: drop the files inside.";

/** What a screen reader hears after a drop: "3 files added for Frontend. Uploading." */
export function dropAnnouncement(added: number, r?: DropRecipient): string {
  if (!added) return 'No files added.';
  return `${plural(added, 'file')} added${r ? ` for ${nameIn(r)}` : ''}. Uploading.`;
}

export const PILL_TEXT = 'Open a chat to drop files there';

/** A drag or drop event as the window guard sees it. */
interface GuardEvent {
  type: string;
  defaultPrevented: boolean;
  preventDefault(): void;
  dataTransfer: { types?: ArrayLike<string> | null; dropEffect?: string } | null;
}

/**
 * The window-level guard: a file drag that no drop target took (nobody called preventDefault) must
 * not open the file in place of kwant. Shows the "no drop" cursor and swallows the drop. Text, links
 * and drags a target already handled pass untouched. Returns true when it stepped in.
 */
export function guardFileDrag(e: GuardEvent): boolean {
  if (e.defaultPrevented || !dragHasFiles(e.dataTransfer)) return false;
  e.preventDefault();
  if (e.type === 'dragover' && e.dataTransfer) e.dataTransfer.dropEffect = 'none';
  return true;
}

/** Puts the guard on a window (bubble phase, so drop targets act first). Returns the cleanup. */
export function installFileDropGuard(win: Pick<Window, 'addEventListener' | 'removeEventListener'>): () => void {
  const on = (e: Event) => void guardFileDrag(e as unknown as GuardEvent);
  win.addEventListener('dragover', on);
  win.addEventListener('drop', on);
  return () => {
    win.removeEventListener('dragover', on);
    win.removeEventListener('drop', on);
  };
}

// ---------- the window watch: is a file drag over the window? ----------

export interface WindowDrag {
  /** A file drag is over the window. */
  active: boolean;
  /** What it carries, as last seen (empty when the browser showed nothing). */
  items: DragItemLike[];
  /** How many open chats can take a drop right now (ChatView registers); 0 shows the pill. */
  chats: number;
}

type Win = Pick<Window, 'addEventListener' | 'removeEventListener' | 'setTimeout' | 'clearTimeout'>;

/**
 * Watches a window for file drags (design §6.1–6.2, Ada's #478 M2). It ends the drag when:
 * - the enter/leave depth counter falls to 0;
 * - a dragleave has no relatedTarget (the pointer left the window), unless a dragenter came in the
 *   same task (WebKit gives no relatedTarget when crossing from one element to the next, and fires
 *   the new element's dragenter just before the old one's dragleave);
 * - the first mousemove or pointermove arrives: mouse events don't fire during an OS drag, so one
 *   arriving means the drag is over (this catches a dragleave WebKit missed on a fast exit);
 * - Esc or dragend;
 * - drop, one task later: the watch listens in the capture phase, and ending at once re-renders the
 *   chat in the microtask checkpoint that follows this listener, which unmounts the drop layer under
 *   the pointer before the drop reaches it (0.8.2: a real OS drop then never reached the chat). A drop
 *   target that takes the drop calls `end()` itself;
 * - and, as a last fallback, no dragover for `watchdogMs` (1500 ms). WebKit, the Mac app's
 *   WKWebView included, sends dragover mostly when the pointer moves, so a hand held still must
 *   not end it; a shorter watchdog made the layer vanish and blink back.
 * A dragover after the drag ended starts it again.
 */
export function createDragWatch(win: Win, watchdogMs = 1500) {
  let state: WindowDrag = { active: false, items: [], chats: 0 };
  let depth = 0;
  let timer: ReturnType<typeof setTimeout> | null = null;
  let dropTimer: ReturnType<typeof setTimeout> | null = null;
  // A dragenter came in this task: a null-relatedTarget dragleave right after it is a crossing, not an exit.
  let justEntered = false;
  const subs = new Set<() => void>();
  const set = (next: Partial<WindowDrag>) => {
    state = { ...state, ...next };
    subs.forEach((f) => f());
  };
  const arm = () => {
    if (timer) win.clearTimeout(timer);
    timer = win.setTimeout(end, watchdogMs);
  };
  function end() {
    depth = 0;
    if (timer) win.clearTimeout(timer);
    timer = null;
    if (dropTimer) win.clearTimeout(dropTimer);
    dropTimer = null;
    if (state.active) set({ active: false, items: [] });
  }
  const take = (e: DragEvent) => {
    const items = dragItemsOf(e.dataTransfer?.items as unknown as ArrayLike<DragItemLike>);
    if (!state.active) set({ active: true, items });
    else if (items.length && items.length !== state.items.length) set({ items });
  };
  const onEnter = (e: Event) => {
    const d = e as DragEvent;
    if (!dragHasFiles(d.dataTransfer)) return;
    depth += 1;
    justEntered = true;
    win.setTimeout(() => {
      justEntered = false;
    }, 0);
    take(d);
    arm();
  };
  const onOver = (e: Event) => {
    const d = e as DragEvent;
    if (!dragHasFiles(d.dataTransfer)) return;
    if (!state.active) depth = Math.max(depth, 1);
    take(d);
    arm();
  };
  const onLeave = (e: Event) => {
    const d = e as DragEvent;
    if (!dragHasFiles(d.dataTransfer)) return;
    depth = Math.max(0, depth - 1);
    if (depth === 0 || (d.relatedTarget == null && !justEntered)) end();
  };
  // No mouse events during an OS drag: one arriving means the drag is over.
  const onMove = () => {
    if (state.active) end();
  };
  // Drop targets get the drop first (see above); the watch ends the drag one task later.
  const onDrop = () => {
    if (dropTimer) win.clearTimeout(dropTimer);
    dropTimer = win.setTimeout(end, 0);
  };
  const onKey = (e: Event) => {
    if ((e as KeyboardEvent).key === 'Escape' && state.active) end();
  };
  const listen: Array<[string, (e: Event) => void]> = [
    ['dragenter', onEnter],
    ['dragover', onOver],
    ['dragleave', onLeave],
    ['drop', onDrop],
    ['dragend', end],
    ['keydown', onKey],
    ['mousemove', onMove],
    ['pointermove', onMove],
  ];
  let installed = false;
  const install = () => {
    if (installed) return;
    installed = true;
    listen.forEach(([t, f]) => win.addEventListener(t, f, true));
  };
  const uninstall = () => {
    if (!installed) return;
    installed = false;
    listen.forEach(([t, f]) => win.removeEventListener(t, f, true));
    end();
  };
  return {
    get: () => state,
    /** Subscribes; the first subscriber puts the listeners on the window, the last takes them off. */
    subscribe(f: () => void) {
      subs.add(f);
      install();
      return () => {
        subs.delete(f);
        if (!subs.size) uninstall();
      };
    },
    /** An open chat that takes drops; returns the cleanup. */
    registerChat() {
      set({ chats: state.chats + 1 });
      return () => set({ chats: Math.max(0, state.chats - 1) });
    },
    end,
  };
}

export type DragWatch = ReturnType<typeof createDragWatch>;

let shared: DragWatch | null = null;
/** The one watch on this window, made on first use. */
export const windowDragWatch = (): DragWatch =>
  (shared ??= createDragWatch(
    typeof window !== 'undefined'
      ? window
      : // Rendered to a string (tests, no window): nothing to watch.
        { addEventListener() {}, removeEventListener() {}, setTimeout: (f: () => void, ms?: number) => setTimeout(f, ms) as unknown as number, clearTimeout: (t?: number) => clearTimeout(t) },
  ));

/**
 * True when a modal dialog is open: the chat's drop layer never shows over one (design §1). The
 * phone's full-screen Settings is a modal too, but it's a page, not a dialog with its own drop zone:
 * it gets the "Open a chat" pill like any page without a chat (#478 L4).
 */
export const modalOpen = (doc: Document = document) =>
  !!doc.querySelector('[aria-modal="true"]:not([data-space-settings]), [role="alertdialog"], dialog[open]');

/**
 * How long a drop layer or pill stays mounted for its leave fade: 100 ms, or 120 ms with reduced
 * motion, where dropLayer.css fades it in 120 ms (#478 L7).
 */
export const leaveMs = (reduce: boolean = typeof window !== 'undefined' && !!window.matchMedia?.('(prefers-reduced-motion: reduce)').matches) =>
  reduce ? 120 : 100;
