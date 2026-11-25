// Smart Results: a bot reply's own structure (a table, a list, typed kwant data) shown as a block, such
// as a sortable comparison, a chart or a checklist. The server picks at most one block per reply and
// copies every value from the reply text or typed kwant data; the client checks the block again before
// anything renders it (each renderer's `validate`, in results/registry.ts). Anything off, and the reply
// renders as markdown. Contract: plan.md §A.3 (version 1).

// ---------------------------------------------------------------------------------------------------
// Contract (plan §A.3)

export type Source =
  | { kind: 'markdown_table'; index: number }
  /** `ordered` absent means true (v1). Ordered and unordered lists are counted separately. */
  | { kind: 'markdown_list'; index: number; ordered?: boolean }
  /** No source in the text: drawn after the reply. */
  | { kind: 'slot' }
  /**
   * A stretch of the reply (recipe, 0.7.6, contract.md §2): from `lead` top-level blocks before the list
   * (index, ordered) through the list (last_index, last_ordered). Lists are counted as for markdown_list.
   * The block is drawn where the stretch was.
   */
  | { kind: 'markdown_section'; index: number; ordered: boolean; last_index: number; last_ordered: boolean; lead: number };

export type Provenance =
  | { kind: 'reply_text' }
  /** Declared by the bot with present_result and grounded against the reply. */
  | { kind: 'bot_declared'; tool_call_id: string }
  /** "task:<id>" or "attachment:<id>". */
  | { kind: 'kwant_data'; ref: string };

export interface BlockCommon<T extends string> {
  id: string;
  type: T;
  title: string | null;
  source: Source;
  provenance: Provenance;
}

export type ColumnRole = 'label' | 'value' | 'pros' | 'cons' | 'note';
export interface ComparisonColumn {
  key: string;
  label: string;
  numeric: boolean;
  unit: string | null;
  /** Absent in v1 payloads: the first column is the label, the rest are values. */
  role: ColumnRole;
}
export interface ComparisonRow {
  id: string;
  cells: string[];
}
/** 3–50 rows, 2–8 columns. */
export interface ComparisonBlock extends BlockCommon<'comparison'> {
  columns: ComparisonColumn[];
  rows: ComparisonRow[];
  highlight_row: string | null;
  highlight_reason: string | null;
}

export interface ChartSeries {
  key: string;
  label: string;
  unit: string | null;
}
export interface ChartPoint {
  id: string;
  /** The first cell, verbatim. */
  x: string;
  /** The row's other cells, verbatim, one per series. */
  cells: string[];
  /** The same cells as numbers; null where a cell has none. */
  values: (number | null)[];
}
/** 3–60 points, 1–4 series. */
export interface ChartBlock extends BlockCommon<'chart'> {
  chart_kind: 'line' | 'bar';
  x: { label: string; kind: 'time' | 'category' };
  series: ChartSeries[];
  points: ChartPoint[];
  /** Quotes cells only; never a computed number. */
  summary: string | null;
}

export interface MetricItem {
  id: string;
  label: string;
  value_text: string;
  value: number;
  unit: string | null;
  /** The rest of the item's text, verbatim ("up 8% vs October"). */
  note: string | null;
}
/** 2–6 items. */
export interface MetricsBlock extends BlockCommon<'metrics'> {
  items: MetricItem[];
}

export interface FactItem {
  id: string;
  label: string;
  value: string;
}
/** 3–12 items. */
export interface FactsBlock extends BlockCommon<'facts'> {
  items: FactItem[];
}

export type StepsVariant = 'steps' | 'checklist' | 'timeline';
export interface StepItem {
  id: string;
  text: string;
  /** checklist: ticked in the reply's markdown; null for the other variants. */
  done: boolean | null;
  /** timeline: the item's time label ("Mon 3 Nov", "Week 2"). */
  when: string | null;
  /** Only from a literal "Owner: X" or "(owner: X)". */
  owner: string | null;
}
/** 3–20 items. */
export interface StepsBlock extends BlockCommon<'steps'> {
  /** Absent in v1 payloads: "steps". */
  variant: StepsVariant;
  items: StepItem[];
}

export interface CardItem {
  id: string;
  title: string;
  /** An http(s) link from the reply, or null. */
  url: string | null;
  description: string;
  meta: string[];
}
/** 3–12 items. */
export interface CardsBlock extends BlockCommon<'cards'> {
  items: CardItem[];
}

export type Severity = 'critical' | 'high' | 'medium' | 'low' | 'info';
export interface FindingRef {
  path: string;
  line: number | null;
}
export interface FindingItem {
  id: string;
  severity: Severity;
  /** The reply's own token: "Blocker", "P1", "Nit". */
  severity_text: string;
  title: string;
  detail: string;
  refs: FindingRef[];
}
/** 2–30 items. */
export interface FindingsBlock extends BlockCommon<'findings'> {
  items: FindingItem[];
}

export type CheckStatus = 'passed' | 'failed' | 'error' | 'pending';
export interface VerificationCheck {
  id: string;
  label: string;
  /** The outcome check kind: command, http, file_exists, file_contains, file_sha256. */
  kind: string;
  status: CheckStatus;
  /** ≤ 300 chars, redacted like check output; untrusted text. */
  excerpt: string | null;
  /** ISO time; null while pending. */
  checked_at: string | null;
}
/** 1–20 checks; source is always slot. */
export interface VerificationBlock extends BlockCommon<'verification'> {
  target: { kind: 'task'; id: string; title: string };
  checks: VerificationCheck[];
  /** "3 of 4 checks passed", counted from typed rows. */
  summary: string;
}

/**
 * Set only when the server cut the preview: totals of the whole file (rows and columns for a table,
 * lines for text), or `truncated: true` when it knows the preview is partial but not the totals.
 */
export interface PreviewCut {
  total_rows?: number;
  total_cols?: number;
  total_lines?: number;
  truncated?: true;
}
export type FilePreview =
  | ({ kind: 'table'; columns: string[]; rows: string[][] } & PreviewCut)
  | ({ kind: 'text'; excerpt: string } & PreviewCut)
  | { kind: 'image' }
  | null;
