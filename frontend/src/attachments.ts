// File attachments in chats: uploads from the composer and files agents share (batch 2 / D).
import { parseMetaObject } from './questions';

export const MAX_ATTACHMENT_BYTES = 20 * 1024 * 1024;
/** Videos (mp4, mov, webm) may be bigger; the server enforces the same caps by the file's bytes. */
export const MAX_VIDEO_BYTES = 100 * 1024 * 1024;
export const MAX_ATTACHMENTS = 10;

/** What a message's `meta.attachments` holds. */
export interface AttachmentRef {
  id: string;
  name: string;
  mime: string;
  size: number;
}

export interface Attachment extends AttachmentRef {
  sha256: string;
  source: 'user' | 'agent';
  kwant_id: string | null;
  context_id: string | null;
  message_id: string | null;
  created_at: string;
  url: string;
}

const INLINE_IMAGES = new Set(['image/png', 'image/jpeg', 'image/gif', 'image/webp']);
const UUID_RE = /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/i;

/** Only these render inline (the server serves everything else as a download). */
export const isInlineImage = (mime: string) => INLINE_IMAGES.has(mime.toLowerCase());

/** Sent videos the chat tries to play (P2); a browser that can't falls back to the file card. */
export const isVideo = (mime: string | undefined) => VIDEO_MIMES.has((mime || '').toLowerCase().split(';')[0].trim());

export const attachmentUrl = (id: string, download = false) =>
  `/api/attachments/${encodeURIComponent(id)}/content${download ? '?download=1' : ''}`;

export function formatBytes(n: number): string {
  if (!Number.isFinite(n) || n < 0) return '';
  if (n < 1024) return `${n} B`;
  if (n < 1024 * 1024) return `${(n / 1024).toFixed(n < 10 * 1024 ? 1 : 0)} KB`;
  return `${(n / (1024 * 1024)).toFixed(n < 10 * 1024 * 1024 ? 1 : 0)} MB`;
}

/** A short type label for a file tile ("PDF", "PNG", "Text"). */
export function fileKind(mime: string, name: string): string {
  const m = mime.toLowerCase();
  if (m === 'application/pdf') return 'PDF';
  if (m.startsWith('image/')) return m.slice(6).toUpperCase().replace('JPEG', 'JPG');
  if (m === 'video/quicktime') return 'MOV';
  if (m === 'video/mp4') return 'MP4';
  if (m === 'video/webm') return 'WebM';
  if (m === 'application/json') return 'JSON';
  if (m === 'text/csv') return 'CSV';
  if (m === 'text/markdown') return 'Markdown';
  if (m.startsWith('text/')) return 'Text';
  const ext = /\.([a-z0-9]{1,6})$/i.exec(name)?.[1];
  return ext ? ext.toUpperCase() : 'File';
}

/** The files on a message (user uploads or an agent's `share_file`), skipping malformed entries. */
export function parseAttachments(meta: unknown): AttachmentRef[] {
  const m = parseMetaObject(meta);
  if (!m || !Array.isArray(m.attachments)) return [];
  return m.attachments.flatMap((a: unknown) => {
    const x = a as Partial<AttachmentRef> | null;
    if (!x || typeof x.id !== 'string' || !UUID_RE.test(x.id) || typeof x.name !== 'string') return [];
    return [{ id: x.id, name: x.name, mime: typeof x.mime === 'string' ? x.mime : 'application/octet-stream', size: typeof x.size === 'number' ? x.size : 0 }];
  });
}

/** An agent's `share_file` message: the file tiles stand in for the text. */
export const isSharedFileMessage = (meta: unknown) => parseMetaObject(meta)?.kind === 'file';

// ---------------------------------------------------------------------------
// What can be attached: checked in the browser so an unsupported file never uploads (0.7.3, P1).
// The server still decides by the file's bytes; this only refuses what it would refuse anyway.

