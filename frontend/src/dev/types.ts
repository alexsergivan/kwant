// Dev mode card types (plan §3.4 plus the §3.6 amendments) and the props the Dev desk passes to the
// renderers in components/dev/. Iris owns this file; Pixel's DevDesk.tsx mounts the renderers with these
// props. The GET routes return `{...card, id, kind, status, created_at}` (impl/lin.md "Card").
//
// Every parse* below takes the route's raw JSON and returns a typed card, or null when the card is not
// one the renderer can draw honestly (wrong kind, no PR number). The desk then shows the plain fallback
// row (`CardFallback`). Strings are trimmed to a sane length and never rendered as HTML.

export type CardStatus = 'pending' | 'ready' | 'failed' | 'stale';
export type RiskLevel = 'low' | 'medium' | 'high';

/** Who wrote a PR: a kwant bot (with its engine) or a person. */
export interface DevPerson {
  kind: 'bot' | 'person';
  name: string;
  engine: string | null;
  model: string | null;
}

export interface DevRisk {
  level: RiskLevel;
  /** Always at least one reason on screen; the renderer adds "Small change, tests nearby" for low with none. */
  reasons: string[];
}

export interface DevLinks {
  pr: string | null;
  commit: string | null;
  ci: string | null;
}

interface CardBase {
  id: string;
  status: CardStatus;
  created_at: string | null;
  /** §3.6 item 9: a failed card says why and keeps its rule-based parts. */
  error: string | null;
  /** 0.7.2: the repo key (`host/owner/name`, lowercased) the card came from; null from an older server. */
  repo: string | null;
  /** `owner/name` for the repo chip, from the card's own facts. */
  repo_name: string | null;
}

export interface LandedCard extends CardBase {
  kind: 'landed';
  /** null for a commit that reached the branch without a PR (a direct push): the card names the commit instead. */
  pr: number | null;
  sha: string;
  merged_at: string | null;
  merged_by: string | null;
  written_by: DevPerson | null;
  /** §3.6 item 12: null when no text was written; the headline is then the PR title. */
  area: string | null;
  headline: string;
  for_you: string | null;
  under_hood: { summary: string | null; files: { path: string; add: number; del: number }[] };
  risk: DevRisk | null;
  tests: { state: 'passed' | 'failed' | 'none' | 'unknown'; summary: string | null };
  reviewed_by: string | null;
  follow_ups: { text: string; ref: string | null }[];
  /** The server's flag. The sticker is shown only when `showVerified` (dev/logic.ts) also agrees. */
  verified: boolean;
  links: DevLinks;
  /** §3.6 item 11: "diff cut, N files not read". */
  cut: string | null;
  /** Optional one-line meta beside the chips, e.g. "redaction code · 249 lines". */
  meta: string | null;
}

export type IntentState = 'ok' | 'missing' | 'extra';
export type TamperKind = 'deleted_assert' | 'skip' | 'loosened' | 'snapshot' | 'coverage_drop';
export type ScanState = 'pass' | 'hit' | 'not_run';
export type Severity = 'critical' | 'high' | 'medium' | 'low';
export type DismissReason = 'not_a_problem' | 'fine_here' | 'later' | 'wrong';

export interface DevFinding {
  id: string;
  severity: Severity;
  title: string;
  file: string | null;
  line: number | null;
  detail: string | null;
  fix: string | null;
  source: 'review' | 'gitleaks' | 'packages' | 'tamper';
  state: 'open' | 'sent' | 'dismissed';
  sent_at: string | null;
  /** §3.6 item 4. */
  sent_to: string | null;
  task_id: string | null;
  send_error: string | null;
  dismissed: { reason: DismissReason; note: string; by: string; at: string | null } | null;
}

export interface TamperFlag {
  kind: TamperKind;
  file: string;
  line: number | null;
  excerpt: string;
  why: string;
}

