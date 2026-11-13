// Engines: whether each way of running a model (a CLI, an API key, Ollama) can run right now, from
// GET /api/engines, and the shared Engine → Model choice stored in a kwant's preferred_model.

export type EngineId = 'claude' | 'codex' | 'grok' | 'anthropic_api' | 'openai_api' | 'xai_api' | 'ollama' | 'custom_api';
export type EngineKind = 'cli' | 'api' | 'local';
export type EngineStatus = 'ready' | 'signed_out' | 'not_installed' | 'error' | 'checking' | 'disabled';
export type Tier = 'fast' | 'balanced' | 'deep';

export interface EngineModel {
  id: string;
  label: string;
  default: boolean;
  /** Set by the server when it knows; otherwise guessed from the name. */
  tier?: Tier | null;
}

export interface EngineInfo {
  id: EngineId;
  name: string;
  kind: EngineKind;
  status: EngineStatus;
  detail: string | null;
  version: string | null;
  models: EngineModel[];
  checked_at: string | null;
  /** Off in Settings: never checked or used. Missing on servers without the switch. */
  enabled?: boolean;
  /** On, and its own login or one of its accounts can run a kwant. Missing on older servers. */
  ready?: boolean;
  /** The account kwants run on now; null when not ready or the engine has no accounts. */
  uses?: { profile_id: string; label: string } | null;
}

/** What a kwant picks: the model family, whichever of its engines (CLI or key) can run it. */
export type Family = 'claude' | 'codex' | 'grok' | 'ollama' | 'custom';

export const FAMILY_OF: Record<EngineId, Family> = {
  claude: 'claude',
  anthropic_api: 'claude',
  codex: 'codex',
  openai_api: 'codex',
  grok: 'grok',
  xai_api: 'grok',
  ollama: 'ollama',
  custom_api: 'custom',
};

export const FAMILIES: Array<{ id: Family; label: string }> = [
  { id: 'claude', label: 'Claude' },
  { id: 'codex', label: 'Codex (OpenAI)' },
  { id: 'grok', label: 'Grok (xAI)' },
  { id: 'ollama', label: 'Ollama (local)' },
  { id: 'custom', label: 'Custom API' },
];

export const familyLabel = (f: Family) => FAMILIES.find((x) => x.id === f)?.label ?? f;

/** `info`: neither good nor bad, e.g. This machine set up for another provider. */
export type Tone = 'ok' | 'warn' | 'error' | 'muted' | 'busy' | 'info';

/** Whether kwants can run on it: the server's `ready` (which counts its accounts), else the host check. */
export const isUsable = (e: Pick<EngineInfo, 'status' | 'enabled' | 'ready'>) => e.enabled !== false && (e.ready ?? e.status === 'ready');

/** Whether anything can run a bot in the space: null while unknown (not loaded, or still
 *  checking with nothing ready yet), so "nothing can run" never flashes during startup checks. */
export function canRunNow(engines: EngineInfo[] | null | undefined): boolean | null {
  if (!engines?.length) return null;
  if (engines.some(isUsable)) return true;
  return engines.some((e) => e.enabled !== false && e.status === 'checking') ? null : false;
}

export function statusView(e: Pick<EngineInfo, 'status' | 'enabled' | 'ready'> & { kind?: EngineKind; id?: EngineId }): { label: string; tone: Tone } {
  if (e.enabled === false || e.status === 'disabled') return { label: 'Off', tone: 'muted' };
  if (isUsable(e)) return { label: 'Ready', tone: 'ok' };
  switch (e.status) {
    case 'ready':
      return { label: 'Ready', tone: 'ok' };
    case 'signed_out':
      return { label: 'Not signed in', tone: 'warn' };
    case 'not_installed':
      // An API engine without a key, or Ollama that isn't running, isn't "installed" either way.
      return { label: e.id === 'custom_api' ? 'No server' : e.kind === 'api' ? 'No key' : e.kind === 'local' ? 'Not running' : 'Not installed', tone: 'muted' };
    case 'error':
      return { label: 'Error', tone: 'error' };
    default:
      return { label: 'Checking…', tone: 'busy' };
  }
}

