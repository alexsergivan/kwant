// The Wall's uploads (design.md "For Iris" 2): one photo at a time in the order picked, the raw file as the body of
// POST /api/pocket/<code>/photo with its caption in X-Caption (URI-encoded), progress from xhr.upload.onprogress.
// A dropped signal or a 5xx marks that row "Try again" and the rest keep going; if the box itself is gone (G8) the row
// goes back in line and the queue waits for it. 429 waits Retry-After and sends again by itself. Closed (410) and a
// full wall stop the queue. Nothing is resized or re-encoded here: the file goes as picked.
import { refusal, Refusal } from './view';

export type RowState = 'next' | 'sending' | 'sent' | 'dropped' | 'refused' | 'held';

export interface PickedFile {
  name: string;
  size: number;
  type: string;
}

export interface Row<F extends PickedFile = PickedFile> {
  key: number;
  file: F;
  state: RowState;
  loaded: number;
  total: number;
  caption: string;
  /** Why it can't go up (G4), in plain words. */
  why?: { title: string; fix?: string };
  /** The server's id once sent. */
  id?: string;
}

export type Sent = { status: number; json: unknown; retryAfter?: number };
export type Send<F> = (path: string, file: F, headers: Record<string, string>, progress: (loaded: number, total: number) => void) => Promise<Sent>;

export interface QueueHooks<F extends PickedFile> {
  send: Send<F>;
  setTimeout: (f: () => void, ms: number) => number;
  clearTimeout: (id: number) => void;
  /** Something about the rows changed (state, a new row). */
  change: () => void;
  /** Bytes moved on the row that is sending (the page moves its bar in place). */
  progress: (row: Row<F>) => void;
  /** A 201 carries the guest's new view. */
  view: (json: unknown) => void;
  /** A refusal for the whole page: the wall is full, or this guest reached the most photos. */
  page: (r: Extract<Refusal, { kind: 'page' }>) => void;
  /** The wall stopped taking photos (410): G7. */
  closed: () => void;
  /** After a failed send: is the box answering at all? false means G8, the queue waits for `resume()`. */
  probe: () => Promise<boolean>;
  away: () => void;
  /** Numbers for the copy (from the view): the host's name, the per-photo cap, the per-guest most. */
  facts: () => { host?: string; capMb?: number; perGuest?: number; captionMax: number };
}

const MB = 1024 * 1024;
const TYPES = ['image/jpeg', 'image/png', 'image/webp'];

/** Turns a file away before any byte moves when the phone already knows (size over the cap, not a photo, HEIC). */
export function precheck(f: PickedFile, capMb?: number): Row['why'] | undefined {
  const t = (f.type || '').toLowerCase();
  const ext = /\.(\w+)$/.exec(f.name)?.[1]?.toLowerCase() ?? '';
  const r = (error: string) => refusal(415, error) as { title: string; fix?: string };
  if (t === 'image/heic' || t === 'image/heif' || ext === 'heic' || ext === 'heif') return r('heic');
  if (t && !TYPES.includes(t)) return r('not_photo');
  if (capMb && f.size > capMb * MB) return refusal(413, 'too_big', { capMb }) as { title: string; fix?: string };
  return undefined;
}

export class UploadQueue<F extends PickedFile = PickedFile> {
  rows: Row<F>[] = [];
  /** Waiting for the box to come back (G8), the wall to have room, or a 429's Retry-After. */
  paused: '' | 'away' | 'full' | 'wait' | 'closed' = '';
  private busy = false;
  private n = 0;
  private timer = 0;

  constructor(private path: string, private on: QueueHooks<F>) {}

  add(files: F[]): void {
    const facts = this.on.facts();
    for (const file of files) {
      const why = precheck(file, facts.capMb);
      this.rows.push({ key: ++this.n, file, state: why ? 'refused' : this.paused === 'full' || this.paused === 'closed' ? 'held' : 'next', loaded: 0, total: file.size, caption: '', why });
    }
    this.on.change();
    this.pump();
  }

  get unsent(): Row<F>[] {
    return this.rows.filter((r) => r.state === 'next' || r.state === 'sending' || r.state === 'held' || r.state === 'dropped');
  }

  get working(): boolean {
    return this.rows.some((r) => r.state === 'next' || r.state === 'sending');
  }

  setCaption(key: number, caption: string): void {
    const r = this.rows.find((x) => x.key === key);
    if (r && (r.state === 'next' || r.state === 'dropped' || r.state === 'held')) r.caption = caption;
  }

  /** "Try again" on one row. */
  retry(key: number): void {
    const r = this.rows.find((x) => x.key === key);
    if (!r || (r.state !== 'dropped' && r.state !== 'refused')) return;
    r.state = 'next';
    r.why = undefined;
    r.loaded = 0;
    this.on.change();
    this.pump();
  }