export interface ReviewCard extends CardBase {
  kind: 'review';
  pr: number;
  title: string;
  branch: string | null;
  base: string | null;
  draft: boolean;
  author: DevPerson | null;
  /** §3.6 item 10: null when no second bot could review; `note` then says so. */
  /** Round 3 (vega.md): `id` is the reviewing bot's id; the server's Verified compares it with the author's. */
  reviewer: { id: string | null; name: string; engine: string | null; model: string | null } | null;
  note: string | null;
  same_engine: boolean;
  risk: DevRisk | null;
  intent: { source: string; asked: string; rows: { state: IntentState; text: string }[]; none: boolean } | null;
  /** `unread`: files GitHub sent no diff for, so nothing in them was checked (round 3, V-T5). */
  tamper: { flags: TamperFlag[]; unread: string[] };
  security: {
    secrets: { state: ScanState; count: number };
    packages: { state: ScanState; count: number };
  };
  findings: DevFinding[];
  cut: string | null;
  links: DevLinks;
  /** 0.7.1 Post to GitHub (plan/post-to-github.md): set once the person posted this card as a PR review. */
  posted: ReviewPosted | null;
  /** Why the last post didn't go through; a successful post clears it. */
  post_error: PostError | null;
}

export type Forge = 'github' | 'gitlab' | 'gitea';

export interface ReviewPosted {
  /** Where it was posted; "github" in 0.7.1. */
  forge: Forge | null;
  at: string | null;
  login: string | null;
  /** https only, or null (then there is no "View on {Forge}" link). */
  url: string | null;
  comments: number | null;
}

export type PostErrorKind = 'read_only' | 'stale' | 'failed';
export interface PostError {
  kind: PostErrorKind;
  message: string | null;
}

export interface ReleaseNotesCard extends CardBase {
  kind: 'release_notes';
  since_tag: string | null;
  markdown: string | null;
  whats_new: string | null;
}

export type DevCard = LandedCard | ReviewCard | ReleaseNotesCard;

/** `GET /api/spaces/:id/dev/landed` → rollup. */
export interface DevRollup {
  since: string | null;
  until: string | null;
  count: number;
  needs_you: number;
  main_green: boolean | null;
  areas: { area: string; count: number; first_headline: string }[];
}

// ---- Production floor (approval `details.prod_floor`, impl/lin.md) ----

export type FloorKind = 'push_main' | 'force_push' | 'delete_ref' | 'deploy' | 'deploy_migration' | 'destructive' | 'prod_key';
export type BackupState = 'fresh' | 'old' | 'none' | 'no_data' | 'no_command' | 'taking' | 'failed';

export interface FloorBackup {
  state: BackupState;
  age_s: number | null;
  bytes: number | null;
  path: string | null;
  error: string | null;
  /** Only while taking, when the server reports it. */
  percent?: number | null;
}

export interface ProdFloor {
  kind: FloorKind;
  why: string;
  blast: [string, string][];
  needs_backup: boolean;
  space_id: string | null;
  backup: FloorBackup;
}

/** `POST /api/spaces/:id/dev/backup`. */
export interface BackupResult {
  id: string | null;
  ok: boolean;
  path: string | null;
  bytes: number | null;
  taken_at: string | null;
  error: string | null;
}

// ---- Props (agreed with Pixel through this file) ----

export type FindingAction = { action: 'fix' } | { action: 'undo_fix' } | { action: 'dismiss'; reason: DismissReason; note?: string };

export interface RollupProps {
  rollup: DevRollup;
  last_tag: string | null;
  /** For the empty state: "Last merge: #290, 19:26." The newest landed card, if any. */
  last_merge: { pr: number; merged_at: string | null } | null;
  /** The watched branch: "3 changes landed on main". Defaults to "main". */
  branch?: string;
  /** Areas whose tile gets the warn border: `needsYouAreas(cards)` from dev/logic.ts. */
  needsYouAreas?: string[];
  onDraftNotes: () => void;
  onMarkSeen: () => void;
  /** True while a release-notes draft is being written (the button waits). */
  drafting?: boolean;
  /** True while the mark-seen POST is in flight (the button waits, like `drafting`). */
  marking?: boolean;
}

export interface LandedListProps {
  cards: LandedCard[];
  /** Review cards of the same space, for the Verified rule (§3.6 item 14). */
  reviews: ReviewCard[];
  /** Epoch ms; tests and shots pass a fixed clock. */
  now?: number;
  /** Opens the PR's review card ("1 finding open in the review · Open review"); the link hides without it. */
  onOpenReview?: (pr: number, repo?: string | null) => void;
  /** `forge` from GET dev_mode: names the link ("Open on GitHub"). */
  forge?: Forge | null;
  /** 0.7.3 (any-git): is this card's repo read with plain git? Then no CI tile and no "Open on {Forge}" link. */
  gitOnly?: (repo: string | null) => boolean;
  /** 0.7.2: the card's repo chip text, or null with one repo in the space (`cardRepoLabel`). */
  repoLabel?: (repo: string | null, name?: string | null) => string | null;
}