const RANK: Record<EngineStatus, number> = { ready: 0, checking: 1, error: 2, signed_out: 3, disabled: 4, not_installed: 5 };

/** Ready first, then ones that need attention, switched-off ones, and not installed last. Stable. */
export function sortEngines(list: EngineInfo[]): EngineInfo[] {
  const rank = (e: EngineInfo) => (e.enabled === false ? 4 : isUsable(e) ? 0 : RANK[e.status] === 0 ? 3 : RANK[e.status] ?? 1);
  return list
    .map((e, i) => ({ e, i }))
    .sort((a, b) => rank(a.e) - rank(b.e) || a.i - b.i)
    .map(({ e }) => e);
}

/** Engines that aren't on this computer at all, to fold away. An API engine with no key reports "not
 *  installed" too, but it stays out: adding a key on its card is how it becomes usable. */
export const isAbsent = (e: EngineInfo) => e.status === 'not_installed' && e.kind !== 'api' && e.enabled !== false && !isUsable(e);

/** Where to get an engine that isn't on this computer. */
export const INSTALL_URL: Partial<Record<EngineId, string>> = {
  claude: 'https://docs.claude.com/en/docs/claude-code/setup',
  codex: 'https://github.com/openai/codex#quickstart',
  ollama: 'https://ollama.com/download',
  grok: 'https://x.ai/cli',
};

/** Whether only CLIs can run things right now (so "no API keys needed" is true). */
export const onlyCliReady = (list: EngineInfo[]) => {
  const ready = list.filter(isUsable);
  return ready.length > 0 && ready.every((e) => e.kind === 'cli');
};

// ---- Tiers ----

export const TIERS: Array<{ id: Tier; label: string }> = [
  { id: 'fast', label: 'Fast' },
  { id: 'balanced', label: 'Balanced' },
  { id: 'deep', label: 'Deep' },
];

/** The server's tier, else a guess from well-known names; null when it can't tell. */
export function tierOf(m: EngineModel): Tier | null {
  if (m.tier) return m.tier;
  const s = `${m.id} ${m.label}`.toLowerCase();
  if (/haiku|mini|nano|flash|fast|small|lite/.test(s)) return 'fast';
  if (/opus|deep|\bmax\b|\bpro\b|heavy/.test(s)) return 'deep';
  if (/sonnet|balanced/.test(s)) return 'balanced';
  return null;
}

/** Models grouped Fast → Balanced → Deep, then the rest under "Other". Empty groups left out. */
export function groupByTier(models: EngineModel[]): Array<{ label: string; models: EngineModel[] }> {
  const groups = [...TIERS.map((t) => ({ tier: t.id as Tier | null, label: t.label })), { tier: null, label: 'Other' }];
  return groups
    .map((g) => ({ label: g.label, models: models.filter((m) => tierOf(m) === g.tier) }))
    .filter((g) => g.models.length > 0);
}

// ---- A kwant's pick, stored in preferred_model ----
// "auto" | "<family>" (kwnt picks the model) | "<family>/<model id>". Older values still read:
// "local_ollama" is Ollama, and a bare model name like "claude-opus-5-5" is that model.

export interface KwantPick {
  family: Family | 'auto' | null;
  model: string | null;
}

export function parsePick(value: string | null | undefined): KwantPick {
  const v = (value ?? '').trim();
  if (!v || v.toLowerCase() === 'auto') return { family: 'auto', model: null };
  if (v === 'local_ollama' || v === 'ollama') return { family: 'ollama', model: null };
  // "custom:<model>" (the server's form) or "custom/<model>"; model ids may hold either character.
  if (/^custom[:/]/.test(v)) return { family: 'custom', model: v.slice(7).trim() || null };
  const slash = v.indexOf('/');
  if (slash > 0) {
    const head = v.slice(0, slash);
    const rest = v.slice(slash + 1) || null;
    if (FAMILIES.some((f) => f.id === head)) return { family: head as Family, model: rest };
  }
  if (FAMILIES.some((f) => f.id === v)) return { family: v as Family, model: null };
  const s = v.toLowerCase();
  const family: Family | null = s.includes('claude')
    ? 'claude'
    : s.includes('grok')
      ? 'grok'
      : s.includes('codex') || s.includes('gpt') || s.includes('openai')
        ? 'codex'
        : null;
  return { family, model: v };
}

