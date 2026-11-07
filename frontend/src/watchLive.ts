// 0.7.7 Watch live (W1–W3): which chats' browsers are active right now, worked out in the app from the
// browser events every stream in the space already gets, plus the running turns. No new route.
//
// A browser belongs to a chat, not to a bot (the server picks the team, then the channel, then the
// bot's direct chat), so everything here is keyed by that owner id.

export interface Browsing {
  /** The browser's owner: a team, a channel, or a bot (its direct chat and threads). */
  owner: string;
  url: string;
  host: string;
  /** ms since epoch of the last browser event for this owner. */
  at: number;
}

export type BrowsingMap = Readonly<Record<string, Browsing>>;

/** No browser event for this long and the owner drops out (the bot stopped browsing). */
export const BROWSING_TTL_MS = 60_000;

/** "shop.example.com" from a page address; the address itself when it isn't one. */
export function hostOf(url: string | undefined | null): string {
  if (!url) return '';
  try {
    return new URL(url).host.replace(/^www\./, '');
  } catch {
    return url;
  }
}

/** A browser event for `owner`: adds it, or refreshes its time (and its page when the event names one). */
export function noteBrowser(map: BrowsingMap, owner: string | undefined | null, url: string | undefined | null, now: number): BrowsingMap {
  if (!owner || owner === 'default') return map;
  const prev = map[owner];
  const nextUrl = url || prev?.url || '';
  return { ...map, [owner]: { owner, url: nextUrl, host: hostOf(nextUrl), at: now } };
}

/** Drops owners with no event for `ttl`. Returns the same map when nothing expired. */
export function expireBrowsing(map: BrowsingMap, now: number, ttl = BROWSING_TTL_MS): BrowsingMap {
  const stale = Object.values(map).filter((b) => now - b.at >= ttl);
  if (!stale.length) return map;
  const out = { ...map };
  for (const b of stale) delete out[b.owner];
  return out;
}

/** Drops one owner (its last running turn ended). */
export function dropBrowsing(map: BrowsingMap, owner: string | null | undefined): BrowsingMap {
  if (!owner || !map[owner]) return map;
  const out = { ...map };
  delete out[owner];
  return out;
}

export interface OwnerTurn {
  turn_id: string;
  kwant_id: string;
  state: 'queued' | 'running';
  context_id?: string;
}
interface Named {
  id: string;
  name: string;
}

/**
 * The browser a turn drives, as the server picks it: a team channel's team, a channel, else the bot
 * (its direct chat and its threads share the bot's browser).
 */
export function ownerOfTurn(
  turn: OwnerTurn,
  channels: readonly Named[],
  teamOf: (c: Named) => Named | undefined,
): string {
  const ch = turn.context_id ? channels.find((c) => c.id === turn.context_id) : undefined;
  if (ch) return teamOf(ch)?.id ?? ch.id;
  return turn.kwant_id;
}

/** True when some other running turn still drives `owner`, so its browsing mark stays. */
export function ownerStillRunning(
  owner: string,
  turns: Iterable<OwnerTurn>,
  channels: readonly Named[],
  teamOf: (c: Named) => Named | undefined,
  exceptTurnId?: string,
): boolean {
  for (const t of turns) {
    if (t.state !== 'running' || t.turn_id === exceptTurnId) continue;
    if (ownerOfTurn(t, channels, teamOf) === owner) return true;
  }
  return false;
}

/**
 * Q1 (a)+(b): the rows that get the globe. Every owner row (DM bot, team, channel), plus each bot whose
 * running turn is in a browsing chat. A bot row maps to the owner its click opens.
 */
export function globeRows(
  map: BrowsingMap,
  turns: Iterable<OwnerTurn>,
  channels: readonly Named[],
  teamOf: (c: Named) => Named | undefined,
): { owners: Record<string, Browsing>; bots: Record<string, Browsing> } {
  const owners: Record<string, Browsing> = { ...map };
  const bots: Record<string, Browsing> = {};
  for (const t of turns) {
    if (t.state !== 'running') continue;
    const b = map[ownerOfTurn(t, channels, teamOf)];
    if (b && (!bots[t.kwant_id] || bots[t.kwant_id].at < b.at)) bots[t.kwant_id] = b;
  }
  return { owners, bots };
}

/** One row in the panel's switcher. */
export interface SwitchEntry extends Browsing {
  kind: 'team' | 'channel' | 'bot';
  label: string;
  /** A bot's avatar creature. */
  sprite?: string;
  needsYou: boolean;
}

/**
 * The panel switcher's rows (Q2): chats that need you first, then the most recent page. Owners the app
 * no longer knows (a deleted chat) are left out.
 */
export function switchEntries(
  map: BrowsingMap,
  describe: (owner: string) => { kind: SwitchEntry['kind']; label: string; sprite?: string } | null,
  needsYou: (owner: string) => boolean,
): SwitchEntry[] {
  const out: SwitchEntry[] = [];
  for (const b of Object.values(map)) {
    const d = describe(b.owner);
    if (d) out.push({ ...b, ...d, needsYou: needsYou(b.owner) });
  }
  return out.sort((a, b) => Number(b.needsYou) - Number(a.needsYou) || b.at - a.at);
}

/** The globe's label and tooltip: "Watch Rook's page: shop.example.com". */
export const watchLabel = (name: string, host: string) => (host ? `Watch ${name}’s page: ${host}` : `Watch ${name}’s page`);

/**
 * Keeps only owners that a running turn still drives. Run it against the newest turns (a ref read inside
 * the state updater), never against a key worked out at an earlier render: that stale key is how a
 * browser event sent right after its `turn_started` got dropped (QA F6).
 */
export function pruneBrowsing(
  map: BrowsingMap,
  turns: Iterable<OwnerTurn>,
  channels: readonly Named[],
  teamOf: (c: Named) => Named | undefined,
): BrowsingMap {
  const list = Array.from(turns);
  const gone = Object.keys(map).filter((o) => !ownerStillRunning(o, list, channels, teamOf));
  if (!gone.length) return map;
  const out = { ...map };
  for (const o of gone) delete out[o];
  return out;
}

interface SeedStep {
  kind: string;
  tool?: string;
  detail?: string;
}

/** True for kwant's own browser tools (`mcp__kwnt__browser_open`, `browser_navigate`…). */
export const isBrowserTool = (tool: string | undefined) => !!tool && /(^|__)browser_[a-z_]+$/.test(tool);

/**
 * A page opened (or reloaded) mid-turn: a running turn whose newest tool step is a browser tool is
 * browsing now, before its next browser event arrives. The page comes from the step's detail when it
 * is an address.
 */
export function seedBrowsing(
  map: BrowsingMap,
  turns: Iterable<OwnerTurn & { steps?: readonly SeedStep[] }>,
  channels: readonly Named[],
  teamOf: (c: Named) => Named | undefined,
  now: number,
): BrowsingMap {
  let out = map;
  for (const t of turns) {
    if (t.state !== 'running' || !t.steps?.length) continue;
    const tools = t.steps.filter((s) => s.kind === 'tool');
    const last = tools[tools.length - 1];
    if (!last || !isBrowserTool(last.tool)) continue;
    const owner = ownerOfTurn(t, channels, teamOf);
    if (out[owner]) continue;
    const url = last.detail && /^https?:\/\//.test(last.detail) ? last.detail : '';
    out = noteBrowser(out, owner, url, now);
  }
  return out;
}
