import React, { useCallback, useEffect, useId, useRef, useState } from 'react';
import { useUndoableDelete } from '../undoDelete';
import { UndoNotice } from './UndoNotice';
import { AlertTriangle, CalendarClock, ChevronDown, ChevronRight, History, Loader2, Pencil, Play, Plus, RefreshCw, Trash2 } from 'lucide-react';
import {
  Routine,
  RoutineRun,
  RunStatus,
  ScheduleForm,
  WEEKDAYS,
  channelLabel,
  continuityApi,
  defaultTimeZone,
  emptyScheduleForm,
  formFromSchedule,
  formatWhen,
  sortMonthDays,
  onContinuity,
  scheduleFromForm,
  timeZoneOptions,
} from '../continuity';
import { asSchedule, scheduleWords } from '../botActions';
import type { Channel, Kwant } from './Sidebar';
import { playSfx } from '../audio/chiptune';

const smallBtn =
  'inline-flex items-center gap-1 px-2 py-1 rounded-md text-[11px] font-medium border transition-colors disabled:opacity-50 focus-visible:outline focus-visible:outline-2 focus-visible:outline-sky-500';
const quietBtn = `${smallBtn} border-zinc-700/70 bg-zinc-800/70 text-zinc-200 hover:bg-zinc-700`;
const inputClass =
  'w-full bg-zinc-950 border border-zinc-800 rounded-lg px-3 py-2 text-xs text-zinc-100 placeholder-zinc-600 focus:border-sky-500 focus:outline-none transition-colors';
const labelClass = 'block text-xs font-medium text-zinc-400 mb-1';

/** The "Posts in" choice that keeps a routine in the thread it was set up in. */
const IN_THREAD = '__thread__';

const REPEAT: Array<{ id: ScheduleForm['type']; label: string }> = [
  { id: 'once', label: 'Once' },
  { id: 'daily', label: 'Daily' },
  { id: 'weekly', label: 'Weekly' },
  { id: 'monthly', label: 'Monthly' },
  { id: 'interval', label: 'Every…' },
  { id: 'cron', label: 'Custom' },
];
const MONTH_DAYS = Array.from({ length: 31 }, (_, i) => i + 1);

/** Which routine to show when Space settings opens from a card, and why. */
export interface RoutineFocus {
  id: string;
  mode: 'edit' | 'pause';
  /** Changes each time a card asks, so the same routine can be opened twice. */
  nonce: number;
}

const STATUS: Record<RunStatus, { label: string; cls: string }> = {
  running: { label: 'Running', cls: 'bg-sky-500/10 text-sky-300 border-sky-500/20' },
  ok: { label: 'OK', cls: 'bg-emerald-500/10 text-emerald-300 border-emerald-500/20' },
  error: { label: 'Failed', cls: 'bg-rose-500/10 text-rose-300 border-rose-500/20' },
  skipped: { label: 'Skipped', cls: 'bg-zinc-700/30 text-zinc-400 border-zinc-700' },
};

const StatusBadge: React.FC<{ status: RunStatus }> = ({ status }) => (
  <span className={`inline-flex items-center gap-1 px-1.5 py-0.5 rounded text-[10px] font-medium border ${STATUS[status]?.cls ?? STATUS.skipped.cls}`}>
    {status === 'running' && <Loader2 size={9} className="animate-spin motion-reduce:animate-none" aria-hidden="true" />}
    {STATUS[status]?.label ?? status}
  </span>
);

interface FormState {
  kwantId: string;
  channelId: string;
  name: string;
  instructions: string;
  enabled: boolean;
  schedule: ScheduleForm;
}

