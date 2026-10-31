import React, { useCallback, useEffect, useId, useRef, useState } from 'react';
import { AlertTriangle, CalendarClock, Check, ChevronDown, ChevronRight, Eye, Loader2, RefreshCw, Search, ShieldCheck, Sparkles, X } from 'lucide-react';
import {
  AcceptIntent,
  DISCOVERY_SETTING_EVENT,
  DiscoveryError,
  DiscoveryList,
  ScanResult,
  Suggestion,
  acceptEdits,
  approvalText,
  blockText,
  checkLine,
  discoveryApi,
  habitLine,
  leftOutText,
  pastText,
  skipText,
  sourceDay,
  statsLine,
} from '../automationDiscovery';
import { channelLabel, formatWhen, onContinuity } from '../continuity';
import { newIdempotencyKey } from '../missions';
import { scheduleWords } from '../botActions';
import { RoutineForm } from './RoutinesPanel';
import { ModernAvatar } from './ModernAvatar';
import type { Channel, Kwant } from './Sidebar';
import { playSfx } from '../audio/chiptune';

const smallBtn =
  'inline-flex items-center gap-1 px-2 py-1 rounded-md text-[11px] font-medium border transition-colors disabled:opacity-50 focus-visible:outline focus-visible:outline-2 focus-visible:outline-sky-500';
const quietBtn = `${smallBtn} border-zinc-700/70 bg-zinc-800/70 text-zinc-200 hover:bg-zinc-700`;
const primaryBtn = `${smallBtn} border-sky-500/50 bg-sky-600 text-white hover:bg-sky-500`;

/** A card the person just decided: stays as one line so they see what happened. */
type Done = { id: string; text: string };

const errText = (e: unknown, fallback: string) => (e instanceof Error && e.message ? e.message : fallback);

/** Where an offer would post, in words. */
function placeText(s: Suggestion, channels: Channel[]): string {
  if (s.delivery?.thread_id) return 'the thread it came from';
  const id = s.delivery?.channel_id;
  if (!id) return 'direct chat';
  const c = channels.find((ch) => ch.id === id);
  return c ? channelLabel(c.name) : 'a deleted channel';
}

const Disclosure: React.FC<{ label: string; children: React.ReactNode; defaultOpen?: boolean }> = ({ label, children, defaultOpen = false }) => {
  const [open, setOpen] = useState(defaultOpen);
  const id = useId();
  return (
    <div>
      <button
        type="button"
        onClick={() => setOpen((v) => !v)}
        aria-expanded={open}
        aria-controls={id}
        className="inline-flex items-center gap-1 text-[11px] font-medium text-zinc-300 hover:text-zinc-100 rounded focus-visible:outline focus-visible:outline-2 focus-visible:outline-sky-500"
      >
        {open ? <ChevronDown size={11} aria-hidden="true" /> : <ChevronRight size={11} aria-hidden="true" />}
        {label}
      </button>
      {open && (
        <div id={id} className="mt-1 pl-4">
          {children}
        </div>
      )}
    </div>
  );
};

/** Checks the routine keeps: each can only be unticked (dropped); its masked spec is shown, never edited. */
const KeepChecks: React.FC<{ suggestion: Suggestion; keep: boolean[]; onChange: (keep: boolean[]) => void }> = ({ suggestion, keep, onChange }) => {
  if (suggestion.checks.length === 0) return null;
  return (
    <fieldset className="space-y-1">
      <legend className="block text-xs font-medium text-zinc-400 mb-1">Checks after each run</legend>
      {suggestion.checks.map((c, i) => (
        <label key={i} className="flex items-start gap-2 min-h-[28px] text-[11px] text-zinc-300 cursor-pointer">
          <input
            type="checkbox"
            checked={keep[i] ?? true}
            onChange={(e) => onChange(keep.map((k, j) => (j === i ? e.target.checked : k)))}
            className="mt-0.5 accent-sky-500"
          />
          <span className="min-w-0 [overflow-wrap:anywhere]">
            {checkLine(c)}
            {!c.required && <span className="text-zinc-500"> (optional)</span>}
          </span>
        </label>
      ))}
      <p className="text-[11px] text-zinc-500">Untick a check to leave it out. Checks can’t be changed here; edit the routine later for that.</p>
    </fieldset>
  );
};

