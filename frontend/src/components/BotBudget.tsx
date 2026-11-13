import React, { useEffect, useRef, useState, useSyncExternalStore } from 'react';
import { Check, Gauge, Loader2, PauseCircle, Play, RotateCcw, SlidersHorizontal } from 'lucide-react';
import {
  BotBudget,
  BudgetFields,
  BudgetNoticeMeta,
  PAUSED_LINE,
  botBudgetApi,
  budgetBody,
  budgetStore,
  budgetTokensError,
  budgetUsdError,
  budgetsNeedingYou,
  fieldsChanged,
  fieldsFromBudget,
  hasBudget,
  meterPercent,
  stateLine,
  takeBudgetFocus,
  usedLine,
} from '../botBudget';

const btnAmber =
  'shrink-0 inline-flex items-center justify-center gap-1.5 min-h-[44px] sm:min-h-[36px] px-3 py-1.5 rounded-lg text-xs font-medium border transition-colors cursor-pointer disabled:cursor-default text-amber-100 bg-amber-500/15 border-amber-500/40 hover:bg-amber-500/25 disabled:opacity-70 disabled:hover:bg-amber-500/15 aria-disabled:cursor-default aria-disabled:opacity-70 aria-disabled:hover:bg-amber-500/15 focus-visible:outline focus-visible:outline-2 focus-visible:outline-amber-400';
const btnQuiet =
  'shrink-0 inline-flex items-center justify-center gap-1.5 min-h-[44px] sm:min-h-[36px] px-3 py-1.5 rounded-lg text-xs font-medium border border-zinc-700 text-zinc-300 hover:bg-zinc-800 disabled:opacity-60 disabled:cursor-default focus-visible:outline focus-visible:outline-2 focus-visible:outline-sky-500';
const btnPrimary =
  'shrink-0 inline-flex items-center justify-center gap-1.5 min-h-[44px] sm:min-h-[36px] px-3 py-1.5 rounded-lg text-xs font-medium border border-sky-500/50 bg-sky-500/15 text-sky-100 hover:bg-sky-500/25 disabled:opacity-50 disabled:cursor-default focus-visible:outline focus-visible:outline-2 focus-visible:outline-sky-400';

/** The space's budgets, shared. The app loads and refreshes the list; this only reads it. */
export function useBudgets() {
  return useSyncExternalStore(budgetStore.subscribe, budgetStore.get, budgetStore.get);
}

// ---------------------------------------------------------------- the meter

/** A small meter: amber from 80%, and full when paused. `size` "row" is 28 px wide, for a sidebar row. */
export const BudgetMeter: React.FC<{ budget: BotBudget; size?: 'row' | 'full'; className?: string }> = ({ budget, size = 'full', className = '' }) => {
  const pct = meterPercent(budget);
  const hot = budget.state === 'warn' || budget.state === 'paused';
  const fill = hot ? 'bg-amber-400' : budget.state === 'continued' ? 'bg-zinc-500' : 'bg-sky-400';
  const label = `${budget.kwant_name || 'This bot'} has used ${pct}% of its monthly budget`;
  return (
    <span
      role="meter"
      aria-valuemin={0}
      aria-valuemax={100}
      aria-valuenow={pct}
      aria-label={label}
      title={`${pct}% of its monthly budget · ${usedLine(budget)}`}
      data-budget-meter={budget.state}
      className={`relative inline-block shrink-0 overflow-hidden rounded-full ${hot ? 'bg-amber-500/20' : 'bg-zinc-800'} ${size === 'row' ? 'h-1 w-7' : 'h-1.5 w-full'} ${className}`}
    >
      <span className={`absolute inset-y-0 left-0 rounded-full ${fill}`} style={{ width: `${pct}%` }} />
    </span>
  );
};

// ---------------------------------------------------------------- Raise / Continue, once

type Act = 'idle' | 'sending' | 'done' | 'error';

