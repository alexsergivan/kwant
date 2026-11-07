// Show it once (0.7.9, spec §S): the person does a browser task once on the bot's Take over stage,
// kwant records the steps, and the bot writes a skill the person reviews.
//
// Built to Vega's contract, next-079/show-once/api.md. Everything the screen shows comes from it:
// step numbers (`n`), mask placeholders, counts, limits and dates. Words and layout are design §S.

import { useSyncExternalStore } from 'react';
import { onContinuity } from './continuity';
import { ENGINE_NAMES } from './failover';

// ---------------------------------------------------------------------------------------------
// Shapes (api.md §2, §3, §4)

export type StepKind = 'page' | 'click' | 'type' | 'select' | 'key' | 'upload' | 'note';
export type MaskKind = 'password' | 'code' | 'payment' | 'secret' | 'saved_login';

export interface RecordedStep {
  id: number | string;
  /** Display number, from the server (A35: deleted-in-undo steps keep theirs) */
  n: number;
  kind: StepKind;
  origin?: string;
  path?: string;
  role?: string;
  name?: string;
  label?: string;
  /** type: the text; select: the option; key: Enter or Tab; note: the note. Never on a masked step. */
  value?: string;
  masked?: MaskKind | null;
  placeholder?: string | null;
  at?: string;
  deleted?: boolean;
  undo_until?: string | null;
}

export type RecordingState = 'recording' | 'stopped' | 'writing' | 'written';
export type StopReason = 'stop' | 'limit_steps' | 'limit_time' | 'handed_back' | 'closed' | 'restart';

export interface Limits {
  steps: number;
  secs: number;
  near_steps_left: number;
  near_secs: number;
}

export interface Recording {
  id: string;
  kwant_id: string;
  space_id?: string;
  stage: string;
  state: RecordingState;
  started_at: string;
  ended_at: string | null;
  elapsed_secs: number;
  /** Steps that count toward the limit (notes don't) */
  step_count: number;
  note_count: number;
  limits: Limits;
  near: boolean;
  stop_reason: StopReason | null;
  origins: string[];
  /** Where Write sends the steps; null = no engine */
  engine: string | null;
  write_error: string | null;
  skill_id: string | null;
  steps_expire_at: string | null;
  steps_deleted_at: string | null;
}

export interface ShownInput {
  name: string;
  description?: string;
  example: string;
}

/** `skill.shown` on a skill drafted from a recording */
export interface SkillShown {
  recording_id: string;
  shown_at: string;
  /** Already formatted in the person's time zone: "Nov 7, 2026" */
  shown_on: string;
  step_count: number;
  inputs: ShownInput[];
  steps_expire_at: string | null;
  steps_deleted: boolean;
}

/** Used only until the server's own `limits` arrive; the same numbers as api.md §8. */
export const DEFAULT_LIMITS: Limits = { steps: 200, secs: 1800, near_steps_left: 8, near_secs: 1680 };

/** The bar's short form, so the chip isn't cut off on a phone (design S2). Other kinds keep the server's text. */
const MASK_SHORT: Partial<Record<MaskKind, { noun: string; chip: string }>> = {
  password: { noun: 'Password', chip: '[your password]' },
  code: { noun: 'Code', chip: '[code]' },
};

/** What a screen reader hears for the chip (design: "password not kept, uses the saved login"). */
export const MASK_LABEL: Record<MaskKind, string> = {
  password: 'password not kept, uses the saved login',
  code: 'code not kept',
  payment: 'card details not kept',
  secret: 'secret not kept',
  saved_login: 'not kept, filled from your saved login',
};

/** The placeholders api.md §3 lists, so the drafted skill's text can show them as chips. */
export const PLACEHOLDERS: Record<string, MaskKind> = {
  '[your password: use the saved login]': 'password',
  '[code]': 'code',
  '[card details]': 'payment',
  '[secret]': 'secret',
  '[filled from your saved login]': 'saved_login',
};

export const engineName = (engine: string | null | undefined): string | null =>
  engine ? ENGINE_NAMES[engine as keyof typeof ENGINE_NAMES] ?? engine : null;

// ---------------------------------------------------------------------------------------------
// API (api.md §1). Errors are {error: code, message}; the message is what the person reads.

export class ShowOnceError extends Error {
  constructor(message: string, readonly status: number, readonly code: string | null, readonly body: Record<string, unknown> | null) {
    super(message);
  }
}

