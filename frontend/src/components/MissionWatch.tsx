// 0.9.0 P1: watch a mission (design: release-090/designs/p1-watch-mission.md). The Watch chip on the
// mission card, its setup popover (desktop) or sheet (phone), the report block under the card, the
// watcher's chat message and the amber Inbox row. All copy is from the design; all numbers are the
// report's own rows. Data that doesn't parse never reaches here (missionWatch.ts drops it).
import React, { useEffect, useId, useLayoutEffect, useRef, useState } from 'react';
import { AlertCircle, AlertTriangle, CheckCircle2, ChevronDown, ChevronUp, Eye, Hand, HelpCircle, Info, Loader2, MessageSquare, RotateCcw, ShieldCheck, XCircle, type LucideIcon } from 'lucide-react';
import {
  CHIP_TONES,
  REPORT_SUB,
  SETUP_BODY,
  SETUP_BODY_PHONE,
  VERDICT_TEXT,
  chatParts,
  chipView,
  codeParts,
  costText,
  defaultCandidate,
  failedChatText,
  firstNeedsYou,
  footerText,
  guardText,
  hasReport,
  hm,
  orderClaims,
  relationText,
  reportOpensItself,
  rowTint,
  stickyText,
  summaryChips,
  taskTitle,
  youText,
  type ChipView,
  type MissionWatch,
  type NeedsYouAction,
  type WatchAction,
  type WatchApi,
  type WatchCandidate,
  type WatchClaim,
  type WatchReport,
  type WatchedMission,
  type Verdict,
} from '../missionWatch';
import { inlineCodeClass } from './markdownInline';
import { OwnerAvatar, UndoToast } from './MissionBoardView';

const ring = 'focus-visible:outline focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-[#9d8fff]';
const ghostBtn = `inline-flex items-center justify-center gap-1.5 min-h-[44px] sm:min-h-[32px] px-3 rounded-lg border border-white/10 bg-white/[.03] text-[12.5px] font-medium text-zinc-100 hover:bg-white/[.07] disabled:opacity-50 ${ring}`;
const primaryBtn = `inline-flex items-center justify-center gap-1.5 min-h-[44px] sm:min-h-[32px] px-3.5 rounded-lg bg-[#6f5ce6] text-[12.5px] font-semibold text-white hover:bg-[#7d6bf0] disabled:opacity-50 ${ring}`;
const linkBtn = `text-[12.5px] font-medium text-[#b9adff] hover:text-[#d3ccff] underline-offset-2 hover:underline rounded ${ring}`;

/** "Ran `npm test`" with the code in code spans; text nodes only. */
const CodeText: React.FC<{ text: string }> = ({ text }) => (
  <>
    {codeParts(text).map((p, i) =>
      p.code ? (
        <code key={i} className={inlineCodeClass}>
          {p.text}
        </code>
      ) : (
        <React.Fragment key={i}>{p.text}</React.Fragment>
      ),
    )}
  </>
);

// ---------- Chip ----------

const ChipIcon: React.FC<{ v: ChipView; size: number }> = ({ v, size }) => {
  if (v.icon === 'loader') return <Loader2 size={size} aria-hidden="true" className="shrink-0 motion-safe:animate-spin" data-watch-loader />;
  if (v.icon === 'alert-triangle') return <AlertTriangle size={size} aria-hidden="true" className="shrink-0" />;
  if (v.icon === 'alert-circle') return <AlertCircle size={size} aria-hidden="true" className="shrink-0" />;
  return <Eye size={size} aria-hidden="true" className="shrink-0" />;
};

export const WatchChip = React.forwardRef<
  HTMLButtonElement,
  { view: ChipView; expanded: boolean; phone?: boolean; onClick: () => void; controls?: string }
>(({ view: v, expanded, phone = false, onClick, controls }, ref) => {
  const [lead, rest] = v.avatar ? ['Watching · ', v.avatar.name] : [v.text, null];
  return (
    <button
      ref={ref}
      type="button"
      onClick={onClick}
      disabled={v.disabled}
      title={v.tooltip}
      aria-expanded={v.click === 'retry' || v.click === 'none' ? undefined : expanded}
      aria-controls={controls}
      aria-label={v.label}
      data-watch-chip={v.tone}
      className={`shrink-0 inline-flex max-w-full items-center gap-1.5 rounded-full border ${phone ? 'h-9 px-3 text-[13px]' : 'h-7 px-[11px] text-[12.5px]'} font-medium transition-colors disabled:cursor-not-allowed disabled:opacity-50 ${CHIP_TONES[v.tone]} ${ring}`}
    >
      <ChipIcon v={v} size={14} />
      {/* The label is announced politely when a check finishes; the button keeps its own aria-label. */}
      <span aria-live="polite" className="inline-flex min-w-0 items-center gap-1.5">
        <span className="truncate">{lead}</span>
        {v.avatar && (
          <>
            <OwnerAvatar kwantId={v.avatar.kwant_id} name={v.avatar.name} size={16} />
            <span className="truncate">{rest}</span>
          </>
        )}
      </span>
    </button>
  );
});
WatchChip.displayName = 'WatchChip';