/** Continue this month: one post however fast the clicks; the answer goes into the shared list. */
function useContinueMonth(kwantId: string, send?: typeof fetch, onContinued?: () => void) {
  const api = useRef(botBudgetApi(send)).current;
  const [state, setState] = useState<Act>('idle');
  const [error, setError] = useState<string | null>(null);
  const lock = useRef(false);
  const run = async () => {
    if (lock.current) return;
    lock.current = true;
    setState('sending');
    setError(null);
    try {
      const next = await api.continueMonth(kwantId);
      // Before the shared list changes, so a caller that keeps this row (Inbox) knows in the same render
      onContinued?.();
      budgetStore.put(next);
      setState('done');
    } catch (e) {
      lock.current = false;
      setState('error');
      setError(e instanceof Error && e.message ? e.message : 'Couldn’t continue. Try again.');
    }
  };
  return { state, error, run };
}

/** The pause's two ways on: raise the limit (the bot's settings, at the budget) or continue this month.
 *  `onContinued` runs once the continue is saved, so the caller can move focus before this unmounts (#536 Mira F4).
 *  The button is aria-disabled, never disabled, while it sends and once done, so focus stays on it. */
export const PausedActions: React.FC<{
  budget: BotBudget;
  onRaise?: (kwantId: string) => void;
  onContinued?: () => void;
  describedBy?: string;
  send?: typeof fetch;
}> = ({ budget, onRaise, onContinued, describedBy, send }) => {
  const { state, error, run } = useContinueMonth(budget.kwant_id, send, onContinued);
  const done = state === 'done' || budget.state === 'continued';
  return (
    <>
      <div className="flex flex-wrap items-center gap-2">
        {onRaise && !done && (
          <button type="button" onClick={() => onRaise(budget.kwant_id)} className={btnQuiet} data-budget-raise aria-describedby={describedBy}>
            <SlidersHorizontal size={13} aria-hidden="true" />
            <span>Raise the limit</span>
          </button>
        )}
        <button
          type="button"
          onClick={() => {
            if (!done) void run();
          }}
          aria-disabled={state === 'sending' || done ? true : undefined}
          className={btnAmber}
          data-budget-continue
          aria-describedby={describedBy}
        >
          {state === 'sending' ? (
            <Loader2 size={13} className="animate-spin motion-reduce:animate-none" aria-hidden="true" />
          ) : done ? (
            <Check size={13} aria-hidden="true" />
          ) : (
            <Play size={13} aria-hidden="true" />
          )}
          <span>{state === 'sending' ? 'Continuing…' : done ? 'Continued this month' : 'Continue this month'}</span>
        </button>
      </div>
      <p className="sr-only" role="status" aria-live="polite">
        {state === 'done' ? `Continued. ${budget.kwant_name} can start new work again.` : ''}
      </p>
      {error && (
        <p role="alert" className="basis-full text-[11px] text-rose-300">
          {error}
        </p>
      )}
    </>
  );
};

// ---------------------------------------------------------------- the chat

/** After Continue in the bar: the bar goes away, so the person can type straight away (#536 Mira F4). */
const focusComposer = () => document.querySelector<HTMLTextAreaElement>('[data-composer-form] textarea')?.focus();

/** Above the composer in a paused bot's chat: why nothing new starts, and the way on. */
export const BudgetPausedBanner: React.FC<{ budget: BotBudget; onRaise?: (kwantId: string) => void; send?: typeof fetch }> = ({ budget, onRaise, send }) => {
  const id = `budget-banner-${budget.kwant_id}`;
  return (
    <section
      aria-labelledby={id}
      data-budget-banner
      className="max-w-4xl mx-auto mb-2 flex flex-wrap items-center gap-x-3 gap-y-2 rounded-xl border border-amber-500/30 bg-amber-950/30 px-3 py-2"
    >
      <div className="min-w-0 flex-1 basis-56">
        <p id={id} className="flex items-center gap-2 text-xs font-medium text-amber-200">
          <PauseCircle size={14} className="shrink-0 text-amber-400" aria-hidden="true" />
          {PAUSED_LINE}
        </p>
        <p className="mt-0.5 pl-[22px] text-[11px] text-amber-200/70">{usedLine(budget)}. Your message waits and goes through once it can start again.</p>
      </div>
      <PausedActions budget={budget} onRaise={onRaise} onContinued={focusComposer} describedBy={id} send={send} />
    </section>
  );
};

