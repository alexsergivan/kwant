// Space documents (space-docs/design.md): a space's own FAQ, policies and price lists, which bots quote
// word for word. The list and the cite check are #601's routes; add, remove and the page text are the
// second server step, and the tab works read-only until they exist.

import { personFetch } from './auth';

export interface SpaceDoc {
  name: string;
  size: number;
  /** RFC 3339. */
  modified: string;
  /** PDFs only. */
  pages?: number;
  /** Over the 2 MB bots can read. */
  too_large?: boolean;
}

export interface SpaceDocList {
  /** The docs folder's absolute path; shown only in the tab's footer. */
  folder: string;
  docs: SpaceDoc[];
  /** The folder has more files than the 200 listed. */
  more: boolean;
  /** How many more, when the server says. */
  more_count?: number;
}

export interface DocPage {
  file: string;
  page: number;
  pages: number;
  text_before: string;
  match: string;
  text_after: string;
  cut_before: boolean;
  cut_after: boolean;
}

export const DOC_MAX_BYTES = 2 * 1024 * 1024;
export const DOC_MAX_COUNT = 200;
export const DOC_ACCEPT = '.md,.markdown,.txt,.pdf,text/markdown,text/plain,application/pdf';

export type DocKind = 'pdf' | 'md' | 'txt';

/** The file's lower-case extension, without the dot ('' when it has none). */
export function extOf(name: string): string {
  const i = name.lastIndexOf('.');
  return i > 0 ? name.slice(i + 1).toLowerCase() : '';
}

export function docKind(name: string): DocKind | null {
  const e = extOf(name);
  if (e === 'pdf') return 'pdf';
  if (e === 'md' || e === 'markdown') return 'md';
  if (e === 'txt') return 'txt';
  return null;
}

/** The type mark's letters: PDF, MD, TXT, or the odd extension upper-cased ("DOCX"). */
export function markOf(name: string): string {
  const k = docKind(name);
  if (k) return k.toUpperCase();
  return (extOf(name) || 'FILE').toUpperCase().slice(0, 4);
}

/** QA F2 and #601's own name rule: no leading dot, no "..", no folders. */
export function badName(name: string): boolean {
  return !name || name !== name.trim() || name.startsWith('.') || name.includes('..') || /[/\\\0]/.test(name);
}

export type FileProblem = { kind: 'bad_name' } | { kind: 'wrong_kind' } | { kind: 'too_large'; size: number };

/** Checked in the browser before anything is sent: the name, then the kind, then the size. */
export function checkFile(f: { name: string; size: number }): FileProblem | null {
  if (badName(f.name)) return { kind: 'bad_name' };
  if (!docKind(f.name)) return { kind: 'wrong_kind' };
  if (f.size > DOC_MAX_BYTES) return { kind: 'too_large', size: f.size };
  return null;
}