const RoutineForm: React.FC<{
  initial?: Routine;
  fixedKwantId?: string;
  kwants: Kwant[];
  channels: Channel[];
  onCancel: () => void;
  onSaved: () => void;
}> = ({ initial, fixedKwantId, kwants, channels, onCancel, onSaved }) => {
  const uid = useId();
  const initialChannel = initial?.thread_id ? IN_THREAD : initial?.channel_id ?? '';
  const initialSchedule = initial ? asSchedule(initial.schedule) : null;
  const [f, setF] = useState<FormState>(() => ({
    kwantId: initial?.kwant_id ?? fixedKwantId ?? kwants[0]?.id ?? '',
    channelId: initialChannel,
    name: initial?.name ?? '',
    instructions: initial?.instructions ?? '',
    enabled: initial?.enabled ?? true,
    schedule: initialSchedule ? formFromSchedule(initialSchedule) : emptyScheduleForm(),
  }));
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const s = f.schedule;
  const setS = (patch: Partial<ScheduleForm>) => setF((prev) => ({ ...prev, schedule: { ...prev.schedule, ...patch } }));

  const submit = async (e: React.FormEvent) => {
    e.preventDefault();
    if (!f.name.trim()) return setError('Give the routine a name.');
    if (!f.instructions.trim()) return setError('Say what the bot should do.');
    if (!f.kwantId) return setError('Pick a bot.');
    const built = scheduleFromForm(s);
    if ('error' in built) return setError(built.error);
    setSaving(true);
    setError(null);
    const body = {
      kwant_id: f.kwantId,
      channel_id: f.channelId === IN_THREAD ? null : f.channelId || null,
      name: f.name.trim(),
      instructions: f.instructions.trim(),
      schedule: built.schedule,
      enabled: f.enabled,
    };
    try {
      if (initial) {
        // Sending the place again would move a follow-up out of its thread and forget what it
        // checks back on, so it only goes when the person picked another one.
        const { channel_id, ...rest } = body;
        await continuityApi.updateRoutine(initial.id, f.channelId === initialChannel ? rest : { ...rest, channel_id });
      } else await continuityApi.createRoutine(body);
      playSfx('coin');
      onSaved();
    } catch (err) {
      setError(err instanceof Error ? err.message : 'Saving failed.');
      setSaving(false);
    }
  };

  const id = (k: string) => `${uid}-${k}`;

  return (
    <form onSubmit={submit} noValidate className="p-3 rounded-xl border border-sky-500/30 bg-zinc-950/70 space-y-3" aria-label={initial ? `Edit ${initial.name}` : 'New routine'}>
      <div className="grid grid-cols-1 sm:grid-cols-2 gap-3">
        <div>
          <label htmlFor={id('name')} className={labelClass}>Name</label>
          <input id={id('name')} value={f.name} onChange={(e) => setF({ ...f, name: e.target.value })} placeholder="Morning digest" className={inputClass} autoFocus />
        </div>
        {!fixedKwantId && (
          <div>
            <label htmlFor={id('kwant')} className={labelClass}>Bot</label>
            <select id={id('kwant')} value={f.kwantId} onChange={(e) => setF({ ...f, kwantId: e.target.value })} className={inputClass}>
              {kwants.map((k) => (
                <option key={k.id} value={k.id}>{k.name}</option>
              ))}
            </select>
          </div>
        )}
        <div>
          <label htmlFor={id('channel')} className={labelClass}>Posts in</label>
          <select id={id('channel')} value={f.channelId} onChange={(e) => setF({ ...f, channelId: e.target.value })} className={inputClass}>
            {initial?.thread_id && <option value={IN_THREAD}>The thread it was set up in</option>}
            <option value="">Direct chat with the bot</option>
            {channels.map((c) => (
              <option key={c.id} value={c.id}>{channelLabel(c.name)}</option>
            ))}
          </select>
        </div>
      </div>

      <div>
        <label htmlFor={id('instr')} className={labelClass}>What to do</label>
        <textarea
          id={id('instr')}
          rows={3}
          value={f.instructions}
          onChange={(e) => setF({ ...f, instructions: e.target.value })}
          placeholder="Summarise yesterday's open pull requests and flag anything blocked."
          className={`${inputClass} resize-y`}
        />
      </div>

      <fieldset className="space-y-2">
        <legend className={labelClass}>When</legend>
        <div className="grid grid-cols-3 sm:grid-cols-6 gap-1 p-1 bg-zinc-950 rounded-lg border border-zinc-800" role="radiogroup" aria-label="Repeat">
          {REPEAT.map((t) => (
            <button
              key={t.id}
              type="button"
              role="radio"
              aria-checked={s.type === t.id}
              onClick={() => setS({ type: t.id })}
              className={`py-1.5 sm:py-1 rounded-md text-[11px] font-medium transition-colors focus-visible:outline focus-visible:outline-2 focus-visible:outline-sky-500 ${s.type === t.id ? 'bg-sky-500/15 text-sky-200' : 'text-zinc-500 hover:text-zinc-300'}`}
            >
              {t.label}
            </button>
          ))}
        </div>

        {s.type === 'once' && (
          <div>
            <label htmlFor={id('at')} className="sr-only">Date and time</label>
            <input id={id('at')} type="datetime-local" value={s.onceAt} onChange={(e) => setS({ onceAt: e.target.value })} className={inputClass} />
            <p className="mt-1 text-[11px] text-zinc-500">In your browser's time zone ({defaultTimeZone()}). Turns itself off after it runs.</p>
          </div>
        )}

        {s.type === 'cron' && (
          <div>
            <label htmlFor={id('cron')} className="text-[11px] text-zinc-500">Rule</label>
            <input
              id={id('cron')}
              value={s.cron}
              onChange={(e) => setS({ cron: e.target.value })}
              placeholder="0 9 * * MON#2"
              spellCheck={false}
              autoCapitalize="off"
              aria-describedby={id('cron-hint')}
              className={`${inputClass} font-mono`}
            />
            <p id={id('cron-hint')} className="mt-1 text-[11px] text-zinc-500 leading-relaxed">
              Five parts: minute, hour, day of month, month, weekday. <span className="font-mono text-zinc-400">0 9 * * MON#2</span> is 09:00 on the
              second Monday; <span className="font-mono text-zinc-400">0 18 L * *</span> is 18:00 on the last day of the month.
            </p>
          </div>
        )}

        {(s.type === 'daily' || s.type === 'weekly' || s.type === 'monthly' || s.type === 'cron') && (
          <div className="grid grid-cols-1 sm:grid-cols-2 gap-2">
            {s.type !== 'cron' && (
              <div>
                <label htmlFor={id('time')} className="text-[11px] text-zinc-500">At</label>
                <input id={id('time')} type="time" value={s.time} onChange={(e) => setS({ time: e.target.value })} className={inputClass} />
              </div>
            )}
            <div>
              <label htmlFor={id('tz')} className="text-[11px] text-zinc-500">Time zone</label>
              <input id={id('tz')} list={id('tzs')} value={s.tz} onChange={(e) => setS({ tz: e.target.value })} className={inputClass} spellCheck={false} />
              <datalist id={id('tzs')}>
                {timeZoneOptions(s.tz).map((z) => (
                  <option key={z} value={z} />
                ))}
              </datalist>
            </div>
          </div>
        )}

        {s.type === 'weekly' && (
          <div className="flex flex-wrap gap-1" role="group" aria-label="Days">
            {WEEKDAYS.map((d) => {
              const on = s.weekdays.includes(d.id);
              return (
                <button
                  key={d.id}
                  type="button"
                  aria-pressed={on}
                  onClick={() => setS({ weekdays: on ? s.weekdays.filter((w) => w !== d.id) : [...s.weekdays, d.id] })}
                  className={`px-2 py-1 rounded-md text-[11px] font-medium border transition-colors ${
                    on ? 'border-sky-500/50 bg-sky-500/15 text-sky-200' : 'border-zinc-800 text-zinc-500 hover:text-zinc-300'
                  }`}
                >
                  {d.label}
                </button>
              );
            })}
          </div>
        )}

        {s.type === 'monthly' && (
          <div className="space-y-1">
            <div className="grid grid-cols-7 gap-1" role="group" aria-label="Days of the month">
              {MONTH_DAYS.map((d) => {
                const on = s.monthDays.includes(d);
                return (
                  <button
                    key={d}
                    type="button"
                    aria-pressed={on}
                    onClick={() => setS({ monthDays: on ? s.monthDays.filter((x) => x !== d) : sortMonthDays([...s.monthDays, d]) })}
                    className={`min-h-[32px] rounded-md text-[11px] font-medium border transition-colors focus-visible:outline focus-visible:outline-2 focus-visible:outline-sky-500 ${
                      on ? 'border-sky-500/50 bg-sky-500/15 text-sky-200' : 'border-zinc-800 text-zinc-500 hover:text-zinc-300'
                    }`}
                  >
                    {d}
                  </button>
                );
              })}
              <button
                type="button"
                aria-pressed={s.monthDays.includes('last')}
                onClick={() =>
                  setS({ monthDays: s.monthDays.includes('last') ? s.monthDays.filter((x) => x !== 'last') : sortMonthDays([...s.monthDays, 'last']) })
                }
                className={`col-span-4 min-h-[32px] rounded-md text-[11px] font-medium border transition-colors focus-visible:outline focus-visible:outline-2 focus-visible:outline-sky-500 ${
                  s.monthDays.includes('last') ? 'border-sky-500/50 bg-sky-500/15 text-sky-200' : 'border-zinc-800 text-zinc-500 hover:text-zinc-300'
                }`}
              >
                Last day
              </button>
            </div>
            <p className="text-[11px] text-zinc-500">A day a month doesn't have, like the 31st in April, runs on that month's last day.</p>
          </div>
        )}

        {s.type === 'interval' && (
          <div className="flex items-center gap-2">
            <label htmlFor={id('every')} className="text-[11px] text-zinc-500 shrink-0">Every</label>
            <input
              id={id('every')}
              type="number"
              min={5}
              max={10080}
              step={5}
              value={s.everyMinutes}
              onChange={(e) => setS({ everyMinutes: e.target.value })}
              className={`${inputClass} w-24`}
            />
            <span className="text-[11px] text-zinc-500">minutes (5 to 10080)</span>
          </div>
        )}
      </fieldset>

      <label className="flex items-center gap-2 text-[11px] text-zinc-300">
        <input type="checkbox" checked={f.enabled} onChange={(e) => setF({ ...f, enabled: e.target.checked })} className="accent-sky-500" />
        Enabled
      </label>

      {error && (
        <p className="text-[11px] text-rose-400 flex items-center gap-1.5" role="alert">
          <AlertTriangle size={12} /> {error}
        </p>
      )}
      <div className="flex justify-end gap-2">
        <button type="button" onClick={onCancel} className="px-3 py-1.5 rounded-lg text-xs font-medium text-zinc-300 hover:text-zinc-100 hover:bg-zinc-800 transition-colors">
          Cancel
        </button>
        <button type="submit" disabled={saving} className="px-3.5 py-1.5 rounded-lg text-xs font-medium text-white bg-sky-600 hover:bg-sky-500 transition-colors disabled:opacity-50">
          {saving ? 'Saving…' : initial ? 'Save routine' : 'Create routine'}
        </button>
      </div>
    </form>
  );
};

