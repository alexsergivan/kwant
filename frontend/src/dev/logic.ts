// Pure rules behind the Dev mode renderers, kept out of the components so the node tests can pin them:
// the Verified sticker (§3.6 item 14, design §4), the production-floor backup state machine (design §6,
// all seven cases), the 500 ms Allow guard, the Fix it undo window and the dismiss flow (design §5.6).

import type {
  BackupResult,
  DevFinding,
  DismissReason,
  Forge,
  FloorBackup,
  LandedCard,
  ProdFloor,
  ReviewCard,
  RiskLevel,
} from './types';

// ---------------------------------------------------------------------------------------------------
// Time and numbers (display only: nothing here computes a new figure from the data)

const pad = (n: number) => String(n).padStart(2, '0');

/** "19:26" in the viewer's own time zone, or null. */
export function clock(iso: string | null): string | null {
  if (!iso) return null;
  const d = new Date(iso);
  return Number.isNaN(d.getTime()) ? null : `${pad(d.getHours())}:${pad(d.getMinutes())}`;
}

const dayKey = (d: Date) => `${d.getFullYear()}-${pad(d.getMonth() + 1)}-${pad(d.getDate())}`;

/** "Today", "Yesterday", or "Mon 2 Nov" for a landed card's day separator. */
export function dayLabel(iso: string | null, now: number): string {
  if (!iso) return 'Earlier';
  const d = new Date(iso);
  if (Number.isNaN(d.getTime())) return 'Earlier';
  const today = new Date(now);
  const yesterday = new Date(now);
  yesterday.setDate(yesterday.getDate() - 1);
  if (dayKey(d) === dayKey(today)) return 'Today';
  if (dayKey(d) === dayKey(yesterday)) return 'Yesterday';
  const wd = ['Sun', 'Mon', 'Tue', 'Wed', 'Thu', 'Fri', 'Sat'][d.getDay()];
  const mo = ['Jan', 'Feb', 'Mar', 'Apr', 'May', 'Jun', 'Jul', 'Aug', 'Sep', 'Oct', 'Nov', 'Dec'][d.getMonth()];
  return `${wd} ${d.getDate()} ${mo}${d.getFullYear() !== today.getFullYear() ? ` ${d.getFullYear()}` : ''}`;
}

/** Cards grouped under day separators, keeping the server's order (newest first). */
export function byDay<T extends { merged_at: string | null }>(cards: T[], now: number): { label: string; cards: T[] }[] {
  const out: { label: string; cards: T[] }[] = [];
  for (const c of cards) {
    const label = dayLabel(c.merged_at, now);
    const last = out[out.length - 1];
    if (last && last.label === label) last.cards.push(c);
    else out.push({ label, cards: [c] });
  }
  return out;
}

/** "14 s", "41 min", "3 h", "2 days". */
export function age(seconds: number): string {
  const s = Math.max(0, Math.floor(seconds));
  if (s < 60) return `${s} s`;
  if (s < 3600) return `${Math.floor(s / 60)} min`;
  if (s < 48 * 3600) return `${Math.floor(s / 3600)} h`;
  return `${Math.floor(s / 86400)} days`;
}

/** "412 MB" from the byte count the server gave. */
export function size(bytes: number | null): string | null {
  if (bytes === null) return null;
  if (bytes < 1000) return `${bytes} B`;
  if (bytes < 1e6) return `${Math.round(bytes / 1e3)} KB`;
  if (bytes < 1e9) return `${Math.round(bytes / 1e6)} MB`;
  return `${(bytes / 1e9).toFixed(1)} GB`;
}

export const fileName = (path: string | null) => (path ? path.split('/').pop() || path : null);

export const RISK_WORD: Record<RiskLevel, string> = { low: 'Low risk', medium: 'Medium risk', high: 'High risk' };

/** Design §5.2: a badge never stands alone. Low with no signals reads "Small change, tests nearby". */
export function riskReasons(risk: { level: RiskLevel; reasons: string[] } | null): string[] {
  if (!risk) return [];
  return risk.reasons.length > 0 ? risk.reasons : risk.level === 'low' ? ['Small change, tests nearby'] : ['No reason given'];
}

// ---------------------------------------------------------------------------------------------------
// Verified sticker (§3.6 item 14)

const sameName = (a: string | null | undefined, b: string | null | undefined) =>
  !!a && !!b && a.trim().toLowerCase() === b.trim().toLowerCase();