// ---------- Setup popover / sheet ----------

type SetupMode = 'setup' | 'settings' | 'change';

const CandidateRow: React.FC<{ c: WatchCandidate; picked: boolean; phone: boolean; name: string; onPick: () => void }> = ({ c, picked, phone, name, onPick }) => (
  <label
    data-watch-candidate={c.kwant_id}
    className={`flex items-center gap-2.5 rounded-[10px] border px-2.5 ${phone ? 'min-h-[48px] text-[14px]' : 'min-h-[36px] text-[13px]'} ${
      picked ? 'border-[#9d8fff] bg-[#8270f5]/12' : 'border-transparent'
    } ${c.can_run ? 'cursor-pointer hover:bg-white/[.04]' : 'opacity-50 cursor-not-allowed'}`}
  >
    <input
      type="radio"
      name={name}
      value={c.kwant_id}
      checked={picked}
      disabled={!c.can_run}
      onChange={onPick}
      className={`h-4 w-4 shrink-0 accent-[#8270f5] ${ring}`}
    />
    <OwnerAvatar kwantId={c.kwant_id} name={c.name} size={20} />
    <span className="min-w-0 break-words">
      <span className="font-semibold text-zinc-100">{c.name}</span> <span className="text-zinc-500">{relationText(c, phone)}</span>
    </span>
  </label>
);