export interface ReviewCardProps {
  card: ReviewCard;
  /** Sends a finding action (`POST /api/dev/cards/:id/findings/:fid`) and resolves with the new card, or throws. */
  onFinding: (cardId: string, findingId: string, action: FindingAction) => Promise<ReviewCard>;
  /** Shows the "Sample PR" chip (design §5: the #294 sample). */
  sample?: boolean;
  now?: number;
  /** Posts the card as a PR review (`POST /api/dev/cards/:id/post`) and resolves with the new card, or throws.
   *  Without it the footer has no Post button (the sample PR). */
  onPost?: (cardId: string) => Promise<ReviewCard>;
  /** `forge` from GET dev_mode: names the button ("Post to GitHub"). Null: no Post button and no lock line. */
  forge?: Forge | null;
  /** `forge_login` from GET dev_mode, for "Posts as {login}." */
  forgeLogin?: string | null;
  /** 0.7.2: the card's repo chip text, or null with one repo in the space (`cardRepoLabel`). */
  repoLabel?: (repo: string | null, name?: string | null) => string | null;
}

export interface ReleaseNotesSheetProps {
  card: ReleaseNotesCard;
  /** Show "Save to Notion" only when Notion is connected; otherwise pass undefined and it's hidden. */
  onSaveToNotion?: (markdown: string) => Promise<void>;
  onDiscard: () => void;
  onClose: () => void;
}

// ---- Parsing ----

const isObj = (v: unknown): v is Record<string, unknown> => typeof v === 'object' && v !== null && !Array.isArray(v);
const str = (v: unknown, max = 2000): string | null => (typeof v === 'string' && v.trim() !== '' ? v.slice(0, max) : null);
const num = (v: unknown): number | null => (typeof v === 'number' && Number.isFinite(v) ? v : null);
const oneOf = <T extends string>(v: unknown, all: readonly T[], fallback: T): T => (all.includes(v as T) ? (v as T) : fallback);
const arr = (v: unknown): unknown[] => (Array.isArray(v) ? v : []);

const STATUSES = ['pending', 'ready', 'failed', 'stale'] as const;
const LEVELS = ['low', 'medium', 'high'] as const;

function base(raw: Record<string, unknown>): CardBase {
  return {
    id: str(raw.id, 200) ?? '',
    status: oneOf(raw.status, STATUSES, 'ready'),
    created_at: str(raw.created_at, 64),
    error: str(raw.error, 400),
    repo: str(raw.repo, 300),
    repo_name: str(raw.repo_name, 300),
  };
}

function person(v: unknown): DevPerson | null {
  if (!isObj(v)) return null;
  const name = str(v.name, 80);
  if (!name) return null;
  return { kind: v.kind === 'person' ? 'person' : 'bot', name, engine: str(v.engine, 40), model: str(v.model, 80) };
}

function risk(v: unknown): DevRisk | null {
  if (!isObj(v) || !LEVELS.includes(v.level as RiskLevel)) return null;
  return { level: v.level as RiskLevel, reasons: arr(v.reasons).map((r) => str(r, 120)).filter((r): r is string => !!r) };
}

function links(v: unknown): DevLinks {
  // Only https links reach an href (lin.md: links are https-only server-side too).
  const https = (u: unknown) => {
    const s = str(u, 1000);
    return s && /^https:\/\//.test(s) ? s : null;
  };
  const o = isObj(v) ? v : {};
  return { pr: https(o.pr), commit: https(o.commit), ci: https(o.ci) };
}