export const SuggestionCard: React.FC<{
  suggestion: Suggestion;
  kwants: Kwant[];
  channels: Channel[];
  /** Accepted or dismissed: the line to show in its place. */
  onDone: (done: Done, accepted: boolean) => void;
  /** Someone else already decided it (409) or it's gone (404): reload and say so. */
  onStale: (message: string, accepted: boolean) => void;
  /** Tests: a fixed key maker. */
  makeKey?: () => string;
}> = ({ suggestion: s, kwants, channels, onDone, onStale, makeKey = newIdempotencyKey }) => {
  const [editing, setEditing] = useState(false);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [keep, setKeep] = useState<boolean[]>(() => s.checks.map(() => true));
  const intent = useRef(new AcceptIntent(makeKey)).current;
  const titleId = useId();
  const kwant = kwants.find((k) => k.id === s.kwant_id);
  const botName = kwant?.name ?? 'a deleted bot';
  const p = s.permissions;
  const e = s.evidence;

  const stale = (err: unknown): boolean => {
    if (!(err instanceof DiscoveryError) || (err.status !== 409 && err.status !== 404)) return false;
    onStale(err.message, err.current?.status === 'accepted');
    return true;
  };

  const accept = async (body: Parameters<NonNullable<React.ComponentProps<typeof RoutineForm>['onSubmit']>>[0], placeChanged: boolean) => {
    const edits = acceptEdits(s, body, placeChanged, keep);
    try {
      const res = await discoveryApi.accept(s.id, intent.current(), edits);
      const when = res.first_run_text ?? s.first_run_text;
      onDone({ id: s.id, text: `Added “${res.routine?.name ?? body.name}” for ${botName}.${when ? ` First run: ${when}.` : ''}` }, true);
    } catch (err) {
      intent.settle(err);
      if (stale(err)) return;
      throw err;
    }
  };

  const dismiss = async () => {
    setBusy(true);
    setError(null);
    playSfx('click');
    try {
      const res = await discoveryApi.dismiss(s.id);
      const until = res.suggestion?.cooldown_until;
      onDone(
        {
          id: s.id,
          text: `Dismissed “${s.name}”. It won’t be suggested again for 30 days${until ? ` (until ${new Date(until).toLocaleDateString(undefined, { day: 'numeric', month: 'short' })})` : ''}.`,
        },
        false,
      );
    } catch (err) {
      if (!stale(err)) setError(errText(err, 'Dismissing failed.'));
      setBusy(false);
    }
  };

  if (editing)
    return (
      <li>
        <RoutineForm
          seed={{
            kwant_id: s.kwant_id,
            channel_id: s.delivery?.channel_id ?? null,
            thread_id: s.delivery?.thread_id ?? null,
            name: s.name,
            instructions: s.instructions,
            schedule: s.schedule,
          }}
          kwants={kwants}
          channels={channels}
          label={`Accept ${s.name}`}
          submitLabel="Add routine"
          hideEnabled
          onSubmit={accept}
          onCancel={() => setEditing(false)}
          onSaved={() => undefined}
        >
          <KeepChecks suggestion={s} keep={keep} onChange={setKeep} />
        </RoutineForm>
      </li>
    );

  return (
    <li aria-labelledby={titleId} className="rounded-xl border border-violet-500/25 bg-zinc-950/60 p-3 space-y-2.5" data-testid="suggestion-card">
      <div className="flex items-start gap-2.5">
        {kwant ? (
          <ModernAvatar role={kwant.avatar_sprite} name={kwant.name} size={28} kwantId={kwant.id} />
        ) : (
          <span className="w-7 h-7 rounded-full bg-zinc-800 shrink-0" aria-hidden="true" />
        )}
        <div className="flex-1 min-w-0">
          <h4 id={titleId} className="text-xs font-medium text-zinc-100 break-words">
            {s.name}
          </h4>
          <p className="text-[11px] text-zinc-400 break-words">
            {botName} · {placeText(s, channels)} · <span title={s.schedule_text || undefined}>{scheduleWords(s.schedule, s.schedule_text)}</span>
          </p>
          {s.first_run_text && (
            <p className="text-[11px] text-zinc-400">
              First run: <span className="text-zinc-200">{s.first_run_text}</span>
            </p>
          )}
        </div>
      </div>

      <div className="pl-[38px] space-y-2 text-[11px] text-zinc-400">
        <div>
          <p className="text-zinc-300">{habitLine(e)}</p>
          {e && e.settled > 0 && (
            <p>
              {e.verified} of {e.settled} finished {e.settled === 1 ? 'task' : 'tasks'} passed {e.verified === 1 && e.settled === 1 ? 'its' : 'their'} checks.
            </p>
          )}
        </div>

        {s.sources.length > 0 && (
          <Disclosure label={`Why: the ${s.sources.length === 1 ? 'task' : `${s.sources.length} tasks`} it learned from`}>
            <ul className="grid grid-cols-[auto_minmax(0,1fr)] gap-x-3 gap-y-0.5" aria-label="Tasks it learned from">
              {s.sources.map((src) => (
                <li key={src.task_id} className="col-span-2 grid grid-cols-subgrid [overflow-wrap:anywhere]">
                  <span className="text-zinc-500 tabular-nums whitespace-nowrap">
                    {sourceDay(src.local_date)} {src.local_time}
                  </span>
                  <span className="text-zinc-300">{src.title}</span>
                </li>
              ))}
            </ul>
          </Disclosure>
        )}

        <Disclosure label="What it would do">
          <p className="whitespace-pre-wrap text-zinc-300 [overflow-wrap:anywhere]">{s.instructions}</p>
          {s.acceptance && <p className="mt-1">Done when: <span className="text-zinc-300">{s.acceptance}</span></p>}
          {s.chain.filter((c) => !c.in_instructions).length > 0 && (
            <p className="mt-1">
              Mentions, but leaves to other bots: {s.chain.filter((c) => !c.in_instructions).map((c) => `“${c.title}”`).join(', ')}.
            </p>
          )}
        </Disclosure>

        {p && (
          <div className="flex items-start gap-1.5" data-testid="permissions-preview">
            <ShieldCheck size={12} className="mt-0.5 shrink-0 text-emerald-300/80" aria-hidden="true" />
            <p>
              <span className="text-zinc-300">{p.flight_school ? p.tier_label : 'Works as this bot does today'}.</span> {approvalText(p.approval_mode)}. Payments always ask you.{' '}
              {pastText(p.past)}
            </p>
          </div>
        )}

        {(s.checks.length > 0 || s.left_out_checks.length > 0) && (
          <div>
            {s.checks.length > 0 ? (
              <>
                <p className="text-zinc-300">Checks it keeps after each run:</p>
                <ul className="list-disc pl-4 space-y-0.5" aria-label="Checks it keeps">
                  {s.checks.map((c, i) => (
                    <li key={i} className="[overflow-wrap:anywhere]">
                      {checkLine(c)}
                      {!c.required && <span className="text-zinc-500"> (optional)</span>}
                    </li>
                  ))}
                </ul>
              </>
            ) : (
              <p>No checks would run after each run.</p>
            )}
            {s.left_out_checks.length > 0 && (
              <>
                <p className="mt-1 text-zinc-300">Left out:</p>
                <ul className="list-disc pl-4 space-y-0.5" aria-label="Checks left out">
                  {s.left_out_checks.map((c, i) => (
                    <li key={i} className="[overflow-wrap:anywhere]">
                      {checkLine(c)} <span className="text-zinc-500">— {leftOutText(c.reason)}</span>
                    </li>
                  ))}
                </ul>
              </>
            )}
          </div>
        )}
      </div>

      <div className="flex flex-wrap items-center gap-1.5 pl-[38px]">
        <button
          type="button"
          disabled={busy || !kwant}
          onClick={() => {
            playSfx('click');
            setError(null);
            setEditing(true);
          }}
          aria-label={`Accept ${s.name}: review and add it`}
          className={primaryBtn}
        >
          <Check size={11} aria-hidden="true" /> Accept…
        </button>
        <button type="button" disabled={busy} onClick={dismiss} aria-label={`Dismiss ${s.name}`} className={quietBtn}>
          {busy ? <Loader2 size={11} className="animate-spin motion-reduce:animate-none" aria-hidden="true" /> : <X size={11} aria-hidden="true" />} Dismiss
        </button>
        {!kwant && <span className="text-[11px] text-amber-200">Its bot is gone, so it can only be dismissed.</span>}
      </div>
      {error && (
        <p className="pl-[38px] text-[11px] text-rose-400 flex items-center gap-1.5" role="alert">
          <AlertTriangle size={12} aria-hidden="true" /> {error}
        </p>
      )}
    </li>
  );
};