/** A finding is open until it is fixed or dismissed (item 14). One sent as a Fix it task is still open. */
export const isOpen = (f: DevFinding) => f.state !== 'dismissed';

/** Findings in a review that still need the person: open or sent, never dismissed. */
export const openFindings = (review: ReviewCard | null | undefined) => (review ? review.findings.filter(isOpen).length : 0);

/**
 * The holo "Verified" sticker shows only when every one of these holds; the server's own `verified` is
 * one of them, so the client can only ever be stricter than the server:
 * - the server says verified;
 * - CI is green (tests passed);
 * - someone reviewed it in kwant, and the reviewer is not the author;
 * - no follow-up is open;
 * - if the PR's review card is known, it has a reviewer who isn't the author and no open finding.
 * A pending or stale card, and a direct commit with no PR, never gets one.
 */
export function showVerified(card: LandedCard, reviews: ReviewCard[] = []): boolean {
  if (card.pr === null || !card.verified || card.status === 'pending' || card.status === 'stale') return false;
  if (card.tests.state !== 'passed') return false;
  if (!card.reviewed_by || sameName(card.reviewed_by, card.written_by?.name)) return false;
  if (card.follow_ups.length > 0) return false;
  const review = reviews.find((r) => r.pr === card.pr);
  if (review) {
    if (!review.reviewer || sameName(review.reviewer.name, review.author?.name)) return false;
    if (review.findings.some(isOpen)) return false;
  }
  return true;
}

/** Areas the roll-up marks with a warn border: a card there has an open follow-up or red CI. */
export function needsYouAreas(cards: LandedCard[]): string[] {
  const out = new Set<string>();
  for (const c of cards) if (c.area && (c.follow_ups.length > 0 || c.tests.state === 'failed')) out.add(c.area);
  return [...out];
}

// ---------------------------------------------------------------------------------------------------
// Production floor: backup state machine (design §6)

export const FRESH_S = 600;
export const ALLOW_GUARD_MS = 500;

/** What the card is doing locally on top of the server's backup state. */
export interface FloorUi {
  /** idle: as the server said. taking: a backup is running. done: a backup this card took is ok. failed: it failed. */
  phase: 'idle' | 'taking' | 'done' | 'failed';
  /** The backup the card took (done), with the epoch ms it finished. */
  taken: { bytes: number | null; path: string | null; at: number } | null;
  error: string | null;
  /** Epoch ms when Allow appeared in place of Back up first; presses before +500 ms are ignored. */
  armedAt: number | null;
}

export type FloorEvent =
  | { type: 'backup_start' }
  | { type: 'backup_done'; result: BackupResult; now: number }
  | { type: 'backup_error'; error: string };

export const initialFloorUi = (floor: ProdFloor): FloorUi => ({
  phase: floor.backup.state === 'taking' ? 'taking' : floor.backup.state === 'failed' ? 'failed' : 'idle',
  taken: null,
  error: floor.backup.state === 'failed' ? floor.backup.error : null,
  armedAt: null,
});

export function floorReducer(ui: FloorUi, ev: FloorEvent): FloorUi {
  switch (ev.type) {
    case 'backup_start':
      // One backup at a time: a second press while one runs changes nothing.
      return ui.phase === 'taking' ? ui : { ...ui, phase: 'taking', error: null, armedAt: null };
    case 'backup_done':
      if (ui.phase !== 'taking') return ui;
      if (!ev.result.ok) return { ...ui, phase: 'failed', error: ev.result.error ?? 'no reason given', armedAt: null };
      return { phase: 'done', taken: { bytes: ev.result.bytes, path: ev.result.path, at: ev.now }, error: null, armedAt: ev.now };
    case 'backup_error':
      return ui.phase !== 'taking' ? ui : { ...ui, phase: 'failed', error: ev.error, armedAt: null };
  }
}

/** True once the new Allow may act: never in its first 500 ms. */
export const allowReady = (ui: FloorUi, now: number) => ui.armedAt === null || now - ui.armedAt >= ALLOW_GUARD_MS;

export type RowTone = 'ok' | 'warn' | 'bad' | 'neutral' | 'info';
export type FloorCase = 'fresh' | 'old_plain' | 'just_taken' | 'no_data' | 'blocked' | 'no_command' | 'taking' | 'failed';