  remove(key: number): void {
    const r = this.rows.find((x) => x.key === key);
    if (!r || r.state === 'sending') return;
    this.rows = this.rows.filter((x) => x !== r);
    this.on.change();
  }

  /** Drops the rows that are done (sent ones now show from the server's `mine`), once nothing is left to send. */
  settle(): void {
    if (this.working || this.rows.some((r) => r.state === 'dropped' || r.state === 'held')) return;
    const before = this.rows.length;
    this.rows = this.rows.filter((r) => r.state !== 'sent');
    if (this.rows.length !== before) this.on.change();
  }

  /** The box is back, or the host made room: rows held back go again. */
  resume(): void {
    if (this.paused === 'closed') return;
    this.paused = '';
    this.on.clearTimeout(this.timer);
    for (const r of this.rows) if (r.state === 'held') r.state = 'next';
    this.on.change();
    this.pump();
  }

  stop(): void {
    this.on.clearTimeout(this.timer);
    this.paused = 'closed';
    for (const r of this.rows) if (r.state === 'next') r.state = 'held';
  }

  private pump(): void {
    if (this.busy || this.paused) return;
    const r = this.rows.find((x) => x.state === 'next');
    if (!r) return this.settle();
    void this.send(r);
  }

  private async send(r: Row<F>): Promise<void> {
    this.busy = true;
    r.state = 'sending';
    r.loaded = 0;
    this.on.change();
    const facts = this.on.facts();
    const caption = [...r.caption.trim()].slice(0, facts.captionMax).join('');
    const headers: Record<string, string> = {};
    if (caption) headers['X-Caption'] = encodeURIComponent(caption);
    let res: Sent;
    try {
      res = await this.on.send(this.path, r.file, headers, (loaded, total) => {
        r.loaded = loaded;
        r.total = total || r.total;
        this.on.progress(r);
      });
    } catch {
      res = { status: 0, json: null };
    }
    this.busy = false;
    if (res.status >= 200 && res.status < 300) {
      r.state = 'sent';
      r.loaded = r.total;
      const photo = (res.json as { photo?: { id?: unknown } } | null)?.photo;
      if (typeof photo?.id === 'string') r.id = photo.id;
      this.on.view(res.json);
      this.on.change();
      return this.pump();
    }
    const body = (res.json && typeof res.json === 'object' ? res.json : {}) as { error?: unknown; limit?: unknown };
    const limit = typeof body.limit === 'number' && body.limit > 0 ? body.limit : undefined;
    const why = refusal(res.status, typeof body.error === 'string' ? body.error : undefined, { limit, host: facts.host, capMb: facts.capMb, perGuest: facts.perGuest, retryAfter: res.retryAfter });
    switch (why.kind) {
      case 'wait':
        r.state = 'next';
        this.paused = 'wait';
        this.on.change();
        this.timer = this.on.setTimeout(() => {
          if (this.paused === 'wait') {
            this.paused = '';
            this.pump();
          }
        }, why.secs * 1000);
        return;
      case 'retry': {
        const up = await this.on.probe();
        if (!up) {
          r.state = 'next';
          this.paused = 'away';
          this.on.change();
          this.on.away();
          return;
        }
        r.state = 'dropped';
        r.why = { title: "Didn't send · signal dropped" };
        this.on.change();
        return this.pump();
      }
      case 'closed':
        r.state = 'next';
        this.stop();
        this.on.change();
        this.on.closed();
        return;
      case 'page':
        r.state = 'held';
        this.paused = 'full';
        for (const x of this.rows) if (x.state === 'next') x.state = 'held';
        this.on.change();
        this.on.page(why);
        return;
      case 'row':
        r.state = 'refused';
        r.why = { title: why.title, fix: why.fix };
        this.on.change();
        return this.pump();
    }
  }
}

/** The browser's sender: XMLHttpRequest, because only it reports upload progress. */
export function xhrSend<F extends PickedFile & Blob>(XHR: typeof XMLHttpRequest): Send<F> {
  return (path, file, headers, progress) =>
    new Promise((resolve) => {
      const x = new XHR();
      x.open('POST', path);
      x.withCredentials = true;
      x.responseType = 'text';
      for (const [k, v] of Object.entries(headers)) x.setRequestHeader(k, v);
      if (file.type) x.setRequestHeader('Content-Type', file.type);
      x.upload.onprogress = (e) => progress(e.loaded, e.lengthComputable ? e.total : file.size);
      x.onload = () => {
        let json: unknown = null;
        try {
          json = JSON.parse(x.responseText || 'null');
        } catch {
          json = null;
        }
        const ra = Number(x.getResponseHeader('Retry-After'));
        resolve({ status: x.status, json, retryAfter: Number.isFinite(ra) && ra > 0 ? ra : undefined });
      };
      x.onerror = x.ontimeout = x.onabort = () => resolve({ status: 0, json: null });
      x.send(file);
    });
}
