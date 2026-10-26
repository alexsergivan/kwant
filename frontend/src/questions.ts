// Ask-user: an agent's one-tap question in a chat (batch 2 / C).
import { request } from './threads';

export type QuestionStatus = 'open' | 'answered' | 'cancelled';

export interface Question {
  id: string;
  kwant_id: string;
  channel_id: string | null;
  direct_kwant_id: string | null;
  thread_id: string | null;
  /** The turn that asked; it may have ended since (then an answer starts a new turn). */
  turn_id: string | null;
  question: string;
  choices: string[];
  status: QuestionStatus;
  answer: string | null;
  message_id: string | null;
  created_at: string;
  answered_at: string | null;
}

/** `meta` of the card message. */
export interface QuestionMeta {
  kind: 'ask_user';
  question_id: string;
  question: string;
  choices: string[];
  status: QuestionStatus;
  answer: string | null;
  /** Why only the person can answer (decide-first); null on older cards. */
  why_you?: string | null;
  /** What the bot already tried; null on older cards. */
  checked?: string | null;
}

export const ANSWER_MAX = 2000;

/** Message `meta` comes as a JSON string (or, from some paths, already parsed). */
export function parseMetaObject(raw: unknown): Record<string, unknown> | null {
  if (!raw) return null;
  if (typeof raw === 'object') return raw as Record<string, unknown>;
  if (typeof raw !== 'string') return null;
  try {
    const v = JSON.parse(raw);
    return v && typeof v === 'object' && !Array.isArray(v) ? v : null;
  } catch {
    return null;
  }
}

export function parseQuestionMeta(raw: unknown): QuestionMeta | null {
  const m = parseMetaObject(raw);
  if (!m || m.kind !== 'ask_user' || typeof m.question_id !== 'string' || typeof m.question !== 'string') return null;
  const choices = Array.isArray(m.choices) ? m.choices.filter((c): c is string => typeof c === 'string') : [];
  const status: QuestionStatus = m.status === 'answered' || m.status === 'cancelled' ? m.status : 'open';
  return {
    kind: 'ask_user',
    question_id: m.question_id,
    question: m.question,
    choices,
    status,
    answer: typeof m.answer === 'string' ? m.answer : null,
    why_you: typeof m.why_you === 'string' ? m.why_you : null,
    checked: typeof m.checked === 'string' && m.checked.trim() ? m.checked.trim() : null,
  };
}

/** The chat a question lives in: the thread, else the channel, else the agent's direct chat. */
export const questionContextId = (q: Pick<Question, 'thread_id' | 'channel_id' | 'direct_kwant_id'>) =>
  q.thread_id || q.channel_id || q.direct_kwant_id || null;

/** Card state: the live record from `question_updated` wins over the (possibly stale) message meta. */
export function questionView(meta: QuestionMeta, live?: Question | null): QuestionMeta {
  if (!live || live.id !== meta.question_id) return meta;
  return { ...meta, status: live.status, answer: live.answer, choices: live.choices.length ? live.choices : meta.choices };
}

/** Keeps the latest known record per question. */
export function applyQuestion(map: Record<string, Question>, q: Question): Record<string, Question> {
  const prev = map[q.id];
  if (prev && prev.status === q.status && prev.answer === q.answer) return map;
  // A closed question never reopens (a late, out-of-order "open" event loses).
  if (prev && prev.status !== 'open' && q.status === 'open') return map;
  return { ...map, [q.id]: q };
}

export function hydrateQuestions(list: Question[]): Record<string, Question> {
  const out: Record<string, Question> = {};
  for (const q of list) out[q.id] = q;
  return out;
}

/** Open questions per chat, newest last. */
export function openQuestionsByContext(map: Record<string, Question>): Record<string, Question[]> {
  const out: Record<string, Question[]> = {};
  const open = Object.values(map)
    .filter((q) => q.status === 'open')
    .sort((a, b) => a.created_at.localeCompare(b.created_at));
  for (const q of open) {
    const ctx = questionContextId(q);
    if (ctx) (out[ctx] ??= []).push(q);
  }
  return out;
}

/** The newest open question an agent asked, e.g. to open from a notification. */
export function latestOpenQuestion(map: Record<string, Question>, kwantId: string): Question | null {
  let best: Question | null = null;
  for (const q of Object.values(map)) if (q.kwant_id === kwantId && q.status === 'open' && (!best || q.created_at > best.created_at)) best = q;
  return best;
}

/** Turn ids that are paused on a question (not working, just waiting for the person). */
export function turnsAwaitingAnswer(map: Record<string, Question>): Set<string> {
  const out = new Set<string>();
  for (const q of Object.values(map)) if (q.status === 'open' && q.turn_id) out.add(q.turn_id);
  return out;
}

/** Problem with a typed answer, or null. */
export function answerTextError(text: string): string | null {
  if (!text.trim()) return 'Type an answer or tap a choice.';
  if (text.length > ANSWER_MAX) return `Answers are at most ${ANSWER_MAX} characters.`;
  return null;
}

const enc = encodeURIComponent;

export const questionsApi = {
  listOpen: () => request<Question[]>('GET', '/api/questions?status=open'),
  get: (id: string) => request<{ question: Question; waiting: boolean }>('GET', `/api/questions/${enc(id)}`),
  answer: (id: string, a: { choice: number } | { text: string }) =>
    request<{ question: Question; resumed: 'turn' | 'new_turn' }>('POST', `/api/questions/${enc(id)}/answer`, a),
  cancel: (id: string) => request<Question>('POST', `/api/questions/${enc(id)}/cancel`),
};