export interface FileItem {
  id: string;
  attachment_id: string;
  name: string;
  mime: string;
  size_bytes: number;
  preview: FilePreview;
}
/**
 * The muted line under a cut preview: "Showing 20 of 143 rows", "Showing 20 of 512 lines", or
 * "Preview, file is longer" when only `truncated` is known. Null for a complete preview. The shown
 * counts are the rows and lines actually drawn; the totals are the server's.
 */
const TOTAL_FULL = new Intl.NumberFormat('en');
const TOTAL_COMPACT = new Intl.NumberFormat('en', { notation: 'compact', maximumFractionDigits: 1 });
/** A count from the server, written with Intl: "143", "12,345", "1.2M". Never exponent notation. */
export function formatTotal(n: number): string {
  return n >= 1_000_000 ? TOTAL_COMPACT.format(n) : TOTAL_FULL.format(n);
}

export function previewNote(preview: FilePreview): string | null {
  if (!preview || preview.kind === 'image') return null;
  const parts: string[] = [];
  if (preview.kind === 'table') {
    if (preview.total_rows !== undefined && preview.total_rows > preview.rows.length) parts.push(`${preview.rows.length} of ${formatTotal(preview.total_rows)} rows`);
    if (preview.total_cols !== undefined && preview.total_cols > preview.columns.length) parts.push(`${preview.columns.length} of ${formatTotal(preview.total_cols)} columns`);
  } else if (preview.total_lines !== undefined) {
    const shown = preview.excerpt.replace(/\n$/, '').split('\n').length;
    if (preview.total_lines > shown) parts.push(`${shown} of ${formatTotal(preview.total_lines)} lines`);
  }
  if (parts.length) return `Showing ${parts.join(', ')}`;
  // A total that is not larger than what is drawn (the server may send totals for every file) is not a cut.
  return preview.truncated ? 'Preview, file is longer' : null;
}

/** 1–10 files; source is always slot. */
export interface FilesBlock extends BlockCommon<'files'> {
  items: FileItem[];
}

export interface RecipeTime {
  /** The reply's own words: "Prep", "Cook", "Total". */
  label: string;
  text: string;
}
export interface RecipeIngredient {
  id: string;
  /** The line after `qty`: qty + text is the reply's full line ("1 onion" + ", diced"). */
  text: string;
  /** The leading amount exactly as written ("2 cups", "½", "1–2 tsp"); null when the line has none. */
  qty: string | null;
  /** The sub-heading it sits under ("For the sauce"). */
  group: string | null;
}
export interface RecipeStep {
  id: string;
  text: string;
}
/** 2–40 ingredients, 2–30 steps, 0–3 times, 0–6 notes. */
export interface RecipeBlock extends BlockCommon<'recipe'> {
  serves: number | null;
  times: RecipeTime[];
  ingredients: RecipeIngredient[];
  steps: RecipeStep[];
  notes: string[];
}

/** Pocket apps (pa4): where a pocket is in its life. `paused` is a flag on `live`, not a state. */
export type PocketState = 'writing' | 'checking' | 'dry_run_stopped' | 'ready' | 'live' | 'ended';
export type PocketJoinMode = 'wifi' | 'anywhere';
export interface PocketRound {
  /** "Round 1" */
  label: string;
  /** "90s music"; null when the round has no topic. */
  topic: string | null;
  written: number;
  total: number;
}
export interface PocketTeam {
  name: string;
  players: number;
  points: number;
  /** An avatar palette ("cyan"); null: the join-order palette (design.md "Team colours"). */
  palette: string | null;
  /** First names, results only; never more than the team's players. */
  members: string[];
}
export interface PocketMiss {
  /** "Q27" */
  label: string;
  question: string;
  answer: string | null;
  /** 0-100 */
  pct_right: number;
}
export interface PocketResults {
  /** 1-12 teams, best first. */
  teams: PocketTeam[];
  /** Points ahead of second place; null with one team. */
  margin: number | null;
  /** 0-3 questions, most missed first. */
  missed: PocketMiss[];
  near_misses: number | null;
  near_misses_accepted: number | null;
}
/** Secret Santa (pa10a): where the pocket is. `reveal_day` is the gift day before the reveal. */
export type SantaPhase = 'collecting' | 'drawn' | 'reveal_day' | 'ended';
/** One row of People and PINs: a first name, a creature and ticks. Never a match. */
export interface SantaPerson {
  /** The player id on the box, for Reset PIN and Remove. */
  id: string;
  name: string;
  /** A guest creature id from the page ("antenna"); null: the first one. */
  avatar: string | null;
  /** Opened their match; null before the draw. */
  looked: boolean | null;
  gift_ready: boolean | null;
  /** The host's own row. */
  you: boolean;
  /** 10 wrong PINs in all: locked until a reset code (pa1, ada-review.md decision 1). */
  locked: boolean;
}
/**
 * The santa kit's host facts on a Pocket card (pa10a/card-impl.md "Shape I read"). Counts, first names and
 * display dates only: the validator rejects any block carrying a match-like field or a question's words.
 */
export interface SantaInfo {
  phase: SantaPhase;
  joined: number;
  /** Everyone the request named; null when the host didn't say. */
  expected: number | null;
  /** First names not in yet (collecting). */
  waiting_for: string[];
  people: SantaPerson[];
  /** Counts after the draw; null before it or when the box doesn't send them. */
  looked: number | null;
  gifts_ready: number | null;
  asks_asked: number | null;
  asks_answered: number | null;
  /** First-name pairs kept apart in the draw. */
  keep_apart: Array<[string, string]>;
  /** "when all 9 are in, or Sun, Dec 6 at 6 PM, whichever comes first" */
  draw_rule: string | null;
  /** "Sun, Dec 6 at 6:00 PM" */
  drawn_at: string | null;
  reveal: 'scheduled' | 'shown' | 'skipped' | 'never';
  /** "7:00 PM" (reveal day) */
  reveal_at: string | null;
  /** "Fri, Dec 18 at 7:00 PM" (ended, after a reveal) */
  revealed_at: string | null;
  /** "Fri, Dec 25" */
  deletes_on: string | null;
  /** "Dec 18" */
  ended_on: string | null;
  /** The draw can run now (Lin's draw.can_draw). */
  can_draw: boolean;
  /** Why the rules leave someone with no one (Santa needs_you); plain text from the box. */
  blocked: string | null;
  /** How people join: own https, else Tailscale Funnel, else Same Wi-Fi. Never a quick tunnel. */
  join: { mode: 'anywhere' | 'wifi' | 'none'; anywhere_kind: 'https' | 'funnel' | null; anywhere_url: string | null; wifi_url: string | null } | null;
}