/** KB below 1 MB, one decimal of MB above. */
export function formatSize(bytes: number): string {
  if (bytes < 1024 * 1024) return `${Math.max(1, Math.round(bytes / 1024))} KB`;
  return `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
}

/** "today", "yesterday", "3 days ago", then "Oct 28". */
export function updatedWhen(iso: string, now: number = Date.now()): string {
  const t = Date.parse(iso);
  if (Number.isNaN(t)) return '';
  const day = (ms: number) => {
    const d = new Date(ms);
    return Date.UTC(d.getFullYear(), d.getMonth(), d.getDate()) / 86_400_000;
  };
  const d = Math.max(0, day(now) - day(t));
  if (d === 0) return 'today';
  if (d === 1) return 'yesterday';
  if (d < 7) return `${d} days ago`;
  return new Date(t).toLocaleDateString('en-US', { month: 'short', day: 'numeric' });
}

/** "4 pages · 312 KB · updated 3 days ago"; pages only for PDFs. */
export function docMeta(d: SpaceDoc, now: number = Date.now()): string {
  const parts: string[] = [];
  if (docKind(d.name) === 'pdf' && d.pages) parts.push(`${d.pages} ${d.pages === 1 ? 'page' : 'pages'}`);
  parts.push(formatSize(d.size));
  const when = updatedWhen(d.modified, now);
  if (when) parts.push(`updated ${when}`);
  return parts.join(' · ');
}

/** Newest first, so a file just added shows at the top. */
export function sortDocs(docs: SpaceDoc[]): SpaceDoc[] {
  return [...docs].sort((a, b) => (Date.parse(b.modified) || 0) - (Date.parse(a.modified) || 0) || a.name.localeCompare(b.name));
}

/** The folder for the footer: the home folder as "~" and a long space id cut after 8 characters ("spc_7f3a…"). */
export function shortFolder(folder: string): string {
  return folder
    .replace(/^(\/home\/[^/]+|\/Users\/[^/]+|[A-Za-z]:\\Users\\[^\\]+)(?=[/\\]|$)/, '~')
    .replace(/([/\\]spaces[/\\])([^/\\]{10,})(?=[/\\]|$)/, (_m, pre: string, id: string) => `${pre}${id.slice(0, 8)}…`);
}

// ── kwant-doc:// addresses ──

export interface DocRef {
  space: string;
  file: string;
  /** PDFs only. */
  page: number | null;
  /** The `:~:text=` part, decoded; null when the link quotes nothing. */
  quote: string | null;
}

export const DOC_SCHEME = 'kwant-doc://';

const decode = (s: string): string | null => {
  try {
    return decodeURIComponent(s);
  } catch {
    return null;
  }
};

/** `kwant-doc://<space>/<file>[#page=N][:~:text=<quote>]`, as #601's cite route writes it. Null if it isn't one. */
export function parseDocUrl(href: string | null | undefined): DocRef | null {
  if (!href || !href.startsWith(DOC_SCHEME)) return null;
  const rest = href.slice(DOC_SCHEME.length);
  const hash = rest.indexOf('#');
  const path = hash < 0 ? rest : rest.slice(0, hash);
  const frag = hash < 0 ? '' : rest.slice(hash + 1);
  const slash = path.indexOf('/');
  if (slash <= 0) return null;
  const space = decode(path.slice(0, slash));
  const file = decode(path.slice(slash + 1));
  if (!space || !file || badName(file) || !docKind(file)) return null;
  const directive = frag.indexOf(':~:');
  const head = directive < 0 ? frag : frag.slice(0, directive);
  const pageMatch = /^page=(\d+)$/.exec(head);
  const page = docKind(file) === 'pdf' && pageMatch ? Math.max(1, Number(pageMatch[1])) : null;
  let quote: string | null = null;
  if (directive >= 0) {
    const text = /(?:^|&)text=([^&]*)/.exec(frag.slice(directive + 3));
    if (text) quote = decode(text[1]) || null;
  }
  return { space, file, page, quote };
}

/** "Returns policy.pdf, page 2" or "faq.md". */
export const docPlace = (r: Pick<DocRef, 'file' | 'page'>) => (r.page ? `${r.file}, page ${r.page}` : r.file);

// ── Re-checking a quote (§2): once per space, file and quote for the session, at most 4 at a time ──

/** ok: the words are in the file now. missing: not found now (422 or 404). unknown: no mark at all. */
export type CiteState = 'ok' | 'missing' | 'gone' | 'unknown';

export const citeKey = (space: string, file: string, quote: string) => JSON.stringify([space, file, quote]);

const citeCache = new Map<string, Promise<CiteState>>();
let citeRunning = 0;
const citeQueue: Array<() => void> = [];
export const CITE_PARALLEL = 4;

async function limited<T>(job: () => Promise<T>): Promise<T> {
  if (citeRunning >= CITE_PARALLEL) await new Promise<void>((go) => citeQueue.push(go));
  citeRunning++;
  try {
    return await job();
  } finally {
    citeRunning--;
    citeQueue.shift()?.();
  }
}

/** Maps the cite route's answer to the chip's mark. */
export function citeStateOf(status: number): CiteState {
  if (status >= 200 && status < 300) return 'ok';
  if (status === 422) return 'missing';
  if (status === 404) return 'gone';
  return 'unknown';
}

/** The chip's check; a second call for the same quote shares the first one's answer. */
export function recheckQuote(space: string, file: string, quote: string, send: typeof fetch = (...a) => fetch(...a)): Promise<CiteState> {
  const key = citeKey(space, file, quote);
  const hit = citeCache.get(key);
  if (hit) return hit;
  const p = limited(() =>
    send(`/api/spaces/${encodeURIComponent(space)}/docs/cite`, {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ file, quote }),
    }).then(
      (r) => citeStateOf(r.status),
      () => 'unknown' as const,
    ),
  );
  citeCache.set(key, p);
  // A quiet chip after a network hiccup can try again on the next mount.
  p.then((s) => s === 'unknown' && citeCache.delete(key));
  return p;
}

/** Tests only. */
export function _resetCiteCache() {
  citeCache.clear();
  citeRunning = 0;
  citeQueue.length = 0;
}

// ── The routes ──