export const WatchSetup: React.FC<{
  mission: WatchedMission;
  watch: MissionWatch | null;
  candidates: WatchCandidate[] | null;
  loadError?: string | null;
  phone?: boolean;
  busy?: boolean;
  error?: string | null;
  /** Start as "Change who checks" (from the can't-run chip's "Pick another bot"). */
  startMode?: SetupMode;
  timeZone?: string;
  onWatch: (kwantId: string) => void;
  onStop: () => void;
  onCheckNow: () => void;
  onClose: () => void;
  id?: string;
}> = ({ mission, watch, candidates, loadError = null, phone = false, busy = false, error = null, startMode, timeZone, onWatch, onStop, onCheckNow, onClose, id }) => {
  const on = !!watch && !['off', 'no_tasks', 'finished'].includes(watch.state);
  const [mode, setMode] = useState<SetupMode>(startMode ?? (on ? 'settings' : 'setup'));
  const [confirmStop, setConfirmStop] = useState(false);
  const cs = candidates ?? [];
  const initial = (on && cs.find((c) => c.kwant_id === watch?.kwant_id && c.can_run)) || defaultCandidate(cs);
  const [pick, setPick] = useState<string | null>(initial?.kwant_id ?? null);
  useEffect(() => {
    if (pick === null && initial) setPick(initial.kwant_id);
  }, [initial, pick]);
  const picked = cs.find((c) => c.kwant_id === pick) ?? null;
  const box = useRef<HTMLDivElement>(null);
  const titleId = useId();
  const radioName = useId();

  // Focus the selected radio (or the first button) when the panel opens or changes mode.
  useLayoutEffect(() => {
    const el = box.current;
    if (!el) return;
    const target = el.querySelector<HTMLElement>('input[type=radio]:checked') ?? el.querySelector<HTMLElement>('button:not([disabled])');
    target?.focus();
  }, [mode, confirmStop, candidates === null]);

  const onKeyDown = (e: React.KeyboardEvent) => {
    if (e.key === 'Escape') {
      e.stopPropagation();
      if (confirmStop) setConfirmStop(false);
      else onClose();
      return;
    }
    if (e.key !== 'Tab' || !box.current) return;
    const f = [...box.current.querySelectorAll<HTMLElement>('button:not([disabled]), input:not([disabled])')].filter((x) => !(x instanceof HTMLInputElement && x.type === 'radio' && !x.checked));
    if (!f.length) return;
    const first = f[0];
    const last = f[f.length - 1];
    if (e.shiftKey && document.activeElement === first) {
      e.preventDefault();
      last.focus();
    } else if (!e.shiftKey && document.activeElement === last) {
      e.preventDefault();
      first.focus();
    }
  };

  const watcher = watch?.kwant_name ?? picked?.name ?? 'the watcher';
  const shell = phone
    ? 'fixed inset-x-0 bottom-0 z-50 max-h-[85vh] overflow-y-auto rounded-t-[18px] border-t border-white/10 bg-surface-overlay px-4 pt-2 pb-[max(1rem,env(safe-area-inset-bottom))] shadow-2xl'
    : 'absolute right-0 top-full z-40 mt-2 w-[380px] max-w-[calc(100vw-2rem)] rounded-[14px] border border-white/10 bg-surface-overlay p-4 shadow-2xl';

  let body: React.ReactNode;
  if (confirmStop) {
    body = (
      <div className="space-y-3" data-watch-stop-confirm>
        <p className="text-[14px] font-semibold text-zinc-100">Stop watching this mission?</p>
        <p className="text-[13px] text-zinc-300">{watcher} won’t check claims any more. Past reports stay in the mission chat.</p>
        <div className="flex justify-end gap-2">
          <button type="button" className={ghostBtn} onClick={() => setConfirmStop(false)}>
            Cancel
          </button>
          <button type="button" className={primaryBtn} disabled={busy} onClick={onStop} data-watch-stop-yes>
            Stop watching
          </button>
        </div>
      </div>
    );
  } else if (mode === 'settings' && watch) {
    const st = watch.state;
    body = (
      <div className="space-y-3" data-watch-settings={st}>
        <div className="flex items-center gap-2 text-[13px] text-zinc-100">
          <OwnerAvatar kwantId={watch.kwant_id} name={watcher} size={20} />
          <span>
            <span className="font-semibold">{watcher}</span> <span className="text-zinc-500">checks the claims</span>
          </span>
        </div>
        {st === 'cant_run' ? (
          <p className="rounded-[10px] border border-amber-300/30 bg-amber-300/8 px-3 py-2 text-[13px] text-amber-200" data-watch-cant-run>
            {watcher} can’t run: {watch.reason?.text ?? 'it can’t run now'}. Pick another bot or fix {watcher}.
          </p>
        ) : st === 'checking' ? (
          <p className="text-[12.5px] text-zinc-400">Checking since {hm(watch.last_check?.started_at, timeZone) || 'just now'}</p>
        ) : st === 'over_limit' ? (
          <p className="text-[12.5px] text-amber-200">The check didn’t start: this mission is over its limit. Raise or remove the limit first.</p>
        ) : (
          <p className="text-[12.5px] text-zinc-400" data-watch-last>
            {watch.last_check?.finished_at ? `Last checked ${hm(watch.last_check.finished_at, timeZone)}` : 'Not checked yet'}
          </p>
        )}
        {error && (
          <p role="alert" className="text-[12.5px] text-rose-300">
            {error}
          </p>
        )}
        <div className={`flex flex-wrap gap-2 ${phone ? '[&>button]:flex-1' : 'justify-end'}`}>
          {watch.can_check_now && (
            <button type="button" className={ghostBtn} disabled={busy} onClick={onCheckNow} data-watch-check-now>
              {st === 'failed' ? 'Try again' : 'Check again now'}
            </button>
          )}
          <button type="button" className={ghostBtn} disabled={busy} onClick={() => setMode('change')} data-watch-change>
            {st === 'cant_run' ? 'Pick another bot' : 'Change who checks'}
          </button>
          <button type="button" className={ghostBtn} disabled={busy} onClick={() => (phone ? onStop() : setConfirmStop(true))} data-watch-stop>
            Stop watching
          </button>
        </div>
      </div>
    );
  } else {
    const who = picked?.name ?? 'a bot';
    body = (
      <form
        className="space-y-3"
        onSubmit={(e) => {
          e.preventDefault();
          if (picked?.can_run && !busy) onWatch(picked.kwant_id);
        }}
      >
        <p className={`${phone ? 'text-[14px]' : 'text-[13px]'} text-zinc-100`}>{phone ? SETUP_BODY_PHONE : SETUP_BODY}</p>
        <fieldset className="space-y-1">
          <legend className="mb-1 text-[12px] font-semibold text-zinc-300">Who checks</legend>
          {candidates === null ? (
            <p className="text-[12.5px] text-zinc-500" role="status">
              {loadError ?? 'Loading…'}
            </p>
          ) : cs.length === 0 ? (
            <p className="text-[12.5px] text-zinc-500">No bot in this space can check yet.</p>
          ) : (
            cs.map((c) => <CandidateRow key={c.kwant_id} c={c} picked={c.kwant_id === pick} phone={phone} name={radioName} onPick={() => setPick(c.kwant_id)} />)
          )}
        </fieldset>
        <p className="flex gap-2 rounded-[10px] border border-white/[.06] bg-white/[.03] px-3 py-2 text-[12.5px] text-zinc-300">
          <ShieldCheck size={14} aria-hidden="true" className="mt-0.5 shrink-0" />
          <span>{guardText(who)}</span>
        </p>
        {!phone && <p className="text-[12px] text-zinc-500">{costText(who)}</p>}
        {error && (
          <p role="alert" className="text-[12.5px] text-rose-300">
            {error}
          </p>
        )}
        <div className={`flex gap-2 ${phone ? '[&>button]:flex-1' : 'justify-end'}`}>
          <button type="button" className={ghostBtn} onClick={() => (mode === 'change' ? setMode('settings') : onClose())}>
            Cancel
          </button>
          <button type="submit" className={primaryBtn} disabled={busy || !picked?.can_run} data-watch-confirm>
            Watch with {who}
          </button>
        </div>
      </form>
    );
  }

  return (
    <>
      {phone && <div aria-hidden="true" className="fixed inset-0 z-40 bg-black/50" onClick={onClose} data-watch-backdrop />}
      <div ref={box} id={id} role="dialog" aria-modal={phone ? true : undefined} aria-labelledby={titleId} onKeyDown={onKeyDown} className={shell} data-watch-setup={phone ? 'sheet' : 'popover'} data-mission-id={mission.id}>
        {phone && <div aria-hidden="true" className="mx-auto mb-3 h-1 w-10 rounded-full bg-white/15" />}
        <h4 id={titleId} className={`mb-2 flex items-center gap-2 font-[650] text-zinc-100 ${phone ? 'text-[17px]' : 'text-[14px]'}`}>
          <Eye size={16} aria-hidden="true" />
          Watch this mission
        </h4>
        {body}
      </div>
    </>
  );
};