/** pa10b: the gather kit's three faces on one engine. */
export type GatherFace = 'signup' | 'rsvp' | 'vote';
/** open (taking answers), closed (final), tie (a vote's final round tied: the host picks or re-runs for 24 h). */
export type GatherPhase = 'open' | 'closed' | 'tie';
export type GatherAnswer = 'yes' | 'maybe' | 'no';
/** One row in People and paid (host view only), every player, answered or not. Never a ballot. */
export interface GatherPerson {
  id: string;
  name: string;
  avatar: string | null;
  answer: GatherAnswer | null;
  plus_ones: number;
  /** "vegetarian, no mushrooms" (food keys in words, then food_other); null: none. */
  food: string | null;
  /** The host's paid tick (`paid_at` set); null on rows that aren't Yes. */
  paid: boolean | null;
}
export interface GatherSlot {
  /** The slot's index as text. */
  id: string;
  name: string;
  cap: number;
  /** Who's in, with their note ("lentil stew"), in join order. */
  who: Array<{ name: string; avatar: string | null; note: string | null }>;
  /** First names on this slot's FIFO waiting list, first in line first. */
  waiting: string[];
  waiting_count: number;
}
/** A count line: an option and its votes (1st choices while open, the final round once closed). */
export interface GatherCount {
  name: string;
  votes: number;
}
/** One round of the ranked count ("How it was counted"). `out` is who dropped out after it. */
export interface GatherRound {
  counts: GatherCount[];
  out: string[];
  /** Ballots with no choice left. */
  exhausted: number;
}
/** Lin's draft kinds. A vote's missing member is not_answered. */
export type GatherKind = 'maybe' | 'not_paid' | 'moved_up' | 'not_answered';
/** A Chaser draft for one person (host view only). Nothing sends: the host copies or shares it. */
export interface GatherNudge {
  /** The draft's `to`, lowercased: the card's row key. */
  key: string;
  /** The draft's `to` as the host typed it; `copied` sends it back. */
  name: string;
  /** Null for a listed member who never joined. */
  player_id: string | null;
  avatar: string | null;
  why: GatherKind;
  /** With `{link}` filled with the join link the card shows. */
  text: string;
  /** "9:12 PM" or "Mon 9:12 PM": when the host last copied it; null: never. */
  copied_at: string | null;
  /** Inside the 48 h window: "Thu 6:40 PM", the next time a copy is allowed; null: copy now. */
  until: string | null;
}
/**
 * The gather card's view. On the wire, block.gather is Lin's `host_view` plus the pocket's `title` and
 * `expires_at` (pa10b/server-shape.md); validGather turns it into this, with times in the person's own zone.
 * Who voted is a list of names apart from the counts: no person ever carries a ballot.
 */
export interface GatherInfo {
  face: GatherFace;
  phase: GatherPhase;
  /** "End-of-season dinner" */
  title: string;
  /** "Sat, Dec 12 · 7:30 PM" */
  when: string | null;
  /** "The Old Mill" */
  place: string | null;
  /** "€35 each, paid to Oleks" */
  price: string | null;
  /** "Thu, Dec 10 at 8:00 PM"; null: no closing time. */
  closes_at: string | null;
  /** "Sat, Dec 12"; null: no event (a vote). */
  event_at: string | null;
  /** "Sat, Dec 19" (pocket.expires_at) */
  deletes_on: string;
  /** The host's first name ("Oleks"), for "Tie: Oleks picks". */
  host_name: string | null;
  /** Members see names and Yes / Maybe / No (RSVP) or the live count after voting (vote). */
  show_names: boolean;
  /** Members on the host's list; null without a list. */
  members: number | null;
  /** Listed members who haven't answered or voted. */
  not_answered: string[];
  rsvp: {
    yes: number;
    maybe: number;
    no: number;
    plus_ones: number;
    /** yes + plus-ones */
    coming: number;
    maybe_names: string[];
    no_names: string[];
    /** Members on the host's list who never answered; null without a list. */
    never: number | null;
    /** "3 vegetarian, 1 vegan, 1 nut allergy, 1 gluten-free"; null: none. */
    food: string | null;
    /** Host-only. Null when the RSVP has no price. */
    paid: { ticked: number; of: number; not_yet: string[] } | null;
  } | null;
  /** Sign-up slots in the host's order. */
  slots: GatherSlot[];
  /** Sign-up: the newest move up from a waiting list the host hasn't seen (state 6); `more` others. */
  moved_up: { name: string; slot: string; at: string; note: string | null; more: number } | null;
  vote: {
    kind: 'one_pick' | 'ranked';
    voted: number;
    members: number | null;
    /** Who voted, first names and creatures only (never what they picked). */
    voters: Array<{ name: string; avatar: string | null }>;
    /** The places in play (after a revote, only the tied ones). */
    options: Array<{ option: number; name: string }>;
    /** Open: 1st choices; closed or tie: the final round. Every place in play. */
    counts: GatherCount[];
    rounds: GatherRound[];
    winner: string | null;
    outcome: 'winner' | 'tie' | 'picked' | 'no_votes' | null;
    /** The tied places, with the option index `pick` sends. */
    tied: Array<GatherCount & { option: number }>;
  } | null;
  people: GatherPerson[];
  nudges: GatherNudge[];
  /** How members join: as santa (own https, else Tailscale Funnel, else Same Wi-Fi). Never a quick tunnel. */
  join: SantaInfo['join'];
}