const FALLBACK: Record<string, string> = {
  not_taken_over: 'Take over the browser first, then record.',
  already_open: 'This bot already has a recording open.',
  stage_busy: 'Another bot is recording in this browser.',
  no_steps: 'There are no steps left to write from.',
  no_engine: 'This bot has no engine to write the skill with.',
  limit_reached: 'This recording has used its 200 steps or 30 minutes.',
  too_many_notes: 'A recording can have 50 notes at most.',
  undo_over: 'That step is already gone.',
  bot_busy: 'The bot is busy. Try again when it has finished.',
  not_proposed: 'Only a skill waiting for review can be tried.',
};

async function api<T>(method: string, url: string, body?: unknown): Promise<T> {
  let res: Response;
  try {
    res = await fetch(url, {
      method,
      headers: body === undefined ? undefined : { 'Content-Type': 'application/json' },
      body: body === undefined ? undefined : JSON.stringify(body),
    });
  } catch {
    throw new ShowOnceError('kwant didn’t answer. Check the connection and try again.', 0, null, null);
  }
  if (!res.ok) {
    let data: Record<string, unknown> | null = null;
    try {
      data = await res.json();
    } catch {
      // no JSON body
    }
    const code = typeof data?.error === 'string' ? data.error : null;
    const message =
      (typeof data?.message === 'string' && data.message) || (code && FALLBACK[code]) || (res.status === 404 ? 'It no longer exists.' : `That failed (HTTP ${res.status}).`);
    throw new ShowOnceError(message, res.status, code, data);
  }
  if (res.status === 204) return undefined as T;
  return res.json();
}

const enc = encodeURIComponent;
const R = '/api/show-once/recordings';

export const showOnceApi = {
  start: (body: { bot: string; stage: string; start_url?: string; first_note?: string }) => api<{ recording: Recording }>('POST', R, body),
  current: (bot: string) => api<{ recording: Recording | null }>('GET', `${R}?bot=${enc(bot)}`),
  get: (id: string) => api<{ recording: Recording; steps: RecordedStep[] }>('GET', `${R}/${enc(id)}`),
  note: (id: string, body: { text: string; after?: string }) => api<{ step: RecordedStep }>('POST', `${R}/${enc(id)}/notes`, body),
  stop: (id: string) => api<{ recording: Recording }>('POST', `${R}/${enc(id)}/stop`),
  resume: (id: string) => api<{ recording: Recording }>('POST', `${R}/${enc(id)}/resume`),
  deleteStep: (id: string, step: string) => api<{ step: RecordedStep }>('DELETE', `${R}/${enc(id)}/steps/${enc(step)}`),
  restoreStep: (id: string, step: string) => api<{ step: RecordedStep }>('POST', `${R}/${enc(id)}/steps/${enc(step)}/restore`),
  discard: (id: string) => api<void>('DELETE', `${R}/${enc(id)}`),
  write: (id: string) => api<{ recording: Recording }>('POST', `${R}/${enc(id)}/write`),
  deleteSkillSteps: (skillId: string) => api<{ recording: Recording }>('DELETE', `/api/show-once/skills/${enc(skillId)}/steps`),
  tryOnce: (skillId: string, inputs: Record<string, string>) =>
    api<{ kwant_id: string; skill_id: string; message_id: string }>('POST', `/api/show-once/skills/${enc(skillId)}/try`, { inputs }),
};

export const errText = (e: unknown): string => (e instanceof Error && e.message ? e.message : 'That failed.');

// ---------------------------------------------------------------------------------------------
// Validation: anything off-shape is left out rather than shown wrong

const KINDS: StepKind[] = ['page', 'click', 'type', 'select', 'key', 'upload', 'note'];
const STATES: RecordingState[] = ['recording', 'stopped', 'writing', 'written'];
const str = (v: unknown): v is string => typeof v === 'string';
const num = (v: unknown): v is number => typeof v === 'number' && Number.isFinite(v);
const MASKS: MaskKind[] = ['password', 'code', 'payment', 'secret', 'saved_login'];

export const stepKey = (s: Pick<RecordedStep, 'id'>) => String(s.id);