const base = (space: string) => `/api/spaces/${encodeURIComponent(space)}/docs`;

async function errorText(r: Response): Promise<string> {
  const body = await r
    .clone()
    .json()
    .catch(() => null);
  return (body && typeof body === 'object' && typeof (body as { error?: unknown }).error === 'string' && (body as { error: string }).error) || '';
}

/** A 404 or 405 from a route the server doesn't have yet (no JSON error of its own). */
async function routeMissing(r: Response): Promise<boolean> {
  if (r.status === 405) return true;
  if (r.status !== 404) return false;
  return !(await errorText(r));
}

export type AddResult =
  | { ok: true; doc: SpaceDoc }
  | { ok: false; kind: 'exists' | 'bad_name' | 'wrong_kind' | 'too_large' | 'no_text' | 'limit' | 'no_route' | 'failed'; detail?: string };

/** Maps an add's answer (Vega's route) to what the row says. */
export function addResultOf(status: number, error: string): Exclude<AddResult, { ok: true }> {
  const e = error.toLowerCase();
  if (status === 400) return { ok: false, kind: 'bad_name', detail: error };
  if (status === 415) return { ok: false, kind: 'wrong_kind', detail: error };
  if (status === 413) return { ok: false, kind: 'too_large', detail: error };
  if (status === 422) return { ok: false, kind: 'no_text', detail: error };
  if (status === 409 && /200|limit|most/.test(e)) return { ok: false, kind: 'limit', detail: error };
  if (status === 409 && /exist|already/.test(e)) return { ok: false, kind: 'exists', detail: error };
  return { ok: false, kind: 'failed', detail: error };
}

export const spaceDocsApi = {
  list: async (space: string): Promise<SpaceDocList> => {
    const r = await fetch(base(space), { cache: 'no-store' });
    if (!r.ok) throw new Error((await errorText(r)) || 'Couldn’t load this space’s documents.');
    return r.json();
  },

  /**
   * Whether add, remove and the page text exist on this server (they come in a second server step). The page
   * route with an empty name answers 400 when it is there; a 404 or 405 without an error of its own means it
   * isn't yet. It is a read, so it never asks "Confirm it's you".
   */
  hasRoutes: async (space: string): Promise<boolean> => {
    try {
      const r = await fetch(`${base(space)}/page?file=&page=1`, { cache: 'no-store' });
      return !(await routeMissing(r));
    } catch {
      return false;
    }
  },

  add: async (space: string, file: File, replace = false): Promise<AddResult> => {
    const form = new FormData();
    form.append('file', file, file.name);
    let r: Response;
    try {
      r = await personFetch(`${base(space)}${replace ? '?replace=1' : ''}`, { method: 'POST', body: form });
    } catch {
      return { ok: false, kind: 'failed' };
    }
    if (r.ok) {
      const doc = (await r.json().catch(() => null)) as SpaceDoc | null;
      return { ok: true, doc: doc && typeof doc.name === 'string' ? doc : { name: file.name, size: file.size, modified: new Date().toISOString() } };
    }
    if (await routeMissing(r)) return { ok: false, kind: 'no_route' };
    return addResultOf(r.status, await errorText(r));
  },

  /** 'ok', 'no_route' (hide remove), or an error to show. */
  remove: async (space: string, name: string): Promise<'ok' | 'no_route' | 'failed'> => {
    try {
      const r = await personFetch(`${base(space)}/${encodeURIComponent(name)}`, { method: 'DELETE' });
      if (r.ok || r.status === 404) return r.ok || (await errorText(r)) ? 'ok' : 'no_route';
      if (await routeMissing(r)) return 'no_route';
      return 'failed';
    } catch {
      return 'failed';
    }
  },

  /** 'no_route' until Vega's page route is on this server: the viewer then shows only the quote. */
  page: async (space: string, file: string, page: number, quote?: string | null): Promise<DocPage | 'gone' | 'no_route' | 'failed'> => {
    const q = new URLSearchParams({ file, page: String(page) });
    if (quote) q.set('quote', quote);
    try {
      const r = await fetch(`${base(space)}/page?${q}`, { cache: 'no-store' });
      if (r.ok) return (await r.json()) as DocPage;
      if (await routeMissing(r)) return 'no_route';
      if (r.status === 404) return 'gone';
      return 'failed';
    } catch {
      return 'failed';
    }
  },
};

/** "faq.md removed" etc., for the polite live region. */
export const copyQuote = (quote: string, file: string, page: number | null) => `${quote} — ${file}${page ? `, p. ${page}` : ''}`;