const RunHistory: React.FC<{ routineId: string; tick: number }> = ({ routineId, tick }) => {
  const [runs, setRuns] = useState<RoutineRun[] | null>(null);
  const [error, setError] = useState<string | null>(null);
  useEffect(() => {
    let cancelled = false;
    continuityApi.routineRuns(routineId).then(
      (r) => !cancelled && (setRuns(r), setError(null)),
      (e) => !cancelled && setError(e instanceof Error ? e.message : 'Could not load runs.'),
    );
    return () => {
      cancelled = true;
    };
  }, [routineId, tick]);
  if (error) return <p className="text-[11px] text-rose-400" role="alert">{error}</p>;
  if (!runs) return <p className="text-[11px] text-zinc-500" role="status">Loading runs…</p>;
  if (runs.length === 0) return <p className="text-[11px] text-zinc-500">No runs yet.</p>;
  return (
    <ol className="space-y-1" aria-label="Run history">
      {runs.map((r) => (
        <li key={r.id} className="flex flex-wrap items-center gap-x-2 gap-y-0.5 text-[11px] text-zinc-400">
          <StatusBadge status={r.status} />
          <span>{formatWhen(r.started_at)}</span>
          <span className="text-zinc-600">{r.trigger === 'manual' ? 'Run now' : 'Scheduled'}</span>
          {r.error && <span className="basis-full min-w-0 text-rose-300/90 [overflow-wrap:anywhere]">{r.error}</span>}
        </li>
      ))}
    </ol>
  );
};