export function isStep(v: unknown): v is RecordedStep {
  if (!v || typeof v !== 'object') return false;
  const s = v as Record<string, unknown>;
  if (!(str(s.id) || num(s.id)) || !num(s.n) || !KINDS.includes(s.kind as StepKind)) return false;
  if (s.masked != null && !(MASKS.includes(s.masked as MaskKind) && str(s.placeholder))) return false;
  switch (s.kind as StepKind) {
    case 'page':
      return str(s.origin) && str(s.path);
    case 'click':
      return str(s.name);
    case 'type':
      return str(s.label) && (s.masked != null || str(s.value));
    case 'select':
      return str(s.label) && str(s.value);
    case 'key':
      return str(s.value);
    case 'upload':
      return str(s.label);
    case 'note':
      return str(s.value) && s.value.trim() !== '';
  }
}

export function isRecording(v: unknown): v is Recording {
  if (!v || typeof v !== 'object') return false;
  const r = v as Record<string, unknown>;
  return str(r.id) && str(r.kwant_id) && STATES.includes(r.state as RecordingState) && num(r.elapsed_secs) && num(r.step_count);
}

export function isSkillShown(v: unknown): v is SkillShown {
  if (!v || typeof v !== 'object') return false;
  const r = v as Record<string, unknown>;
  return str(r.shown_on) && num(r.step_count);
}

export const limitsOf = (r: Recording | null): Limits => {
  const l = r?.limits;
  return l && num(l.steps) && num(l.secs) && num(l.near_steps_left) && num(l.near_secs) ? l : DEFAULT_LIMITS;
};

// ---------------------------------------------------------------------------------------------
// Words. Each step is a list of parts the components render as text; never HTML.

export type Part =
  | { t: 'text'; v: string }
  | { t: 'q'; v: string }
  | { t: 'url'; host: string; path: string }
  | { t: 'mask'; text: string; label: string }
  | { t: 'note'; v: string };