// ---------- Report ----------

const VERDICT_LOOK: Record<Verdict, { icon: LucideIcon; cls: string }> = {
  holds: { icon: CheckCircle2, cls: 'text-teal-300' },
  doesnt_hold: { icon: XCircle, cls: 'text-rose-300' },
  cant_tell: { icon: HelpCircle, cls: 'text-zinc-400' },
};

const VerdictText: React.FC<{ v: Verdict }> = ({ v }) => {
  const L = VERDICT_LOOK[v];
  const Icon = L.icon;
  return (
    <span className={`inline-flex items-center gap-1.5 font-semibold ${L.cls}`} data-verdict={v}>
      <Icon size={14} aria-hidden="true" className="shrink-0" />
      {VERDICT_TEXT[v]}
    </span>
  );
};

const CHIP_LOOK = {
  rose: 'border-rose-300/40 bg-rose-300/8 text-rose-300',
  amber: 'border-amber-300/40 bg-amber-300/8 text-amber-300',
  zinc: 'border-white/10 bg-white/[.03] text-zinc-300',
  teal: 'border-teal-400/35 bg-teal-400/8 text-teal-300',
};
const TINT = { rose: 'bg-rose-300/5', amber: 'bg-amber-400/5' };
const CARD_TINT = { rose: 'border-rose-300/25 bg-rose-300/5', amber: 'border-amber-400/30 bg-amber-400/5' };

const ClaimCard: React.FC<{ c: WatchClaim }> = ({ c }) => {
  const t = rowTint(c);
  return (
    <li className={`rounded-xl border px-3 py-2.5 ${t ? CARD_TINT[t] : 'border-hairline'}`} data-claim={c.task_id} data-tint={t ?? ''}>
      <div className="flex items-start gap-2">
        {c.owner_name && <OwnerAvatar kwantId={c.owner_kwant_id} name={c.owner_name} size={18} />}
        <p className="min-w-0 flex-1 break-words text-[14px] font-semibold text-zinc-100">{c.task_title}</p>
        <span className="shrink-0 text-[13px]">
          <VerdictText v={c.verdict} />
        </span>
      </div>
      <p className="mt-1 break-words text-[13.5px] text-zinc-100">
        {c.owner_name ? `${c.owner_name}: ` : ''}
        {c.claimed}
      </p>
      <p className="mt-0.5 break-words text-[12.5px] text-zinc-400">
        <CodeText text={c.checked} />
      </p>
    </li>
  );
};

const ClaimRow: React.FC<{ c: WatchClaim }> = ({ c }) => {
  const t = rowTint(c);
  return (
    <tr className={`border-t border-hairline align-top ${t ? TINT[t] : ''}`} data-claim={c.task_id} data-tint={t ?? ''}>
      <th scope="row" className="px-3 py-2.5 text-left font-normal">
        <span className="block break-words text-[12.5px] font-semibold text-zinc-100">{c.task_title}</span>
        <span className="mt-0.5 block truncate font-mono text-[11px] text-zinc-500" title={c.task_id}>
          {c.task_id}
        </span>
      </th>
      <td className="px-3 py-2.5">
        <span className="inline-flex min-w-0 items-center gap-1.5 text-[12.5px] text-zinc-200">
          {c.owner_name && <OwnerAvatar kwantId={c.owner_kwant_id} name={c.owner_name} size={16} />}
          <span className="break-words">{c.owner_name ?? 'No owner'}</span>
        </span>
      </td>
      <td className="px-3 py-2.5 break-words text-[12.5px] text-zinc-200">{c.claimed}</td>
      <td className="px-3 py-2.5 break-words text-[12.5px] text-zinc-300">
        <CodeText text={c.checked} />
      </td>
      <td className="px-3 py-2.5 text-[12.5px] whitespace-nowrap">
        <VerdictText v={c.verdict} />
      </td>
    </tr>
  );
};

const noDot = (s: string) => s.replace(/[.\s]+$/, '');

