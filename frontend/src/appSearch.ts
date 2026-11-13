// One list for any app (one-app-search/design.md §2, §3): built-in tools, Composio's apps and the public list of
// tool servers become one row per app. Each row has a lead (the way the button runs) and the other ways to add the
// same app, folded under it. Pure functions; FindTool renders them.
//
// Works without the server's `by_maker` / `sign_in` fields (design §4 "Vega"): the same rules run here on what the
// directory already sends. When the server sends them, they win.

import type { Connection, DirectoryEntry } from './connections';
import type { McpPreset } from './mcp';
import { ToolMatch, matchTool, normalize, rankTools } from './toolSearch';

// ── App key: what "the same app" means ─────────────────────────────────────────────────────────

/** Words that say what kind of thing it is, not which app (dropped unless they are the whole name). */
const KIND_WORDS = new Set(['mcp', 'server', 'official', 'tools', 'api']);

/** Other spellings of the same app (from the directory's ALIASES, only the ones that name one app for sure). */
const KEY_ALIASES: Record<string, string> = {
  '11labs': 'elevenlabs',
  googlemail: 'gmail',
  gcal: 'googlecalendar',
  twitter: 'x',
  postgres: 'postgresql',
};

/**
 * "Stripe MCP by InsightfulPipe" → "stripe"; "ElevenLabs" → "elevenlabs"; "11labs" → "elevenlabs";
 * "Stripe Billing Reports" → "stripebillingreports" (a different app).
 */
export function appKey(name: string): string {
  let s = (name ?? '').trim();
  const cut = s.replace(/\s+by\s+.+$/i, '').trim();
  if (cut) s = cut;
  const unbracketed = s.replace(/[([{][^)\]}]*[)\]}]/g, ' ').trim();
  if (unbracketed) s = unbracketed;
  const words = s.split(/[\s\-_./:]+/).filter(Boolean);
  const kept = words.filter((w) => !KIND_WORDS.has(w.toLowerCase()));
  const key = normalize((kept.length ? kept : words).join(' '));
  return KEY_ALIASES[key] ?? key;
}

// ── Who makes it ───────────────────────────────────────────────────────────────────────────────

/** "by notion.com" for a remote built-in tool: the last two labels of its address. */
export function presetPublisher(p: McpPreset): string | null {
  if (p.form.transport !== 'http') return null;
  try {
    const host = new URL(p.form.url).hostname.split('.');
    return host.slice(-2).join('.');
  } catch {
    return null;
  }
}

const hostOf = (url: string | null | undefined): string => {
  if (!url) return '';
  try {
    return new URL(url.trim()).hostname.toLowerCase();
  } catch {
    return '';
  }
};

/**
 * Made by the app's maker (design §2): the publisher is a domain (never io.github.*), the server's address is on that
 * domain, and the title is that domain's name (stripe.com → Stripe). The server's own `by_maker` wins when sent.
 */
export function byMaker(entry: DirectoryEntry): boolean {
  if (typeof entry.by_maker === 'boolean') return entry.by_maker;
  if ((entry.registry_name ?? '').toLowerCase().startsWith('io.github.')) return false;
  const kind = entry.publisher?.kind ?? 'domain';
  const domain = (entry.publisher?.label ?? '').trim().toLowerCase();
  if (kind !== 'domain' || !domain.includes('.')) return false;
  const host = hostOf(entry.url);
  if (!host || !(host === domain || host.endsWith(`.${domain}`))) return false;
  const namePart = domain.split('.').slice(-2)[0];
  return appKey(entry.title || '') === namePart;
}

// ── Sources and rows ───────────────────────────────────────────────────────────────────────────

/** Where a way to add an app comes from, in the words of its chip. */
export type Origin = 'official' | 'composio' | 'community' | 'builtin';

export interface ComposioSourceApp {
  slug: string;
  name: string;
  description: string;
  tools: number | null;
  connected: boolean;
}

interface SourceBase {
  /** Unique across the list: b-<preset>, c-<slug>, d-<registry name>. */
  id: string;
  name: string;
  description: string;
  origin: Origin;
  /** The domain after "Official ·", or the publisher after "Community ·". */
  by: string | null;
  remote: boolean;
  /** Connected (Composio) or already added (a server). */
  connected: boolean;
  match: ToolMatch | null;
  /** The app a key is for, in "Your own {App} API key". */
  keyApp: string;
}

export type Source =
  | (SourceBase & { kind: 'builtin'; preset: McpPreset; added: Connection | null })
  | (SourceBase & { kind: 'composio'; app: ComposioSourceApp })
  | (SourceBase & { kind: 'directory'; entry: DirectoryEntry });

export interface MergedRow {
  /** `app-<app key>`: stays the same when the public list lands, so the highlight keeps its app. */
  key: string;
  appKey: string;
  name: string;
  lead: Source;
  others: Source[];
  match: ToolMatch | null;
}