export const hostOf = (origin: string): string => origin.replace(/^[a-z][a-z0-9+.-]*:\/\//i, '').replace(/\/+$/, '');

/** "Opened shop.example.com/admin/orders", "Clicked button “Export CSV”", and so on (design S2). */
export function stepParts(s: RecordedStep, opts: { short?: boolean } = {}): Part[] {
  switch (s.kind) {
    case 'page':
      return [{ t: 'text', v: 'Opened ' }, { t: 'url', host: hostOf(s.origin ?? ''), path: s.path ?? '' }];
    case 'click':
      return [{ t: 'text', v: `Clicked ${s.role ? `${s.role} ` : ''}` }, { t: 'q', v: s.name ?? '' }];
    case 'type': {
      if (s.masked) {
        const label = MASK_LABEL[s.masked];
        const short = opts.short ? MASK_SHORT[s.masked] : undefined;
        if (short) return [{ t: 'text', v: `${short.noun} kept as ` }, { t: 'mask', text: short.chip, label }];
        return [{ t: 'text', v: 'Typed in ' }, { t: 'q', v: s.label ?? '' }, { t: 'text', v: ', kept as ' }, { t: 'mask', text: s.placeholder ?? '', label }];
      }
      return [{ t: 'text', v: 'Typed ' }, { t: 'q', v: s.value ?? '' }, { t: 'text', v: ' in ' }, { t: 'q', v: s.label ?? '' }];
    }
    case 'select':
      return [{ t: 'text', v: 'Chose ' }, { t: 'q', v: s.value ?? '' }, { t: 'text', v: ' in ' }, { t: 'q', v: s.label ?? '' }];
    case 'key':
      return [{ t: 'text', v: `Pressed ${s.value ?? ''}` }];
    case 'upload':
      return [{ t: 'text', v: 'Uploaded a file in ' }, { t: 'q', v: s.label ?? '' }];
    case 'note':
      return [{ t: 'note', v: s.value ?? '' }];
  }
}

/** The same step as one plain sentence, for labels and the screen reader. */
export function stepSentence(s: RecordedStep, opts: { short?: boolean } = {}): string {
  return stepParts(s, opts)
    .map((p) => (p.t === 'text' ? p.v : p.t === 'q' ? `“${p.v}”` : p.t === 'url' ? `${p.host}${p.path}` : p.t === 'note' ? `Note: ${p.v}` : p.label))
    .join('');
}

/** "11 steps · 2:48" and friends */
export function clock(ms: number): string {
  const total = Math.max(0, Math.floor(ms / 1000));
  return `${Math.floor(total / 60)}:${String(total % 60).padStart(2, '0')}`;
}

/** "3 min 40 s", as in the review header. */
export function duration(ms: number): string {
  const total = Math.max(0, Math.floor(ms / 1000));
  const m = Math.floor(total / 60);
  const s = total % 60;
  if (m === 0) return `${s} s`;
  return s ? `${m} min ${s} s` : `${m} min`;
}

export const stepsWord = (n: number) => `${n} ${n === 1 ? 'step' : 'steps'}`;

/** A34: null while there's room; else what's left, as the bar says it ("2 min or 8 steps left"). */
export function nearLimit(stepCount: number, elapsedMs: number, limits: Limits, serverNear = false): string | null {
  const stepsLeft = limits.steps - stepCount;
  if (!serverNear && stepsLeft > limits.near_steps_left && elapsedMs < limits.near_secs * 1000) return null;
  const minLeft = Math.max(0, Math.ceil((limits.secs * 1000 - elapsedMs) / 60_000));
  return `${minLeft} min or ${stepsWord(Math.max(0, stepsLeft))} left`;
}

/** "15 steps · 3 min 40 s · shop.example.com, crm.example.com", or the limit variant. */
export function reviewSub(r: Recording, listed: number): string {
  if (r.stop_reason === 'limit_time' || r.stop_reason === 'limit_steps') return `${stepsWord(listed)} · ${clock(r.elapsed_secs * 1000)} · stopped at the limit`;
  const hosts = (r.origins ?? []).filter(str).map(hostOf);
  return [stepsWord(listed), duration(r.elapsed_secs * 1000), hosts.join(', ')].filter(Boolean).join(' · ');
}

/** Why the recording stopped, when it wasn't the person's Stop (warn banner). */
export function stopBanner(r: Recording, listed: number): { lead: string; rest: string } | null {
  const limits = limitsOf(r);
  switch (r.stop_reason) {
    case 'limit_time':
      return { lead: `Stopped at ${Math.round(limits.secs / 60)} minutes,`, rest: ` the most one recording can take. All ${stepsWord(listed)} up to there are kept.` };
    case 'limit_steps':
      return { lead: `Stopped at ${limits.steps} steps,`, rest: ' the most one recording can take. All of them are kept.' };
    case 'closed':
      return { lead: 'The recording stopped when the browser closed.', rest: ' The steps up to there are kept.' };
    case 'restart':
      return { lead: 'The recording stopped when kwant restarted.', rest: ' The steps up to there are kept.' };
    default:
      return null;
  }
}

const DATE_LONG: Intl.DateTimeFormatOptions = { month: 'short', day: 'numeric', year: 'numeric' };
const DATE_SHORT: Intl.DateTimeFormatOptions = { month: 'short', day: 'numeric' };
export function dayOf(iso: string | null | undefined, withYear = true): string | null {
  if (!iso) return null;
  const d = new Date(iso);
  if (Number.isNaN(d.getTime())) return null;
  return d.toLocaleDateString('en-US', withYear ? DATE_LONG : DATE_SHORT);
}

/** "Nov 7, 2026" → "Nov 7", for the In use line. */
export const shortDay = (shownOn: string) => shownOn.replace(/,\s*\d{4}$/, '');

/** What Write the skill can't do yet, in the person's words; null when it can. */
export function writeBlocker(r: Recording, steps: RecordedStep[], botName: string): string | null {
  if (!r.engine) return `${botName} has no engine to write the skill with.`;
  if (!steps.some((s) => !s.deleted && s.kind !== 'note')) return 'There are no steps left to write from. Keep recording, or discard.';
  return null;
}

// ---------------------------------------------------------------------------------------------
// The drafted SKILL.md, split into the sections the card shows (A33). Null when it doesn't parse:
// the card then shows the body as it is.

export interface SkillInput {
  name: string;
  example: string;
}
export interface SkillSections {
  goal: string;
  whenToUse: string | null;
  inputs: SkillInput[];
  /** The Inputs section when it isn't a table or list of names; shown as text */
  inputsText: string | null;
  steps: string[];
  checks: string | null;
  wrong: string | null;
  sites: string[];
}

const SECTION_KEYS: Record<string, keyof SkillSections> = {
  goal: 'goal',
  'when to use': 'whenToUse',
  'when to use it': 'whenToUse',
  inputs: 'inputs',
  steps: 'steps',
  checks: 'checks',
  'what can go wrong': 'wrong',
  sites: 'sites',
  'sites used': 'sites',
};

const joinLines = (lines: string[]) => lines.map((l) => l.trim()).filter(Boolean).join(' ').trim();

export function parseSkillSections(body: string): SkillSections | null {
  if (typeof body !== 'string') return null;
  const sections = new Map<keyof SkillSections, string[]>();
  let current: keyof SkillSections | null = null;
  for (const raw of body.split(/\r?\n/)) {
    const h = /^#{1,4}\s+(.+?)\s*:?\s*$/.exec(raw);
    if (h) {
      const key = SECTION_KEYS[h[1].toLowerCase().replace(/\s+/g, ' ')];
      if (key) {
        current = key;
        sections.set(key, []);
        continue;
      }
      if (sections.size === 0) continue; // the skill's own title
      current = null;
      continue;
    }
    if (current) sections.get(current)!.push(raw);
  }
  const goal = joinLines(sections.get('goal') ?? []);
  const stepLines = (sections.get('steps') ?? []).map((l) => /^\s*(?:\d+[.)]|[-*])\s+(.*)$/.exec(l)?.[1]?.trim()).filter((l): l is string => !!l);
  if (!goal || stepLines.length === 0) return null;

  const inputLines = (sections.get('inputs') ?? []).filter((l) => l.trim());
  const inputs: SkillInput[] = [];
  let tableish = true;
  for (const l of inputLines) {
    const row = /^\s*\|(.*)\|\s*$/.exec(l);
    if (row) {
      const cells = row[1].split('|').map((c) => c.trim());
      if (cells.every((c) => /^:?-{2,}:?$/.test(c))) continue;
      const name = /^`([^`]+)`$/.exec(cells[0])?.[1];
      if (!name) continue; // the header row
      inputs.push({ name, example: cells[1] ?? '' });
      continue;
    }
    const item = /^\s*[-*]\s+`([^`]+)`\s*[:—-]?\s*(.*)$/.exec(l);
    if (item) {
      inputs.push({ name: item[1], example: item[2].replace(/^(?:e\.g\.|example:?)\s*/i, '').replace(/^[“"](.*)[”"]$/, '$1') });
      continue;
    }
    tableish = false;
  }
  const inputsText = !tableish || (inputLines.length > 0 && inputs.length === 0) ? joinLines(inputLines) : null;

  // Sites are the recorded origins kwant wrote; the rule line and the "Shown by you" line aren't sites
  const sites: string[] = [];
  for (const raw of sections.get('sites') ?? []) {
    for (const w of raw.replace(/[`*]/g, ' ').split(/[\s,]+/)) {
      const host = hostOf(w.replace(/[.;:]+$/, ''));
      if (/^[a-z0-9-]+(\.[a-z0-9-]+)+(:\d+)?$/i.test(host) && /[a-z]/i.test(host) && !sites.includes(host)) sites.push(host);
    }
  }

  return {
    goal,
    whenToUse: joinLines(sections.get('whenToUse') ?? []) || null,
    inputs: inputsText ? [] : inputs,
    inputsText,
    steps: stepLines,
    checks: joinLines(sections.get('checks') ?? []) || null,
    wrong: joinLines(sections.get('wrong') ?? []) || null,
    sites,
  };
}

/** Inline text of the draft: `code` becomes a chip and the placeholders become mask chips. */
export type InlinePart = { t: 'text'; v: string } | { t: 'code'; v: string } | { t: 'mask'; text: string; label: string };

const escapeRe = (x: string) => x.replace(/[.*+?^${}()|[\]\\]/g, '\\$&');
const INLINE_RE = new RegExp(`\`([^\`]+)\`|${Object.keys(PLACEHOLDERS).map(escapeRe).join('|')}`, 'g');

export function inlineParts(text: string): InlinePart[] {
  const out: InlinePart[] = [];
  let last = 0;
  for (const m of text.matchAll(INLINE_RE)) {
    if (m.index! > last) out.push({ t: 'text', v: text.slice(last, m.index) });
    if (m[1] !== undefined) out.push({ t: 'code', v: m[1] });
    else out.push({ t: 'mask', text: m[0], label: MASK_LABEL[PLACEHOLDERS[m[0]]] });
    last = m.index! + m[0].length;
  }
  if (last < text.length) out.push({ t: 'text', v: text.slice(last) });
  return out;
}

// ---------------------------------------------------------------------------------------------
// The one recording in progress. The Skills tab or the stage starts it; the chat's browser pane
// (stage on desktop, full-screen overlay on phone) shows the bar and the review.

/**
 * taking: the Skills dialog asked for Take over and waits for it (api.md: Start needs it first);
 * starting: Start was sent; recording; review (stopped, incl. a failed write); writing; written.
 */
export type Phase = 'taking' | 'starting' | 'recording' | 'review' | 'writing' | 'written';

export interface ShowOnceSession {
  kwantId: string;
  botName: string;
  /** The stream id the recording runs on (api.md §0) */
  stage: string;
  rec: Recording | null;
  steps: RecordedStep[];
  phase: Phase;
  /** From the start dialog, sent once Take over holds */
  pending: { startUrl: string | null; firstNote: string | null } | null;
  /** A27: Hand back was pressed while recording, so hand back when the review closes */
  handBackAfter: boolean;
  /** Local time of the last recording update, so the bar's clock runs between updates */
  syncedAt: number;
  /** The last action's failure, said where it happened */
  error: string | null;
}

type Listener = () => void;
let session: ShowOnceSession | null = null;
const listeners = new Set<Listener>();
const endListeners = new Set<(e: { stage: string; handBack: boolean }) => void>();
const emit = () => listeners.forEach((l) => l());

export const showOnceStore = {
  get: (): ShowOnceSession | null => session,
  set(next: ShowOnceSession | null) {
    session = next;
    emit();
  },
  update(fn: (s: ShowOnceSession) => ShowOnceSession) {
    if (session) showOnceStore.set(fn(session));
  },
  subscribe(l: Listener) {
    listeners.add(l);
    return () => void listeners.delete(l);
  },
  /** The session ended; `handBack` when the browser should go back to the bot (A27, Writing's Close). */
  onEnd(l: (e: { stage: string; handBack: boolean }) => void) {
    endListeners.add(l);
    return () => void endListeners.delete(l);
  },
};

export const useShowOnce = (): ShowOnceSession | null => useSyncExternalStore(showOnceStore.subscribe, showOnceStore.get, showOnceStore.get);

const now = () => Date.now();

function end(handBack: boolean) {
  const s = session;
  showOnceStore.set(null);
  if (s) endListeners.forEach((l) => l({ stage: s.stage, handBack }));
}

export function phaseOf(state: RecordingState): Phase {
  return state === 'recording' ? 'recording' : state === 'stopped' ? 'review' : state;
}

/** A recording from the server lands in the session. Steps come separately (they're never on it). */
export function applyRecording(s: ShowOnceSession, rec: Recording): ShowOnceSession {
  if (s.rec && rec.id !== s.rec.id) return s;
  return { ...s, rec, phase: phaseOf(rec.state), syncedAt: now() };
}

/** Steps in the server's order; a malformed one is left out. */
export const cleanSteps = (steps: unknown): RecordedStep[] => (Array.isArray(steps) ? steps.filter(isStep) : []);

function mergeStep(steps: RecordedStep[], step: RecordedStep): RecordedStep[] {
  const i = steps.findIndex((x) => stepKey(x) === stepKey(step));
  if (i < 0) return [...steps, step];
  const out = steps.slice();
  out[i] = step;
  return out;
}

/** GET the recording and its steps: after any edit, so numbers and counts are the server's. */
async function refresh(): Promise<void> {
  const id = session?.rec?.id;
  if (!id) return;
  try {
    const { recording, steps } = await showOnceApi.get(id);
    if (!session || session.rec?.id !== id || !isRecording(recording)) return;
    const next = applyRecording(session, recording);
    // While a write is out, keep saying "writing" until the server says otherwise
    showOnceStore.set({ ...next, steps: cleanSteps(steps) });
  } catch (e) {
    if (e instanceof ShowOnceError && e.status === 404 && session?.rec?.id === id) end(false);
  }
}

const purgeTimers = new Map<string, ReturnType<typeof setTimeout>>();
/** A deleted step is purged at undo_until; read the list again then, so the rest renumber (A35). */
function refreshAfterUndo(step: RecordedStep) {
  const key = stepKey(step);
  clearTimeout(purgeTimers.get(key));
  const at = step.undo_until ? new Date(step.undo_until).getTime() : NaN;
  const wait = Number.isNaN(at) ? 10_000 : Math.max(0, at - now());
  purgeTimers.set(
    key,
    setTimeout(() => {
      purgeTimers.delete(key);
      void refresh();
    }, wait + 400),
  );
}

/** Frames on the stage's stream socket (api.md §5); only the socket that holds Take over gets them. */
export function onRecordingFrame(m: Record<string, unknown>) {
  const s = session;
  if (!s?.rec || m.recording_id !== s.rec.id && !(m.type === 'skill.recording.state' && isRecording(m.recording) && m.recording.id === s.rec.id)) return;
  switch (m.type) {
    case 'skill.recording.step':
      if (!isStep(m.step)) return;
      {
        const step = m.step;
        const known = s.steps.some((x) => stepKey(x) === stepKey(step));
        // step_count mirrors the server's until its next update (notes don't count, api.md §8)
        const rec = !known && step.kind !== 'note' ? { ...s.rec, step_count: s.rec.step_count + 1 } : s.rec;
        showOnceStore.set({ ...s, rec, steps: mergeStep(s.steps, step) });
      }
      return;
    case 'skill.recording.near':
      showOnceStore.set({ ...s, rec: { ...s.rec, near: true } });
      return;
    case 'skill.recording.stopped':
      if (s.phase === 'recording') showOnceStore.set({ ...s, phase: 'review' });
      void refresh();
      return;
    case 'skill.recording.state':
      if (isRecording(m.recording)) showOnceStore.set(applyRecording(s, m.recording));
      return;
  }
}

onContinuity('recording_updated', (rec) => {
  if (!session?.rec || !isRecording(rec) || rec.id !== session.rec.id) return;
  const was = session.phase;
  showOnceStore.set(applyRecording(session, rec));
  // A stop the stage didn't see (the limit, the sweep), or a write that came back failed: fetch the steps
  if (rec.state === 'stopped' && was !== 'review') void refresh();
});

async function run<T>(fn: () => Promise<T>, then: (s: ShowOnceSession, v: T) => ShowOnceSession | null): Promise<string | null> {
  try {
    const v = await fn();
    if (session) {
      const next = then(session, v);
      if (next) showOnceStore.set({ ...next, error: null });
    }
    return null;
  } catch (e) {
    return errText(e);
  }
}

const withRec = (s: ShowOnceSession, rec: unknown) => (isRecording(rec) ? applyRecording(s, rec) : s);

export const showOnce = {
  /** Start recording dialog (A26): open the stage, then Start once Take over holds. */
  begin(kwantId: string, botName: string, stage: string, startUrl: string | null, firstNote: string | null) {
    showOnceStore.set({
      kwantId,
      botName,
      stage,
      rec: null,
      steps: [],
      phase: 'taking',
      pending: { startUrl, firstNote },
      handBackAfter: false,
      syncedAt: now(),
      error: null,
    });
  },

  /** Record steps on the stage: Take over already holds, so Start now on the current page. */
  record(kwantId: string, botName: string, stage: string): Promise<string | null> {
    showOnce.begin(kwantId, botName, stage, null, null);
    return showOnce.startNow();
  },

  /** Sends Start (the stage calls this once it holds Take over). */
  async startNow(): Promise<string | null> {
    const s = session;
    if (!s || (s.phase !== 'taking' && s.phase !== 'starting') || s.rec) return null;
    const p = s.pending ?? { startUrl: null, firstNote: null };
    showOnceStore.set({ ...s, phase: 'starting', error: null });
    try {
      const body: { bot: string; stage: string; start_url?: string; first_note?: string } = { bot: s.kwantId, stage: s.stage };
      if (p.startUrl) body.start_url = p.startUrl;
      if (p.firstNote) body.first_note = p.firstNote;
      const { recording } = await showOnceApi.start(body);
      if (!session || !isRecording(recording)) return null;
      showOnceStore.set({ ...applyRecording(session, recording), pending: null, error: null });
      void refresh();
      return null;
    } catch (e) {
      // already_open: the bot's open recording comes back, so resume it or show its review
      if (e instanceof ShowOnceError && e.code === 'already_open' && isRecording(e.body?.recording) && session) {
        showOnceStore.set({ ...applyRecording(session, e.body!.recording as Recording), pending: null, error: null });
        void refresh();
        return null;
      }
      const msg = errText(e);
      showOnceStore.update((x) => ({ ...x, error: msg }));
      return msg;
    }
  },

  /** S-B5: on opening the stage, a recording left open (reload, closed tab, restart) is offered again. */
  async resumeOpen(kwantId: string, botName: string, stage: string): Promise<void> {
    if (session) return;
    try {
      const { recording } = await showOnceApi.current(kwantId);
      if (session || !isRecording(recording)) return;
      showOnceStore.set({
        kwantId,
        botName,
        stage: recording.stage || stage,
        rec: recording,
        steps: [],
        phase: phaseOf(recording.state),
        pending: null,
        handBackAfter: false,
        syncedAt: now(),
        error: null,
      });
      await refresh();
    } catch {
      // nothing open, or no access: the stage stays as it was
    }
  },

  refresh,

  note: (text: string, after?: string): Promise<string | null> => {
    const id = session?.rec?.id;
    if (!id) return Promise.resolve(null);
    return run(
      () => showOnceApi.note(id, after ? { text, after } : { text }),
      (s, v) => (isStep(v.step) ? { ...s, steps: after ? s.steps : mergeStep(s.steps, v.step) } : s),
    ).then((err) => {
      if (!err) void refresh();
      return err;
    });
  },

  /** Stop, or Hand back while recording (`handBack`, A27). */
  async stop(handBack = false): Promise<string | null> {
    const s = session;
    if (!s?.rec || s.phase !== 'recording') return null;
    const id = s.rec.id;
    showOnceStore.set({ ...s, phase: 'review', handBackAfter: s.handBackAfter || handBack });
    const err = await run(() => showOnceApi.stop(id), (x, v) => withRec(x, v.recording));
    if (err) showOnceStore.update((x) => ({ ...x, error: err }));
    void refresh();
    return err;
  },

  /** A30: back to the stage, adding to the same list. */
  async resume(): Promise<string | null> {
    const id = session?.rec?.id;
    if (!id) return null;
    const err = await run(() => showOnceApi.resume(id), (x, v) => ({ ...withRec(x, v.recording), phase: 'recording', handBackAfter: false }));
    if (err) showOnceStore.update((x) => ({ ...x, error: err }));
    return err;
  },

  deleteStep: (stepId: string): Promise<string | null> => {
    const id = session?.rec?.id;
    if (!id) return Promise.resolve(null);
    return run(
      () => showOnceApi.deleteStep(id, stepId),
      (s, v) => {
        if (!isStep(v.step)) return s;
        refreshAfterUndo(v.step);
        return { ...s, steps: mergeStep(s.steps, v.step) };
      },
    );
  },

  restoreStep: (stepId: string): Promise<string | null> => {
    const id = session?.rec?.id;
    if (!id) return Promise.resolve(null);
    clearTimeout(purgeTimers.get(stepId));
    purgeTimers.delete(stepId);
    return run(
      () => showOnceApi.restoreStep(id, stepId),
      (s, v) => (isStep(v.step) ? { ...s, steps: mergeStep(s.steps, v.step) } : s),
    ).then((err) => {
      if (err) void refresh();
      return err;
    });
  },

  async write(): Promise<string | null> {
    const s = session;
    if (!s?.rec) return null;
    const id = s.rec.id;
    showOnceStore.set({ ...s, phase: 'writing', error: null });
    const err = await run(() => showOnceApi.write(id), (x, v) => withRec(x, v.recording));
    if (err) showOnceStore.update((x) => ({ ...x, phase: 'review', error: err }));
    return err;
  },

  async discard(): Promise<string | null> {
    const id = session?.rec?.id;
    if (!id) {
      end(session?.handBackAfter ?? false);
      return null;
    }
    try {
      await showOnceApi.discard(id);
      end(session?.handBackAfter ?? false);
      return null;
    } catch (e) {
      return errText(e);
    }
  },

  /** Close while writing or once written: the draft arrives in Skills. Closing hands the browser back. */
  close() {
    end(true);
  },

  /** Cancel a start that didn't go through. */
  cancel() {
    end(false);
  },
};

/** Recorded time now: the server's figure plus the time since it came, while recording. */
export function elapsedNow(s: ShowOnceSession, at = now()): number {
  if (!s.rec) return 0;
  return s.rec.elapsed_secs * 1000 + (s.phase === 'recording' ? Math.max(0, at - s.syncedAt) : 0);
}