/** pa13 front desk: one guest on the host's list (Lin's `host/state` guests, newest activity first). */
export interface DeskGuest {
  id: string;
  /** First name or nickname the guest typed. */
  name: string;
  creature: string | null;
  waiting: boolean;
  /** "Late check-out: 2 PM, Sun Dec 8" or a short label ("Bell couldn't answer this message"); null when not waiting. */
  handoff_reason: string | null;
  /** The newest message, cut to 90 characters by the server; null before the guest writes. */
  last: { from: DeskFrom; text: string; at: string } | null;
  unread: number;
  msgs: number;
  /** "2026-12-14": when this guest's chat is deleted; null before they write. */
  delete_on: string | null;
  /** pa13b, a booking desk's hand-off that names a time the host hasn't listed ("2026-12-01T18:00"); else null. */
  ask_slot_start?: string | null;
}
export type DeskFrom = 'guest' | 'bot' | 'host' | 'note';
/** One message in a guest's conversation (`?view=guest:<id>` → `conversation.messages`). */
export interface DeskMessage {
  id: string;
  from: DeskFrom;
  text: string;
  at: string;
  note: 'handoff' | 'host_away' | 'daily_cap' | 'booking_requested' | 'booking_confirmed' | 'booked_by_host' | 'booking_declined' | 'booking_cancelled' | null;
  /** pa13b booking notes: the slot's wall-clock start ("2026-12-02T10:00"). */
  start?: string | null;
}

/** pa13b: one person on a slot (host view). `name` is null once that guest's chat is deleted. */
export interface DeskSlotPerson {
  guest: string;
  name: string | null;
  note: string | null;
  at: string | null;
  by: 'guest' | 'host' | null;
}
/** pa13b: one time on a booking desk. `start` is the studio's wall-clock time, never converted. */
export interface DeskSlot {
  /** The server's id; null for a time the host just added and hasn't saved. */
  id: string | null;
  start: string;
  minutes: number;
  label: string | null;
  seats: number;
  requested: DeskSlotPerson[];
  confirmed: DeskSlotPerson[];
}
/** pa13b: the booking part of a desk (`spec.mode: "booking"`). */
export interface DeskBooking {
  slots: DeskSlot[];
  open_left: number;
  requests: number;
  booked_upcoming: number;
  booked_total: number;
}

/**
 * The front desk card's view (pa13). On the wire, block.desk is Lin's host summary (`GET /api/pockets/:code` →
 * `summary`) plus the face (`title`, `place`, `host_name`) and the desk's own `join`; validDesk turns it into this.
 * The answering bot's id is never kept: only its name and model.
 */
export interface DeskInfo {
  title: string;
  place: string | null;
  host_name: string | null;
  bot: { name: string; model: string | null };
  counts: { guests: number; waiting: number; replies_today: number; daily_cap: number; messages_today: number };
  /** RFC 3339 or display text; null: "midnight". */
  cap_resets_at: string | null;
  cost_per_reply_usd: number | null;
  guide_chars: number | null;
  guide_facts: number | null;
  /** Set-up only: the start of the house guide as the host pasted it. */
  guide_preview: string | null;
  guests: DeskGuest[];
  join: SantaInfo['join'];
  /** Set-up: the start was refused for want of an address that stays the same (409 no_quick_tunnel / needs_stable_address). */
  needs_address: boolean;
  /** Stopped: the day it stopped, and when the last chat goes. Totals fall back to counts. */
  ended: { at: string | null; deletes_on: string | null; days: number | null; bot_replies: number | null; host_replies: number | null; messages: number | null } | null;
  /** pa13b: "guide" (pa13a, the default) or "booking". */
  mode?: 'guide' | 'booking';
  /** pa13b: the times, when mode is "booking"; else null. */
  booking?: DeskBooking | null;
}

/** pa11b queue: one ticket on the host's list (Vega's host view `tickets`: waiting, called, and skipped ones still inside
 * their 10-minute recall window). Times are RFC 3339 from the box's clock. `name` is a first name only when `ask_name`. */
export interface QueueTicket {
  number: number;
  name: string | null;
  state: 'waiting' | 'called' | 'skipped';
  taken_at: string | null;
  called_at: string | null;
  /** Set only while called. */
  call_ends_at: string | null;
  /** Set only while skipped and still recallable. */
  recall_until: string | null;
}

/**
 * The queue card's view (pa11b). On the wire, block.queue is Vega's host view (`GET /api/pockets/:code` → `host_view`)
 * plus the pocket's `join` (and `ended_at` once ended); in set-up it may be the spec alone. validQueue turns it into this.
 */
export interface QueueInfo {
  /** "setup" while the bot is still setting it up (block state "writing"). */
  state: 'setup' | 'open' | 'paused' | 'closed' | 'ended';
  spec: { title: string; line: string | null; hours_until: string | null; call_minutes: number; ask_name: boolean; max_waiting: number; host_name: string | null };
  pause_text: string | null;
  close_text: string | null;
  paused_at: string | null;
  closed_at: string | null;
  auto_closed: boolean;
  closed_waiting: number;
  /** "13:00" on the box's clock, or null: until the host closes it. */
  hours_until: string | null;
  now_serving: number | null;
  next_number: number;
  tickets: QueueTicket[];
  summary: {
    served_today: number;
    skipped_today: number;
    left_today: number;
    waiting: number;
    /** null until 3 calls today. */
    avg_minutes: number | null;
    longest_wait_minutes: number | null;
    days: number;
    served_total: number;
    avg_total_minutes: number | null;
  };
  /** The box's clock when the view was made (RFC 3339): countdowns run against it, never the browser alone. */
  now: string | null;
  join: SantaInfo['join'];
  ended_at: string | null;
}

/**
 * The wall kit's host facts on a Pocket card (pa14/card-impl.md "Shape I read"): Juno's host summary (#1139,
 * server-impl.md "Host view" without `limits`, `queue` and `wall`) plus display text the bot writes. Counts,
 * sizes in MB and photo ids only: the photo lists come from the box when the card opens, never in the block.
 */