/** What Scan now or Preview found: the offers, and why the other habits weren't suggested. */
const ScanReport: React.FC<{ result: ScanResult; onClose: () => void }> = ({ result, onClose }) => {
  const n = result.offered.length;
  const head = result.preview
    ? n === 0
      ? 'Preview: nothing would be suggested now.'
      : `Preview: ${n === 1 ? '1 routine would' : `${n} routines would`} be suggested. Nothing was saved.`
    : n === 0
      ? 'Looked just now: nothing new to suggest.'
      : `Looked just now: ${n === 1 ? '1 new suggestion' : `${n} new suggestions`}.`;
  return (
    <div className="rounded-lg border border-zinc-800 bg-zinc-950/60 p-3 space-y-2 text-[11px] text-zinc-400" data-testid="scan-report">
      <div className="flex items-start justify-between gap-2">
        <p className="text-zinc-200" role="status">
          {head}
        </p>
        <button type="button" onClick={onClose} aria-label="Close the scan results" className="p-0.5 rounded text-zinc-500 hover:text-zinc-200 focus-visible:outline focus-visible:outline-2 focus-visible:outline-sky-500">
          <X size={12} />
        </button>
      </div>
      {result.preview && n > 0 && (
        <ul className="space-y-1" aria-label="Would be suggested">
          {result.offered.map((o) => (
            <li key={o.cluster_key}>
              <span className="text-zinc-200">{o.name}</span> · {scheduleWords(o.schedule)}
              {o.first_run_text ? ` · first run ${o.first_run_text}` : ''}
              <span className="block text-zinc-500">{habitLine(o.evidence)}</span>
            </li>
          ))}
        </ul>
      )}
      {result.skipped.length > 0 && (
        <Disclosure label={`Not suggested (${result.skipped.length})`}>
          <ul className="space-y-0.5" aria-label="Not suggested">
            {result.skipped.map((k) => (
              <li key={k.cluster_key} className="[overflow-wrap:anywhere]">
                <span className="text-zinc-300">{k.title}</span>
                <span className="text-zinc-500">
                  {' '}
                  ({k.asks === 1 ? 'asked once' : `asked ${k.asks} times`}): {skipText(k.reason)}.
                </span>
              </li>
            ))}
          </ul>
        </Disclosure>
      )}
    </div>
  );
};