export function encodePick(family: Family | 'auto', model: string | null): string {
  if (family === 'auto') return 'auto';
  if (!model) return family === 'ollama' ? 'local_ollama' : family;
  return family === 'custom' ? `custom:${model}` : `${family}/${model}`;
}

export interface FamilyOption {
  id: Family;
  label: string;
  /** Engines of this family that can run now, e.g. "Claude Code CLI". */
  via: string[];
  /** Models any of them offers, deduplicated; the first default wins. */
  models: EngineModel[];
}

/** A ready Custom API account's models, which the engine list doesn't carry (they're per account). */
export interface CustomSource {
  name: string;
  models: EngineModel[];
}

/** Families with at least one engine that can run, in FAMILIES order. Custom API lists its accounts
 *  (`custom`) as its "via" and their models, since each account has its own server. */
export function familyOptions(engines: EngineInfo[], custom: CustomSource[] = []): FamilyOption[] {
  return FAMILIES.flatMap(({ id, label }): FamilyOption[] => {
    const usable = engines.filter((e) => FAMILY_OF[e.id] === id && isUsable(e));
    // A ready Custom API account is proof enough while the engine list catches up.
    if (usable.length === 0 && !(id === 'custom' && custom.length > 0)) return [];
    if (id === 'custom') {
      const seen = new Set<string>();
      const models = custom.flatMap((a) => a.models).filter((m) => !seen.has(m.id) && !!seen.add(m.id));
      return [{ id, label, via: custom.length ? custom.map((a) => a.name) : usable.map((e) => e.name), models }];
    }
    const seen = new Set<string>();
    let hasDefault = false;
    const models: EngineModel[] = [];
    for (const e of usable) {
      for (const m of e.models) {
        if (seen.has(m.id)) continue;
        seen.add(m.id);
        const isDefault: boolean = m.default && !hasDefault;
        hasDefault ||= isDefault;
        models.push({ ...m, default: isDefault });
      }
    }
    return [{ id, label, via: usable.map((e) => e.name), models }];
  });
}

export type PickHealth =
  | { kind: 'ok' }
  | { kind: 'loading' }
  | { kind: 'none_ready' }
  /** The saved engine can't run now; `reason` is the first engine's explanation. */
  | { kind: 'engine_unavailable'; family: Family; reason: string | null }
  /** The engine can run but no longer offers this model. */
  | { kind: 'model_missing'; family: Family; model: string }
  | { kind: 'unknown'; value: string };

/** Whether a saved pick can run with the engines as last checked. `engines` null while loading. */
export function pickHealth(value: string, engines: EngineInfo[] | null, custom: CustomSource[] = []): PickHealth {
  if (!engines) return { kind: 'loading' };
  const options = familyOptions(engines, custom);
  const pick = parsePick(value);
  if (pick.family === 'auto') return options.length ? { kind: 'ok' } : { kind: 'none_ready' };
  if (pick.family === null) return { kind: 'unknown', value };
  const family = pick.family;
  const opt = options.find((o) => o.id === family);
  if (!opt) {
    // Still being checked isn't broken yet.
    const mine = engines.filter((e) => FAMILY_OF[e.id] === family);
    if (mine.some((e) => e.status === 'checking' && e.enabled !== false)) return { kind: 'loading' };
    // The CLI that's installed but off says more than "No API key" on its API sibling.
    const installed = mine.filter((e) => e.status !== 'not_installed');
    const reason = installed.some((e) => e.enabled === false) || (mine.length > 0 && mine.every((e) => e.enabled === false))
      ? 'Switched off in Settings'
      : (installed.find((e) => e.detail) ?? mine.find((e) => e.enabled !== false && e.detail))?.detail ?? null;
    return { kind: 'engine_unavailable', family, reason };
  }
  // A model list we can't see (a CLI that lists none) can't prove a model is gone.
  if (pick.model && opt.models.length > 0 && !opt.models.some((m) => m.id === pick.model)) {
    return { kind: 'model_missing', family, model: pick.model };
  }
  return { kind: 'ok' };
}