/** Which source leads a row, lower first (design §2 "Which source leads the row"). */
export function leadTier(s: Source): number {
  if (s.connected) return 0;
  if (s.origin === 'official') return s.remote ? 1 : 3;
  if (s.origin === 'composio') return 2;
  if (s.origin === 'builtin') return 4;
  return s.remote ? 5 : 6;
}

const presetHost = (p: McpPreset): string => (p.form.transport === 'http' ? hostOf(p.form.url) : '');
const presetUrl = (p: McpPreset) => (p.form.transport === 'http' ? p.form.url.trim().replace(/\/+$/, '').toLowerCase() : '');

/** The added server that is this built-in tool: same name, and for a remote tool the same host. A server a person named
 *  "Stripe" at another address is not the built-in Stripe; only a server with no address falls back to the name alone. */
export function addedServer(p: McpPreset, connections: Connection[]): Connection | null {
  const names = [p.form.name.toLowerCase(), p.label.toLowerCase()];
  const host = presetHost(p);
  return (
    connections.find((c) => {
      if (c.kind !== 'mcp' || !names.includes(c.name.toLowerCase())) return false;
      const at = (c.detail ?? '').trim().toLowerCase();
      return !host || !at || at === host;
    }) ?? null
  );
}

/** The longest known app name a title starts with, word by word ("Stripe Billing Reports" → "Stripe"). */
function knownAppIn(title: string, known: string[]): string | null {
  const words = title.trim().split(/\s+/);
  let best: string | null = null;
  for (const k of known) {
    const n = k.trim().split(/\s+/).length;
    if (n > words.length || (best && k.length <= best.length)) continue;
    if (normalize(words.slice(0, n).join(' ')) === normalize(k)) best = k;
  }
  return best;
}

export interface MergeInput {
  presets: McpPreset[];
  composio: ComposioSourceApp[];
  /** null: the public list isn't there (fewer than 2 letters, or the server has none). */
  directory: DirectoryEntry[] | null;
  connections: Connection[];
  /** Rows whose button was already pressed for this query: app key → the source id that keeps leading. */
  pinned?: Record<string, string>;
}

/** One row per app for `query`, best first (design §2). */
export function mergeRows(query: string, input: MergeInput): MergedRow[] {
  if (!normalize(query)) return [];
  const { presets, composio, directory, connections, pinned = {} } = input;
  const known = [...presets.map((p) => p.label), ...composio.map((a) => a.name)];
  const sources: Source[] = [];

  rankTools(query, presets.map((p) => ({ name: p.label, slug: p.id, p }))).forEach(({ entry: { p }, match }) => {
    const added = addedServer(p, connections);
    const pub = presetPublisher(p);
    sources.push({
      kind: 'builtin',
      id: `b-${p.id}`,
      name: p.label,
      description: p.blurb,
      origin: pub ? 'official' : 'builtin',
      by: pub,
      remote: p.form.transport === 'http',
      connected: !!added,
      match,
      keyApp: p.label,
      preset: p,
      added,
    });
  });

  rankTools(query, composio).forEach(({ entry: app, match }) =>
    sources.push({
      kind: 'composio',
      id: `c-${app.slug}`,
      name: app.name,
      description: app.description,
      origin: 'composio',
      by: null,
      remote: true,
      connected: app.connected,
      match,
      keyApp: app.name,
      app,
    }),
  );

  if (directory) {
    const presetUrls = new Set(presets.map(presetUrl).filter(Boolean));
    const seen = new Set<string>();
    for (const entry of directory) {
      // The same server as a built-in tool is listed once, as built in.
      if (entry.url && presetUrls.has(entry.url.trim().replace(/\/+$/, '').toLowerCase())) continue;
      if (seen.has(entry.registry_name)) continue;
      seen.add(entry.registry_name);
      const name = entry.title?.trim() || entry.registry_name;
      const official = byMaker(entry);
      sources.push({
        kind: 'directory',
        id: `d-${entry.registry_name}`,
        name,
        description: entry.description ?? '',
        origin: official ? 'official' : 'community',
        by: entry.publisher?.label || null,
        remote: entry.kind === 'remote',
        connected: !!entry.already_added,
        match: matchTool(query, { name }),
        keyApp: knownAppIn(name, known) ?? name,
        entry,
      });
    }
  }

  // Fold sources with the same app key (a Composio app also by its slug, a built-in tool by its id).
  const byKey = new Map<string, number>();
  const groups: Array<{ key: string; sources: Array<{ s: Source; i: number }> }> = [];
  sources.forEach((s, i) => {
    const keys = [appKey(s.name)];
    if (s.kind === 'composio') keys.push(normalize(s.app.slug));
    if (s.kind === 'builtin') keys.push(normalize(s.preset.id));
    const at = keys.map((k) => byKey.get(k)).find((g) => g !== undefined);
    const g = at ?? groups.push({ key: keys[0], sources: [] }) - 1;
    groups[g].sources.push({ s, i });
    keys.forEach((k) => k && !byKey.has(k) && byKey.set(k, g));
  });

  const rows = groups.map((g) => {
    const sorted = [...g.sources].sort((a, b) => leadTier(a.s) - leadTier(b.s) || a.i - b.i);
    const pin = pinned[g.key];
    const leadAt = Math.max(0, pin ? sorted.findIndex((x) => x.s.id === pin) : 0);
    const lead = sorted[leadAt].s;
    const others = sorted.filter((_, k) => k !== leadAt).map((x) => x.s);
    const rank = lead.match?.rank ?? Math.min(9, ...others.map((o) => o.match?.rank ?? 9));
    const tools = lead.kind === 'composio' ? lead.app.tools ?? 0 : 0;
    return { row: { key: `app-${g.key}`, appKey: g.key, name: lead.name, lead, others, match: lead.match } as MergedRow, rank, tools, first: g.sources[0].i };
  });
  rows.sort((a, b) => a.rank - b.rank || leadTier(a.row.lead) - leadTier(b.row.lead) || b.tools - a.tools || a.first - b.first);
  return rows.map((r) => r.row);
}