/** The bot's 80% note and its stop notice, in the chat. The buttons follow the bot's state now, not when the line was posted. */
export const BudgetNotice: React.FC<{
  messageId: string;
  text: string;
  meta: BudgetNoticeMeta;
  onRaise?: (kwantId: string) => void;
  canAct?: boolean;
  flash?: boolean;
  send?: typeof fetch;
}> = ({ messageId, text, meta, onRaise, canAct = true, flash, send }) => {
  const { byBot } = useBudgets();
  const live = byBot[meta.kwant_id];
  const headingId = `budget-${messageId}`;
  // Only this month's line still has a way on; an older one stays as a record
  const current = !!live && live.month === meta.month;
  const stopped = meta.kind === 'bot_budget_stopped';
  return (
    <section
      data-message-id={messageId}
      data-budget-notice={meta.kind}
      aria-labelledby={headingId}
      className={`w-full max-w-5xl rounded-xl border border-amber-500/30 bg-amber-950/20 px-3.5 py-2.5 sm:px-4 ${flash ? 'kw-flash' : ''}`}
    >
      <div className="flex flex-wrap items-center gap-x-3 gap-y-2">
        <div className="min-w-0 flex-1 basis-56">
          <p id={headingId} className="flex items-start gap-2 text-sm text-amber-200 break-words">
            {stopped ? (
              <PauseCircle size={15} className="mt-0.5 shrink-0 text-amber-400" aria-hidden="true" />
            ) : (
              <Gauge size={15} className="mt-0.5 shrink-0 text-amber-400" aria-hidden="true" />
            )}
            <span className="min-w-0">{text}</span>
          </p>
          {(current || meta.detail) && <p className="mt-1 pl-[23px] text-[11px] text-amber-200/60 font-mono">{current ? usedLine(live) : meta.detail}</p>}
        </div>
        {canAct && current && stopped && (live.state === 'paused' || live.state === 'continued') && (
          <PausedActions budget={live} onRaise={onRaise} describedBy={headingId} send={send} />
        )}
        {canAct && current && !stopped && meta.kind === 'bot_budget_warn' && live.state === 'warn' && onRaise && (
          <button type="button" onClick={() => onRaise(meta.kwant_id)} className={btnQuiet} data-budget-raise aria-describedby={headingId}>
            <SlidersHorizontal size={13} aria-hidden="true" />
            <span>Change the budget</span>
          </button>
        )}
      </div>
    </section>
  );
};

// ---------------------------------------------------------------- Needs you