/** Short name for a pick, e.g. "Auto", "Claude", "Claude · Sonnet". */
export function pickLabel(value: string, engines: EngineInfo[] | null = null): string {
  const pick = parsePick(value);
  if (pick.family === 'auto') return 'Auto';
  if (pick.family === null) return value;
  const family = familyLabel(pick.family).replace(/ \(.*\)$/, '');
  if (!pick.model) return family;
  const fam = pick.family;
  const known = engines?.filter((e) => FAMILY_OF[e.id] === fam).flatMap((e) => e.models).find((m) => m.id === pick.model);
  return `${family} · ${known?.label ?? pick.model}`;
}

/** A bot's engine in words, for its tooltip in the bot list: "Laya picks the model", "Runs on Claude · Sonnet". */
export function engineTip(value: string, engines: EngineInfo[] | null = null): string {
  return parsePick(value).family === 'auto' ? 'Laya picks the model' : `Runs on ${pickLabel(value, engines)}`;
}

/** "Claude Haiku (latest)" → "Haiku" when shown after the family name. */
function shortModelLabel(label: string, family: string): string {
  const out = label.replace(/\s*\(latest\)$/i, '');
  return out.toLowerCase().startsWith(family.toLowerCase() + ' ') ? out.slice(family.length + 1) : out;
}

/** "Claude · Sonnet" for a reply, from the `ran_on` the server adds to a message's meta. */
export function ranOnLabel(meta: string | null | undefined, engines: EngineInfo[] | null = null): string | null {
  if (!meta) return null;
  try {
    const ran = JSON.parse(meta)?.ran_on;
    if (!ran || typeof ran.engine !== 'string') return null;
    const engine = ran.engine as EngineId;
    const family: Family | undefined = FAMILIES.some((f) => f.id === ran.family) ? ran.family : FAMILY_OF[engine];
    const name = family ? familyLabel(family).replace(/ \(.*\)$/, '') : engine;
    if (typeof ran.model !== 'string' || !ran.model) return name;
    const label = typeof ran.model_label === 'string' && ran.model_label
      ? ran.model_label
      : [...(engines?.filter((e) => e.id === engine) ?? []), ...(engines?.filter((e) => e.id !== engine && FAMILY_OF[e.id] === family) ?? [])]
          .flatMap((e) => e.models)
          .find((m) => m.id === ran.model)?.label ?? ran.model;
    return `${name} · ${shortModelLabel(label, name)}`;
  } catch {
    return null;
  }
}

/** "checked 3 min ago" style age, or null when never checked. */
export function checkedAgo(at: string | null, now = Date.now()): string | null {
  if (!at) return null;
  const t = Date.parse(at);
  if (Number.isNaN(t)) return null;
  const s = Math.max(0, Math.round((now - t) / 1000));
  if (s < 60) return 'just now';
  const m = Math.round(s / 60);
  if (m < 60) return `${m} min ago`;
  const h = Math.round(m / 60);
  return h < 24 ? `${h} h ago` : `${Math.round(h / 24)} d ago`;
}

/** An account as the card summary needs it. */
export interface SummaryAccount {
  name: string;
  /** Signed in (or keyed), on, and not out of usage. */
  usable: boolean;
  is_default: boolean;
  priority: number;
  /** "back in 2 h" when it's out of usage. */
  outUntil?: string | null;
}