/**
 * "Suggested routines" on top of the Routines list (Automation Discovery, docs §20): what kwant
 * noticed you ask for on a rhythm, to accept (with edits) or dismiss. Hidden on servers without it.
 */
export const SuggestedRoutines: React.FC<{
  kwants: Kwant[];
  channels: Channel[];
  /** After an accept: the routine list below should show the new routine. */
  onAccepted: () => void;
  onOpenSettings?: () => void;
}> = ({ kwants, channels, onAccepted, onOpenSettings }) => {
  const [list, setList] = useState<DiscoveryList | null>(null);
  const [unsupported, setUnsupported] = useState(false);
  const [loadError, setLoadError] = useState<string | null>(null);
  const [done, setDone] = useState<Done[]>([]);
  const [notice, setNotice] = useState<string | null>(null);
  const [scanning, setScanning] = useState<'scan' | 'preview' | null>(null);
  const [report, setReport] = useState<ScanResult | null>(null);
  const [scanError, setScanError] = useState<string | null>(null);
  const headingId = useId();
  const doneRef = useRef<HTMLParagraphElement>(null);

  const reload = useCallback(async () => {
    try {
      setList(await discoveryApi.list());
      setLoadError(null);
    } catch (e) {
      // A server without Automation Discovery: show nothing, as before
      if (e instanceof DiscoveryError && (e.status === 404 || e.status === 405)) setUnsupported(true);
      else setLoadError(errText(e, 'Couldn’t load suggestions.'));
    }
  }, []);

  useEffect(() => {
    reload();
    const onSetting = () => reload();
    window.addEventListener(DISCOVERY_SETTING_EVENT, onSetting);
    const off = onContinuity('automation_suggested', () => reload());
    return () => {
      window.removeEventListener(DISCOVERY_SETTING_EVENT, onSetting);
      off();
    };
  }, [reload]);

  // The line that replaced a decided card takes focus, so a keyboard or screen reader user isn't lost
  useEffect(() => {
    if (done.length) doneRef.current?.focus();
  }, [done.length]);

  const decided = (d: Done, accepted: boolean) => {
    playSfx(accepted ? 'coin' : 'click');
    setNotice(null);
    setDone((prev) => [...prev.filter((x) => x.id !== d.id), d]);
    setList((l) => (l ? { ...l, suggestions: l.suggestions.filter((x) => x.id !== d.id) } : l));
    if (accepted) onAccepted();
    reload();
  };

  const stale = (message: string, accepted: boolean) => {
    setNotice(message);
    if (accepted) onAccepted();
    reload();
  };

  const scan = async (preview: boolean) => {
    setScanning(preview ? 'preview' : 'scan');
    setScanError(null);
    setReport(null);
    try {
      const r = await discoveryApi.scan(preview);
      if (r.blocked_by) setScanError(`${blockText(r.blocked_by).title} ${blockText(r.blocked_by).fix}`);
      else setReport(r);
      if (!preview) reload();
    } catch (e) {
      setScanError(errText(e, 'Looking failed.'));
    } finally {
      setScanning(null);
    }
  };

  if (unsupported) return null;

  const blocked = list?.blocked_by ?? [];
  const offers = list?.suggestions.filter((s) => s.status === 'offered') ?? [];
  const onlyOff = blocked.length === 1 && blocked[0] === 'off';
  const settingsBtn = onOpenSettings && (
    <button type="button" onClick={onOpenSettings} className={quietBtn}>
      Open Settings
    </button>
  );

  return (
    <section aria-labelledby={headingId} className="space-y-2" data-testid="suggested-routines">
      <div className="flex flex-wrap items-center justify-between gap-2">
        <h3 id={headingId} className="flex items-center gap-1.5 text-xs font-semibold text-zinc-200">
          <Sparkles size={13} className="text-violet-300" aria-hidden="true" />
          Suggested routines
          {offers.length > 0 && <span className="px-1.5 rounded-full bg-violet-500/15 text-violet-200 text-[10px] font-medium">{offers.length}</span>}
        </h3>
        {list && blocked.length === 0 && (
          <div className="flex items-center gap-1.5">
            <button type="button" onClick={() => scan(true)} disabled={!!scanning} className={quietBtn} title="See what would be suggested now, without saving anything">
              {scanning === 'preview' ? <Loader2 size={11} className="animate-spin motion-reduce:animate-none" aria-hidden="true" /> : <Eye size={11} aria-hidden="true" />} Preview
            </button>
            <button type="button" onClick={() => scan(false)} disabled={!!scanning} className={quietBtn}>
              {scanning === 'scan' ? <Loader2 size={11} className="animate-spin motion-reduce:animate-none" aria-hidden="true" /> : <Search size={11} aria-hidden="true" />} Scan now
            </button>
          </div>
        )}
      </div>

      {!list && !loadError && (
        <p className="flex items-center gap-2 text-[11px] text-zinc-400" role="status">
          <Loader2 size={12} className="animate-spin motion-reduce:animate-none" aria-hidden="true" /> Checking for suggestions…
        </p>
      )}

      {loadError && (
        <div className="flex items-center justify-between gap-2 bg-rose-500/10 border border-rose-500/20 text-rose-300 px-3 py-2 rounded-lg text-[11px]" role="alert">
          <span>{loadError}</span>
          <button type="button" onClick={reload} className={quietBtn}>
            <RefreshCw size={11} aria-hidden="true" /> Retry
          </button>
        </div>
      )}

      {list && onlyOff && (
        <div className="flex flex-wrap items-center justify-between gap-2 text-[11px] text-zinc-400 border border-dashed border-zinc-800 rounded-lg px-3 py-2" data-testid="discovery-blocked">
          <p>
            You can get a suggested routine when you ask for the same board task on a regular rhythm. {blockText('off').title} {blockText('off').fix}
          </p>
          {settingsBtn}
        </div>
      )}

      {list && blocked.length > 0 && !onlyOff && (
        <div className="text-[11px] text-amber-100 bg-amber-500/10 border border-amber-500/20 rounded-lg px-3 py-2 space-y-1.5" role="status" data-testid="discovery-blocked">
          <p className="font-medium">Suggestions are paused:</p>
          <ul className="space-y-1">
            {blocked.map((b) => (
              <li key={b}>
                {blockText(b).title} <span className="text-amber-200/80">{blockText(b).fix}</span>
              </li>
            ))}
          </ul>
          <div className="flex gap-1.5">
            {settingsBtn}
            <button type="button" onClick={reload} className={quietBtn}>
              <RefreshCw size={11} aria-hidden="true" /> Check again
            </button>
          </div>
        </div>
      )}

      {notice && (
        <p className="text-[11px] text-sky-100 bg-sky-500/10 border border-sky-500/20 rounded-lg px-3 py-2" role="status">
          {notice}
        </p>
      )}
      {scanError && (
        <p className="text-[11px] text-rose-300 flex items-center gap-1.5" role="alert">
          <AlertTriangle size={12} aria-hidden="true" /> {scanError}
        </p>
      )}
      {report && <ScanReport result={report} onClose={() => setReport(null)} />}

      {done.length > 0 && (
        <div className="space-y-1" role="status">
          {done.map((d, i) => (
            <p
              key={d.id}
              ref={i === done.length - 1 ? doneRef : undefined}
              tabIndex={-1}
              className="flex items-start gap-1.5 text-[11px] text-emerald-200 bg-emerald-500/10 border border-emerald-500/20 rounded-lg px-3 py-2 focus:outline-none focus-visible:ring-1 focus-visible:ring-emerald-400/50"
            >
              <Check size={12} className="mt-0.5 shrink-0" aria-hidden="true" /> {d.text}
            </p>
          ))}
        </div>
      )}

      {list && blocked.length === 0 && offers.length === 0 && (
        <div className="text-[11px] text-zinc-500 border border-dashed border-zinc-800 rounded-lg px-3 py-2.5 flex items-start gap-2" data-testid="discovery-empty">
          <CalendarClock size={14} className="mt-0.5 shrink-0 text-zinc-600" aria-hidden="true" />
          <p>
            {list.daily_cap === 0
              ? 'No suggestions. The daily limit in Settings is 0, so nothing new is suggested; Preview shows what it would suggest.'
              : `No suggestions right now. The board is checked every hour for tasks you ask for on a regular rhythm, with at most ${list.daily_cap} new ${list.daily_cap === 1 ? 'suggestion' : 'suggestions'} a day.`}
          </p>
        </div>
      )}

      {offers.length > 0 && (
        <ul className="space-y-2" aria-label="Suggested routines">
          {offers.map((s) => (
            <SuggestionCard key={s.id} suggestion={s} kwants={kwants} channels={channels} onDone={decided} onStale={stale} />
          ))}
        </ul>
      )}

      {list && (list.setting === 'on' || list.stats.offered_30d > 0) && (
        <p className="text-[11px] text-zinc-500" data-testid="discovery-stats">
          {statsLine(list.stats)} {list.last_scan_at ? `Last looked ${formatWhen(list.last_scan_at)}.` : list.setting === 'on' ? 'Hasn’t looked yet.' : ''}
        </p>
      )}
    </section>
  );
};
