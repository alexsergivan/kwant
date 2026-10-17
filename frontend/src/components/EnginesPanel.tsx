import React, { useState } from 'react';
import { AlertTriangle, ArrowRight, ChevronDown, ChevronRight, Cpu, Key, Loader2, RefreshCw, Route, Server } from 'lucide-react';
import { EngineInfo, checkedAgo, engineSummary, isAbsent, isUsable, onlyCliReady, sortEngines, statusView } from '../engines';
import { playSfx } from '../audio/chiptune';
import type { EnginesApi } from '../useEngines';
import { claudeNeedsSignIn } from '../claudeLogin';
import { ClaudeSignIn, ClaudeSignOut } from './ClaudeSignIn';
import { AccountList, StatusPill } from './EngineAccounts';
import { Profile, accountUsable, accountsForCard, activeLimit, formatUntil } from '../profiles';
import type { ProfilesState } from '../useProfiles';
import { RouteRow, routeSummary } from '../routing';

const SUMMARY_TEXT = {
  ok: 'text-emerald-300',
  warn: 'text-amber-200',
  error: 'text-rose-300',
  muted: 'text-zinc-400',
  busy: 'text-sky-300',
  info: 'text-cyan-200',
} as const;
const KIND_ICON = { cli: Cpu, api: Key, local: Server } as const;

