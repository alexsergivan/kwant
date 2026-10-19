// Bring bots over from another app, and export them again (IMPORT-API.md). The export prompt
// text lives only on the server (src/import/prompts.rs); this file never copies it.

export type ImportSourceId = 'grok' | 'chatgpt' | 'claude' | 'other';

export interface ExportPrompt {
  id: ImportSourceId | string;
  label: string;
  where_to_send: string;
  prompt: string;
}

export interface BundleSkill {
  name: string;
  description: string;
  body: string;
}

export interface BundleRoutine {
  name: string;
  instructions: string;
  schedule: unknown;
  schedule_text?: string | null;
}

export interface BundleBot {
  ref: string;
  name: string;
  title?: string | null;
  persona_prompt?: string | null;
  soul?: string | null;
  avatar_hint?: string | null;
  model_hint?: string | null;
  starters?: string[];
  memories?: string[];
  skills?: BundleSkill[];
  routines?: BundleRoutine[];
  tool_hints?: string[];
  knowledge_files?: string[];
}

export interface BundleTeam {
  name: string;
  lead?: string | null;
  members: string[];
  instructions?: string | null;
}

export interface Bundle {
  format: string;
  source: { kind: string; label?: string | null; exported_at?: string | null };
  bots: BundleBot[];
  teams?: BundleTeam[];
  skills?: BundleSkill[];
  omitted?: string[];
}

export interface Notice {
  /** The bot's ref, absent for notices about the whole import. */
  bot?: string;
  message: string;
}

export interface Rename {
  ref: string;
  from: string;
  to: string;
}

export interface Preview {
  bundle: Bundle;
  warnings: Notice[];
  dropped: Notice[];
  problems: Notice[];
  renames: Rename[];
  can_apply: boolean;
}

export interface ImportRecord {
  id: string;
  space_id: string;
  source_kind: string;
  status: 'applied' | 'undone';
  created: {
    kwants: Array<{ ref: string; id: string; name: string; fingerprint?: string }>;
    skills: string[];
    routines: string[];
    teams: string[];
    channels: string[];
    memories: number;
  };
  created_at: string;
  undone_at: string | null;
}

export interface Applied {
  import: ImportRecord;
  warnings: Notice[];
  dropped: Notice[];
  renames: Rename[];
}

/** An import route's `{error, code}` reply, or a network failure (`code: 'network'`). */
export class ImportError extends Error {
  constructor(
    message: string,
    readonly code: string,
    readonly status: number,
    /** `edited` / `in_use`: the names involved. */
    readonly names: string[] = [],
    readonly problems: Notice[] = [],
  ) {
    super(message);
  }
}

/** Limits the server enforces, checked first so a big file fails before it uploads. */
export const IMPORT_LIMITS = { fileBytes: 20 * 1024 * 1024, textChars: 256 * 1024 } as const;

/** Files the "Upload a file" step accepts (the server sniffs the content anyway). */
export const IMPORT_ACCEPT = '.json,.md,.png,.zip,.txt';

const enc = encodeURIComponent;

async function call<T>(method: string, url: string, init: RequestInit = {}): Promise<T> {
  let res: Response;
  try {
    res = await fetch(url, { method, ...init });
  } catch {
    throw new ImportError('Couldn’t reach the server. Check your connection and try again.', 'network', 0);
  }
  if (!res.ok) {
    let data: any = null;
    try {
      data = await res.json();
    } catch {
      // no JSON body
    }
    const names = Array.isArray(data?.edited) ? data.edited : Array.isArray(data?.in_use) ? data.in_use : [];
    throw new ImportError(
      typeof data?.error === 'string' && data.error ? data.error : '',
      typeof data?.code === 'string' ? data.code : res.status === 413 ? 'too_large' : res.status === 404 ? 'not_found' : 'internal',
      res.status,
      names.filter((n: unknown): n is string => typeof n === 'string'),
      Array.isArray(data?.problems) ? data.problems : [],
    );
  }
  return res.json();
}

const json = (body: unknown): RequestInit => ({ headers: { 'Content-Type': 'application/json' }, body: JSON.stringify(body) });

export const importApi = {
  prompts: () => call<ExportPrompt[]>('GET', '/api/import/prompts'),
  previewText: (text: string) => call<Preview>('POST', '/api/import/preview', json({ text })),
  previewFile: (file: Blob, filename: string) =>
    call<Preview>('POST', `/api/import/preview?filename=${enc(filename)}`, {
      headers: { 'Content-Type': 'application/octet-stream' },
      body: file,
    }),
  apply: (bundle: Bundle, skip: string[]) => call<Applied>('POST', '/api/import/apply', json({ bundle, skip })),
  list: () => call<ImportRecord[]>('GET', '/api/imports'),
  undo: (id: string, force = false) => call<ImportRecord>('POST', `/api/imports/${enc(id)}/undo`, force ? json({ force: true }) : {}),
};