const ActionLine: React.FC<{ r: WatchReport; a: WatchAction; timeZone?: string; onOpenInbox?: (a: NeedsYouAction) => void }> = ({ r, a, timeZone, onOpenInbox }) => {
  const icon = 'mt-[3px] shrink-0';
  const task = (id: string) => <b className="font-semibold text-zinc-100">{taskTitle(r, id)}</b>;
  switch (a.kind) {
    case 'restart':
      return (
        <li className="flex gap-2" data-action="restart" data-applied={String(a.applied)}>
          <RotateCcw size={13} aria-hidden="true" className={`${icon} text-zinc-400`} />
          {a.applied ? (
            <span>
              Restarted {task(a.task_id)} with {a.bot_name}: “{a.reason}”
            </span>
          ) : (
            <span>
              Couldn’t restart {task(a.task_id)} with {a.bot_name}
              {a.why ? `: ${noDot(a.why)}` : ''}.
            </span>
          )}
        </li>
      );
    case 'ask':
      return (
        <li className="flex gap-2" data-action="ask" data-applied={String(a.applied)}>
          <MessageSquare size={13} aria-hidden="true" className={`${icon} text-zinc-400`} />
          <span>
            {a.applied ? 'Asked' : 'Couldn’t ask'} {a.bot_name} for {noDot(a.what)} on {task(a.task_id)}
            {!a.applied && a.why ? `: ${noDot(a.why)}` : ''}.
          </span>
        </li>
      );
    case 'needs_you':
      return (
        <li className="flex gap-2 text-amber-200" data-action="needs_you">
          <Hand size={13} aria-hidden="true" className={`${icon} text-amber-300`} />
          <span>
            <b className="font-semibold text-amber-300">You:</b> {youText(r, a, timeZone)}{' '}
            {onOpenInbox && (
              <button type="button" className={linkBtn} onClick={() => onOpenInbox(a)} data-open-inbox>
                Open it
              </button>
            )}
          </span>
        </li>
      );
    case 'refused':
      return (
        <li className="flex gap-2" data-action="refused">
          <ShieldCheck size={13} aria-hidden="true" className={`${icon} text-rose-300`} />
          <span>
            {r.kwant_name} tried to {noDot(a.what)}. {a.why ? noDot(a.why) : 'Watchers can’t do that'}, so kwant left it for you.
          </span>
        </li>
      );
  }
};