/** Inline `code` in a reason like "Not signed in: run `claude login`". */
const withCode = (text: string) =>
  text.split(/(`[^`]+`)/).map((part, i) =>
    part.startsWith('`') && part.endsWith('`') ? (
      <code key={i} className="font-mono text-[10.5px] px-1 py-px rounded bg-zinc-800 text-zinc-200 select-text whitespace-nowrap">
        {part.slice(1, -1)}
      </code>
    ) : (
      <React.Fragment key={i}>{part}</React.Fragment>
    ),
  );

/** What the card summary needs from each account. */
const summaryAccounts = (accounts: Profile[], now: number) =>
  accounts.map((p) => {
    const limit = activeLimit(p.limited_until, now);
    return { name: p.kind === 'system' ? 'This machine' : p.name, usable: accountUsable(p, now), is_default: p.is_default, priority: p.priority, outUntil: limit ? formatUntil(limit, now) : null };
  });

/** One engine: its status, switch and models up top, and the accounts it can run on underneath. */
export const EngineCard: React.FC<{
  engine: EngineInfo;
  onToggle?: (on: boolean) => void;
  /** Its accounts, or undefined when they aren't known (still loading, or an older server). */
  accounts?: Profile[];
  onAccountsChanged?: () => Promise<void>;
  /** Sign in or out of "This machine". */
  hostAction?: React.ReactNode;
  now?: number;
}> = ({ engine, onToggle, accounts, onAccountsChanged, hostAction, now = Date.now() }) => {
  const view = statusView(engine);
  const Icon = KIND_ICON[engine.kind] ?? Cpu;
  const off = engine.enabled === false;
  const usable = isUsable(engine);
  const labelId = `engine-${engine.id}-name`;
  const hasMachineRow = !!accounts?.some((p) => p.kind === 'system');
  // With no "This machine" row, Claude's sign-in button sits under the name and replaces "run `claude login`".
  const signInHere = !hasMachineRow && engine.id === 'claude' && !!hostAction;
  const summary =
    signInHere && engine.status === 'signed_out' && !usable && !off
      ? { text: 'Not signed in · Sign in below', tone: 'warn' as const }
      : engineSummary(engine, accounts ? summaryAccounts(accounts, now) : []);
  const showAccounts = engine.kind !== 'local' && accounts !== undefined && !!onAccountsChanged;
  return (
    <li data-engine={engine.id} data-status={off ? 'off' : usable ? 'ready' : engine.status} aria-labelledby={labelId} className="rounded-xl border border-zinc-800 bg-zinc-900/90 text-xs">
      <div className="flex items-start gap-2.5 p-3">
        <Icon size={14} className={`mt-0.5 shrink-0 ${usable ? 'text-sky-400' : 'text-zinc-500'}`} aria-hidden="true" />
        <div className="flex-1 min-w-0 space-y-1">
          <div className="flex items-center gap-x-1.5 gap-y-1 flex-wrap">
            <h4 id={labelId} className={`font-semibold ${off ? 'text-zinc-400' : 'text-zinc-100'}`}>
              {engine.name}
            </h4>
            {engine.version && <span className="text-[10px] font-mono text-zinc-500">v{engine.version}</span>}
            <StatusPill tone={view.tone}>{view.label}</StatusPill>
          </div>
          <p data-summary className={`text-[11px] leading-snug break-words ${SUMMARY_TEXT[summary.tone]}`}>
            {withCode(summary.text)}
          </p>
          {usable && engine.models.length > 0 && (
            <p className="text-[11px] text-zinc-500 truncate" title={engine.models.map((m) => m.label).join(', ')}>
              {engine.models.length === 1 ? '1 model' : `${engine.models.length} models`}
              {' · '}
              {engine.models.slice(0, 3).map((m) => m.label).join(', ')}
              {engine.models.length > 3 ? '…' : ''}
            </p>
          )}
          {/* Without a "This machine" row to hold it, the sign-in sits here. */}
          {signInHere && <div className="pt-0.5">{hostAction}</div>}
        </div>
        {onToggle && typeof engine.enabled === 'boolean' && (
          <button
            type="button"
            role="switch"
            aria-checked={!off}
            aria-labelledby={labelId}
            title={off ? 'Off: kwnt never uses it' : 'On: kwnt may use it'}
            onClick={() => {
              playSfx('click');
              onToggle(off);
            }}
            className={`relative shrink-0 mt-px w-8 h-[18px] rounded-full transition-colors focus-visible:outline focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-sky-400 ${
              off ? 'bg-zinc-700' : 'bg-sky-600'
            }`}
          >
            <span className="sr-only">{off ? 'Off' : 'On'}</span>
            <span
              aria-hidden="true"
              className={`absolute top-[2px] left-[2px] w-[14px] h-[14px] rounded-full bg-white transition-transform motion-reduce:transition-none ${off ? '' : 'translate-x-[14px]'}`}
            />
          </button>
        )}
      </div>
      {showAccounts && (
        <div className="border-t border-zinc-800/80 px-3 pt-2 pb-3">
          <AccountList engine={engine} accounts={accounts!} onChanged={onAccountsChanged!} hostAction={hostAction} />
        </div>
      )}
    </li>
  );
};

/** "This machine"'s sign-in or sign-out: Claude's in the page, the other CLIs' as the command to run. */
const hostAction = (e: EngineInfo, api: EnginesApi, onSignedIn: () => void, focusClaudeSignIn?: number) => {
  if (e.enabled === false || e.kind !== 'cli') return null;
  if (e.id === 'claude') {
    if (e.status === 'signed_out') return <ClaudeSignIn onSignedIn={onSignedIn} focusRequest={focusClaudeSignIn} />;
    if (e.status === 'ready') return <ClaudeSignOut onSignedOut={api.apply} />;
    return null;
  }
  if (e.status === 'signed_out' && e.detail) return <p className="text-[11px] text-zinc-400 leading-snug">{withCode(e.detail)}</p>;
  return null;
};

/** The headline over the cards: how many can run, or the one thing to do when none can. */
function headline(engines: EngineInfo[], profiles: Profile[] | null, now: number): string {
  const ready = engines.filter(isUsable).length;
  if (ready > 0) {
    return `${ready} ready${onlyCliReady(engines) ? ', all through your signed-in CLIs, so no API keys are needed' : ''}. Kwants only use engines that are ready.`;
  }
  // An engine that's off but has an account ready is the quickest fix.
  for (const e of sortEngines(engines)) {
    if (e.enabled !== false || !profiles) continue;
    const s = engineSummary(e, summaryAccounts(accountsForCard(profiles, e.id), now));
    if (s.text.startsWith('Off · Turn on to use ') && !s.text.endsWith(' it')) return `No engine can run right now. ${e.name} is off: turn it on to use ${s.text.slice('Off · Turn on to use '.length)}.`;
  }
  return claudeNeedsSignIn(engines)
    ? 'No engine can run right now. Sign in with Claude below, or add an account.'
    : 'No engine can run right now. Turn on an engine and sign in to one of its accounts below.';
}

/** The answer to "what will run my next kwant?": engine → account → model per family, and the fallback. */
export const RouteSummary: React.FC<{ rows: RouteRow[]; pending: boolean }> = ({ rows, pending }) => (
  <div data-route-summary className="rounded-xl border border-sky-500/25 bg-gradient-to-b from-sky-500/[0.07] to-transparent p-3 space-y-2.5">
    <h4 className="flex items-center gap-1.5 text-[11px] font-semibold uppercase tracking-wide text-sky-200">
      <Route size={13} aria-hidden="true" /> What runs your kwants
    </h4>
    <ul className="space-y-2.5">
      {rows.map((r) => (
        <li key={r.family} data-route={r.family} className="text-[11px] leading-snug">
          <div className="flex flex-wrap items-center gap-x-1.5 gap-y-1">
            <span className="w-14 shrink-0 font-semibold text-zinc-100">{r.label}</span>
            <span className="px-1.5 py-0.5 rounded-md bg-zinc-800/80 text-zinc-300">{r.engine}</span>
            <ArrowRight size={11} className="text-zinc-500" aria-label="on" />
            <span className={`px-1.5 py-0.5 rounded-md font-medium ${r.account.otherProvider || r.note ? 'bg-amber-500/15 text-amber-100' : 'bg-emerald-500/15 text-emerald-100'}`}>
              {r.account.name}
            </span>
            {r.account.detail && <span className={r.account.otherProvider ? 'text-amber-200' : 'text-zinc-400'}>{r.account.detail}</span>}
          </div>
          <p className="mt-1 pl-0 sm:pl-[3.875rem] text-zinc-500">
            {r.note && <span className="text-amber-200">{r.note} </span>}
            {r.then.length > 0 ? `If it’s out of usage or signed out: ${r.then.join(', then ')}.` : r.note ? '' : 'No backup account is ready.'}
            {r.defaultModel && <> Kwants on Auto get {r.defaultModel}.</>}
          </p>
          {r.fallback && <p className="pl-0 sm:pl-[3.875rem] text-zinc-500">{r.fallback}</p>}
        </li>
      ))}
    </ul>
    <p className="text-[10.5px] text-zinc-500 border-t border-zinc-800/80 pt-2">
      {pending ? 'Loading accounts… ' : ''}A kwant set to a specific account in its own settings starts with that account instead.
    </p>
  </div>
);

/** Settings → "Engines & accounts": one card per engine with the accounts it runs on, ready ones first.
 *  `focusClaudeSignIn` changes to bring "Sign in with Claude" into view and focus it. */
export const EnginesPanel: React.FC<{ api: EnginesApi; accounts?: ProfilesState; now?: number; focusClaudeSignIn?: number }> = ({
  api,
  accounts,
  now = Date.now(),
  focusClaudeSignIn,
}) => {
  const { engines, error, rechecking, recheck, setEnabled, reload } = api;
  const [showAbsent, setShowAbsent] = useState(false);
  const profiles = accounts?.profiles ?? null;
  const sorted = engines ? sortEngines(engines) : [];
  const cardAccounts = (e: EngineInfo) => (profiles ? accountsForCard(profiles, e.id) : undefined);
  // An engine missing here but with accounts of its own (an API key account, say) still gets a card.
  const present = sorted.filter((e) => !isAbsent(e) || (cardAccounts(e)?.length ?? 0) > 0);
  const absent = sorted.filter((e) => !present.includes(e));
  const lastChecked = sorted.map((e) => e.checked_at).filter(Boolean).sort().pop() ?? null;
  const ago = checkedAgo(lastChecked, now);
  const routes = engines ? routeSummary(engines, profiles, now) : [];

  // An account change can change whether its engine is ready; the server re-checks accounts in the
  // background after a read, so look once more shortly after.
  const accountsChanged = async () => {
    await Promise.all([accounts?.reload(), reload()]);
    window.setTimeout(reload, 1500);
  };
  const signedIn = () => void accountsChanged();

  const card = (e: EngineInfo) => (
    <EngineCard
      key={e.id}
      engine={e}
      now={now}
      onToggle={(on) => setEnabled(e.id, on)}
      accounts={cardAccounts(e)}
      onAccountsChanged={accounts ? accountsChanged : undefined}
      hostAction={hostAction(e, api, signedIn, focusClaudeSignIn)}
    />
  );

  return (
    <section aria-labelledby="engines-title" className="bg-zinc-950/60 p-3 sm:p-4 rounded-xl border border-zinc-800 space-y-3">
      <div className="flex items-center gap-2">
        <Cpu size={15} className="shrink-0 text-sky-400" aria-hidden="true" />
        <h3 id="engines-title" className="text-xs font-semibold text-zinc-200 whitespace-nowrap">Engines &amp; accounts</h3>
        {ago && <span className="ml-auto min-w-0 truncate text-[10.5px] text-zinc-500" title={`Checked ${ago}`}><span className="hidden sm:inline">Checked </span>{ago}</span>}
        <button
          type="button"
          onClick={() => {
            playSfx('click');
            recheck();
            accounts?.reload();
          }}
          disabled={rechecking || engines === null}
          className={`${ago ? '' : 'ml-auto '}shrink-0 whitespace-nowrap flex items-center gap-1.5 px-2 py-1 rounded-md text-[11px] font-medium text-zinc-300 border border-zinc-800 hover:text-sky-200 hover:border-sky-500/40 disabled:opacity-50 transition-colors focus-visible:outline focus-visible:outline-2 focus-visible:outline-sky-400`}
        >
          {rechecking ? <Loader2 size={12} className="animate-spin motion-reduce:animate-none" aria-hidden="true" /> : <RefreshCw size={12} aria-hidden="true" />}
          {rechecking ? 'Checking…' : 'Re-check'}
        </button>
      </div>

      {routes.length > 0 ? (
        <RouteSummary rows={routes} pending={!!accounts && !profiles && !accounts.error} />
      ) : (
        <p className="text-[11px] text-zinc-400 leading-relaxed" aria-live="polite">
          {engines === null
            ? 'Checking which engines can run…'
            : engines.length === 0
              ? 'This server doesn’t report engine status yet.'
              : headline(engines, profiles, now)}
        </p>
      )}

      {error && (
        <p role="alert" className="flex items-center gap-1.5 text-[11px] text-rose-300">
          <AlertTriangle size={12} aria-hidden="true" /> {error}
        </p>
      )}
      {accounts?.error && (
        <div role="alert" className="flex flex-wrap items-center gap-2 rounded-lg border border-rose-500/20 bg-rose-500/10 px-3 py-2 text-[11px] text-rose-200">
          <AlertTriangle size={12} aria-hidden="true" />
          <span className="flex-1 min-w-0">Couldn’t load accounts: {accounts.error}</span>
          <button
            type="button"
            onClick={() => accounts.reload()}
            className="inline-flex items-center gap-1 px-2 py-1 rounded-md border border-rose-500/30 text-rose-100 hover:bg-rose-500/20 focus-visible:outline focus-visible:outline-2 focus-visible:outline-sky-400"
          >
            <RefreshCw size={11} aria-hidden="true" /> Try again
          </button>
        </div>
      )}
      {accounts && !profiles && !accounts.error && engines && engines.length > 0 && (
        <p role="status" className="flex items-center gap-1.5 text-[11px] text-zinc-500">
          <Loader2 size={11} className="animate-spin motion-reduce:animate-none" aria-hidden="true" /> Loading accounts…
        </p>
      )}

      {present.length > 0 && (
        <ul className="grid grid-cols-1 gap-2.5" aria-label="Engines on this computer">
          {present.map(card)}
        </ul>
      )}

      {absent.length > 0 && (
        <div>
          <button
            type="button"
            aria-expanded={showAbsent}
            aria-controls="engines-absent"
            onClick={() => setShowAbsent((v) => !v)}
            className="flex items-center gap-1 text-[11px] text-zinc-500 hover:text-zinc-300 transition-colors focus-visible:outline focus-visible:outline-2 focus-visible:outline-sky-400 rounded"
          >
            {showAbsent ? <ChevronDown size={12} aria-hidden="true" /> : <ChevronRight size={12} aria-hidden="true" />}
            Not available on this computer ({absent.length})
          </button>
          {showAbsent && (
            <ul id="engines-absent" className="mt-2 grid grid-cols-1 gap-2.5">
              {absent.map(card)}
            </ul>
          )}
        </div>
      )}
    </section>
  );
};