/** What to tell the person for an import error: a short heading, and the server's sentence (or ours). */
export function importErrorText(err: unknown): { title: string; detail: string } {
  const e = err instanceof ImportError ? err : null;
  const detail = e?.message || '';
  switch (e?.code) {
    case 'too_large':
      return { title: 'That’s too big to import', detail: detail || 'Files can be up to 20 MB, and pasted replies up to 256 KB.' };
    case 'unsupported':
      return {
        title: 'We can’t read that kind of file',
        detail: detail || 'Try a .json, .md, .png character card or .zip, or paste the reply your bot gave you.',
      };
    case 'invalid':
      return { title: 'Something in it isn’t right', detail: detail || 'It looks like a file we know, but part of it is broken.' };
    case 'empty':
      return { title: 'Nothing to import', detail: detail || 'It was empty, or every bot was left out.' };
    case 'problems':
      return { title: 'Fix these first', detail: detail || 'Some bots can’t be imported as they are.' };
    case 'name_taken':
      return { title: 'A name was just taken', detail: detail || 'Someone made a bot with the same name. Check the preview again.' };
    case 'not_found':
      return { title: 'That import is gone', detail: detail || 'It may have been in another space.' };
    case 'network':
      return { title: 'No connection', detail };
    default:
      return { title: 'Something went wrong', detail: 'The server couldn’t finish this. Try again in a moment.' };
  }
}

/** Client-side checks before anything is sent. */
export function fileTooBig(size: number): boolean {
  return size > IMPORT_LIMITS.fileBytes;
}
export function textTooLong(text: string): boolean {
  return text.length > IMPORT_LIMITS.textChars;
}

export interface PreviewBotRow {
  ref: string;
  name: string;
  /** The name before a clash rename, when it was renamed. */
  renamedFrom: string | null;
  title: string;
  excerpt: string;
  memories: number;
  skills: number;
  routines: number;
  warnings: string[];
  dropped: string[];
  problems: string[];
}

const EXCERPT = 180;

/** One row per bot for the preview, with its own notices, and the notices for the whole import. */
export function previewRows(p: Preview): { bots: PreviewBotRow[]; general: { warnings: string[]; dropped: string[]; problems: string[] } } {
  const refs = new Set(p.bundle.bots.map((b) => b.ref));
  const forBot = (list: Notice[], ref: string) => list.filter((n) => n.bot === ref).map((n) => n.message);
  const general = (list: Notice[]) => list.filter((n) => !n.bot || !refs.has(n.bot)).map((n) => n.message);
  const bots = p.bundle.bots.map((b) => {
    const rename = p.renames.find((r) => r.ref === b.ref);
    const text = (b.persona_prompt || b.soul || '').replace(/\s+/g, ' ').trim();
    return {
      ref: b.ref,
      name: rename?.to ?? b.name,
      renamedFrom: rename && rename.from !== rename.to ? rename.from : null,
      title: b.title ?? '',
      excerpt: text.length > EXCERPT ? `${text.slice(0, EXCERPT - 1).trimEnd()}…` : text,
      memories: b.memories?.length ?? 0,
      skills: b.skills?.length ?? 0,
      routines: b.routines?.length ?? 0,
      warnings: forBot(p.warnings, b.ref),
      dropped: forBot(p.dropped, b.ref),
      problems: forBot(p.problems, b.ref),
    };
  });
  return { bots, general: { warnings: general(p.warnings), dropped: general(p.dropped), problems: general(p.problems) } };
}

/** Whether Import can be pressed: the server says yes, and at least one bot (or shared skill) is left. */
export function canImport(p: Preview, skip: ReadonlySet<string>): boolean {
  if (!p.can_apply) return false;
  return p.bundle.bots.some((b) => !skip.has(b.ref)) || (p.bundle.skills?.length ?? 0) > 0;
}

const SOURCE_NAMES: Record<string, string> = {
  grok: 'Grok',
  chatgpt: 'ChatGPT',
  claude: 'Claude',
  'claude-code': 'a Claude Code file',
  skill: 'a SKILL.md',
  'character-card': 'a character card',
  grokbot: 'Grok Bot',
  openmaus: 'OpenMausBot',
  'openmaus-grok': 'OpenMausBot',
  zip: 'a zip',
  'plain-text': 'pasted text',
};

/** Where an import comes from, for "from …". `source.label` is often a bot name, so it isn't used. */
export const sourceName = (kind: string): string | null => SOURCE_NAMES[kind] ?? null;

export const plural = (n: number, one: string, many = `${one}s`) => `${n} ${n === 1 ? one : many}`;

/** The file name from `Content-Disposition`, or a fallback. */
export function downloadName(header: string | null, fallback: string): string {
  if (header) {
    const star = /filename\*=UTF-8''([^;]+)/i.exec(header);
    if (star) {
      try {
        return decodeURIComponent(star[1].trim());
      } catch {
        // fall through to the plain name
      }
    }
    const plain = /filename="?([^";]+)"?/i.exec(header);
    if (plain) return plain[1].trim();
  }
  return fallback;
}

/** Downloads a bot (`kwantId`) or the whole open space as a bundle file. */
export async function downloadExport(kwantId?: string): Promise<string> {
  const url = kwantId ? `/api/kwants/${enc(kwantId)}/export` : '/api/export';
  let res: Response;
  try {
    res = await fetch(url);
  } catch {
    throw new ImportError('Couldn’t reach the server. Check your connection and try again.', 'network', 0);
  }
  if (!res.ok) {
    let msg = '';
    try {
      msg = (await res.json())?.error ?? '';
    } catch {
      // no JSON body
    }
    throw new ImportError(msg || 'The export didn’t work. Try again in a moment.', res.status === 404 ? 'not_found' : 'internal', res.status);
  }
  const name = downloadName(res.headers.get('content-disposition'), 'export.kwant.json');
  const blob = await res.blob();
  const href = URL.createObjectURL(blob);
  const a = document.createElement('a');
  a.href = href;
  a.download = name;
  document.body.appendChild(a);
  a.click();
  a.remove();
  setTimeout(() => URL.revokeObjectURL(href), 1000);
  return name;
}
