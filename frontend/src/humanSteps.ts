// Human steps: a bot asks the person to do one thing in its browser (sign in, a CAPTCHA, a code),
// then waits for Done or Skip (docs/live-activity.md, part 2).
import { parseMetaObject } from './questions';
import { request } from './threads';

export type HumanStepKind = 'sign_in' | 'captcha' | 'two_factor' | 'other';
export type HumanStepStatus = 'open' | 'done' | 'declined' | 'cancelled' | 'resolved_auto' | 'timed_out';

/** Where a CAPTCHA widget sits on the bot's page, in page pixels (1280×800). */
export interface CaptchaTarget {
  kind: string;
  x: number;
  y: number;
  width: number;
  height: number;
}

export interface HumanStep {
  id: string;
  kwant_id: string;
  channel_id: string | null;
  direct_kwant_id: string | null;
  thread_id: string | null;
  turn_id: string | null;
  kind: HumanStepKind;
  reason: string;
  url: string | null;
  sandbox_target: string | null;
  /** CAPTCHA steps: the widget's box, while known. */
  target?: CaptchaTarget | null;
  source: 'bot' | 'auto';
  status: HumanStepStatus;
  message_id: string | null;
  created_at: string;
  closed_at: string | null;
}

/** `meta` of the card message. */
export interface HumanStepMeta {
  kind: 'human_step';
  human_step_id: string;
  step_kind: HumanStepKind;
  reason: string;
  url: string | null;
  sandbox_target: string | null;
  target: CaptchaTarget | null;
  source: 'bot' | 'auto';
  status: HumanStepStatus;
}

const STEP_KINDS: HumanStepKind[] = ['sign_in', 'captcha', 'two_factor', 'other'];
const STATUSES: HumanStepStatus[] = ['open', 'done', 'declined', 'cancelled', 'resolved_auto', 'timed_out'];
const kindOf = (v: unknown): HumanStepKind => (STEP_KINDS.includes(v as HumanStepKind) ? (v as HumanStepKind) : 'other');
const statusOf = (v: unknown): HumanStepStatus => (STATUSES.includes(v as HumanStepStatus) ? (v as HumanStepStatus) : 'open');

/** A usable widget box (finite, non-empty), else null. */
export function parseCaptchaTarget(raw: unknown): CaptchaTarget | null {
  if (!raw || typeof raw !== 'object') return null;
  const t = raw as Record<string, unknown>;
  const n = (v: unknown) => (typeof v === 'number' && Number.isFinite(v) ? v : NaN);
  const [x, y, width, height] = [n(t.x), n(t.y), n(t.width), n(t.height)];
  if ([x, y, width, height].some(Number.isNaN) || width <= 0 || height <= 0) return null;
  return { kind: typeof t.kind === 'string' ? t.kind : 'puzzle', x, y, width, height };
}

export function parseHumanStepMeta(raw: unknown): HumanStepMeta | null {
  const m = parseMetaObject(raw);
  if (!m || m.kind !== 'human_step' || typeof m.human_step_id !== 'string') return null;
  return {
    kind: 'human_step',
    human_step_id: m.human_step_id,
    step_kind: kindOf(m.step_kind),
    reason: typeof m.reason === 'string' ? m.reason : '',
    url: typeof m.url === 'string' && m.url ? m.url : null,
    sandbox_target: typeof m.sandbox_target === 'string' && m.sandbox_target ? m.sandbox_target : null,
    target: parseCaptchaTarget(m.target),
    source: m.source === 'auto' ? 'auto' : 'bot',
    status: statusOf(m.status),
  };
}

/** Card state: the live record from `human_step_updated` wins over the (possibly stale) meta. */
export function humanStepView(meta: HumanStepMeta, live?: HumanStep | null): HumanStepMeta {
  if (!live || live.id !== meta.human_step_id) return meta;
  return {
    ...meta,
    status: live.status,
    url: live.url ?? meta.url,
    sandbox_target: live.sandbox_target ?? meta.sandbox_target,
    target: parseCaptchaTarget(live.target) ?? meta.target,
  };
}