export const WatchReportBlock: React.FC<{
  report: WatchReport;
  phone?: boolean;
  /** Checking again: the last report greyed with "Checking again…" over it. */
  checking?: boolean;
  timeZone?: string;
  onHide?: () => void;
  onOpenChat?: () => void;
  onOpenInbox?: (a: NeedsYouAction) => void;
  id?: string;
}> = ({ report: r, phone = false, checking = false, timeZone, onHide, onOpenChat, onOpenInbox, id }) => {
  const [showHolds, setShowHolds] = useState(false);
  const headingId = useId();
  const k = r.counts;
  if (!k) return null;
  const { shown, folded } = orderClaims(r);
  const rows = showHolds ? [...shown, ...folded] : shown;
  const you = firstNeedsYou(r);
  const watcher = r.kwant_name;
  const holdsLink = folded.length > 0 && !showHolds && (
    <button type="button" className={linkBtn} onClick={() => setShowHolds(true)} data-show-holds={folded.length}>
      Show {folded.length} that hold
    </button>
  );
  return (
    <section
      id={id}
      aria-labelledby={headingId}
      aria-busy={checking || undefined}
      className={`relative ${phone ? '' : 'mt-3.5 border-t border-hairline pt-3.5'}`}
      data-watch-report={r.id}
    >
      {checking && (
        <p role="status" className="absolute inset-x-0 top-10 z-10 mx-auto w-fit rounded-full border border-white/10 bg-surface-overlay px-3 py-1 text-[12.5px] text-zinc-200 shadow" data-checking-again>
          Checking again…
        </p>
      )}
      <div className={checking ? 'opacity-60' : ''}>
        <div className="flex flex-wrap items-center gap-x-2.5 gap-y-1.5">
          <OwnerAvatar kwantId={r.kwant_id} name={watcher} size={22} />
          <h4 id={headingId} className={`font-[650] text-zinc-100 ${phone ? 'text-[16px]' : 'text-[14px]'}`}>
            {watcher} checked {k.claims} claim{k.claims === 1 ? '' : 's'}
          </h4>
          <span className={`text-[12px] text-zinc-500 ${phone ? 'ml-auto' : ''}`}>
            {phone ? hm(r.finished_at, timeZone) : `${hm(r.finished_at, timeZone)} · the mission stopped at ${hm(r.stopped_at, timeZone)}`}
          </span>
          {!phone && (onOpenChat || onHide) && (
            <span className="ml-auto flex gap-2">
              {onOpenChat && (
                <button type="button" className={ghostBtn} onClick={onOpenChat} data-open-chat>
                  <MessageSquare size={14} aria-hidden="true" /> Open in the mission chat
                </button>
              )}
              {onHide && (
                <button type="button" className={ghostBtn} onClick={onHide} data-hide-report>
                  <ChevronDown size={14} aria-hidden="true" /> Hide
                </button>
              )}
            </span>
          )}
        </div>
        {!phone && <p className="mt-1.5 text-[12.5px] text-zinc-400">{REPORT_SUB}</p>}
        <ul className="mt-2.5 flex flex-wrap gap-1.5" aria-label="Summary">
          {summaryChips(k).map((c) => (
            <li key={c.tone} className={`rounded-full border px-2.5 py-0.5 text-[12px] font-medium ${CHIP_LOOK[c.tone]}`} data-summary={c.tone}>
              {c.text}
            </li>
          ))}
        </ul>

        {phone ? (
          <ul className="mt-3 space-y-2.5" aria-label="Claims">
            {rows.map((c) => (
              <ClaimCard key={c.task_id} c={c} />
            ))}
          </ul>
        ) : (
          <div className="mt-3 overflow-x-auto rounded-xl border border-hairline">
            <table className="w-full min-w-[720px] table-fixed border-collapse text-left">
              <caption className="sr-only">Claimed vs checked</caption>
              <colgroup>
                <col className="w-[200px]" />
                <col className="w-[96px]" />
                <col className="w-[170px]" />
                <col />
                <col className="w-[124px]" />
              </colgroup>
              <thead>
                <tr className="text-[11px] font-semibold uppercase tracking-[.04em] text-zinc-500">
                  <th scope="col" className="px-3 py-2">
                    Task
                  </th>
                  <th scope="col" className="px-3 py-2">
                    Who
                  </th>
                  <th scope="col" className="px-3 py-2">
                    Claimed
                  </th>
                  <th scope="col" className="px-3 py-2">
                    What {watcher} checked
                  </th>
                  <th scope="col" className="px-3 py-2">
                    Verdict
                  </th>
                </tr>
              </thead>
              <tbody>
                {rows.map((c) => (
                  <ClaimRow key={c.task_id} c={c} />
                ))}
              </tbody>
            </table>
          </div>
        )}
        {holdsLink && <div className="mt-2.5">{holdsLink}</div>}

        {r.actions.length > 0 && (
          <div className="mt-3">
            <h5 className="mb-1.5 text-[12px] font-semibold text-zinc-300">What {watcher} did</h5>
            <ul className="space-y-1.5 text-[12.5px] text-zinc-200">
              {r.actions.map((a, i) => (
                <ActionLine key={i} r={r} a={a} timeZone={timeZone} onOpenInbox={onOpenInbox} />
              ))}
            </ul>
          </div>
        )}
        <p className="mt-3 flex gap-1.5 text-[11.5px] text-zinc-500">
          <Info size={13} aria-hidden="true" className="mt-px shrink-0" />
          {footerText(watcher)}
        </p>
        {phone && onOpenChat && (
          <button type="button" className={`${ghostBtn} mt-3 w-full`} onClick={onOpenChat} data-open-chat>
            <MessageSquare size={14} aria-hidden="true" /> Open in the mission chat
          </button>
        )}
        {phone && onHide && (
          <button type="button" className={`${ghostBtn} mt-2 w-full`} onClick={onHide} data-hide-report>
            <ChevronUp size={14} aria-hidden="true" /> Hide
          </button>
        )}
      </div>
      {phone && you && onOpenInbox && r.inbox?.open !== false && (
        <div className="sticky bottom-0 z-10 mt-4 flex items-center gap-3 rounded-2xl border border-amber-400/30 bg-[#1d1710] px-3 py-2.5 shadow-lg" data-sticky-you>
          <Hand size={18} aria-hidden="true" className="shrink-0 text-amber-300" />
          <p className="min-w-0 flex-1 break-words text-[14px] text-amber-100">{stickyText(r, you)}</p>
          <button type="button" onClick={() => onOpenInbox(you)} className={`min-h-[44px] shrink-0 rounded-xl bg-amber-400 px-4 text-[14px] font-semibold text-zinc-950 hover:bg-amber-300 ${ring}`}>
            Open
          </button>
        </div>
      )}
    </section>
  );
};

// ---------- On the mission card ----------

/** What the board hands every mission card; without a provider the card has no Watch chip. */
export interface WatchBoard {
  api: WatchApi;
  phone: boolean;
  timeZone?: string;
  /** The server's new watch for a mission, to put on the board's copy. */
  onChanged: (missionId: string, w: MissionWatch) => void;
  onOpenChat?: (m: WatchedMission) => void;
  /** "Open it" on a needs-you line: the task it names. */
  onOpenTask?: (taskId: string) => void;
  /** The mission whose report should open (from "See the claims" in the chat). */
  openReportFor?: string | null;
}
export const WatchBoardContext = React.createContext<WatchBoard | null>(null);

/**
 * The chip, its popover or sheet, and the report block, for one mission card. Who can check and
 * the report itself are fetched only when they're opened; the chip comes from `mission.watch`.
 */