/** The line under an engine's name: which account kwants use, or the one next step. */
export function engineSummary(e: EngineInfo, accounts: SummaryAccount[] = []): { text: string; tone: Tone } {
  // The order turns try them: the default, then by priority.
  const next = [...accounts]
    .sort((a, b) => Number(b.is_default) - Number(a.is_default) || a.priority - b.priority)
    .find((a) => a.usable);
  if (e.enabled === false || e.status === 'disabled') {
    if (next) return { text: `Off · Turn on to use ${next.name}`, tone: 'muted' };
    return { text: e.kind === 'local' || accounts.length === 0 ? 'Off · Turn on to use it' : 'Off · Turn on, then sign in to an account', tone: 'muted' };
  }
  if (isUsable(e)) return e.uses ? { text: `Bots use ${e.uses.label === 'This machine' ? 'the sign-in on this computer' : e.uses.label}`, tone: 'ok' } : { text: 'Bots can use it', tone: 'ok' };
  switch (e.status) {
    case 'checking':
      return { text: 'Checking…', tone: 'busy' };
    case 'not_installed':
      if (e.id === 'custom_api')
        return accounts.length
          ? { text: 'None of its servers answered · Check them below', tone: 'warn' }
          : { text: 'Add a server below: Agent Maestro (GitHub Copilot), OpenRouter, NVIDIA, LM Studio or any OpenAI- or Anthropic-compatible one', tone: 'muted' };
      if (e.kind === 'api') return { text: 'No key yet · Add a key below to use it', tone: 'muted' };
      return { text: e.detail ?? (e.kind === 'local' ? 'Not running on this computer' : 'Not installed on this computer'), tone: 'muted' };
    case 'error':
      return { text: `${e.detail ?? 'The check failed'} · Try Re-check`, tone: 'error' };
    default: {
      const out = accounts.find((a) => a.outUntil);
      if (out) return { text: `${out.name.charAt(0).toUpperCase()}${out.name.slice(1)} is out of usage, back ${out.outUntil} · Sign in to another account below`, tone: 'warn' };
      if (accounts.length === 0) return { text: e.detail ?? 'Not signed in', tone: 'warn' };
      return { text: 'No account is signed in · Sign in to one below', tone: 'warn' };
    }
  }
}

// ---- The model badge on a bot's avatar (autonomy/model-badge/brief.md) ----

/** What a turn runs on: the live `turn_runs_on` event, or a bot's `last_ran_on` from GET /api/kwants. */
export interface RunsOn {
  engine: string;
  family?: string | null;
  model?: string | null;
  model_label?: string | null;
  tier?: string | null;
  /** "laya", "pinned" or "fallback" (the turn moved off the bot's first-choice account). */
  picked_by?: string | null;
  at?: string | null;
}

/** The mark drawn in the badge: a provider's, or Laya's ✦ for an auto bot that never ran. */
export type BadgeMark = Family | 'laya';

export interface ModelBadge {
  mark: BadgeMark;
  /** Auto and resting: the last reply's mark at 60%, with a ✦ in its corner. */
  resting: boolean;
  /** Laya picks this bot's model (the ✦ corner when resting). */
  auto: boolean;
  /** The engine can't run: signed out, switched off, or not on this computer at all (no CLI, key or
   *  server). Greyed out. */
  cantRun: 'signed_out' | 'off' | 'not_set_up' | null;
  /** Tooltip and aria-label; the name's tooltip in the bot list says the same. */
  label: string;
  /** Changes exactly when the mark changes, for the cross-fade. */
  key: string;
}

const familyOfRun = (r: RunsOn): Family | null => {
  if (r.family && FAMILIES.some((f) => f.id === r.family)) return r.family as Family;
  return FAMILY_OF[r.engine as EngineId] ?? null;
};

/** "Claude · Haiku" for a turn, worded exactly like the reply's own ran-on line. */
export function runsOnLabel(r: RunsOn, engines: EngineInfo[] | null = null): string {
  const family = familyOfRun(r);
  return ranOnLabel(JSON.stringify({ ran_on: { ...r, family: family ?? r.family } }), engines) ?? (family ? familyLabel(family).replace(/ \(.*\)$/, '') : r.engine);
}