export const RoutineRow: React.FC<{
  routine: Routine;
  kwantName?: string;
  placeLabel: string;
  tick: number;
  /** Opened from a card's Pause: scrolls here, focuses the switch and says how to pause. */
  focusPause?: number;
  onEdit: () => void;
  onChanged: () => void;
  /** Tests: start in the Undo window. */
  initialPending?: boolean;
}> = ({ routine, kwantName, placeLabel, tick, focusPause, onEdit, onChanged, initialPending = false }) => {
  const rowRef = useRef<HTMLLIElement>(null);
  const switchRef = useRef<HTMLInputElement>(null);
  const [highlight, setHighlight] = useState(false);
  useEffect(() => {
    if (!focusPause) return;
    rowRef.current?.scrollIntoView({ block: 'center', behavior: 'smooth' });
    switchRef.current?.focus({ preventScroll: true });
    setHighlight(true);
  }, [focusPause]);
  const [open, setOpen] = useState(false);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [notice, setNotice] = useState<string | null>(null);
  const historyId = useId();
  const removal = useUndoableDelete(onChanged, initialPending);

  const act = async (fn: () => Promise<unknown>) => {
    setBusy(true);
    setError(null);
    try {
      await fn();
      onChanged();
    } catch (e) {
      setError(e instanceof Error ? e.message : 'That failed.');
    } finally {
      setBusy(false);
    }
  };

  const runNow = () =>
    act(async () => {
      const run = await continuityApi.runRoutine(routine.id);
      setNotice(run.status === 'skipped' ? 'Already running; this one was skipped.' : 'Started.');
      setOpen(true);
      playSfx('click');
    });

  if (removal.pending) return <UndoNotice as="li" text={`Deleted “${routine.name}”. It won’t run again.`} onUndo={removal.undo} />;

  return (
    <li ref={rowRef} className={`rounded-xl border bg-zinc-950/60 p-3 space-y-2 ${highlight ? 'border-sky-500/60 ring-1 ring-sky-500/30' : 'border-zinc-800'}`}>
      <div className="flex items-start gap-2.5">
        <label className="relative inline-flex items-center p-1 -m-1 mt-0 cursor-pointer" title={routine.enabled ? 'Turn off' : 'Turn on'}>
          <input
            type="checkbox"
            role="switch"
            ref={switchRef}
            className="sr-only peer"
            checked={routine.enabled}
            disabled={busy}
            aria-label={`${routine.name} enabled`}
            onChange={(e) => act(() => continuityApi.updateRoutine(routine.id, { enabled: e.target.checked }))}
          />
          <span className="w-7 h-4 rounded-full bg-zinc-700 peer-checked:bg-sky-600 peer-focus-visible:outline peer-focus-visible:outline-2 peer-focus-visible:outline-sky-500 transition-colors" />
          <span className="absolute left-1.5 top-1.5 w-3 h-3 rounded-full bg-white transition-transform motion-reduce:transition-none peer-checked:translate-x-3" />
        </label>
        <div className="flex-1 min-w-0">
          <div className="flex flex-wrap items-center gap-x-2 gap-y-1">
            <span className="text-xs font-medium text-zinc-100 break-words">{routine.name}</span>
            {routine.last_status && <StatusBadge status={routine.last_status} />}
          </div>
          <p className="text-[11px] text-zinc-500 break-words">
            {kwantName ? `${kwantName} · ` : ''}
            {placeLabel} · <span title={routine.schedule_text || undefined}>{scheduleWords(routine.schedule, routine.schedule_text)}</span>
          </p>
          <p className="text-[11px] text-zinc-400">
            Next run: <span className="text-zinc-200">{routine.enabled ? formatWhen(routine.next_run_at) : 'off'}</span>
            {routine.last_run_at && <span className="text-zinc-500"> · last {formatWhen(routine.last_run_at)}</span>}
          </p>
        </div>
      </div>
      <div className="flex flex-wrap items-center gap-1.5 pl-9">
        <button type="button" onClick={runNow} disabled={busy} className={`${smallBtn} border-sky-500/40 bg-sky-500/10 text-sky-200 hover:bg-sky-500/20`}>
          <Play size={11} /> Run now
        </button>
        <button type="button" onClick={() => setOpen((v) => !v)} aria-expanded={open} aria-controls={historyId} className={quietBtn}>
          {open ? <ChevronDown size={11} /> : <ChevronRight size={11} />} <History size={11} /> History
        </button>
        <button type="button" onClick={onEdit} className={quietBtn}>
          <Pencil size={11} /> Edit
        </button>
        <button
          type="button"
          disabled={busy}
          onClick={() => removal.start(() => continuityApi.removeRoutine(routine.id))}
          className={`${smallBtn} border-zinc-700/70 bg-zinc-800/70 text-zinc-300 hover:text-rose-200`}
        >
          <Trash2 size={11} /> Delete
        </button>
        {notice && <span className="text-[11px] text-zinc-400" role="status">{notice}</span>}
        {highlight && routine.enabled && !notice && (
          <span className="text-[11px] text-sky-200" role="status">Turn the switch off to pause it.</span>
        )}
      </div>
      {(error ?? removal.error) && <p className="pl-9 text-[11px] text-rose-400" role="alert">{error ?? `Couldn’t delete it: ${removal.error}`}</p>}
      {open && (
        <div id={historyId} className="pl-9 pt-1">
          <RunHistory routineId={routine.id} tick={tick} />
        </div>
      )}
    </li>
  );
};