export const MissionWatchControls: React.FC<{
  mission: WatchedMission;
  /** Draws the parts where the card wants them: the chip in the header, the report under the meta line. */
  children: (parts: { chip: React.ReactNode; report: React.ReactNode }) => React.ReactNode;
}> = ({ mission: m, children }) => {
  const board = React.useContext(WatchBoardContext);
  if (!board) return <>{children({ chip: null, report: null })}</>;
  return <Controls mission={m} board={board} children={children} />;
};

const Controls: React.FC<{
  mission: WatchedMission;
  board: WatchBoard;
  children: (parts: { chip: React.ReactNode; report: React.ReactNode }) => React.ReactNode;
}> = ({ mission: m, board, children }) => {
  const { api, phone, timeZone } = board;
  const w = m.watch ?? null;
  const view = chipView(m, timeZone);
  const checkId = w?.last_check?.id ?? null;
  const autoOpen = reportOpensItself(w);
  const askedOpen = board.openReportFor === m.id;
  const [reportOpen, setReportOpen] = useState<boolean>(autoOpen || askedOpen);
  const [report, setReport] = useState<WatchReport | null>(null);
  const [reportError, setReportError] = useState<string | null>(null);
  const [candidates, setCandidates] = useState<WatchCandidate[] | null>(null);
  const [loadError, setLoadError] = useState<string | null>(null);
  const [panel, setPanel] = useState<null | 'setup' | 'change'>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [undo, setUndo] = useState<{ kwant: string } | null>(null);
  const chipRef = useRef<HTMLButtonElement>(null);
  const wrap = useRef<HTMLDivElement>(null);
  const panelId = useId();
  const reportId = useId();

  // A new finished check opens the report when it has something to say.
  const seenCheck = useRef(checkId);
  useEffect(() => {
    if (checkId !== seenCheck.current) {
      seenCheck.current = checkId;
      if (autoOpen) setReportOpen(true);
    }
  }, [checkId, autoOpen]);
  useEffect(() => {
    if (askedOpen) setReportOpen(true);
  }, [askedOpen]);

  // Fetch the report for the last finished check when it's open and we don't have that one.
  const doneCheck = w?.last_check && w.last_check.status !== 'checking' ? w.last_check.id : report?.id ?? null;
  const wantReport = reportOpen && hasReport(w);
  useEffect(() => {
    if (!wantReport || !doneCheck || report?.id === doneCheck) return;
    let live = true;
    setReportError(null);
    api.get(m.id).then(
      (x) => {
        if (!live) return;
        if (x.report) setReport(x.report);
        else setReportError('The report couldn’t be read. Open the mission chat to see the message.');
        if (x.candidates.length) setCandidates(x.candidates);
      },
      (e: Error) => live && setReportError(e.message),
    );
    return () => {
      live = false;
    };
  }, [wantReport, doneCheck, m.id]);

  const close = () => {
    setPanel(null);
    setError(null);
    requestAnimationFrame(() => chipRef.current?.focus());
  };
  const openPanel = (mode: 'setup' | 'change') => {
    setPanel(mode);
    setError(null);
    setLoadError(null);
    api.get(m.id).then(
      (x) => setCandidates(x.candidates),
      (e: Error) => setLoadError(e.message),
    );
  };
  // Desktop popover: a click outside closes it.
  useEffect(() => {
    if (!panel || phone) return;
    const onDown = (e: MouseEvent) => {
      if (wrap.current && !wrap.current.contains(e.target as Node)) setPanel(null);
    };
    document.addEventListener('mousedown', onDown);
    return () => document.removeEventListener('mousedown', onDown);
  }, [panel, phone]);
  // The phone's Undo for "Stop watching" lasts 5 s.
  useEffect(() => {
    if (!undo) return;
    const t = window.setTimeout(() => setUndo(null), 5000);
    return () => window.clearTimeout(t);
  }, [undo]);

  const run = async (f: () => Promise<MissionWatch | null>, after?: () => void) => {
    setBusy(true);
    setError(null);
    try {
      const next = await f();
      if (next) board.onChanged(m.id, next);
      after?.();
    } catch (e) {
      setError((e as Error).message);
    } finally {
      setBusy(false);
    }
  };

  if (!view) return <>{children({ chip: null, report: null })}</>;

  const onChip = () => {
    if (view.click === 'report') setReportOpen((o) => !o);
    else if (view.click === 'retry') void run(() => api.checkNow(m.id));
    else if (view.click === 'setup' || view.click === 'settings') {
      if (panel) close();
      else openPanel('setup');
    }
  };
  const expanded = view.click === 'report' ? reportOpen : !!panel;
  const prevKwant = w?.kwant_id ?? null;

  const chip = (
    <div ref={wrap} className="relative" data-watch-wrap>
      <WatchChip ref={chipRef} view={view} expanded={expanded} phone={phone} onClick={onChip} controls={view.click === 'report' ? reportId : panel ? panelId : undefined} />
      {view.click === 'retry' && error && (
        <p role="alert" className="mt-1 text-[12px] text-rose-300">
          {error}
        </p>
      )}
      {panel && (
        <WatchSetup
          id={panelId}
          mission={m}
          watch={w}
          candidates={candidates}
          loadError={loadError}
          phone={phone}
          busy={busy}
          error={error}
          startMode={panel === 'change' ? 'change' : undefined}
          timeZone={timeZone}
          onClose={close}
          onWatch={(id) => void run(() => api.set(m.id, id), close)}
          onCheckNow={() => void run(() => api.checkNow(m.id), close)}
          onStop={() =>
            void run(
              () => api.stop(m.id),
              () => {
                close();
                setReportOpen(false);
                if (phone && prevKwant) setUndo({ kwant: prevKwant });
              },
            )
          }
        />
      )}
      {undo && (
        <UndoToast
          text="Stopped watching"
          onUndo={() => {
            const k = undo.kwant;
            setUndo(null);
            void run(() => api.set(m.id, k));
          }}
          onClose={() => setUndo(null)}
        />
      )}
    </div>
  );

  const shown = report;
  let reportNode: React.ReactNode = null;
  if (wantReport) {
    reportNode = shown ? (
      <WatchReportBlock
        id={reportId}
        report={shown}
        phone={phone}
        checking={w?.state === 'checking'}
        timeZone={timeZone}
        onHide={() => {
          setReportOpen(false);
          requestAnimationFrame(() => chipRef.current?.focus());
        }}
        onOpenChat={board.onOpenChat ? () => board.onOpenChat!(m) : undefined}
        onOpenInbox={board.onOpenTask ? (a) => board.onOpenTask!(a.task_id) : undefined}
      />
    ) : (
      <p id={reportId} role="status" className={`text-[12.5px] ${reportError ? 'text-rose-300' : 'text-zinc-500'} ${phone ? '' : 'mt-3.5 border-t border-hairline pt-3.5'}`} data-watch-report-loading>
        {reportError ?? 'Loading the report…'}
      </p>
    );
  }
  return <>{children({ chip, report: reportNode })}</>;
};