export interface FloorView {
  case: FloorCase;
  tone: RowTone;
  /** Bold lead, then the rest of the line. */
  lead: string;
  text: string;
  /** The file name (mono) after the size, when known. */
  file: string | null;
  /** The main button: Allow, Back up first (in Allow's place), Try the backup again, or none. */
  main: 'allow' | 'backup' | 'retry' | 'none';
  /** A secondary Back up first beside Allow (plain deploy, old backup). */
  secondaryBackup: boolean;
  /** Every button waits (taking). */
  busy: boolean;
  /** The right-hand note. */
  note: string;
}

/**
 * The backup row and buttons for a floor card at time `now`. `mountedAt` is when the card got the
 * server's `age_s`, so a fresh backup ages on screen and turns old after 10 minutes.
 */
export function floorView(floor: ProdFloor, ui: FloorUi, now: number, mountedAt: number = now): FloorView {
  const b: FloorBackup = floor.backup;
  const blockedNote = 'Nothing runs without a fresh backup.';
  const plainNote = 'No “always allow” on the production floor.';
  const note = floor.needs_backup ? blockedNote : plainNote;
  const sz = (bytes: number | null) => (size(bytes) ? ` · ${size(bytes)}` : '');

  if (ui.phase === 'taking') {
    const pct = typeof b.percent === 'number' && b.state === 'taking' ? ` ${Math.round(b.percent)} %` : '';
    return { case: 'taking', tone: 'info', lead: '', text: `Taking a backup…${pct}`, file: null, main: floor.needs_backup ? 'backup' : 'allow', secondaryBackup: !floor.needs_backup, busy: true, note };
  }
  if (ui.phase === 'failed') {
    const why = (ui.error ?? 'no reason given').replace(/[.\s]+$/, '');
    return { case: 'failed', tone: 'bad', lead: 'Backup failed:', text: ` ${why}.`, file: null, main: 'retry', secondaryBackup: false, busy: false, note: blockedNote };
  }
  if (ui.phase === 'done' && ui.taken) {
    const secs = (now - ui.taken.at) / 1000;
    if (secs <= FRESH_S) {
      return { case: 'just_taken', tone: 'ok', lead: `Backup taken ${age(secs)} ago`, text: sz(ui.taken.bytes), file: null, main: 'allow', secondaryBackup: false, busy: false, note: plainNote };
    }
  }
  if (b.state === 'no_data') {
    return { case: 'no_data', tone: 'neutral', lead: '', text: 'Nothing to back up: this site has no database or uploads.', file: null, main: 'allow', secondaryBackup: false, busy: false, note: plainNote };
  }
  if (b.state === 'no_command' && floor.needs_backup) {
    return { case: 'no_command', tone: 'bad', lead: 'No backup command.', text: ' Set a backup command in Dev mode settings.', file: null, main: 'none', secondaryBackup: false, busy: false, note: blockedNote };
  }
  const liveAge = b.age_s === null ? null : b.age_s + Math.max(0, now - mountedAt) / 1000;
  const fresh = (b.state === 'fresh' || b.state === 'old') && liveAge !== null && liveAge <= FRESH_S;
  if (fresh && liveAge !== null) {
    return { case: 'fresh', tone: 'ok', lead: `Backup taken ${age(liveAge)} ago`, text: sz(b.bytes), file: fileName(b.path), main: 'allow', secondaryBackup: false, busy: false, note: plainNote };
  }
  const never = liveAge === null || b.state === 'none' || b.state === 'no_command';
  if (floor.needs_backup) {
    return { case: 'blocked', tone: 'bad', lead: never ? 'No backup yet.' : 'No fresh backup.', text: never ? '' : ` The last one is ${age(liveAge!)} old.`, file: null, main: 'backup', secondaryBackup: false, busy: false, note: blockedNote };
  }
  return {
    case: 'old_plain',
    tone: 'warn',
    lead: never ? 'No backup yet.' : `Last backup ${age(liveAge!)} ago.`,
    text: ' No data changes, so you can still allow it.',
    file: null,
    main: 'allow',
    // Without a backup command there is nothing to run, so only Allow and Deny.
    secondaryBackup: b.state !== 'no_command',
    busy: false,
    note: plainNote,
  };
}

/** Header verb per floor kind (design §6). */
export function floorHeader(floor: ProdFloor, bot: string): string {
  return `Production floor: ${floorWho(floor, bot)}`;
}