/** Routines for one kwant (`kwantId`) or for everyone. Live: refetches on `routine_run`. */
export const RoutinesPanel: React.FC<{ kwantId?: string; kwants: Kwant[]; channels: Channel[]; focus?: RoutineFocus | null }> = ({
  kwantId,
  kwants,
  channels,
  focus,
}) => {
  const [routines, setRoutines] = useState<Routine[] | null>(null);
  const [loadError, setLoadError] = useState<string | null>(null);
  const [editing, setEditing] = useState<Routine | 'new' | null>(null);
  const [tick, setTick] = useState(0);

  const reload = useCallback(async () => {
    try {
      const all = await continuityApi.routines();
      setRoutines(kwantId ? all.filter((r) => r.kwant_id === kwantId) : all);
      setLoadError(null);
    } catch (e) {
      setLoadError(e instanceof Error ? e.message : 'Could not load routines.');
    }
  }, [kwantId]);

  useEffect(() => {
    reload();
    return onContinuity('routine_run', () => {
      reload();
      setTick((t) => t + 1);
    });
  }, [reload]);

  // A card's Edit opens that routine's form; Pause points at its switch.
  const [missing, setMissing] = useState(false);
  const handled = useRef<number | null>(null);
  useEffect(() => {
    if (!focus || !routines || handled.current === focus.nonce) return;
    handled.current = focus.nonce;
    const r = routines.find((x) => x.id === focus.id);
    setMissing(!r);
    if (r && focus.mode === 'edit') setEditing(r);
  }, [focus, routines]);

  const kwantName = (id: string) => kwants.find((k) => k.id === id)?.name;
  const place = (r: Routine) => {
    if (r.thread_id) return 'a thread';
    if (!r.channel_id) return 'direct chat';
    const c = channels.find((ch) => ch.id === r.channel_id);
    return c ? channelLabel(c.name) : 'a deleted channel';
  };
  const saved = () => {
    setEditing(null);
    reload();
  };

  return (
    <div className="space-y-3">
      <div className="flex items-center justify-between gap-2">
        <p className="text-[11px] text-zinc-500 leading-relaxed">
          A routine posts its instructions to the bot on a schedule, then the bot answers as usual.
        </p>
        {editing !== 'new' && (
          <button
            type="button"
            onClick={() => setEditing('new')}
            disabled={kwants.length === 0}
            className={`${smallBtn} shrink-0 border-sky-500/50 bg-sky-600 text-white hover:bg-sky-500`}
          >
            <Plus size={11} /> New routine
          </button>
        )}
      </div>

      {editing === 'new' && (
        <RoutineForm fixedKwantId={kwantId} kwants={kwants} channels={channels} onCancel={() => setEditing(null)} onSaved={saved} />
      )}

      {loadError && (
        <div className="flex items-center justify-between gap-2 bg-rose-500/10 border border-rose-500/20 text-rose-300 px-3 py-2 rounded-lg text-xs" role="alert">
          <span>{loadError}</span>
          <button type="button" onClick={reload} className={quietBtn}>
            <RefreshCw size={12} /> Retry
          </button>
        </div>
      )}
      {!routines && !loadError && (
        <p className="flex items-center gap-2 text-xs text-zinc-400" role="status">
          <Loader2 size={14} className="animate-spin motion-reduce:animate-none" /> Loading routines…
        </p>
      )}
      {missing && (
        <p className="text-[11px] text-amber-200 bg-amber-500/10 border border-amber-500/20 rounded-lg px-3 py-2" role="status">
          That routine no longer exists. It may have been undone or deleted.
        </p>
      )}
      {routines && routines.length === 0 && editing !== 'new' && (
        <div className="text-xs text-zinc-500 border border-dashed border-zinc-800 rounded-lg p-4 text-center space-y-1">
          <CalendarClock size={18} className="mx-auto text-zinc-600" aria-hidden="true" />
          <p>No routines yet.</p>
        </div>
      )}
      {routines && routines.length > 0 && (
        <ul className="space-y-2" aria-label="Routines">
          {routines.map((r) =>
            editing !== 'new' && editing?.id === r.id ? (
              <li key={r.id}>
                <RoutineForm initial={r} fixedKwantId={kwantId} kwants={kwants} channels={channels} onCancel={() => setEditing(null)} onSaved={saved} />
              </li>
            ) : (
              <RoutineRow
                key={r.id}
                routine={r}
                kwantName={kwantId ? undefined : kwantName(r.kwant_id) ?? 'deleted bot'}
                placeLabel={place(r)}
                tick={tick}
                focusPause={focus?.mode === 'pause' && focus.id === r.id ? focus.nonce : undefined}
                onEdit={() => setEditing(r)}
                onChanged={reload}
              />
            ),
          )}
        </ul>
      )}
    </div>
  );
};