// ── How you sign in (design §3) ────────────────────────────────────────────────────────────────

export type SignInIcon = 'account' | 'key' | 'local' | 'none';
export interface SignIn {
  icon: SignInIcon;
  text: string;
  /** How the button reads for this way: "Connect" for a sign-in, else "Add". */
  button: 'Connect' | 'Add';
}

const keyCount = (e: DirectoryEntry): number => (e.inputs ?? []).filter((i) => i.secret || /key|token|secret/i.test(i.name)).length;

const localNeeds = (s: Source): string | null => {
  if (s.kind === 'builtin') return s.preset.needs.find((n) => n === 'Node.js' || n === 'uv') ?? null;
  if (s.kind === 'directory') {
    const cmd = (s.entry.command ?? '').trim().toLowerCase();
    if (/^(npx|node|npm|pnpm|bunx?)\b/.test(cmd)) return 'Node.js';
    if (/^(uvx|uv|python3?|pipx)\b/.test(cmd)) return 'uv';
  }
  return null;
};

/**
 * The sentence after the chip. `rowName` is the row's app; `other` is true in the other-ways block, where the app's
 * name is right above ("your account" instead of "your {App} account").
 */
export function signInOf(s: Source, opts: { rowName: string; composioAdded: boolean; other?: boolean }): SignIn {
  const account: SignIn = { icon: 'account', text: opts.other ? 'Sign in with your account' : `Sign in with your ${opts.rowName} account`, button: 'Connect' };
  const keys = (n: number): SignIn => ({ icon: 'key', text: n > 1 ? `${n} keys from ${s.keyApp}` : `Your own ${s.keyApp} API key`, button: 'Add' });
  const local = (): SignIn => {
    const needs = localNeeds(s);
    return { icon: 'local', text: needs ? `Runs on this computer · needs ${needs}` : 'Runs on this computer', button: 'Add' };
  };
  const none: SignIn = { icon: 'none', text: 'No sign-in needed', button: 'Add' };

  if (s.kind === 'composio') return opts.composioAdded ? account : { ...account, text: `${account.text} · free Composio key first` };
  if (s.kind === 'builtin') {
    const n = s.preset.needs;
    if (n.includes('Sign-in')) return account;
    if (n.includes('API key') || n.includes('Token')) return keys(1);
    return s.remote ? none : local();
  }
  const e = s.entry;
  const count = keyCount(e);
  switch (e.sign_in) {
    case 'account':
      return account;
    case 'key':
      return keys(Math.max(1, count));
    case 'none':
      return s.remote ? none : local();
  }
  if (count) return keys(count);
  if (!s.remote) return local();
  return s.origin === 'official' ? account : { icon: 'account', text: 'Signs in when you add it, if it asks', button: 'Add' };
}

// ── Words ──────────────────────────────────────────────────────────────────────────────────────

/** The source in words, for the row's aria-label: "official server by elevenlabs.io". */
export function sourceWords(s: Source): string {
  if (s.origin === 'official') return s.by ? `official server by ${s.by}` : 'official server';
  if (s.origin === 'composio') return 'through Composio';
  if (s.origin === 'community') return s.by ? `community server by ${s.by}` : 'community server';
  return 'built in';
}

/** The link that opens a row's other ways. */
export function otherWaysLabel(others: Source[]): string {
  if (others.length > 1) return `${others.length} other ways to add it`;
  const o = others[0];
  if (!o) return '';
  if (o.origin === 'composio') return 'Also through Composio';
  if (o.origin === 'community') return 'Also as a community server';
  if (o.origin === 'official') return 'Also as an official server';
  return 'Also built in';
}

/** At most 4 other ways show; past that one quiet line. */
export const OTHER_WAYS_SHOWN = 4;
export const moreOtherWays = (n: number): string => `and ${n} more community ${n === 1 ? 'server' : 'servers'}`;
