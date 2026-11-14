// 0.9.8 first screen for anyone (autonomy/first-screen/design.md, Mira): the pure parts, so tests can
// check the words and the gating without a DOM. A: the space header's status words and the "moved"
// pointer. D: Get started step 3 when the space already has a team. E: the "Connect an AI account" card.

import type { SpaceCounts } from './spaces';
import { spaceSummary } from './spaces';

// ------------------------------------------------------------------ A: the header sub-line

export type SpaceApprovals = 'ask' | 'smart' | 'off' | string | null | undefined;

/** One run of the sub-line; `tone` colours a status word (Dev mode sky, Approvals off amber). */
export interface SublinePart {
  text: string;
  tone?: 'dev' | 'off';
}

/**
 * The space header's sub-line: the bot count, then the status words (Dev mode, then Approvals off), then
 * the rest of today's summary. Status sits right after the count so `truncate` cuts the accounts first.
 * Ask and When risky add nothing: only the settings that change how bots behave are named.
 */
export function spaceSublineParts(counts: SpaceCounts | undefined, s: { devOn?: boolean; approvals?: SpaceApprovals }): SublinePart[] {
  const summary = spaceSummary(counts);
  if (!summary) return [];
  const [count, ...rest] = summary.split(' · ');
  const out: SublinePart[] = [{ text: count }];
  if (s.devOn) out.push({ text: 'Dev mode', tone: 'dev' });
  if (s.approvals === 'off') out.push({ text: 'Approvals off', tone: 'off' });
  for (const r of rest) out.push({ text: r });
  return out;
}

/** The whole sub-line as one string, for its `title`. */
export const sublineText = (parts: SublinePart[]): string => parts.map((p) => p.text).join(' · ');

// ------------------------------------------------------------------ A: the "moved" pointer (0.9.8 and 0.9.9)

/** Set once Open or ✕ was pressed. Remove the pointer and this key in 0.9.10. */
export const MOVED_KEY = 'kwnt:moved-space-controls';

const store = (): Storage | null => {
  try {
    return typeof localStorage === 'undefined' ? null : localStorage;
  } catch {
    return null;
  }
};

export const movedSeen = (): boolean => {
  try {
    return !!store()?.getItem(MOVED_KEY);
  } catch {
    return false;
  }
};

export function markMovedSeen(): void {
  try {
    store()?.setItem(MOVED_KEY, '1');
  } catch {
    /* private mode: it shows again next time, which is harmless */
  }
}

/**
 * Only someone who used kwant before sees it: the same `established` What's new works out (true only, so a
 * brand-new person and "can't tell yet" both get nothing), and never again once Open or ✕ set the key.
 */
export const movedPointerShows = (opts: { established: boolean | null; seen: boolean }): boolean => opts.established === true && !opts.seen;

// ------------------------------------------------------------------ E: the account card

/**
 * "Connect an AI account to start": only when nothing can run (`canRun === false`; null is still checking),
 * and never while Get started is on screen, whose own steps already cover it.
 */
export const accountCardShows = (opts: { canRun: boolean | null | undefined; getStartedShowing: boolean }): boolean =>
  opts.canRun === false && !opts.getStartedShowing;

/** The four ways in, as static chips: the name and, for two of them, the plan it needs. */
export const ACCOUNT_WAYS: ReadonlyArray<{ name: string; plan?: string }> = [
  { name: 'Claude', plan: 'Pro or Max' },
  { name: 'ChatGPT', plan: 'plan' },
  { name: 'Grok' },
  { name: 'API key' },
];

// ------------------------------------------------------------------ D: Get started step 3

/** "A", "A and B", "A, B and C". */
export function namesLine(names: string[]): string {
  const n = names.filter((x) => x && x.trim());
  if (n.length <= 1) return n[0] ?? '';
  return `${n.slice(0, -1).join(', ')} and ${n[n.length - 1]}`;
}

export interface StepTeam {
  id: string;
  name: string;
  lead_kwant_id?: string | null;
  member_kwants?: string[] | string | null;
  created_at?: string | null;
  /** The server's record of which ready-made team made it, when it has one. */
  created_from_template?: string | null;
}
export interface StepBot {
  id: string;
  name: string;
  avatar_sprite?: string;
}
export interface StepTemplate {
  id: string;
  handle: string;
  card_line: string;
  used_in_space?: boolean;
}

export interface StartTile {
  handle: string;
  /** The members, lead first: their real names and faces. */
  members: StepBot[];
  /** "Assistant, Researcher and Writer." plus the template's card line when a template made the team. */
  line: string;
  /** The template this team came from, if any. */
  templateId: string | null;
}

export interface TeamStepView {
  title: string;
  intro: string;
  /** Null when the space has no team: no Start tile, the old step 3. */
  start: StartTile | null;
  /** The template tiles to offer, without any template a team here was already made from. */
  templateIds: string[];
}

const ids = (v: StepTeam['member_kwants']): string[] => {
  if (Array.isArray(v)) return v;
  if (typeof v !== 'string') return [];
  try {
    const p = JSON.parse(v);
    return Array.isArray(p) ? p.filter((x): x is string => typeof x === 'string') : [];
  } catch {
    return [];
  }
};

/**
 * Which template made a team. The server's catalogue says per template whether a live team in this space came
 * from it (`used_in_space`); a seeded team (the fresh install's @helpers) may not have that record, so a team
 * whose handle is a template's handle counts too.
 */
export function templateOfTeam(team: StepTeam, templates: StepTemplate[], madeFrom?: string | null): StepTemplate | null {
  if (madeFrom) return templates.find((t) => t.id === madeFrom) ?? null;
  return templates.find((t) => t.handle === team.name) ?? null;
}