const IMAGE_MIMES = new Set(['image/png', 'image/jpeg', 'image/jpg', 'image/gif', 'image/webp']);
const VIDEO_MIMES = new Set(['video/mp4', 'video/quicktime', 'video/webm']);
const IMAGE_EXTS = new Set(['png', 'jpg', 'jpeg', 'gif', 'webp']);
const VIDEO_EXTS = new Set(['mp4', 'm4v', 'mov', 'webm']);
/** Text and code: the server keeps these when the bytes are UTF-8. */
const TEXT_EXTS = new Set(
  (
    'txt text md markdown csv tsv json jsonl ndjson log yaml yml toml ini cfg conf env xml html htm css scss sass less svg ' +
    'js mjs cjs jsx ts tsx mts cts vue svelte astro py pyi rb php java kt kts scala go rs c h cc cpp cxx hpp hh m mm swift ' +
    'cs fs vb lua pl pm r jl dart ex exs erl hs ml clj sql sh bash zsh fish ps1 bat cmd make mk cmake gradle properties ' +
    'gitignore gitattributes editorconfig dockerfile lock patch diff tex bib rst adoc org srt vtt'
  ).split(' '),
);
/** Common extension-less text files. */
const TEXT_NAMES = new Set(['makefile', 'dockerfile', 'readme', 'license', 'licence', 'changelog', 'procfile', 'gemfile', 'rakefile', 'vagrantfile', 'justfile']);

const extOf = (name: string) => /\.([a-z0-9]{1,12})$/i.exec(name)?.[1]?.toLowerCase() ?? '';

export type AttachKind = 'image' | 'pdf' | 'text' | 'video';

/**
 * What kind of file this is, from the browser's MIME type and then the extension, or null when
 * kwant can't take it. Extension first for code: browsers call `.ts` "video/mp2t".
 */
export function attachKind(f: { name: string; type?: string }): AttachKind | null {
  const mime = (f.type || '').toLowerCase().split(';')[0].trim();
  const name = (f.name || '').toLowerCase();
  const ext = extOf(name);
  if (TEXT_EXTS.has(ext)) return 'text';
  if (IMAGE_EXTS.has(ext)) return 'image';
  if (VIDEO_EXTS.has(ext)) return 'video';
  if (ext === 'pdf') return 'pdf';
  if (IMAGE_MIMES.has(mime)) return 'image';
  if (VIDEO_MIMES.has(mime)) return 'video';
  if (mime === 'application/pdf') return 'pdf';
  if (mime.startsWith('text/') || mime === 'application/json') return 'text';
  // No extension and nothing the browser recognised (a pasted snippet, a Makefile): let the server look.
  if (!ext && (!mime || mime === 'application/octet-stream')) return 'text';
  if (!ext && TEXT_NAMES.has(name)) return 'text';
  return null;
}

/** The one line shown when a file's type can't be attached. */
export const SUPPORTED_TYPES_TEXT = 'Images, videos (MP4, MOV, WebM), PDFs and text or code files work.';

export const capFor = (kind: AttachKind | null) => (kind === 'video' ? MAX_VIDEO_BYTES : MAX_ATTACHMENT_BYTES);

/** Checked before uploading, so an unsupported or too-big file never leaves the browser. */
export function preflightFile(f: { name: string; size: number; type?: string }): string | null {
  const label = f.name || 'That file';
  const kind = attachKind(f);
  if (!kind) return `${label} can't be attached. ${SUPPORTED_TYPES_TEXT}`;
  if (f.size === 0) return `${label} is empty.`;
  const cap = capFor(kind);
  if (f.size > cap) {
    const mb = (f.size / (1024 * 1024)).toFixed(1);
    return kind === 'video' ? `${label} is too big (${mb} MB). Videos can be up to 100 MB.` : `${label} is too big (${mb} MB). Max 20 MB (videos 100 MB).`;
  }
  return null;
}