// ---------- Chat message ----------

/**
 * The watcher's message in the mission chat. A report that doesn't parse shows the message text as
 * the server wrote it.
 */
export const WatchChatMessage: React.FC<{
  report: WatchReport | null;
  failed?: string | null;
  /** The message text as written, shown when the report doesn't parse. */
  fallbackText: string;
  watcherName: string;
  watcherId?: string | null;
  at?: string | null;
  timeZone?: string;
  onSeeClaims?: () => void;
  messageId?: string;
  flash?: boolean;
}> = ({ report: r, failed = null, fallbackText, watcherName, watcherId = null, at = null, timeZone, onSeeClaims, messageId, flash = false }) => {
  const name = r?.kwant_name ?? watcherName;
  const parts = r ? chatParts(r, timeZone) : [];
  const allHold = !!r?.counts && r.counts.dont_hold === 0 && r.counts.cant_tell === 0 && r.counts.needs_you === 0;
  return (
    <div
      data-message-id={messageId}
      data-watch-message={parts.length ? 'report' : failed ? 'failed' : 'text'}
      className={`flex w-full max-w-3xl gap-3 rounded-2xl p-2 ${flash ? 'ring-2 ring-[#9d8fff]/50' : ''}`}
    >
      <OwnerAvatar kwantId={r?.kwant_id ?? watcherId} name={name} size={32} />
      <div className="min-w-0 flex-1">
        <div className="flex flex-wrap items-center gap-2 text-[13px]">
          <span className="font-semibold text-zinc-100">{name}</span>
          <span className="inline-flex items-center gap-1 rounded-full border border-white/10 bg-white/[.03] px-2 py-px text-[11px] text-zinc-300" data-watch-tag>
            <Eye size={11} aria-hidden="true" /> Watch
          </span>
          {at && <span className="text-[12px] text-zinc-500">{hm(at, timeZone)}</span>}
        </div>
        <div className="mt-1.5 rounded-2xl border border-white/[.08] bg-white/[.03] px-4 py-2.5 text-[13.5px] leading-relaxed text-zinc-100 break-words">
          {parts.length ? (
            <>
              {parts.map((p, i) =>
                p.bold ? (
                  <b key={i} className="font-semibold">
                    {p.text}
                  </b>
                ) : (
                  <React.Fragment key={i}>{p.text}</React.Fragment>
                ),
              )}
              {onSeeClaims && !allHold && (
                <>
                  {' '}
                  <button type="button" className={linkBtn} onClick={onSeeClaims} data-see-claims>
                    See the claims
                  </button>
                </>
              )}
            </>
          ) : failed ? (
            failedChatText(failed)
          ) : (
            <span className="whitespace-pre-wrap">{fallbackText}</span>
          )}
        </div>
      </div>
    </div>
  );
};