export interface WallInfo {
  state: 'open' | 'closed';
  /** "Sun 12 PM": when uploads close (display text). */
  closes_at: string | null;
  /** "Tue, Jan 5": when the photos leave the box (display text). */
  delete_on: string | null;
  /** The host's first name for guest-facing lines ('Oleks can make room'); null when unset. */
  host_name: string | null;
  captions: boolean;
  best_set: boolean;
  /** pa14b: "snap" when Snap's check picked the best set; anything else (or null) is the host's own pick. */
  best_by: string | null;
  counts: { total: number; waiting: number; approved: number; hidden: number; guests: number; senders: number };
  used_mb: number;
  pocket_cap_mb: number;
  /** Photo ids in the best set (≤ best_max), or proposed while `best_set` is false. */
  best: string[];
  best_max: number;
  keep_days: number;
  /** Spec limits for the meter line; null when the bot didn't send them. */
  photo_cap_mb: number | null;
  per_guest_max: number | null;
  /** pa14b: Snap is picking the best 30 now (closed, nothing proposed yet). */
  picking: boolean;
}

/**
 * A pocket app's card in chat (pa4, pa0a/design.md §1 and §6); source is always slot. Every time and
 * date is display text the server wrote for the person's time zone ("Fri, Nov 27 · 8:00 PM").
 * Emitted by the bot's pocket tools (pa3); impl.md documents the shape.
 */
export interface PocketBlock extends BlockCommon<'pocket'> {
  /** The pocket's code on the box (pocketApi). */
  code: string;
  /** "quiz" */
  kit: string;
  /** "Friday Pub Quiz" */
  name: string;
  /** "Fri, Nov 27 · 8:00 PM · 4 rounds · 40 questions · teams of up to 4" */
  when: string | null;
  state: PocketState;
  /** Live only: the timer is stopped. */
  paused: boolean;
  /** Writing: questions written; checking: answers checked. */
  progress: { done: number; total: number } | null;
  /** Writing: 0-8 rounds with their counts. */
  rounds: PocketRound[];
  /** Answers checked against a source; `swapped` names the replaced questions ("Q14"). */
  checks: { matched: number; total: number; swapped: string[] } | null;
  /** The dry run with pretend players. `stopped_at`, `what_happened`, `what_im_doing` on dry_run_stopped. */
  dry_run: { players: number; passed: boolean; stopped_at: string | null; what_happened: string | null; what_im_doing: string | null } | null;
  /** "About 1 more minute." while writing or checking. */
  eta: string | null;
  /** Ready: "The lobby opens Fri at 7:30 PM by itself. I'll remind you on Thursday evening." */
  opens_line: string | null;
  /** Live: the game so far. */
  live: { players: number; teams: number; leader: { name: string; points: number } | null; now: string | null; palettes: string[] } | null;
  /** Ready and live: how friends join. A URL that isn't known yet is null. */
  join: { mode: PocketJoinMode; wifi_url: string | null; anywhere_url: string | null; quick_tunnel?: boolean } | null;
  /** pa11: the class quiz's own line (`pocket.notice`), shown as is. Absent on a normal quiz. */
  notice?: string;
  /** Ended: "10:12 PM", the winner, the stats line, the deletion date ("Fri, Dec 4"). */
  ended: { at: string; winner: { name: string; points: number } | null; stats: string | null; deletes_on: string | null } | null;
  /** Bot calls and spend so far on the person's account. */
  budget: { calls: number; usd: number } | null;
  /** Ended with results: the card is the results card (design.md §6). */
  results: PocketResults | null;
  /** kit "santa" only: the Secret Santa card (pa10a). `state` is "live" until the phase is "ended". */
  santa: SantaInfo | null;
  /** kit "wall" only: the photo wall card (pa14). `state` is "live" while open, "ended" once closed. */
  wall?: WallInfo | null;
  /** kit "gather" only (pa10b): Sign-up, RSVP or Vote. `state` is "live" until the phase is "closed". */
  gather?: GatherInfo | null;
  /** kit "desk" only (pa13): the front desk (Guest Guide). `state` is "writing" in set-up, "live", then "ended". */
  desk?: DeskInfo | null;
  /** kit "queue" only (pa11b): the queue ticket host card. `state` is "writing" in set-up, "live", then "ended". */
  queue?: QueueInfo | null;
  /** pa4c: the quiz_drafts id (the bot card's `id`) that Edit questions opens. Null or absent: no editor. */
  draft?: string | null;
  /** pa4c: questions the person wrote or changed themselves (not source-checked). */
  yours?: number | null;
}

export type SmartBlock =
  | PocketBlock
  | RecipeBlock
  | ComparisonBlock
  | ChartBlock
  | MetricsBlock
  | FactsBlock
  | StepsBlock
  | CardsBlock
  | FindingsBlock
  | VerificationBlock
  | FilesBlock;
export type BlockType = SmartBlock['type'];
export type BlockOf<T extends BlockType> = Extract<SmartBlock, { type: T }>;

/** `results` on a message: 0 or 1 block in 0.6.5. */
export interface ResultSet {
  version: 1;
  blocks: SmartBlock[];
}

// ---------------------------------------------------------------------------------------------------
// Validation helpers, shared by every renderer's `validate`

export const MAX_PAYLOAD = 64 * 1024;
export const MAX_TEXT = 300;
const HTML_TAG = /<\/?[a-z][^>]*>/i;
export const isObj = (v: unknown): v is Record<string, unknown> => typeof v === 'object' && v !== null && !Array.isArray(v);
/** A string of at most `max` chars with no HTML tag; blank only when `empty`. */
export const isText = (v: unknown, max = MAX_TEXT, empty = false): v is string =>
  typeof v === 'string' && v.length <= max && (empty || v.trim() !== '') && !HTML_TAG.test(v);
/** The most a table cell or label may show (the server's `MAX_CELL`). */
export const MAX_CELL = 200;
/** The most a table cell may hold raw, URLs included (the server's `MAX_CELL_RAW`). */
export const MAX_CELL_RAW = 1000;
const MD_LINK = /!?\[([^\]]*)\]\([^)]*\)/g;
/**
 * A text's length as the reader sees it: a markdown link `[text](url)` counts as its text, since a
 * source link's URL isn't shown; a bare URL is shown, so it counts. The server's `shown_len` agrees.
 */