export function parseLanded(raw: unknown): LandedCard | null {
  if (!isObj(raw) || raw.kind !== 'landed') return null;
  // A direct commit has no PR (poller.rs builds it with `pr: null`); it still needs its commit to be named.
  const pr = num(raw.pr);
  const sha = (str(raw.sha, 40) ?? '').slice(0, 7);
  if (pr === null && !/^[0-9a-f]{7}$/i.test(sha)) return null;
  const uh = isObj(raw.under_hood) ? raw.under_hood : {};
  const tests = isObj(raw.tests) ? raw.tests : {};
  return {
    ...base(raw),
    kind: 'landed',
    pr,
    sha,
    merged_at: str(raw.merged_at, 64),
    merged_by: str(raw.merged_by, 80),
    written_by: person(raw.written_by),
    area: str(raw.area, 60),
    headline: str(raw.headline, 200) ?? str(raw.title, 200) ?? (pr !== null ? `PR #${pr}` : `Commit ${sha}`),
    for_you: str(raw.for_you, 600),
    under_hood: {
      summary: str(uh.summary, 600),
      files: arr(uh.files)
        .filter(isObj)
        .map((f) => ({ path: str(f.path, 400) ?? '', add: num(f.add) ?? 0, del: num(f.del) ?? 0 }))
        .filter((f) => f.path),
    },
    risk: risk(raw.risk),
    tests: { state: oneOf(tests.state, ['passed', 'failed', 'none', 'unknown'] as const, 'unknown'), summary: str(tests.summary, 120) },
    reviewed_by: str(raw.reviewed_by, 80),
    follow_ups: arr(raw.follow_ups)
      .filter(isObj)
      .map((f) => ({ text: str(f.text, 400) ?? '', ref: str(f.ref, 40) }))
      .filter((f) => f.text),
    // Verified needs a PR review (§3.6 item 14): a direct commit never has one.
    verified: pr !== null && raw.verified === true,
    links: links(raw.links),
    cut: str(raw.cut, 120),
    meta: str(raw.meta, 80),
  };
}

function finding(v: unknown): DevFinding | null {
  if (!isObj(v)) return null;
  const id = str(v.id, 80);
  const title = str(v.title, 200);
  if (!id || !title) return null;
  const d = isObj(v.dismissed) ? v.dismissed : null;
  return {
    id,
    title,
    severity: oneOf(v.severity, ['critical', 'high', 'medium', 'low'] as const, 'low'),
    file: str(v.file, 400),
    line: num(v.line),
    detail: str(v.detail, 1200),
    fix: str(v.fix, 600),
    source: oneOf(v.source, ['review', 'gitleaks', 'packages', 'tamper'] as const, 'review'),
    state: oneOf(v.state, ['open', 'sent', 'dismissed'] as const, 'open'),
    sent_at: str(v.sent_at, 64),
    sent_to: str(v.sent_to, 80),
    task_id: str(v.task_id, 80),
    send_error: str(v.send_error, 200),
    dismissed: d
      ? { reason: oneOf(d.reason, ['not_a_problem', 'fine_here', 'later', 'wrong'] as const, 'not_a_problem'), note: str(d.note, 500) ?? '', by: str(d.by, 80) ?? 'person', at: str(d.at, 64) }
      : null,
  };
}

const scan = (v: unknown) => {
  const o = isObj(v) ? v : {};
  return { state: oneOf(o.state, ['pass', 'hit', 'not_run'] as const, 'not_run'), count: num(o.count) ?? 0 };
};

export function parseReview(raw: unknown): ReviewCard | null {
  if (!isObj(raw) || raw.kind !== 'review') return null;
  const pr = num(raw.pr);
  if (pr === null) return null;
  const intent = isObj(raw.intent) ? raw.intent : null;
  const rv = isObj(raw.reviewer) ? raw.reviewer : null;
  const sec = isObj(raw.security) ? raw.security : {};
  return {
    ...base(raw),
    kind: 'review',
    pr,
    title: str(raw.title, 300) ?? `PR #${pr}`,
    branch: str(raw.branch, 200),
    base: str(raw.base, 200),
    draft: raw.draft === true,
    author: person(raw.author),
    reviewer: rv && str(rv.name, 80) ? { id: str(rv.id, 120), name: str(rv.name, 80)!, engine: str(rv.engine, 40), model: str(rv.model, 80) } : null,
    note: str(raw.note, 200),
    same_engine: raw.same_engine === true,
    risk: risk(raw.risk),
    intent: intent
      ? {
          source: str(intent.source, 40) ?? 'pr',
          asked: str(intent.asked, 600) ?? '',
          none: intent.none === true,
          rows: arr(intent.rows)
            .filter(isObj)
            .map((r) => ({ state: oneOf(r.state, ['ok', 'missing', 'extra'] as const, 'ok'), text: str(r.text, 300) ?? '' }))
            .filter((r) => r.text),
        }
      : null,
    tamper: {
      flags: arr(isObj(raw.tamper) ? raw.tamper.flags : [])
        .filter(isObj)
        .map((f) => ({
          kind: oneOf(f.kind, ['deleted_assert', 'skip', 'loosened', 'snapshot', 'coverage_drop'] as const, 'loosened'),
          file: str(f.file, 400) ?? '',
          line: num(f.line),
          excerpt: str(f.excerpt, 2000) ?? '',
          why: str(f.why, 400) ?? '',
        })),
      unread: arr(isObj(raw.tamper) ? raw.tamper.unread : [])
        .map((p) => str(p, 400))
        .filter((p): p is string => !!p),
    },
    security: { secrets: scan(sec.secrets), packages: scan(sec.packages) },
    findings: arr(raw.findings).map(finding).filter((f): f is DevFinding => !!f),
    cut: str(raw.cut, 120),
    links: links(raw.links),
    posted: posted(raw.posted),
    post_error: postError(raw.post_error),
  };
}