/** The eyebrow's second half (style v2 §8): "{bot} wants to deploy". */
export function floorWho(floor: ProdFloor, bot: string): string {
  const what: Record<ProdFloor['kind'], string> = {
    deploy: 'deploy',
    deploy_migration: 'deploy a database change',
    push_main: 'push to main',
    force_push: 'force push',
    delete_ref: 'delete a branch',
    destructive: 'delete data',
    prod_key: 'use a production key',
  };
  return `${bot} wants to ${what[floor.kind]}`;
}

/** The LED on the backup line (style v2 §8): warn old/none, run taking, ok fresh, bad failed/blocked. */
export function floorLed(tone: RowTone, kase: string): 'ok' | 'run' | 'warn' | 'bad' | 'off' {
  if (kase === 'taking') return 'run';
  if (tone === 'ok') return 'ok';
  if (tone === 'bad') return 'bad';
  if (tone === 'warn') return 'warn';
  return 'off';
}

/** Leads of the clauses 0.7.0 sent ("runs deploy.sh, a deploy script", "a force-push to main"):
 * a card saved before 0.7.1 still gets a capital. A command name ("git push …") stays as typed. */
const OLD_LEADS = /^(a|an|runs|starts)\b/;

/** `why` as its own sentence: the server sends one with its own subject and no period (0.7.1). */
export function floorSentence(why: string): string {
  const t = why.trim().replace(/[.\s]+$/, '');
  if (!t) return '';
  return `${OLD_LEADS.test(t) ? t[0].toUpperCase() + t.slice(1) : t}.`;
}

export function floorSubline(floor: ProdFloor): string {
  if (floor.kind === 'deploy_migration') return 'This deploy changes the database, so it counts as destructive: it needs a backup from the last 10 minutes first.';
  if (floor.kind === 'destructive') return 'Destructive commands need a backup from the last 10 minutes first.';
  if (floor.kind === 'deploy') return 'Dev mode is on, so deploys always ask you, even with approvals off.';
  if (floor.kind === 'prod_key') return 'Dev mode is on, so a production key always asks you, even with approvals off.';
  return 'Dev mode is on, so this always asks you, even with approvals off.';
}

// ---------------------------------------------------------------------------------------------------
// Findings: Fix it undo window and the dismiss flow (design §5.6)

export const UNDO_MS = 10_000;

/** Ms left to undo a Fix it, or 0. */
export function undoLeft(f: DevFinding, now: number): number {
  if (f.state !== 'sent' || !f.sent_at || f.task_id) return 0;
  const at = Date.parse(f.sent_at);
  return Number.isNaN(at) ? 0 : Math.max(0, at + UNDO_MS - now);
}

export const DISMISS_REASONS: { id: DismissReason; label: string }[] = [
  { id: 'not_a_problem', label: 'Not a problem' },
  { id: 'fine_here', label: 'Fine in this PR' },
  { id: 'later', label: 'I’ll fix it later' },
  { id: 'wrong', label: 'Wrong' },
];

export const reasonLabel = (r: DismissReason) => DISMISS_REASONS.find((x) => x.id === r)?.label ?? r;

/** "Dismissed · Fine in this PR · you". `by` is "person" from the server (§3.6 item 4). */
export function dismissedLine(f: DevFinding): string {
  const d = f.dismissed;
  if (!d) return 'Dismissed';
  const by = !d.by || d.by === 'person' ? 'you' : d.by;
  return `Dismissed · ${reasonLabel(d.reason)} · ${by}`;
}

/** The inline dismiss form's own state. */
export interface DismissDraft {
  open: boolean;
  reason: DismissReason | null;
  note: string;
  sending: boolean;
  error: string | null;
}

export type DismissEvent =
  | { type: 'open' }
  | { type: 'cancel' }
  | { type: 'reason'; reason: DismissReason }
  | { type: 'note'; note: string }
  | { type: 'submit' }
  | { type: 'failed'; error: string }
  | { type: 'done' };

export const closedDraft: DismissDraft = { open: false, reason: null, note: '', sending: false, error: null };

export function dismissReducer(d: DismissDraft, ev: DismissEvent): DismissDraft {
  switch (ev.type) {
    case 'open':
      return { ...closedDraft, open: true };
    case 'cancel':
    case 'done':
      return closedDraft;
    case 'reason':
      return d.open && !d.sending ? { ...d, reason: ev.reason, error: null } : d;
    case 'note':
      return d.open && !d.sending ? { ...d, note: ev.note.slice(0, 500) } : d;
    case 'submit':
      // A reason is required; a second submit while one is in flight is ignored.
      return d.open && d.reason && !d.sending ? { ...d, sending: true, error: null } : d;
    case 'failed':
      return { ...d, sending: false, error: ev.error };
  }
}