export const shownLength = (s: string): number => s.replace(MD_LINK, '$1').length;
/**
 * A table cell or label, as the server's `cell_fits` takes it: at most `MAX_CELL` shown characters
 * (`shownLength`) and `MAX_CELL_RAW` raw ones, no HTML tag; blank only when `empty`.
 */
export const isCell = (v: unknown, empty = false): v is string =>
  typeof v === 'string' && v.length <= MAX_CELL_RAW && shownLength(v) <= MAX_CELL && (empty || v.trim() !== '') && !HTML_TAG.test(v);
/** isText, or null/absent (returned as null). `undefined` means invalid. */
export const optText = (v: unknown, max = MAX_TEXT): string | null | undefined => (v === null || v === undefined ? null : isText(v, max) ? v : undefined);
export const isIndex = (v: unknown): v is number => typeof v === 'number' && Number.isInteger(v) && v >= 0;
export const isNum = (v: unknown): v is number => typeof v === 'number' && Number.isFinite(v);
export const inRange = (a: unknown, min: number, max: number): a is unknown[] => Array.isArray(a) && a.length >= min && a.length <= max;
export const uniqueIds = (items: unknown[]) =>
  items.every((x) => isObj(x) && typeof x.id === 'string' && x.id !== '') && new Set(items.map((x) => (x as { id: string }).id)).size === items.length;
/**
 * An http(s) URL, or null. Anything else (javascript:, data:, relative) is not a link a block may show,
 * and neither is one that carries a username or password (https://user:pw@host/): a link must not leak
 * a credential into the page, the history or a copy, and "user@" in front of a host is a spoofing trick.
 */
export const isHttpUrl = (v: unknown): v is string => {
  if (typeof v !== 'string' || v.length > 2048) return false;
  try {
    const u = new URL(v);
    return (u.protocol === 'https:' || u.protocol === 'http:') && u.username === '' && u.password === '';
  } catch {
    return false;
  }
};

export function parseSource(v: unknown): Source | null {
  if (!isObj(v)) return null;
  if (v.kind === 'slot') return { kind: 'slot' };
  if (v.kind === 'markdown_section') {
    const lead = v.lead ?? 0;
    if (!isIndex(v.index) || typeof v.ordered !== 'boolean' || !isIndex(v.last_index) || typeof v.last_ordered !== 'boolean') return null;
    if (!isIndex(lead) || lead > 12) return null;
    return { kind: 'markdown_section', index: v.index, ordered: v.ordered, last_index: v.last_index, last_ordered: v.last_ordered, lead };
  }
  if (!isIndex(v.index)) return null;
  if (v.kind === 'markdown_table') return { kind: 'markdown_table', index: v.index };
  if (v.kind === 'markdown_list') {
    if (v.ordered !== undefined && typeof v.ordered !== 'boolean') return null;
    return { kind: 'markdown_list', index: v.index, ordered: v.ordered !== false };
  }
  return null;
}

/** Provenance; absent (v1 payloads) means the reply text. */
export function parseProvenance(v: unknown): Provenance | null {
  if (v === undefined || v === null) return { kind: 'reply_text' };
  if (!isObj(v)) return null;
  if (v.kind === 'reply_text') return { kind: 'reply_text' };
  if (v.kind === 'bot_declared' && isText(v.tool_call_id, 200)) return { kind: 'bot_declared', tool_call_id: v.tool_call_id };
  if (v.kind === 'kwant_data' && typeof v.ref === 'string' && /^(task|attachment|pocket):[\w.-]{1,120}$/.test(v.ref)) return { kind: 'kwant_data', ref: v.ref };
  return null;
}

/** The fields every block has, checked; null if any is off. `title` absent (v1) means null. */
export function parseCommon<T extends BlockType>(b: unknown, type: T): (BlockCommon<T> & Record<string, unknown>) | null {
  if (!isObj(b) || b.type !== type || typeof b.id !== 'string' || b.id === '' || b.id.length > 120) return null;
  const title = optText(b.title);
  const source = parseSource(b.source);
  const provenance = parseProvenance(b.provenance);
  if (title === undefined || !source || !provenance) return null;
  return { ...b, id: b.id, type, title, source, provenance };
}

/** The blocks of a message's `results`, unchecked, or [] when it isn't a v1 result set this client can trust. */
export function resultBlocks(results: unknown): Record<string, unknown>[] {
  if (!isObj(results) || results.version !== 1 || !Array.isArray(results.blocks)) return [];
  try {
    if (JSON.stringify(results).length > MAX_PAYLOAD) return [];
  } catch {
    return [];
  }
  const blocks = results.blocks.filter(isObj);
  return uniqueIds(blocks) ? blocks : [];
}

/** Where a block sits: a table or list in the reply (as counted by rehypeSmartIndex), or after it. */
export type Place = { kind: 'table'; index: number } | { kind: 'list'; index: number; ordered: boolean } | { kind: 'slot' };

/** The unchecked block placed at `place`, if any. */
export function blockAt(results: unknown, place: Place): Record<string, unknown> | null {
  for (const b of resultBlocks(results)) {
    const src = parseSource(b.source);
    if (!src) continue;
    if (place.kind === 'slot' && src.kind === 'slot') return b;
    if (place.kind === 'table' && src.kind === 'markdown_table' && src.index === place.index) return b;
    if (place.kind === 'list' && src.kind === 'markdown_list' && src.index === place.index && src.ordered === place.ordered) return b;
  }
  return null;
}

// ---------------------------------------------------------------------------------------------------
// Comparison and steps (Pixel)

const ROLES: ColumnRole[] = ['label', 'value', 'pros', 'cons', 'note'];