function posted(v: unknown): ReviewPosted | null {
  if (!isObj(v)) return null;
  const url = str(v.url, 1000);
  const n = num(v.comments);
  const fg = v.forge;
  return { forge: fg === 'github' || fg === 'gitlab' || fg === 'gitea' ? fg : null, at: str(v.at, 64), login: str(v.login, 80), url: url && /^https:\/\//.test(url) ? url : null, comments: n !== null && n >= 0 ? Math.floor(n) : null };
}

function postError(v: unknown): PostError | null {
  if (!isObj(v)) return null;
  return { kind: oneOf(v.kind, ['read_only', 'stale', 'failed'] as const, 'failed'), message: str(v.message, 300) };
}

export function parseReleaseNotes(raw: unknown): ReleaseNotesCard | null {
  if (!isObj(raw) || raw.kind !== 'release_notes') return null;
  return { ...base(raw), kind: 'release_notes', since_tag: str(raw.since_tag, 80), markdown: str(raw.markdown, 20000), whats_new: str(raw.whats_new, 2000) };
}

const FLOOR_KINDS = ['push_main', 'force_push', 'delete_ref', 'deploy', 'deploy_migration', 'destructive', 'prod_key'] as const;
const BACKUP_STATES = ['fresh', 'old', 'none', 'no_data', 'no_command', 'taking', 'failed'] as const;

export function parseBackup(v: unknown): FloorBackup {
  const b = isObj(v) ? v : {};
  return {
    state: oneOf(b.state, BACKUP_STATES, 'none'),
    age_s: num(b.age_s),
    bytes: num(b.bytes),
    path: str(b.path, 400),
    error: str(b.error, 200),
    percent: num(b.percent),
  };
}

/** `details.prod_floor` of an approval, or null when the card is not a production-floor card. */
export function parseProdFloor(details: unknown): ProdFloor | null {
  if (!isObj(details) || !isObj(details.prod_floor)) return null;
  const p = details.prod_floor;
  if (!FLOOR_KINDS.includes(p.kind as FloorKind)) return null;
  const blast = arr(p.blast)
    .filter((r): r is unknown[] => Array.isArray(r) && r.length === 2)
    .map((r) => [str(r[0], 40), str(r[1], 400)] as const)
    .filter((r): r is readonly [string, string] => !!r[0] && !!r[1])
    .map((r) => [r[0], r[1]] as [string, string]);
  return {
    kind: p.kind as FloorKind,
    why: str(p.why, 300) ?? '',
    blast,
    // Destructive and migration deploys always need one, whatever the payload says (§3.5).
    needs_backup: p.needs_backup === true || p.kind === 'destructive' || p.kind === 'deploy_migration',
    space_id: str(p.space_id, 80),
    backup: parseBackup(p.backup),
  };
}

export function parseBackupResult(v: unknown): BackupResult {
  const b = isObj(v) ? v : {};
  return { id: str(b.id, 80), ok: b.ok === true, path: str(b.path, 400), bytes: num(b.bytes), taken_at: str(b.taken_at, 64), error: str(b.error, 200) };
}