/** Friendly text for an upload the server refused. */
export function uploadErrorText(status: number, serverMessage?: string): string {
  switch (status) {
    case 413:
      return serverMessage || 'Too big. Max 20 MB, videos 100 MB.';
    case 415:
      return `That file type isn't supported. ${SUPPORTED_TYPES_TEXT}`;
    case 400:
      return serverMessage || 'The file is empty or its name is not allowed.';
    case 401:
    case 403:
      return 'Your session ended. Sign in again.';
    case 0:
      return 'Upload interrupted. Check the connection and retry.';
    case 502:
    case 503:
    case 504:
      return "The server didn't answer in time. Retry in a moment.";
    default:
      return serverMessage || `Upload failed (HTTP ${status}).`;
  }
}

/**
 * Whether retrying the same file can help: not when the server refused the file itself. A 400 whose
 * `code` is `interrupted` (the body stream broke on the way) is a blip, not a refusal.
 */
export const uploadRetryable = (status: number, code?: string) => (status === 400 ? code === 'interrupted' : status !== 413 && status !== 415);

/** A network blip or a gateway hiccup: retried by itself, without the person pressing Retry (P3). */
export const uploadAutoRetryable = (status: number, code?: string) =>
  status === 0 || status === 502 || status === 503 || status === 504 || (status === 400 && code === 'interrupted');

/** Waits before the first and second automatic retry. */
export const AUTO_RETRY_DELAYS_MS = [1000, 3000] as const;

/** An upload the server refused, or that never reached it (`status` 0). */
export class UploadError extends Error {
  /** The server's stable error `code` (`too_large`, `unsupported_type`, `empty`, `interrupted`), when it sent one. */
  constructor(readonly status: number, serverMessage?: string, readonly code?: string) {
    super(uploadErrorText(status, serverMessage));
  }
}

/** Why `/chat/send` refused a message's files, or null when the status isn't about them. */
export function sendAttachmentErrorText(status: number): string | null {
  if (status === 409) return 'A file here was already sent. Attach it again to resend.';
  if (status === 400) return `Couldn't attach the files (unknown file, or more than ${MAX_ATTACHMENTS}).`;
  return null;
}

// ---------------------------------------------------------------------------
// Composer uploads

export type UploadState = 'uploading' | 'done' | 'error';

export interface PendingUpload {
  key: string;
  name: string;
  size: number;
  /** The browser's MIME type, for the chip's icon. */
  mime?: string;
  /** 0–1 */
  progress: number;
  state: UploadState;
  /** True while waiting to retry by itself after a blip. */
  retrying?: boolean;
  error?: string;
  /** False when Retry can't help, e.g. an unsupported file type. */
  retryable?: boolean;
  attachment?: Attachment;
}

/** Room for more files: at most 10 per message. */
export const uploadSlotsLeft = (items: PendingUpload[]) => Math.max(0, MAX_ATTACHMENTS - items.length);

/** Sendable when there's text or a finished file, and nothing still uploading or failed. */
export function composerReady(text: string, items: PendingUpload[]): boolean {
  if (items.some((i) => i.state !== 'done')) return false;
  return text.trim().length > 0 || items.length > 0;
}

export const readyAttachments = (items: PendingUpload[]): AttachmentRef[] =>
  items.flatMap((i) => (i.state === 'done' && i.attachment ? [{ id: i.attachment.id, name: i.attachment.name, mime: i.attachment.mime, size: i.attachment.size }] : []));

/**
 * Uploads one file as the raw request body (`?name=`), reporting progress. XHR because fetch has
 * no upload progress. Rejects with a friendly message; an abort rejects with `AbortError`.
 */
