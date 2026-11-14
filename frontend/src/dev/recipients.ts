// Fix it "Send to…" (Mira, autonomy/fix-it-send-to/design.md; Juno's #609 API). Pure helpers for the
// "Goes to" line and the picker: the why words, a row's meta, the client-side search, the per-card pick in
// sessionStorage, and the plain error lines. No React here, so the tests can call them directly.

import type { DevRecipients, RecipientBot, RecipientDefault } from './types';

/** The `{why}` after the name (design §1): "wrote it", "lead of Edge", or "your pick". */
export function whyLabel(def: RecipientDefault | null, pick: RecipientBot | null): string | null {
  if (pick && (!def || pick.id !== def.id)) return 'your pick';
  if (!def) return null;
  if (def.via === 'author') return 'wrote it';
  return def.team ? `lead of ${def.team}` : 'team lead';
}

/** Who Fix it goes to right now: the pick, else the default, else nobody. */
export const target = (r: DevRecipients | null, pick: RecipientBot | null): RecipientBot | null => pick ?? r?.default ?? null;

/**
 * The `to` a Fix it press or a line-notes send carries (Rigel's N1 on #609). Before the picker has been shown on
 * the card, only a pick needs `to`. Once it has been shown, or the card already has a finding sent to a chosen bot
 * (`sent_to_id`), every press names its bot, the default included: a second press with no `to` would make the
 * server swap the person's pick for the default while the first fix is still waiting to go out.
 */
export function toFor(r: DevRecipients | null, pick: RecipientBot | null, pinned: boolean): string | undefined {
  const t = target(r, pick);
  if (!t) return undefined;
  if (pinned) return t.id;
  return pick && !(r?.default && r.default.id === pick.id) ? pick.id : undefined;
}

/** A picker row's meta (design §2): the default's reason, "Edge · owns this repo", the team, or nothing. */
export function rowMeta(bot: RecipientBot, def: RecipientDefault | null): string | null {
  if (def && bot.id === def.id) {
    if (def.via === 'author') return 'Default · wrote it';
    return def.team ? `Default · lead of ${def.team}, which owns this repo` : 'Default · team lead';
  }
  if (def && def.via === 'team_lead' && def.team && bot.team === def.team) return `${bot.team} · owns this repo`;
  return bot.team ?? null;
}

/** The picker's rows: the default pinned first, then the rest in the server's order. */
export function pickerRows(r: DevRecipients): RecipientBot[] {
  const d = r.default;
  if (!d) return r.bots;
  const first = r.bots.find((b) => b.id === d.id) ?? { id: d.id, name: d.name, ...(d.team ? { team: d.team } : {}) };
  return [first, ...r.bots.filter((b) => b.id !== d.id)];
}

/** Search by name or team, case-insensitive (design §2). */
export function filterBots(bots: readonly RecipientBot[], q: string): RecipientBot[] {
  const s = q.trim().toLowerCase();
  if (!s) return [...bots];
  return bots.filter((b) => b.name.toLowerCase().includes(s) || (b.team ?? '').toLowerCase().includes(s));
}

export const SEARCH_FROM = 8;
export const noMatch = (q: string) => `No bot matches "${q.trim()}".`;
export const PICKER_TITLE = 'Who fixes this PR?';
export const PICKER_CAPTION = 'Fix it and line notes on this PR go here.';
export const nobodyFooter = (repo: string | null) =>
  `No bot wrote this PR and no team owns ${repo ?? 'this repo'}, so there’s no default. To set one, add the repo to a team (Manage team › Git Repo).`;
export const LIST_FAILED = 'couldn’t load the list';
export const goesLabel = (name: string, why: string | null) => `Fixes go to ${name}${why ? `, ${why}` : ''}. Change`;
export const pickedSay = (name: string) => `Fixes go to ${name}`;

/** Fix it's 400 bad_recipient (design §5). */
export const badRecipientFix = (name: string | null) =>
  `Couldn’t send it: ${name ? `${name} isn’t in this space` : 'That bot isn’t in this space'}. Pick another bot and press Fix it again.`;
/** Line notes' 400 bad_recipient (design §5). */
export const badRecipientNotes = (name: string | null) =>
  `The notes didn’t send: ${name ? `${name} isn’t in this space` : 'That bot isn’t in this space'}. Pick another bot.`;
/** Line notes with no default bot (design §7). */
export const noDefaultNotes = (login: string | null) =>
  `This PR is by ${login ?? 'a person'} and no team owns this repo, so notes have no default bot. Pick one when you send, or add the repo to a team (Manage team › Git Repo).`;

// ---- The pick, per card, in sessionStorage (design §1) ----

export const pickKey = (cardId: string) => `kwnt:fix-to:${cardId}`;

function store(): Storage | null {
  try {
    return typeof sessionStorage === 'undefined' ? null : sessionStorage;
  } catch {
    return null;
  }
}

export function loadPick(cardId: string): RecipientBot | null {
  try {
    const raw = store()?.getItem(pickKey(cardId));
    if (!raw) return null;
    const v = JSON.parse(raw) as unknown;
    if (!v || typeof v !== 'object') return null;
    const o = v as Record<string, unknown>;
    return typeof o.id === 'string' && typeof o.name === 'string' ? { id: o.id, name: o.name, ...(typeof o.team === 'string' ? { team: o.team } : {}) } : null;
  } catch {
    return null;
  }
}

export function savePick(cardId: string, pick: RecipientBot | null): void {
  try {
    const s = store();
    if (!s) return;
    if (pick) s.setItem(pickKey(cardId), JSON.stringify(pick));
    else s.removeItem(pickKey(cardId));
  } catch {
    /* private mode: the pick lasts until the card unmounts */
  }
}

/** "The picker has been shown on this card", kept beside the pick so a reload keeps sending `to` (N1). */
export const shownKey = (cardId: string) => `kwnt:fix-to-shown:${cardId}`;

export function loadShown(cardId: string): boolean {
  try {
    return store()?.getItem(shownKey(cardId)) === '1';
  } catch {
    return false;
  }
}

export function saveShown(cardId: string): void {
  try {
    store()?.setItem(shownKey(cardId), '1');
  } catch {
    /* private mode: it lasts until the card unmounts */
  }
}

/** A stored pick holds only while the bot is still in the list; picking the default is no pick. */
export function keepPick(pick: RecipientBot | null, r: DevRecipients | null): RecipientBot | null {
  if (!pick || !r) return pick;
  const b = r.bots.find((x) => x.id === pick.id);
  if (!b) return null;
  if (r.default && r.default.id === b.id) return null;
  return b;
}