/** Inbox › New, at the top: each bot paused at its budget, then each at 80% or more. Nothing when none. */
export const BudgetNeedsYou: React.FC<{ onRaise?: (kwantId: string) => void; send?: typeof fetch }> = ({ onRaise, send }) => {
  const { list } = useBudgets();
  // Continued during this visit: the row stays, done, so focus and the confirmation stay put (#536 Mira F4).
  // It goes on the next visit to the Inbox. A ref, not state: the shared list's update renders synchronously,
  // and a state update would land one render later, dropping the row (and its focused button) in between.
  const continued = useRef<Set<string>>(new Set()).current;
  const items = list
    ? budgetsNeedingYou(list.bots.map((b) => (continued.has(b.kwant_id) && b.state === 'continued' ? { ...b, state: 'paused' as const } : b)))
    : [];
  if (!items.length) return null;
  // A block whose rows are all done drops data-needs-you, so the all-clear line can come back
  const allDone = items.every((b) => continued.has(b.kwant_id));
  return (
    <section aria-labelledby="needs-budget-h" data-needs-budget data-needs-you={allDone ? undefined : ''}>
      <h3 id="needs-budget-h" className="mb-2 flex items-center gap-1.5 text-[11px] font-semibold text-zinc-500 uppercase tracking-wider">
        <Gauge size={12} aria-hidden="true" />
        Needs you
      </h3>
      <ul className="space-y-2">
        {items.map((b) => {
          const id = `needs-budget-${b.kwant_id}`;
          const paused = b.state === 'paused';
          const done = continued.has(b.kwant_id);
          const live = list?.bots.find((x) => x.kwant_id === b.kwant_id) ?? b;
          return (
            <li key={b.kwant_id} data-needs-budget-item={done ? 'continued' : b.state} className="rounded-xl border border-amber-500/30 bg-amber-950/20 px-3.5 py-2.5 sm:px-4">
              <div className="flex flex-wrap items-center gap-x-3 gap-y-2">
                <div className="min-w-0 flex-1 basis-56">
                  {done ? (
                    <p id={id} className="flex items-start gap-2 text-sm text-zinc-300 break-words">
                      <Check size={15} className="mt-0.5 shrink-0 text-emerald-400" aria-hidden="true" />
                      <span className="min-w-0">{`${b.kwant_name} can start new work again.`}</span>
                    </p>
                  ) : (
                    <p id={id} className="flex items-start gap-2 text-sm text-amber-200 break-words">
                      {paused ? (
                        <PauseCircle size={15} className="mt-0.5 shrink-0 text-amber-400" aria-hidden="true" />
                      ) : (
                        <Gauge size={15} className="mt-0.5 shrink-0 text-amber-400" aria-hidden="true" />
                      )}
                      <span className="min-w-0">
                        {paused ? `${b.kwant_name} · ${PAUSED_LINE}` : `${b.kwant_name} has used ${meterPercent(b)}% of its monthly budget.`}
                      </span>
                    </p>
                  )}
                  <div className="mt-1.5 pl-[23px] flex flex-wrap items-center gap-x-2 gap-y-1">
                    <BudgetMeter budget={b} className="max-w-[160px]" />
                    <span className="min-w-0 break-words text-[11px] text-amber-200/60 font-mono">{usedLine(b)}</span>
                  </div>
                </div>
                {paused ? (
                  <PausedActions
                    budget={live}
                    onRaise={onRaise}
                    onContinued={() => continued.add(b.kwant_id)}
                    describedBy={id}
                    send={send}
                  />
                ) : (
                  onRaise && (
                    <button type="button" onClick={() => onRaise(b.kwant_id)} className={btnQuiet} data-budget-raise aria-describedby={id}>
                      <SlidersHorizontal size={13} aria-hidden="true" />
                      <span>Change the budget</span>
                    </button>
                  )
                )}
              </div>
            </li>
          );
        })}
      </ul>
    </section>
  );
};

// ---------------------------------------------------------------- bot settings › Usage

const field = (err: boolean) =>
  `w-full bg-zinc-950 border rounded-lg px-3 py-2 min-h-[40px] sm:min-h-[36px] text-xs text-zinc-100 focus:outline-none ${err ? 'border-rose-500/60 focus:border-rose-400' : 'border-zinc-800 focus:border-sky-500'}`;