/** Whether a family's engines can run now; why not when they can't. null while unknown. */
function familyCantRun(family: Family, engines: EngineInfo[] | null): ModelBadge['cantRun'] {
  if (!engines?.length) return null;
  const mine = engines.filter((e) => FAMILY_OF[e.id] === family);
  if (!mine.length || mine.some(isUsable)) return null;
  if (mine.some((e) => e.enabled !== false && e.status === 'checking')) return null;
  // Signed in but not ready right now (out of usage until it resets): failover still runs the bot, so it isn't greyed.
  if (mine.some((e) => e.enabled !== false && e.status === 'ready')) return null;
  // Off when every one that's here at all is switched off; otherwise it needs a sign-in (or a key).
  const here = mine.filter((e) => e.status !== 'not_installed' || e.enabled === false);
  if (!here.length) return 'not_set_up';
  return here.every((e) => e.enabled === false || e.status === 'disabled') ? 'off' : 'signed_out';
}

/** pickLabel, with the model named the way a reply's ran-on line names it: "Claude · Sonnet", not "Claude · Claude Sonnet (latest)". */
function shortPickLabel(value: string, engines: EngineInfo[] | null): string {
  const full = pickLabel(value, engines);
  const at = full.indexOf(' · ');
  if (at < 0) return full;
  const model = shortModelLabel(full.slice(at + 3), full.slice(0, at));
  // A bare id the engine list doesn't know yet ("sonnet") reads as a name.
  return `${full.slice(0, at)} · ${/^[a-z]+$/.test(model) ? model[0].toUpperCase() + model.slice(1) : model}`;
}

/**
 * The badge for a bot: its pinned engine, or (auto) Laya's pick for the reply it's working on, else
 * the engine of its last reply. `liveTurn` is the running turn's `turn_runs_on`, null when resting.
 * null when there's nothing to show (no bot, or a pick this app can't read).
 */
export function modelBadgeFor(
  kwant: { preferred_model?: string | null; last_ran_on?: RunsOn | null } | null | undefined,
  liveTurn: RunsOn | null | undefined,
  engines: EngineInfo[] | null = null,
): ModelBadge | null {
  if (!kwant) return null;
  const pick = parsePick(kwant.preferred_model);
  const auto = pick.family === 'auto';
  if (liveTurn) {
    const family = familyOfRun(liveTurn);
    if (!family) return null;
    const what = runsOnLabel(liveTurn, engines);
    const label = liveTurn.picked_by === 'fallback'
      ? `Moved to ${what} for this reply`
      : auto
        ? `Laya picked ${what} for this reply`
        : `Runs on ${what}`;
    return { mark: family, resting: false, auto, cantRun: null, label, key: `${family}:${liveTurn.model ?? ''}` };
  }
  if (auto) {
    const last = kwant.last_ran_on;
    const family = last ? familyOfRun(last) : null;
    if (!last || !family) return { mark: 'laya', resting: false, auto, cantRun: null, label: 'Laya picks the model', key: 'laya' };
    return {
      mark: family,
      resting: true,
      auto,
      cantRun: null,
      label: `Laya picks the model. Last reply: ${runsOnLabel(last, engines)}`,
      key: `${family}:${last.model ?? ''}:rest`,
    };
  }
  const family = pick.family;
  if (family === null || family === 'auto') return null;
  const cantRun = familyCantRun(family, engines);
  const suffix = cantRun ? ` · ${cantRun === 'not_set_up' ? 'not set up' : cantRun === 'off' ? 'off' : 'signed out'}` : '';
  return {
    mark: family,
    resting: false,
    auto,
    cantRun,
    label: `Runs on ${shortPickLabel(kwant.preferred_model ?? '', engines)}${suffix}`,
    key: `${family}:${pick.model ?? ''}`,
  };
}

/** The badge's diameter for an avatar of `size` px: none under 24, 14 at 28–34, larger on big avatars. */
export function modelBadgeSize(size: number): number | null {
  if (size < 24) return null;
  if (size < 28) return 12;
  return Math.max(14, Math.min(28, Math.round(size * 0.28)));
}
