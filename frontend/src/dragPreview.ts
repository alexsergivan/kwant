// 0.8.0 drag and drop into chats. What a file drag carries, read while it is still in the air, and
// what a drop actually brought. Pure functions: the chat pane and the window guard call these.
import { MAX_ATTACHMENTS, attachKind } from './attachments';

/** One dragged item as the browser shows it before the drop: its kind and MIME type, never its name. */
export interface DragItemLike {
  kind: string;
  type: string;
}

export type DragKind = 'image' | 'video' | 'pdf' | 'text' | 'refused' | 'unknown';

export interface DragPreview {
  /** Files in the drag (strings such as a dragged link or text are not counted). */
  count: number;
  kinds: Record<DragKind, number>;
  /** Room left in the tray for this message. */
  slotsLeft: number;
  /** How many won't fit: 0 when they all do. */
  overBy: number;
  /** Files that would be kept, at most `slotsLeft`. */
  fits: number;
}

const EMPTY_KINDS = (): Record<DragKind, number> => ({ image: 0, video: 0, pdf: 0, text: 0, refused: 0, unknown: 0 });

/**
 * The kind of a dragged file from its MIME type alone. An empty type is `unknown`: the browser can't
 * tell a folder, a code file or a format it doesn't know apart until the drop. `video/mp2t` is a `.ts`
 * file just as often as a video, so it waits for the name too.
 */
export function dragKind(type: string): DragKind {
  const mime = (type || '').toLowerCase().split(';')[0].trim();
  if (!mime || mime === 'application/octet-stream' || mime === 'video/mp2t') return 'unknown';
  const k = attachKind({ name: '', type: mime });
  return k ?? 'refused';
}

/** True when a drag carries files (as opposed to text or a link), from `dataTransfer.types`. */
export function dragHasFiles(dt: { types?: ArrayLike<string> | readonly string[] | null } | null | undefined): boolean {
  if (!dt?.types) return false;
  return Array.from(dt.types as ArrayLike<string>).includes('Files');
}

/**
 * What a file drag holds, during dragenter or dragover. `items` is `dataTransfer.items` (types only);
 * `inTray` is how many files the message already has.
 */
export function previewDrag(items: ArrayLike<DragItemLike> | null | undefined, inTray = 0): DragPreview {
  const kinds = EMPTY_KINDS();
  let count = 0;
  for (const item of Array.from(items ?? [])) {
    if (item?.kind !== 'file') continue;
    count += 1;
    kinds[dragKind(item.type)] += 1;
  }
  const slotsLeft = Math.max(0, MAX_ATTACHMENTS - inTray);
  const fits = Math.min(count, slotsLeft);
  return { count, kinds, slotsLeft, overBy: count - fits, fits };
}

const plural = (n: number, one: string, many = `${one}s`) => `${n} ${n === 1 ? one : many}`;

/**
 * A plain line for the placeholder overlay ("3 files · 2 images, 1 PDF"). Mira's design replaces the
 * wording; the counts stay.
 */
export function dragSummary(p: DragPreview): string {
  if (!p.count) return 'Drop files to attach';
  const parts = [
    p.kinds.image && plural(p.kinds.image, 'image'),
    p.kinds.video && plural(p.kinds.video, 'video'),
    p.kinds.pdf && plural(p.kinds.pdf, 'PDF'),
    p.kinds.text && plural(p.kinds.text, 'text file'),
    p.kinds.refused && `${p.kinds.refused} that can't be attached`,
  ].filter(Boolean);
  return parts.length ? `${plural(p.count, 'file')} · ${parts.join(', ')}` : plural(p.count, 'file');
}

/** The "won't fit" line before the drop, or null. */
export function dragWarning(p: DragPreview): string | null {
  if (!p.count) return null;
  if (p.slotsLeft === 0) return `This message already has ${MAX_ATTACHMENTS} files.`;
  if (p.overBy) return `Only ${plural(p.fits, 'file')} fit${p.fits === 1 ? 's' : ''}: up to ${MAX_ATTACHMENTS} per message.`;
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

/** The note shown (and read out) when a drop held folders. */
export const folderNote = (folders: string[]) =>
  folders.length === 1 ? `${folders[0]} is a folder. Folders can't go: drop the files inside.` : `Folders can't go: drop the files inside.`;

/** What a screen reader hears after a drop. */
export function dropAnnouncement(added: number): string {
  return added ? `${plural(added, 'file')} added` : 'No files added';
}

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