export const PICK_TITLE = 'Pick a team';
export const PICK_INTRO = 'Each team has a lead, helpers and skills for one job. You can add more teams later.';
export const YOUR_TEAM_TITLE = 'Your team';
export const YOUR_TEAM_INTRO = 'Your space comes with a team for everyday asks. Start with it, or add a team made for one job.';

/**
 * Step 3: with a team in the space, "Your team" and a Start tile for its first (oldest) team; without one, the
 * old "Pick a team". Templates already used here are hidden, so Helpers never shows twice.
 */
export function teamStepView(opts: {
  teams: StepTeam[];
  bots: StepBot[];
  templates: StepTemplate[];
  /** What this browser or the server knows made a team: team id → template id. */
  madeFrom?: (teamId: string) => string | null | undefined;
}): TeamStepView {
  const { teams, bots, templates } = opts;
  const byId = new Map(bots.map((b) => [b.id, b]));
  const ordered = [...teams].sort((a, b) => (a.created_at ?? '').localeCompare(b.created_at ?? ''));
  const team = ordered.find((t) => byId.has(t.lead_kwant_id ?? '') || ids(t.member_kwants).some((id) => byId.has(id))) ?? null;
  const usedIds = new Set<string>();
  for (const t of templates) if (t.used_in_space) usedIds.add(t.id);
  for (const t of teams) {
    const tpl = templateOfTeam(t, templates, opts.madeFrom?.(t.id) ?? t.created_from_template ?? null);
    if (tpl) usedIds.add(tpl.id);
  }
  const templateIds = templates.filter((t) => !usedIds.has(t.id)).map((t) => t.id);
  if (!team) return { title: PICK_TITLE, intro: PICK_INTRO, start: null, templateIds };
  const order = [team.lead_kwant_id ?? '', ...ids(team.member_kwants)].filter((id, i, a) => id && a.indexOf(id) === i);
  const members = order.map((id) => byId.get(id)).filter((b): b is StepBot => !!b);
  const tpl = templateOfTeam(team, templates, opts.madeFrom?.(team.id) ?? team.created_from_template ?? null);
  const names = namesLine(members.map((m) => m.name));
  const line = [names ? `${names}.` : '', tpl?.card_line ?? ''].filter(Boolean).join(' ');
  return {
    title: YOUR_TEAM_TITLE,
    intro: YOUR_TEAM_INTRO,
    start: { handle: team.name, members, line, templateId: tpl?.id ?? null },
    templateIds,
  };
}

/** Step 3's done value: "@helpers" after Start, else what it said before. */
export function teamDoneValue(opts: { imported: boolean; mainSpace: boolean; startHandle: string | null }): string {
  if (opts.imported) return 'Your imported bots';
  if (opts.startHandle) return `@${opts.startHandle}`;
  return opts.mainSpace ? 'Starter bots' : 'Your bots';
}

/** Ring classes for the Dev mode card after Open; Tailwind sees them here. */
const RING_ON = ['ring-2', 'ring-sky-400/55', 'transition-[box-shadow]', 'duration-[160ms]', 'motion-reduce:transition-none'];

/**
 * After Open: find the Dev mode card in Space settings › General (it mounts a frame or two later), scroll it
 * into view and ring it once, 1.2 s, then a 160 ms fade (static and simply removed under reduced motion).
 */
export function ringDevSettings(doc: Document = document, tries = 20): void {
  const el = doc.querySelector<HTMLElement>('[data-dev-settings]');
  if (!el) {
    if (tries > 0) setTimeout(() => ringDevSettings(doc, tries - 1), 50);
    return;
  }
  const reduce = !!doc.defaultView?.matchMedia?.('(prefers-reduced-motion: reduce)').matches;
  el.scrollIntoView({ block: 'center', behavior: reduce ? 'auto' : 'smooth' });
  el.classList.add(...RING_ON);
  el.setAttribute('data-moved-ring', '');
  setTimeout(() => {
    el.classList.remove('ring-sky-400/55');
    el.classList.add('ring-transparent');
    setTimeout(() => {
      el.classList.remove(...RING_ON, 'ring-transparent');
      el.removeAttribute('data-moved-ring');
    }, reduce ? 0 : 160);
  }, 1200);
}

/** The static arrival outline on the Dev mode card (design.md §3); Tailwind sees the classes here. */
const ARRIVED = ['outline', 'outline-2', 'outline-offset-2', 'outline-sky-400/60'];

/**
 * From the #general suggestion (dev-mode-visibility design.md §3): once the Dev mode card has mounted in
 * Settings › General, bring it into view and focus its Turn on button (the card itself when Dev mode is
 * already on), with a static outline on the card until focus first leaves it. No animation.
 */
export function focusDevSettings(doc: Document = document, tries = 40): void {
  const el = doc.querySelector<HTMLElement>('[data-dev-settings]');
  if (!el) {
    if (tries > 0) setTimeout(() => focusDevSettings(doc, tries - 1), 50);
    return;
  }
  el.scrollIntoView?.({ block: 'start', behavior: 'auto' });
  (el.querySelector<HTMLElement>('[data-dev-turn-on]') ?? el).focus({ preventScroll: true });
  el.classList.add(...ARRIVED);
  el.setAttribute('data-dev-arrived', '');
  const off = (e: FocusEvent) => {
    if (e.relatedTarget instanceof Node && el.contains(e.relatedTarget)) return;
    el.classList.remove(...ARRIVED);
    el.removeAttribute('data-dev-arrived');
    el.removeEventListener('focusout', off);
  };
  el.addEventListener('focusout', off);
}
