// File attachments in chats: uploads from the composer and files agents share (batch 2 / D).
import { parseMetaObject } from './questions';

export const MAX_ATTACHMENT_BYTES = 20 * 1024 * 1024;
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

/** Checked before uploading, so a too-big file never leaves the browser. */
export function preflightFile(f: { name: string; size: number }): string | null {
  if (f.size === 0) return `${f.name || 'That file'} is empty.`;
  if (f.size > MAX_ATTACHMENT_BYTES) return `${f.name || 'That file'} is too big (${(f.size / (1024 * 1024)).toFixed(1)} MB). Max 20 MB.`;
  return null;
}

/** Friendly text for an upload the server refused. */
export function uploadErrorText(status: number, serverMessage?: string): string {
  switch (status) {
    case 413:
      return 'Too big. Max 20 MB.';
    case 415:
      return "That file type isn't supported. Images, PDFs and text files work.";
    case 400:
      return serverMessage || 'The file is empty or its name is not allowed.';
    case 401:
    case 403:
      return 'Your session ended. Sign in again.';
    case 0:
      return 'Upload interrupted. Check the connection and retry.';
    default:
      return serverMessage || `Upload failed (HTTP ${status}).`;
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
  /** 0–1 */
  progress: number;
  state: UploadState;
  error?: string;
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
        const msg = body && typeof (body as { error?: unknown }).error === 'string' ? (body as { error: string }).error : undefined;
        reject(new Error(uploadErrorText(xhr.status, msg)));
      }
    };
    xhr.onerror = () => reject(new Error(uploadErrorText(0)));
    xhr.onabort = () => reject(new DOMException('Upload cancelled', 'AbortError'));
    signal?.addEventListener('abort', () => xhr.abort(), { once: true });
    xhr.send(file);
  });
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