/** "1 high, 1 medium, 1 low" over the findings that are not dismissed. */
export function findingsSummary(findings: DevFinding[]): string {
  const order = ['critical', 'high', 'medium', 'low'] as const;
  return order
    .map((s) => [s, findings.filter((f) => f.severity === s && f.state !== 'dismissed').length] as const)
    .filter(([, n]) => n > 0)
    .map(([s, n]) => `${n} ${s}`)
    .join(', ');
}

// ---------------------------------------------------------------------------------------------------
// Post to {Forge} (0.7.1, plan/post-to-github.md + amendment): the review card footer's one state

/** Findings the post turns into comments: every one not dismissed (design §5.7 "N = open findings"). */
export const postCount = (findings: Pick<DevFinding, 'state'>[]): number => findings.filter((f) => f.state !== 'dismissed').length;

/** Display names for `forge` (amendment, 21:50): posting is GitHub-only in 0.7.1, GitLab and Gitea come in 0.7.2. */
export const FORGE_NAME: Record<Forge, string> = { github: 'GitHub', gitlab: 'GitLab', gitea: 'Gitea', forgejo: 'Forgejo' };

/** "Post to GitHub · 3 findings"; no count when nothing is open (the review is then its body only). F6: findings, not
 * comments, because a finding outside the diff goes in the review body; the posted line counts inline comments. */
/** The link to the PR (or a direct commit) on its forge: "Open on GitHub". Landed cards keep the link with no
 *  forge (the URL is in the card), named plainly; the review footer hides it instead (user-flow.md F7). */
export const forgeOpen = (forge: Forge | null, what: 'pr' | 'commit' = 'pr'): string =>
  forge ? `Open on ${FORGE_NAME[forge]}` : what === 'pr' ? 'Open pull request' : 'Open commit';

export function postLabel(n: number, forge: Forge): string {
  const to = `Post to ${FORGE_NAME[forge]}`;
  return n === 0 ? to : `${to} · ${n} ${n === 1 ? 'finding' : 'findings'}`;
}

/** "Nothing is posted until you press it. Posts as octocat." (the second sentence only with a login). */
export function postLine(login: string | null | undefined): string {
  return login ? `Nothing is posted until you press it. Posts as @${login}.` : 'Nothing is posted until you press it.';
}

/** "3 inline comments", "1 inline comment", or null for none or unknown (the line then drops it). `posted.comments`
 * counts only the comments on diff lines (F6, QA 0.7.1). */
export const commentsWord = (n: number | null | undefined): string | null => (n ? `${n} inline ${n === 1 ? 'comment' : 'comments'}` : null);

export const readOnlyTip = (forge: Forge): string => `Your ${FORGE_NAME[forge]} connection is read only`;
/** Style v2 §7: what to give the token. The scope name is GitHub's; other forges get plain words. */
export const readOnlyFix = (forge: Forge): string => (forge === 'github' ? 'pull_requests: write' : 'write access');
/** The whole read-only line, for screen readers and tests: "Your GitHub connection is read only. Give the token pull_requests: write to post." */
export const readOnlyLine = (forge: Forge): string => `${readOnlyTip(forge)}. Give the token ${readOnlyFix(forge)} to post.`;
export const STALE_LINE = 'This PR has new commits. A new review is on its way.';

export type PostState =
  | { kind: 'none' } // no Post button: the sample PR, or a card still being written
  | { kind: 'no_review' } // F4: the review failed to generate, so there is nothing to post (no lock line, no button)
  | { kind: 'ready' }
  | { kind: 'posting' }
  | { kind: 'posted'; at: string | null; url: string | null; login: string | null; comments: number | null }
  | { kind: 'read_only' }
  | { kind: 'stale' }
  | { kind: 'failed'; message: string; retry: boolean };

/**
 * Which footer to draw. `posted` wins over everything (the server never posts twice, P6); then the
 * in-flight press; then the last error; a pending card has no review to post yet.
 */