export function uploadAttachment(file: Blob & { name?: string }, onProgress: (fraction: number) => void, signal?: AbortSignal): Promise<Attachment> {
  return new Promise((resolve, reject) => {
    const xhr = new XMLHttpRequest();
    xhr.open('POST', `/api/attachments?name=${encodeURIComponent(file.name || 'file')}`);
    xhr.upload.onprogress = (e) => {
      if (e.lengthComputable && e.total > 0) onProgress(Math.min(1, e.loaded / e.total));
    };
    xhr.onload = () => {
      let body: unknown = null;
      try {
        body = JSON.parse(xhr.responseText);
      } catch {
        // not JSON
      }
      if (xhr.status === 201 || xhr.status === 200) {
        onProgress(1);
        resolve(body as Attachment);
      } else {
        const b = (body && typeof body === 'object' ? body : {}) as { error?: unknown; code?: unknown };
        reject(new UploadError(xhr.status, typeof b.error === 'string' ? b.error : undefined, typeof b.code === 'string' ? b.code : undefined));
      }
    };
    xhr.onerror = () => reject(new UploadError(0));
    xhr.onabort = () => reject(new DOMException('Upload cancelled', 'AbortError'));
    signal?.addEventListener('abort', () => xhr.abort(), { once: true });
    xhr.send(file);
  });
}

/** "5.2 of 8.0 MB" for a chip while it uploads (P4); KB for small files. */
export function uploadProgressText(fraction: number, size: number): string {
  const f = Number.isFinite(fraction) ? Math.min(1, Math.max(0, fraction)) : 0;
  if (!Number.isFinite(size) || size <= 0) return `${Math.round(f * 100)}%`;
  const MB = 1024 * 1024;
  if (size >= MB) return `${((f * size) / MB).toFixed(1)} of ${(size / MB).toFixed(1)} MB`;
  return `${Math.round((f * size) / 1024)} of ${Math.max(1, Math.round(size / 1024))} KB`;
}

const sleep = (ms: number, signal?: AbortSignal) =>
  new Promise<void>((resolve, reject) => {
    if (signal?.aborted) return reject(new DOMException('Upload cancelled', 'AbortError'));
    const t = setTimeout(() => {
      signal?.removeEventListener('abort', onAbort);
      resolve();
    }, ms);
    const onAbort = () => {
      clearTimeout(t);
      reject(new DOMException('Upload cancelled', 'AbortError'));
    };
    signal?.addEventListener('abort', onAbort, { once: true });
  });

export interface RetryOptions {
  /** Called before each automatic retry (attempt 1 or 2), so the chip can say "Retrying…". */
  onRetrying?: (attempt: number) => void;
  delays?: readonly number[];
  /** For tests. */
  upload?: typeof uploadAttachment;
  wait?: (ms: number, signal?: AbortSignal) => Promise<void>;
}

/**
 * `uploadAttachment`, retried by itself after a dropped connection or a 502/503/504: twice, after
 * 1 s and then 3 s. A refusal (413, 415, 400) or anything else fails at once, as before.
 */
export async function uploadWithRetry(
  file: Blob & { name?: string },
  onProgress: (fraction: number) => void,
  signal?: AbortSignal,
  opts: RetryOptions = {},
): Promise<Attachment> {
  const delays = opts.delays ?? AUTO_RETRY_DELAYS_MS;
  const upload = opts.upload ?? uploadAttachment;
  const wait = opts.wait ?? sleep;
  for (let attempt = 0; ; attempt++) {
    try {
      return await upload(file, onProgress, signal);
    } catch (err) {
      const blip = err instanceof UploadError && uploadAutoRetryable(err.status, err.code);
      if (!blip || attempt >= delays.length || signal?.aborted) throw err;
      opts.onRetrying?.(attempt + 1);
      await wait(delays[attempt], signal);
      onProgress(0);
    }
  }
}

/** Files from a paste or drop (ignores plain text and dragged links). */
export function filesFrom(list: DataTransferItemList | FileList | null | undefined): File[] {
  if (!list) return [];
  const out: File[] = [];
  for (let i = 0; i < list.length; i++) {
    const item = list[i] as DataTransferItem | File;
    if ('kind' in item) {
      if (item.kind === 'file') {
        const f = item.getAsFile();
        if (f) out.push(f);
      }
    } else out.push(item);
  }
  return out;
}