/** "Monthly budget" in a bot's settings: two optional limits (off by default), used / limit, when it starts over, and the way on when paused. */
export const BotBudgetSection: React.FC<{ kwantId: string; kwantName: string; send?: typeof fetch }> = ({ kwantId, kwantName, send }) => {
  const api = useRef(botBudgetApi(send)).current;
  const { byBot } = useBudgets();
  const [loaded, setLoaded] = useState<BotBudget | null>(null);
  const [loadError, setLoadError] = useState<string | null>(null);
  const [fields, setFields] = useState<BudgetFields>({ tokens: '', usd: '' });
  const [saving, setSaving] = useState(false);
  const [saveError, setSaveError] = useState<string | null>(null);
  const [saved, setSaved] = useState<string | null>(null);
  const [undoing, setUndoing] = useState(false);
  const first = useRef<HTMLInputElement>(null);
  const section = useRef<HTMLElement>(null);

  const load = React.useCallback(() => {
    setLoadError(null);
    setLoaded(null);
    api.get(kwantId).then(
      (b) => {
        setLoaded(b);
        setFields(fieldsFromBudget(b));
        budgetStore.put(b);
      },
      (e: unknown) => setLoadError(e instanceof Error ? e.message : 'Couldn’t load the budget.'),
    );
  }, [api, kwantId]);
  useEffect(load, [load]);
  // Raise the limit: focus the first field once it is on screen
  useEffect(() => {
    if (loaded && takeBudgetFocus(kwantId)) {
      first.current?.scrollIntoView?.({ block: 'center', behavior: 'auto' });
      first.current?.focus({ preventScroll: true });
    }
  }, [loaded, kwantId]);

  // The shared list may be newer (a Continue elsewhere, a turn that crossed 80%)
  const b = byBot[kwantId] ?? loaded;
  const tErr = budgetTokensError(fields.tokens);
  const uErr = budgetUsdError(fields.usd);
  const body = budgetBody(fields);
  const dirty = !!b && fieldsChanged(fields, b);

  const save = async (e?: React.FormEvent) => {
    e?.preventDefault();
    if (!body || saving) return;
    const prev = b?.state;
    setSaving(true);
    setSaveError(null);
    setSaved(null);
    try {
      const next = await api.set(kwantId, body);
      setLoaded(next);
      setFields(fieldsFromBudget(next));
      budgetStore.put(next);
      setSaved(
        next.state === 'off'
          ? 'Budget off.'
          : next.state === 'paused'
            ? 'Saved. It is still over this limit, so it stays paused.'
            : prev === 'paused'
              ? `Saved. ${kwantName} can start new work again.`
              : 'Saved.',
      );
    } catch (err) {
      setSaveError(err instanceof Error ? err.message : 'Couldn’t save the budget. Try again.');
    } finally {
      setSaving(false);
    }
  };

  const pauseAgain = async () => {
    if (undoing) return;
    setUndoing(true);
    setSaveError(null);
    try {
      const next = await api.pauseAgain(kwantId);
      setLoaded(next);
      budgetStore.put(next);
    } catch (err) {
      setSaveError(err instanceof Error ? err.message : 'Couldn’t save. Try again.');
    } finally {
      setUndoing(false);
    }
  };

  return (
    <section ref={section} aria-labelledby="bot-budget-h" data-bot-budget className="space-y-3 rounded-xl border border-zinc-800 bg-zinc-950/60 p-3">
      <div>
        <h3 id="bot-budget-h" className="flex items-center gap-1.5 text-xs font-semibold text-zinc-200">
          <Gauge size={13} className="text-sky-300" aria-hidden="true" />
          Monthly budget
        </h3>
        <p className="text-[11px] text-zinc-500">
          When {kwantName} reaches either limit, it finishes what it’s doing and starts nothing new until you raise the limit, continue, or the month
          ends. You get a note at 80%. Leave both empty for no budget.
        </p>
      </div>

      {loadError ? (
        <p role="alert" className="flex items-center gap-2 text-[11px] text-rose-300" data-budget-load-error>
          <span className="flex-1">{loadError}</span>
          <button type="button" onClick={load} className="inline-flex items-center gap-1 min-h-[44px] sm:min-h-0 px-2 -mx-2 text-zinc-300 hover:text-zinc-100 underline underline-offset-2">
            <RotateCcw size={11} aria-hidden="true" /> Retry
          </button>
        </p>
      ) : !b ? (
        <p className="flex items-center gap-2 text-[11px] text-zinc-500" role="status" data-budget-loading>
          <Loader2 size={12} className="animate-spin motion-reduce:animate-none" aria-hidden="true" /> Loading the budget…
        </p>
      ) : (
        <>
          <div role="status" aria-live="polite" className="space-y-1.5" data-budget-state={b.state}>
            {hasBudget(b) && <BudgetMeter budget={b} />}
            <p className="font-mono text-[11px] text-zinc-300">{usedLine(b)}</p>
            <p className={`text-[11px] ${b.state === 'paused' || b.state === 'warn' ? 'text-amber-300' : 'text-zinc-400'}`}>{stateLine(b)}</p>
            {b.cost_unreported && b.budget_usd > 0 && (
              <p className="text-[11px] text-zinc-500">This bot’s engine reports no cost, so the dollar limit can’t count its use. Set a token limit too.</p>
            )}
          </div>

          {b.state === 'paused' && (
            <div className="flex flex-wrap items-center gap-2">
              <PausedActions
                budget={b}
                send={send}
                onContinued={() => {
                  // The paused row swaps for Pause again once this renders: focus moves there, not to <body> (#536 Mira F4)
                  requestAnimationFrame(() => section.current?.querySelector<HTMLButtonElement>('[data-budget-pause-again]')?.focus());
                }}
              />
              <span className="text-[11px] text-zinc-500">or raise a limit below.</span>
            </div>
          )}
          {b.state === 'continued' && (
            <div className="flex flex-wrap items-center gap-2">
              <button type="button" onClick={pauseAgain} disabled={undoing} className={btnQuiet} data-budget-pause-again>
                {undoing ? <Loader2 size={13} className="animate-spin motion-reduce:animate-none" aria-hidden="true" /> : <PauseCircle size={13} aria-hidden="true" />}
                <span>Pause at the budget again</span>
              </button>
            </div>
          )}

          <form onSubmit={save} className="space-y-2" aria-label={`${kwantName}’s monthly budget`}>
            <div className="grid gap-3 sm:grid-cols-2">
              <div className="space-y-1 min-w-0">
                <label htmlFor="bot-budget-tokens" className="block text-[11px] font-medium text-zinc-300">
                  Tokens per month
                </label>
                <input
                  ref={first}
                  id="bot-budget-tokens"
                  type="text"
                  inputMode="decimal"
                  autoComplete="off"
                  placeholder="No limit"
                  value={fields.tokens}
                  onChange={(e) => setFields((f) => ({ ...f, tokens: e.target.value }))}
                  aria-invalid={!!tErr}
                  aria-describedby="bot-budget-tokens-hint"
                  className={field(!!tErr)}
                />
                <p id="bot-budget-tokens-hint" className={`text-[11px] ${tErr ? 'text-rose-300' : 'text-zinc-500'}`}>
                  {tErr ?? 'Like 500k or 2M. Cache reads don’t count.'}
                </p>
              </div>
              <div className="space-y-1 min-w-0">
                <label htmlFor="bot-budget-usd" className="block text-[11px] font-medium text-zinc-300">
                  Dollars per month
                </label>
                <input
                  id="bot-budget-usd"
                  type="text"
                  inputMode="decimal"
                  autoComplete="off"
                  placeholder="No limit"
                  value={fields.usd}
                  onChange={(e) => setFields((f) => ({ ...f, usd: e.target.value }))}
                  aria-invalid={!!uErr}
                  aria-describedby="bot-budget-usd-hint"
                  className={field(!!uErr)}
                />
                <p id="bot-budget-usd-hint" className={`text-[11px] ${uErr ? 'text-rose-300' : 'text-zinc-500'}`}>
                  {uErr ?? 'Like 20 or 2.50. Counts only engines that report a cost.'}
                </p>
              </div>
            </div>
            <div className="flex flex-wrap items-center gap-2">
              <button type="submit" disabled={!body || !dirty || saving} className={btnPrimary} data-budget-save>
                {saving ? <Loader2 size={13} className="animate-spin motion-reduce:animate-none" aria-hidden="true" /> : <Check size={13} aria-hidden="true" />}
                <span>{saving ? 'Saving…' : 'Save budget'}</span>
              </button>
              {hasBudget(b) && (
                <button
                  type="button"
                  disabled={saving}
                  className={btnQuiet}
                  data-budget-off
                  onClick={() => {
                    setFields({ tokens: '', usd: '' });
                  }}
                >
                  Clear limits
                </button>
              )}
              <span role="status" aria-live="polite" className="text-[11px] text-emerald-300">
                {saved}
              </span>
            </div>
            {saveError && (
              <p role="alert" className="text-[11px] text-rose-300" data-budget-save-error>
                {saveError}
              </p>
            )}
          </form>
        </>
      )}
    </section>
  );
};