export function postState(
  card: { status: string; posted: { at: string | null; url: string | null; login?: string | null; comments?: number | null } | null; post_error: { kind: string; message: string | null } | null },
  opts: { canPost: boolean; posting: boolean; forge: Forge | null; noReview?: boolean },
): PostState {
  if (card.posted) return { kind: 'posted', at: card.posted.at, url: card.posted.url, login: card.posted.login ?? null, comments: card.posted.comments ?? null };
  if (!opts.canPost || !opts.forge || card.status === 'pending') return { kind: 'none' };
  // F4: a card whose review didn't finish has nothing to post.
  if (card.status === 'failed') return { kind: 'no_review' };
  if (opts.posting) return { kind: 'posting' };
  const e = card.post_error;
  if (e?.kind === 'read_only') return { kind: 'read_only' };
  if (e?.kind === 'stale') return { kind: 'stale' };
  // noReview: the server answered 409 (no review on this card): say why, but offer no repost.
  if (e) return { kind: 'failed', message: e.message ?? `${FORGE_NAME[opts.forge]} didn’t say why.`, retry: !opts.noReview };
  return { kind: 'ready' };
}

/** "Posted 19:58 as @oleks-i · 3 comments"; the time, login and count each drop out when unknown. */
export const postedLine = (at: string | null, login: string | null = null, comments: number | null = null): string => {
  const t = clock(at);
  const n = commentsWord(comments);
  return `Posted${t ? ` ${t}` : ''}${login ? ` as @${login}` : ''}${n ? ` · ${n}` : ''}`;
};
// Style v2 parts (Mira, style-v2.md §2): pure helpers behind <Led>, <DiffStat> and <Sha>

export type LedState = 'ok' | 'run' | 'warn' | 'bad' | 'off';

/**
 * The 32×4 split bar: px for the add part and the del part (1 px gap between them when both show).
 * Add share = round(31·a/(a+d)), at least 3 px when a > 0 and at least 3 px left for del when d > 0.
 */
export function diffBar(add: number, del: number): { add: number; del: number } {
  const a = Math.max(0, add);
  const d = Math.max(0, del);
  if (a + d === 0) return { add: 0, del: 0 };
  if (d === 0) return { add: 32, del: 0 };
  if (a === 0) return { add: 0, del: 32 };
  const ap = Math.min(28, Math.max(3, Math.round((31 * a) / (a + d))));
  return { add: ap, del: 31 - ap };
}

/** "157 lines added, 15 removed" (one line / no lines read right). */
export function diffLabel(add: number, del: number): string {
  const l = (n: number) => (n === 1 ? 'line' : 'lines');
  return `${add} ${l(add)} added, ${del} removed`;
}

/** The first 7 characters of a commit sha. */
export const shortSha = (sha: string): string => sha.trim().slice(0, 7);


/**
 * Style v2 §6 LGTM stamp: only when the review really found nothing to fix. All of: ready; no open finding;
 * no tamper flag; no intent row missing; secrets and packages not hit; a reviewer on a different engine.
 */
export function lgtm(card: {
  status: string;
  reviewer: { name: string } | null;
  same_engine: boolean;
  findings: { state: string }[];
  tamper: { flags: unknown[] };
  intent: { rows: { state: string }[] } | null;
  security: { secrets: { state: string }; packages: { state: string } };
}): boolean {
  return (
    card.status === 'ready' &&
    !!card.reviewer &&
    !card.same_engine &&
    !card.findings.some((f) => f.state === 'open') &&
    card.tamper.flags.length === 0 &&
    !(card.intent?.rows ?? []).some((r) => r.state === 'missing') &&
    card.security.secrets.state !== 'hit' &&
    card.security.packages.state !== 'hit'
  );
}

/** "Looks good to me: Rigel found nothing to fix". */
export const lgtmLabel = (reviewer: string): string => `Looks good to me: ${reviewer} found nothing to fix`;
/** "Rigel is reading the diff" (pending review); no ETA, we don't have one. */
export const readingLine = (reviewer: string | null | undefined): string => `${reviewer || 'A reviewer'} is reading the diff`;

/** The landed card's tests line in words (§5): LED + "Tests passed" / "Tests failed" / "No tests ran". */
export function testsWords(state: 'passed' | 'failed' | 'none' | 'unknown'): string {
  return state === 'passed' ? 'Tests passed' : state === 'failed' ? 'Tests failed' : state === 'none' ? 'No tests ran' : 'Tests not known';
}

/** The git rail node for a landed card (§5): a card still being written pings; else the tests decide. */
export function landedNode(card: { status: string; tests: { state: 'passed' | 'failed' | 'none' | 'unknown' } }): 'ok' | 'bad' | 'off' | 'run' {
  if (card.status === 'pending') return 'run';
  return card.tests.state === 'passed' ? 'ok' : card.tests.state === 'failed' ? 'bad' : 'off';
}