export function validComparison(raw: unknown): ComparisonBlock | null {
  const b = parseCommon(raw, 'comparison');
  if (!b || b.source.kind !== 'markdown_table') return null;
  const cols = b.columns, rows = b.rows;
  if (!inRange(cols, 2, 8) || !inRange(rows, 3, 50)) return null;
  const columns: ComparisonColumn[] = [];
  for (const [j, c] of cols.entries()) {
    if (!isObj(c) || typeof c.key !== 'string' || !isCell(c.label) || typeof c.numeric !== 'boolean') return null;
    const unit = optText(c.unit, 40);
    if (unit === undefined) return null;
    const role = c.role === undefined || c.role === null ? (j === 0 ? 'label' : 'value') : c.role;
    if (!ROLES.includes(role as ColumnRole)) return null;
    columns.push({ key: c.key, label: c.label, numeric: c.numeric, unit, role: role as ColumnRole });
  }
  if (new Set(columns.map((c) => c.key)).size !== columns.length) return null;
  const rowsOk = rows.every((r) => isObj(r) && inRange(r.cells, columns.length, columns.length) && r.cells.every((c) => isCell(c, true)));
  if (!rowsOk || !uniqueIds(rows)) return null;
  const typed = rows as unknown as ComparisonRow[];
  const labels = typed.map((r) => plainText(r.cells[0]).toLowerCase());
  if (new Set(labels).size !== labels.length) return null;
  const hl = b.highlight_row ?? null;
  if (hl !== null && !typed.some((r) => r.id === hl)) return null;
  const why = optText(b.highlight_reason);
  if (why === undefined) return null;
  return {
    id: b.id,
    type: 'comparison',
    title: b.title,
    source: b.source,
    provenance: b.provenance,
    columns,
    rows: typed.map((r) => ({ id: r.id, cells: [...r.cells] })),
    highlight_row: hl as string | null,
    highlight_reason: hl === null ? null : why,
  };
}

const VARIANTS: StepsVariant[] = ['steps', 'checklist', 'timeline'];

export function validSteps(raw: unknown): StepsBlock | null {
  const b = parseCommon(raw, 'steps');
  if (!b || b.source.kind !== 'markdown_list') return null;
  const variant = (b.variant ?? 'steps') as StepsVariant;
  if (!VARIANTS.includes(variant)) return null;
  if (!inRange(b.items, 3, 20) || !uniqueIds(b.items)) return null;
  const items: StepItem[] = [];
  for (const it of b.items as Record<string, unknown>[]) {
    const when = optText(it.when, 80), owner = optText(it.owner, 80);
    const done = it.done ?? null;
    if (!isText(it.text) || when === undefined || owner === undefined || (done !== null && typeof done !== 'boolean')) return null;
    items.push({ id: it.id as string, text: it.text, done: variant === 'checklist' ? !!done : null, when, owner });
  }
  if (variant === 'timeline' && !items.every((i) => i.when)) return null;
  return { id: b.id, type: 'steps', title: b.title, source: b.source, provenance: b.provenance, variant, items };
}

/** A cell's text without inline markdown: what sorting, CSV and matching use. */
export function plainText(md: string): string {
  return md
    .replace(/!?\[([^\]]*)\]\([^)]*\)/g, '$1')
    .replace(/`([^`]*)`/g, '$1')
    .replace(/(\*\*|__)(.+?)\1/g, '$2')
    .replace(/(\*|_)(.+?)\1/g, '$2')
    .replace(/~~(.+?)~~/g, '$1')
    .replace(/\\([\\`*_{}[\]()#+\-.!|~])/g, '$1')
    .replace(/\s+/g, ' ')
    .trim();
}

/** Letters and digits only, so the rendered table and the block can be compared loosely. */
export const matchKey = (s: string) => plainText(s).toLowerCase().replace(/[^\p{L}\p{N}]+/gu, '');

/**
 * Does `caption` say what the reply's own text already says? A chart's summary that the reply's closing
 * sentence repeats ("October had 2,260 signups and 455 paid.") is not shown a second time under the chart.
 * Compared loosely, on letters and digits only, and only for a caption long enough to be a sentence.
 */
export function repeatsReply(caption: string | null | undefined, replyText: string | null | undefined): boolean {
  if (!caption || !replyText) return false;
  const key = matchKey(caption);
  return key.length >= 8 && matchKey(replyText).includes(key);
}

/**
 * The number in a numeric cell: "EUR 1,200", "12 GB", "$9.50", "-3%", "1 200". NaN when there isn't one.
 * The column's unit and a leading currency sign are dropped first.
 */
export function cellNumber(cell: string, unit: string | null): number {
  let s = plainText(cell);
  if (unit) s = s.split(unit).join(' ');
  s = s.replace(/[€$£¥₹]/g, ' ').trim();
  const m = /^([+-]?)\s*(\d{1,3}(?:[ ,\u00a0\u202f]\d{3})+|\d+)?(?:\.(\d+))?\s*%?$/.exec(s);
  if (!m || (m[2] === undefined && m[3] === undefined)) return NaN;
  const whole = (m[2] ?? '0').replace(/[ ,\u00a0\u202f]/g, '');
  const n = Number(`${whole}.${m[3] ?? '0'}`);
  return m[1] === '-' ? -n : n;
}

export type SortDir = 'ascending' | 'descending';
export interface Sort {
  col: number;
  dir: SortDir;
}

/** Rows in the chosen order; the reply's order when `sort` is null. Cells without a number go last. */
export function sortRows(block: ComparisonBlock, sort: Sort | null): ComparisonRow[] {
  if (!sort) return block.rows;
  const col = block.columns[sort.col];
  if (!col) return block.rows;
  const sign = sort.dir === 'ascending' ? 1 : -1;
  const keyed = block.rows.map((r, i) => ({ r, i, v: r.cells[sort.col] ?? '' }));
  if (col.numeric) {
    const num = (v: string) => cellNumber(v, col.unit);
    keyed.sort((a, b) => {
      const x = num(a.v), y = num(b.v);
      if (Number.isNaN(x) || Number.isNaN(y)) return Number.isNaN(x) === Number.isNaN(y) ? a.i - b.i : Number.isNaN(x) ? 1 : -1;
      return (x - y) * sign || a.i - b.i;
    });
  } else {
    const coll = new Intl.Collator(undefined, { numeric: true, sensitivity: 'base' });
    keyed.sort((a, b) => coll.compare(plainText(a.v), plainText(b.v)) * sign || a.i - b.i);
  }
  return keyed.map((k) => k.r);
}