// ---------------------------------------------------------------- Settings › Usage

const STATE_CHIP: Record<BotBudget['state'], { text: string; tone: string }> = {
  off: { text: 'No budget', tone: 'text-zinc-500 border-zinc-800' },
  under: { text: 'On track', tone: 'text-zinc-300 border-zinc-700' },
  warn: { text: '% used', tone: 'text-amber-200 border-amber-500/40 bg-amber-500/10' },
  paused: { text: 'Paused', tone: 'text-amber-100 border-amber-500/60 bg-amber-500/20' },
  continued: { text: 'Continued', tone: 'text-zinc-300 border-zinc-700' },
};

/** Every bot in this space with its budget and this month's use. */
export const BudgetOverview: React.FC<{ open: boolean; onOpenBot?: (kwantId: string) => void }> = ({ open, onOpenBot }) => {
  const { list, error, loaded } = useBudgets();
  useEffect(() => {
    if (open) void budgetStore.refresh();
  }, [open]);
  const withBudget = list?.bots.filter(hasBudget).length ?? 0;
  return (
    <div className="space-y-2" role="group" aria-labelledby="budget-overview-h" data-budget-overview>
      <div>
        <p id="budget-overview-h" className="text-[11px] font-semibold text-zinc-200">
          Monthly budget per bot
        </p>
        <p className="text-[11px] text-zinc-500">The bots in the space you’re in, and their use this month. Set a bot’s budget in its settings, under Usage.</p>
      </div>
      {error && !list ? (
        <p role="alert" className="flex items-center gap-2 text-[11px] text-rose-300">
          <span className="flex-1">{error}</span>
          <button type="button" onClick={() => void budgetStore.refresh()} className="inline-flex items-center gap-1 min-h-[44px] sm:min-h-0 px-2 -mx-2 text-zinc-300 hover:text-zinc-100 underline underline-offset-2">
            <RotateCcw size={11} aria-hidden="true" /> Retry
          </button>
        </p>
      ) : !list || !loaded ? (
        <p className="flex items-center gap-2 text-[11px] text-zinc-500" role="status">
          <Loader2 size={12} className="animate-spin motion-reduce:animate-none" aria-hidden="true" /> Loading budgets…
        </p>
      ) : !list.bots.length ? (
        <p className="text-[11px] text-zinc-500">No bots in this space yet.</p>
      ) : (
        <>
          {withBudget === 0 && <p className="text-[11px] text-zinc-400" data-budget-none>No bot has a budget yet, so none of them will pause.</p>}
          <ul className="divide-y divide-zinc-800/80 rounded-lg border border-zinc-800 bg-zinc-950/60" aria-label="Monthly budget per bot">
            {list.bots.map((b) => {
              const chip = STATE_CHIP[b.state];
              return (
                <li key={b.kwant_id} className="flex flex-wrap lg:flex-nowrap items-center gap-x-3 gap-y-1 px-3 py-2" data-budget-row={b.state}>
                  <span className="min-w-0 flex-1 basis-32 truncate text-[11px] font-medium text-zinc-200">{b.kwant_name || 'Unnamed bot'}</span>
                  <span className={`shrink-0 rounded-full border px-1.5 py-px text-[10px] ${chip.tone}`}>{b.state === 'warn' ? `${meterPercent(b)}% used` : chip.text}</span>
                  <span className="basis-full sm:order-last lg:order-none lg:basis-auto lg:w-80 flex items-center gap-2 min-w-0">
                    {hasBudget(b) && <BudgetMeter budget={b} className="max-w-[72px]" />}
                    <span className="min-w-0 break-words font-mono text-[11px] tabular-nums text-zinc-400">
                      {usedLine(b)}
                    </span>
                  </span>
                  {onOpenBot && (
                    <button
                      type="button"
                      onClick={() => onOpenBot(b.kwant_id)}
                      className="shrink-0 min-h-[44px] sm:min-h-[32px] px-3 sm:px-2 rounded-md text-[11px] text-sky-300 hover:text-sky-200 hover:bg-sky-500/10 focus-visible:outline focus-visible:outline-2 focus-visible:outline-sky-400"
                      aria-label={`${hasBudget(b) ? 'Change' : 'Set'} ${b.kwant_name}’s budget`}
                    >
                      {hasBudget(b) ? 'Change' : 'Set'}
                    </button>
                  )}
                </li>
              );
            })}
          </ul>
        </>
      )}
    </div>
  );
};