/** Keeps the latest known record per step; a closed step never reopens. */
export function applyHumanStep(map: Record<string, HumanStep>, s: HumanStep): Record<string, HumanStep> {
  const prev = map[s.id];
  if (prev && prev.status === s.status && sameTarget(prev.target, s.target)) return map;
  if (prev && prev.status !== 'open' && s.status === 'open') return map;
  return { ...map, [s.id]: s };
}

function sameTarget(a: CaptchaTarget | null | undefined, b: CaptchaTarget | null | undefined): boolean {
  if (!a || !b) return !a === !b;
  return a.kind === b.kind && a.x === b.x && a.y === b.y && a.width === b.width && a.height === b.height;
}

/**
 * The CAPTCHA step to show over a bot browser: its newest open one, else the newest that closed
 * in the last minute (so the view can say it was solved).
 */
export function captchaStepFor(map: Record<string, HumanStep>, sandboxId: string | undefined, now = Date.now()): HumanStep | null {
  if (!sandboxId) return null;
  let open: HumanStep | null = null;
  let closed: HumanStep | null = null;
  for (const s of Object.values(map)) {
    if (s.kind !== 'captcha' || s.sandbox_target !== sandboxId) continue;
    if (s.status === 'open') {
      if (!open || s.created_at > open.created_at) open = s;
    } else if (s.closed_at && now - Date.parse(s.closed_at) < 60_000 && (!closed || s.closed_at > (closed.closed_at ?? ''))) {
      closed = s;
    }
  }
  return open ?? closed;
}

/** The chat a step lives in: the thread, else the channel, else the bot's direct chat. */
export const humanStepContextId = (s: Pick<HumanStep, 'thread_id' | 'channel_id' | 'direct_kwant_id'>) =>
  s.thread_id || s.channel_id || s.direct_kwant_id || null;

/** Open steps per bot, for the sidebar badge. */
export function openStepsByKwant(map: Record<string, HumanStep>): Record<string, number> {
  const out: Record<string, number> = {};
  for (const s of Object.values(map)) if (s.status === 'open') out[s.kwant_id] = (out[s.kwant_id] ?? 0) + 1;
  return out;
}

/** The newest open step a bot raised, e.g. to open from a notification. */
export function latestOpenStep(map: Record<string, HumanStep>, kwantId: string): HumanStep | null {
  let best: HumanStep | null = null;
  for (const s of Object.values(map)) if (s.kwant_id === kwantId && s.status === 'open' && (!best || s.created_at > best.created_at)) best = s;
  return best;
}

/** Turn ids paused on a step (waiting for the person, not working). */
export function turnsAwaitingStep(map: Record<string, HumanStep>): Set<string> {
  const out = new Set<string>();
  for (const s of Object.values(map)) if (s.status === 'open' && s.turn_id) out.add(s.turn_id);
  return out;
}

/** Heading and the Done button's words for each kind. */
export const KIND_TEXT: Record<HumanStepKind, { heading: string; done: string }> = {
  sign_in: { heading: 'Sign in needed', done: 'Done, I’m signed in' },
  captcha: { heading: 'Human check needed', done: 'Done, I solved it' },
  two_factor: { heading: 'Sign-in code needed', done: 'Done, I entered the code' },
  other: { heading: 'Your help is needed', done: 'Done' },
};

/** Only the host of a step's address ("github.com"), so the card never shows paths. */
export function domainOf(url: string | null | undefined): string | null {
  if (!url) return null;
  try {
    const host = new URL(url).hostname.replace(/^www\./, '');
    return host || null;
  } catch {
    return null;
  }
}

const enc = encodeURIComponent;

export const humanStepsApi = {
  listOpen: () => request<HumanStep[]>('GET', '/api/human-steps?status=open'),
  get: (id: string) => request<{ step: HumanStep; waiting: boolean }>('GET', `/api/human-steps/${enc(id)}`),
  done: (id: string) => request<{ step: HumanStep; resumed: 'turn' | 'new_turn' }>('POST', `/api/human-steps/${enc(id)}/done`),
  decline: (id: string) => request<{ step: HumanStep; resumed: 'turn' | 'new_turn' }>('POST', `/api/human-steps/${enc(id)}/decline`),
};