/** The next sort when a column header is pressed: ascending, then descending, then the reply's order. */
export function nextSort(cur: Sort | null, col: number): Sort | null {
  if (!cur || cur.col !== col) return { col, dir: 'ascending' };
  return cur.dir === 'ascending' ? { col, dir: 'descending' } : null;
}

export const csvCell = (s: string) => {
  // A leading = + - @ would run as a formula in a spreadsheet; a plain signed number is left alone.
  const v = /^[=+\-@\t\r]/.test(s) && !/^[+-]?\d[\d.,\s]*%?$/.test(s) ? `'${s}` : s;
  return /[",\n\r]/.test(v) ? `"${v.replace(/"/g, '""')}"` : v;
};

/** The comparison as CSV, header first, rows in the order shown. */
export function comparisonCsv(block: ComparisonBlock, rows: ComparisonRow[] = block.rows): string {
  const lines = [block.columns.map((c) => plainText(c.label)), ...rows.map((r) => r.cells.map(plainText))];
  return lines.map((l) => l.map(csvCell).join(',')).join('\n') + '\n';
}

/** The steps as markdown, as the reply wrote them: numbered, or a task list for a checklist. */
export const stepsMarkdown = (block: StepsBlock, done?: Set<string>) =>
  block.items
    .map((it, i) =>
      block.variant === 'checklist'
        ? `- [${(done ? done.has(it.id) : it.done) ? 'x' : ' '}] ${it.text}`
        : block.variant === 'timeline'
          ? `- ${it.when}: ${it.text}`
          : `${i + 1}. ${it.text}`,
    )
    .join('\n') + '\n';

export const stepsStorageKey = (messageId: string, blockId: string) => `kwnt:steps:${messageId}:${blockId}`;

/** Ticked step ids for this message's block: what the person ticked here, else what the reply ticked. */
export function readDoneSteps(messageId: string, block: StepsBlock): Set<string> {
  const fromReply = new Set(block.items.filter((i) => i.done).map((i) => i.id));
  try {
    const stored = localStorage.getItem(stepsStorageKey(messageId, block.id));
    if (stored === null) return fromReply;
    const raw = JSON.parse(stored);
    const ids = new Set(block.items.map((i) => i.id));
    return new Set(Array.isArray(raw) ? raw.filter((x): x is string => typeof x === 'string' && ids.has(x)) : []);
  } catch {
    return fromReply;
  }
}

export function saveDoneSteps(messageId: string, block: StepsBlock, done: Set<string>) {
  try {
    const key = stepsStorageKey(messageId, block.id);
    const fromReply = block.items.filter((i) => i.done).map((i) => i.id);
    if (done.size === fromReply.length && fromReply.every((id) => done.has(id))) localStorage.removeItem(key);
    else localStorage.setItem(key, JSON.stringify([...done]));
  } catch {
    // Private mode or a full store: the ticks just don't outlast the page.
  }
}

/** The bits of a parsed (hast) node this file reads. */
export interface HastNode {
  type: string;
  tagName?: string;
  value?: string;
  children?: HastNode[];
  properties?: Record<string, unknown>;
  data?: Record<string, unknown>;
  position?: { start: { offset?: number }; end: { offset?: number } };
}

/** The unchecked block whose source is a stretch of the reply (a recipe), if any. */
export function sectionBlock(results: unknown): (Record<string, unknown> & { source: Extract<Source, { kind: 'markdown_section' }> }) | null {
  for (const b of resultBlocks(results)) {
    const src = parseSource(b.source);
    if (src?.kind === 'markdown_section') return { ...b, source: src };
  }
  return null;
}

/**
 * Runs after rehypeSmartIndex. When a block's source is a markdown_section, the reply's top-level blocks
 * from `lead` before the first list through the last list become one placeholder `div` that carries
 * that stretch's markdown (data.smartSection), so the block is drawn once in their place and never
 * repeats them. When the lists aren't where the block says, nothing changes and the reply stays text.
 */
export function rehypeSmartSection(markdown: string, results: unknown) {
  return () => (tree: HastNode) => {
    const raw = sectionBlock(results);
    const kids = tree.children;
    if (!raw || !kids) return;
    const { index, ordered, last_index, last_ordered, lead } = raw.source;
    const els = kids.filter((n) => n.type === 'element');
    const at = (i: number, o: boolean) => els.findIndex((n) => (n.tagName === 'ol' || n.tagName === 'ul') && n.data?.smartIndex === i && n.data.smartOrdered === o);
    const a = at(index, ordered), z = at(last_index, last_ordered);
    if (a < 0 || z < a || a - lead < 0) return;
    const from = els[a - lead], to = els[z];
    const start = from.position?.start.offset, end = to.position?.end.offset;
    if (start === undefined || end === undefined) return;
    const i = kids.indexOf(from), j = kids.indexOf(to);
    const holder: HastNode = {
      type: 'element',
      tagName: 'div',
      properties: {},
      children: [],
      data: { smartSection: true, markdown: markdown.slice(start, end) },
    };
    kids.splice(i, j - i + 1, holder);
  };
}

export const hastText = (n: HastNode | undefined): string =>
  !n ? '' : n.type === 'text' ? n.value ?? '' : (n.children ?? []).map(hastText).join('');

/**
 * Numbers the reply's tables, its ordered lists and its unordered lists, each in its own count, in
 * document order at any depth (pre-order, nested included), the way the server counts `source.index`.
 * Code fences and raw HTML never become these elements.
 */
export function rehypeSmartIndex() {
  return (tree: HastNode) => {
    let tables = 0;
    let ordered = 0;
    let unordered = 0;
    const walk = (n: HastNode, parent: HastNode | null) => {
      if (n.type === 'element' && n.tagName === 'table') n.data = { ...n.data, smartIndex: tables++ };
      if (n.type === 'element' && (n.tagName === 'ol' || n.tagName === 'ul')) {
        const isOl = n.tagName === 'ol';
        n.data = { ...n.data, smartIndex: isOl ? ordered++ : unordered++, smartOrdered: isOl, smartTop: parent?.type === 'root' };
      }
      n.children?.forEach((c) => walk(c, n));
    };
    walk(tree, null);
  };
}
