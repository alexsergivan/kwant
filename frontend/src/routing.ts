// "What runs your kwants": for each model family that can run, the engine and account a turn starts
// on and the accounts it falls back to, in the server's order (src/profiles/mod.rs failover_order):
// the family's switched-on accounts, the default first, then by priority; at most 3 tries.

import { EngineInfo, FAMILIES, FAMILY_OF, Family, isUsable } from './engines';
import { Profile, accountUsable, cardOf, isThirdPartyLabel } from './profiles';

/** Turns try at most this many accounts (MAX_ATTEMPTS on the server). */
const MAX_TRIES = 3;

export interface RouteAccount {
  name: string;
  /** Email, provider or "API key", to tell accounts apart. */
  detail: string | null;
  /** Runs through Bedrock, Vertex or the like instead of a Claude login. */
  otherProvider: boolean;
}

export interface RouteRow {
  family: Family;
  /** "Claude", "Codex", … */
  label: string;
  /** The engine the first account runs on, e.g. "Claude Code CLI". */
  engine: string;
  /** The account a turn starts on. */
  account: RouteAccount;
  /** Then these, if the first is out of usage or signed out. */
  then: string[];
  /** The model a kwant on "Auto" for this family gets, when the engine says. */
  defaultModel: string | null;
  /** One sentence on what happens when every account is off, when it matters. */
  fallback: string | null;
}

const hostName = 'This machine';
const accountName = (p: Profile) => (p.kind === 'system' ? hostName : p.name);
const familyName = (f: Family) => FAMILIES.find((x) => x.id === f)!.label.replace(/ \(.*\)$/, '');

function describe(p: Profile): RouteAccount {
  const otherProvider = isThirdPartyLabel(p.account_label);
  const detail = p.kind === 'api_key' ? 'API key' : otherProvider ? `via ${p.account_label === 'third_party' ? 'another provider' : p.account_label}` : p.account_label;
  return { name: accountName(p), detail, otherProvider };
}

/** The family's accounts in the order a turn tries them; empty when none is switched on. */
export function tryOrder(profiles: Profile[], family: Family, now = Date.now()): Profile[] {
  const on = profiles
    .filter((p) => p.engine === family && p.enabled)
    .sort((a, b) => Number(b.is_default) - Number(a.is_default) || a.priority - b.priority || a.name.localeCompare(b.name));
  const [first, ...rest] = on;
  if (!first) return [];
  const usableRest = rest.filter((p) => accountUsable(p, now));
  const order = accountUsable(first, now) || usableRest.length === 0 ? [first, ...usableRest] : usableRest;
  return order.slice(0, MAX_TRIES);
}

/** One row per family that has a ready engine; [] when nothing can run. */
export function routeSummary(engines: EngineInfo[], profiles: Profile[] | null, now = Date.now()): RouteRow[] {
  return FAMILIES.flatMap(({ id: family }) => {
    const ready = engines.filter((e) => FAMILY_OF[e.id] === family && isUsable(e));
    if (ready.length === 0) return [];
    const mine = profiles?.filter((p) => p.engine === family) ?? [];
    const order = profiles ? tryOrder(profiles, family, now) : [];
    // Accounts on an engine that's off can't run, so skip them like the server's plan does.
    const onEngine = (p: Profile) => ready.some((e) => e.id === cardOf(p));
    const usable = order.filter((p) => accountUsable(p, now) && onEngine(p));
    const start = usable[0];
    const cli = ready.find((e) => e.kind === 'cli');
    const host = mine.find((p) => p.kind === 'system');
    let account: RouteAccount;
    let engine: EngineInfo;
    if (start) {
      account = describe(start);
      engine = ready.find((e) => e.id === cardOf(start)) ?? ready[0];
    } else if (profiles && cli && order.length === 0) {
      // No account switched on: the turn runs on the CLI's own login.
      account = host ? describe(host) : { name: hostName, detail: null, otherProvider: false };
      engine = cli;
    } else {
      // A ready engine with no account of its own (an API key from Settings, Ollama).
      engine = ready[0];
      account = { name: engine.uses?.label ?? (engine.kind === 'local' ? 'This computer' : engine.kind === 'api' ? 'API key in Settings' : hostName), detail: null, otherProvider: false };
    }
    const then = usable.slice(1).map(accountName);
    const hostOff = !!host && !host.enabled;
    const fallback = hostOff && engine.kind === 'cli' && mine.some((p) => p.kind !== 'system')
      ? `If you switch off every ${familyName(family)} account, kwants use This machine.`
      : null;
    const defaultModel = engine.models.find((m) => m.default)?.label ?? null;
    return [{ family, label: familyName(family), engine: engine.name, account, then, defaultModel, fallback }];
  });
}
