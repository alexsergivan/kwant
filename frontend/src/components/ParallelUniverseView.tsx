import React, { useCallback, useEffect, useId, useRef, useState } from 'react';
import { useUndoableDelete } from '../undoDelete';
import { UndoNotice } from './UndoNotice';
import { AlertTriangle, ChevronLeft, ExternalLink, Loader2, Pencil, Play, Plus, RefreshCw, Split, Trash2 } from 'lucide-react';
import { PERSON_SIGNED_IN_EVENT, requestPersonToken } from '../auth';
import { playSfx } from '../audio/chiptune';
import {
  FormErrors,
  INTERRUPTED_TEXT,
  LIMIT_RANGES,
  LimitKey,
  MAX_OPTIONS,
  MAX_TOTAL_MEMORY_MB,
  OptionDraft,
  OptionReport,
  ParallelError,
  Price,
  Report,
  Run,
  Scenario,
  ScenarioBody,
  ScenarioDraft,
  Tone,
  branchStatus,
  draftFrom,
  emptyOption,
  emptyScenario,
  figureKind,
  formatValue,
  leaderLabel,
  metricLabel,
  noteLabel,
  parallelApi,
  reservedMemory,
  runProgress,
  runStatus,
  safeHttps,
  sourceHost,
  validateScenario,
} from '../parallelUniverse';

const TONES: Record<Tone, string> = {
  emerald: 'border-emerald-500/30 bg-emerald-500/10 text-emerald-200',
  amber: 'border-amber-500/30 bg-amber-500/10 text-amber-200',
  violet: 'border-violet-500/30 bg-violet-500/10 text-violet-200',
  rose: 'border-rose-500/30 bg-rose-500/10 text-rose-200',
  sky: 'border-sky-500/30 bg-sky-500/10 text-sky-200',
  zinc: 'border-zinc-700 bg-zinc-800 text-zinc-300',
};

const actionBtn =
  'inline-flex items-center gap-1.5 min-h-[36px] sm:min-h-[30px] px-3 py-1 rounded-lg text-xs font-medium transition-colors disabled:opacity-50 focus-visible:outline focus-visible:outline-2 focus-visible:outline-sky-500';
const quietBtn = `${actionBtn} border border-zinc-700 bg-zinc-800/80 text-zinc-100 hover:bg-zinc-700 hover:border-zinc-600`;
const mainBtn = `${actionBtn} bg-sky-600 text-white hover:bg-sky-500`;
const field =
  'w-full min-h-[36px] px-2.5 py-1.5 bg-zinc-950 border border-zinc-800 rounded-lg text-xs text-zinc-100 placeholder:text-zinc-600 focus:outline-none focus:border-sky-500 aria-[invalid=true]:border-rose-500/60';
const card = 'rounded-xl border border-zinc-800 bg-zinc-900/40 p-3 sm:p-4';

const Badge: React.FC<{ tone: Tone; children: React.ReactNode; title?: string }> = ({ tone, children, title }) => (
  <span title={title} className={`inline-flex items-center shrink-0 px-1.5 py-px rounded border text-[10px] font-semibold uppercase tracking-wide ${TONES[tone]}`}>
    {children}
  </span>
);

/** The Measured / Estimate / Guess label every number carries. */
export const KindBadge: React.FC<{ tag: string }> = ({ tag }) => {
  const k = figureKind(tag);
  return (
    <Badge tone={k.tone} title={k.hint}>
      {k.label}
    </Badge>
  );
};

/** Calm notices: amber for things to know, emerald for done. */
const Notice: React.FC<{ text: string; error?: boolean }> = ({ text, error = false }) => (
  <p className={`px-3 py-2 rounded-lg border text-xs ${error ? 'border-amber-500/30 bg-amber-500/10 text-amber-200' : 'border-emerald-500/30 bg-emerald-500/10 text-emerald-200'}`}>{text}</p>
);

// ---------- Scenario list ----------

export const ScenarioList: React.FC<{
  scenarios: Scenario[];
  onOpen: (s: Scenario) => void;
  onNew: () => void;
}> = ({ scenarios, onOpen, onNew }) =>
  scenarios.length === 0 ? (
    <div className="py-12 flex flex-col items-center text-center gap-2" data-empty>
      <Split size={28} className="text-sky-400" aria-hidden="true" />
      <p className="text-sm font-medium text-zinc-200">No scenarios yet</p>
      <p className="text-xs text-zinc-500 max-w-xs">
        A scenario is one question, like “VPS or serverless?”, with up to {MAX_OPTIONS} ways to run your app. Each one is tried here and compared.
      </p>
      <button type="button" onClick={onNew} className={`${mainBtn} mt-2`}>
        <Plus size={13} aria-hidden="true" /> New scenario
      </button>
    </div>
  ) : (
    <div className="space-y-3">
      <div className="flex items-center justify-between gap-2">
        <h3 className="text-xs font-semibold text-zinc-300">Scenarios</h3>
        <button type="button" onClick={onNew} className={quietBtn}>
          <Plus size={13} aria-hidden="true" /> New scenario
        </button>
      </div>
      <ul className="space-y-2">
        {scenarios.map((s) => (
          <li key={s.id}>
            <button
              type="button"
              onClick={() => onOpen(s)}
              className={`${card} w-full text-left hover:border-zinc-700 hover:bg-zinc-900/70 focus-visible:outline focus-visible:outline-2 focus-visible:outline-sky-500`}
            >
              <span className="block text-sm font-medium text-zinc-100 break-words">{s.title}</span>
              {s.question && <span className="block mt-0.5 text-xs text-zinc-400 break-words line-clamp-2">{s.question}</span>}
              <span className="block mt-1.5 text-[11px] text-zinc-500 break-words">
                {s.options.length} option{s.options.length === 1 ? '' : 's'}: {s.options.map((o) => o.label).join(' · ')}
              </span>
            </button>
          </li>
        ))}
      </ul>
    </div>
  );

// ---------- Form ----------

const Field: React.FC<{ label: string; error?: string; hint?: string; children: (id: string, describedBy: string | undefined) => React.ReactNode; className?: string }> = ({
  label,
  error,
  hint,
  children,
  className = '',
}) => {
  const id = useId();
  const msg = `${id}-msg`;
  return (
    <div className={className}>
      <label htmlFor={id} className="block text-[11px] font-medium text-zinc-400 mb-1">
        {label}
      </label>
      {children(id, error || hint ? msg : undefined)}
      {error ? (
        <p id={msg} className="mt-1 text-[11px] text-rose-300">
          {error}
        </p>
      ) : hint ? (
        <p id={msg} className="mt-1 text-[11px] text-zinc-500">
          {hint}
        </p>
      ) : null}
    </div>
  );
};

const TextInput: React.FC<{
  label: string;
  value: string;
  onChange: (v: string) => void;
  error?: string;
  hint?: string;
  placeholder?: string;
  inputMode?: 'numeric' | 'decimal' | 'text';
  className?: string;
}> = ({ label, value, onChange, error, hint, placeholder, inputMode = 'text', className }) => (
  <Field label={label} error={error} hint={hint} className={className}>
    {(id, describedBy) => (
      <input
        id={id}
        className={field}
        value={value}
        placeholder={placeholder}
        inputMode={inputMode}
        aria-invalid={error ? true : undefined}
        aria-describedby={describedBy}
        onChange={(e) => onChange(e.target.value)}
      />
    )}
  </Field>
);

const LIMIT_UNITS: Record<LimitKey, string> = { cpus: 'CPUs', memory_mb: 'Memory (MB)', pids: 'Processes', timeout_secs: 'Time limit (s)' };

const OptionFields: React.FC<{
  o: OptionDraft;
  i: number;
  errors: FormErrors;
  prices: Price[] | null;
  canRemove: boolean;
  onChange: (o: OptionDraft) => void;
  onRemove: () => void;
}> = ({ o, i, errors, prices, canRemove, onChange, onRemove }) => {
  const e = (f: string) => errors[`options.${i}.${f}`];
  const set = (patch: Partial<OptionDraft>) => onChange({ ...o, ...patch });
  const planId = useId();
  return (
    <fieldset className={`${card} space-y-3`}>
      <legend className="sr-only">Option {i + 1}</legend>
      <div className="flex items-center justify-between gap-2">
        <span className="text-xs font-semibold text-zinc-300">Option {i + 1}</span>
        {canRemove && (
          <button type="button" onClick={onRemove} className={`${quietBtn} !min-h-[32px]`} aria-label={`Remove option ${i + 1}`}>
            <Trash2 size={12} aria-hidden="true" /> Remove
          </button>
        )}
      </div>
      <div className="grid grid-cols-1 sm:grid-cols-2 gap-3">
        <TextInput label="Name" value={o.label} onChange={(v) => set({ label: v })} error={e('label')} placeholder="DigitalOcean droplet" />
        <div>
          <label htmlFor={planId} className="block text-[11px] font-medium text-zinc-400 mb-1">
            Price plan
          </label>
          <select
            id={planId}
            className={field}
            value={o.plan}
            aria-invalid={e('plan') ? true : undefined}
            onChange={(ev) => set({ plan: ev.target.value })}
          >
            <option value="">None — cost will be a guess</option>
            {(prices ?? []).map((p) => (
              <option key={p.id} value={p.id}>
                {p.provider} · {p.plan} ({p.monthly > 0 ? `${p.monthly.toFixed(2)} ${p.currency}/mo` : `pay per use, ${p.currency}`})
              </option>
            ))}
            {o.plan && prices && !prices.some((p) => p.id === o.plan) && <option value={o.plan}>{o.plan}</option>}
          </select>
          {e('plan') && <p className="mt-1 text-[11px] text-rose-300">{e('plan')}</p>}
        </div>
      </div>

      <div role="radiogroup" aria-label={`Option ${i + 1}: where the app comes from`} className="flex flex-wrap gap-x-4 gap-y-1 text-xs text-zinc-300">
        {(
          [
            ['image', 'An image on this server'],
            ['context', 'Build from a folder'],
          ] as const
        ).map(([v, l]) => (
          <label key={v} className="inline-flex items-center gap-1.5 min-h-[32px] cursor-pointer">
            <input type="radio" name={`source-${planId}`} checked={o.source === v} onChange={() => set({ source: v })} className="accent-sky-500" />
            {l}
          </label>
        ))}
      </div>
      <div className="grid grid-cols-1 sm:grid-cols-2 gap-3">
        {o.source === 'image' ? (
          <TextInput label="Docker image" value={o.image} onChange={(v) => set({ image: v })} error={e('image')} placeholder="myapp:latest" />
        ) : (
          <>
            <TextInput label="Folder" value={o.context} onChange={(v) => set({ context: v })} error={e('context')} placeholder="/home/me/myapp" />
            <TextInput label="Dockerfile (optional)" value={o.dockerfile} onChange={(v) => set({ dockerfile: v })} error={e('dockerfile')} placeholder="Dockerfile" />
          </>
        )}
        <div className="grid grid-cols-2 gap-3">
          <TextInput label="Port" value={o.port} onChange={(v) => set({ port: v })} error={e('port')} inputMode="numeric" />
          <TextInput label="Health path" value={o.health_path} onChange={(v) => set({ health_path: v })} error={e('health_path')} />
        </div>
      </div>

      <div className="grid grid-cols-2 sm:grid-cols-4 gap-3">
        {(Object.keys(LIMIT_RANGES) as LimitKey[]).map((k) => (
          <TextInput
            key={k}
            label={LIMIT_UNITS[k]}
            value={o[k]}
            onChange={(v) => set({ [k]: v } as Partial<OptionDraft>)}
            error={e(k)}
            hint={e(k) ? undefined : `${LIMIT_RANGES[k].min}–${LIMIT_RANGES[k].max}`}
            inputMode={k === 'cpus' ? 'decimal' : 'numeric'}
          />
        ))}
      </div>

      <details className="group">
        <summary className="cursor-pointer text-[11px] text-zinc-400 hover:text-zinc-200 min-h-[32px] flex items-center">Your notes on running and rolling back (optional)</summary>
        <div className="grid grid-cols-1 sm:grid-cols-2 gap-3 mt-2">
          {(
            [
              ['ops_notes', 'Running it'],
              ['rollback_notes', 'Rolling back'],
            ] as const
          ).map(([k, l]) => (
            <Field key={k} label={l} error={k === 'ops_notes' ? e('ops_notes') : undefined}>
              {(id, d) => <textarea id={id} aria-describedby={d} rows={3} className={`${field} resize-y`} value={o[k]} onChange={(ev) => set({ [k]: ev.target.value } as Partial<OptionDraft>)} />}
            </Field>
          ))}
        </div>
        <p className="mt-1 text-[11px] text-zinc-500">These show in the comparison labelled as a guess. Without them you get a general note for the plan.</p>
      </details>
    </fieldset>
  );
};

export const ScenarioForm: React.FC<{
  initial: ScenarioDraft;
  editing: boolean;
  prices: Price[] | null;
  saving: boolean;
  serverError: string | null;
  /** Tests show the errors without a click. */
  initialShowErrors?: boolean;
  onSave: (body: ScenarioBody) => void;
  onCancel: () => void;
}> = ({ initial, editing, prices, saving, serverError, initialShowErrors = false, onSave, onCancel }) => {
  const [d, setD] = useState<ScenarioDraft>(initial);
  const [showErrors, setShowErrors] = useState(initialShowErrors);
  const planIds = prices ? new Set(prices.map((p) => p.id)) : undefined;
  const { errors, body } = validateScenario(d, planIds);
  const shown = showErrors ? errors : {};
  const total = reservedMemory(d);
  const set = (patch: Partial<ScenarioDraft>) => setD((x) => ({ ...x, ...patch }));
  const errorCount = Object.keys(errors).length;

  const submit = (ev: React.FormEvent) => {
    ev.preventDefault();
    setShowErrors(true);
    if (body) onSave(body);
  };

  return (
    <form onSubmit={submit} noValidate className="space-y-4" aria-label={editing ? 'Edit scenario' : 'New scenario'}>
      <div className="flex items-center justify-between gap-2">
        <h3 className="text-sm font-semibold text-zinc-100">{editing ? 'Edit scenario' : 'New scenario'}</h3>
      </div>

      <div className={`${card} space-y-3`}>
        <TextInput label="Name" value={d.title} onChange={(v) => set({ title: v })} error={shown.title} placeholder="Move booktun.ing off managed hosting?" />
        <Field label="The question (optional)" error={shown.question}>
          {(id, desc) => (
            <textarea
              id={id}
              aria-describedby={desc}
              rows={2}
              className={`${field} resize-y`}
              value={d.question}
              placeholder="Stay where we are, move to a VPS or go serverless?"
              onChange={(e) => set({ question: e.target.value })}
            />
          )}
        </Field>
      </div>

      <fieldset className={`${card} space-y-3`}>
        <legend className="sr-only">Traffic and load test</legend>
        <p className="text-xs font-semibold text-zinc-300">Traffic, for the cost estimate</p>
        <div className="grid grid-cols-2 sm:grid-cols-4 gap-3">
          <TextInput label="Requests a month" value={d.monthly_requests} onChange={(v) => set({ monthly_requests: v })} error={shown.monthly_requests} inputMode="numeric" />
          <TextInput label="Busiest req/s" value={d.peak_rps} onChange={(v) => set({ peak_rps: v })} error={shown.peak_rps} inputMode="decimal" />
          <TextInput label="Response size (KB)" value={d.avg_response_kb} onChange={(v) => set({ avg_response_kb: v })} error={shown.avg_response_kb} inputMode="decimal" />
          <TextInput label="Work per request (ms)" value={d.avg_duration_ms} onChange={(v) => set({ avg_duration_ms: v })} error={shown.avg_duration_ms} inputMode="decimal" />
        </div>
        <p className="text-xs font-semibold text-zinc-300 pt-1">Load test, run here</p>
        <div className="grid grid-cols-2 sm:grid-cols-3 gap-3">
          <TextInput label="Requests" value={d.load_requests} onChange={(v) => set({ load_requests: v })} error={shown.load_requests} inputMode="numeric" />
          <TextInput label="At once" value={d.load_concurrency} onChange={(v) => set({ load_concurrency: v })} error={shown.load_concurrency} inputMode="numeric" />
          <TextInput label="Path" value={d.load_path} onChange={(v) => set({ load_path: v })} error={shown.load_path} className="col-span-2 sm:col-span-1" />
        </div>
      </fieldset>

      <div className="space-y-3">
        <div className="flex flex-wrap items-center justify-between gap-2">
          <p className="text-xs font-semibold text-zinc-300">
            Options <span className="font-normal text-zinc-500">({d.options.length} of {MAX_OPTIONS})</span>
          </p>
          <p className={`text-[11px] ${total > MAX_TOTAL_MEMORY_MB ? 'text-rose-300' : 'text-zinc-500'}`} data-memory-total>
            Memory reserved: {total} of {MAX_TOTAL_MEMORY_MB} MB
          </p>
        </div>
        {shown.memory_total && (
          <p role="alert" className="text-[11px] text-rose-300">
            {shown.memory_total}
          </p>
        )}
        {shown.options && <p className="text-[11px] text-rose-300">{shown.options}</p>}
        {d.options.map((o, i) => (
          <OptionFields
            key={i}
            o={o}
            i={i}
            errors={shown}
            prices={prices}
            canRemove={d.options.length > 1}
            onChange={(next) => set({ options: d.options.map((x, j) => (j === i ? next : x)) })}
            onRemove={() => set({ options: d.options.filter((_, j) => j !== i) })}
          />
        ))}
        {d.options.length < MAX_OPTIONS ? (
          <button type="button" onClick={() => set({ options: [...d.options, emptyOption()] })} className={quietBtn}>
            <Plus size={13} aria-hidden="true" /> Add an option
          </button>
        ) : (
          <p className="text-[11px] text-zinc-500">That’s the most options a scenario can have.</p>
        )}
      </div>

      <div className={`${card} space-y-1`}>
        <label className="flex items-start gap-2 text-xs text-zinc-200 cursor-pointer min-h-[32px]">
          <input type="checkbox" checked={d.build_network} onChange={(e) => set({ build_network: e.target.checked })} className="mt-0.5 accent-sky-500" />
          <span>Let the build step use the internet</span>
        </label>
        <p className={`text-[11px] pl-6 ${d.build_network ? 'text-amber-200' : 'text-zinc-500'}`}>
          Only while building, for apps that download packages. The app itself is always tested offline, and results will say the build had internet.
        </p>
      </div>

      <div role="status" aria-live="polite">
        {serverError && <Notice text={`Couldn’t save: ${serverError}`} error />}
        {showErrors && errorCount > 0 && <Notice text={`Fix ${errorCount === 1 ? 'the highlighted field' : `the ${errorCount} highlighted fields`} to save.`} error />}
      </div>

      <div className="flex flex-wrap gap-2">
        <button type="submit" disabled={saving} className={mainBtn}>
          {saving && <Loader2 size={13} className="animate-spin motion-reduce:animate-none" aria-hidden="true" />}
          {editing ? 'Save changes' : 'Save scenario'}
        </button>
        <button type="button" onClick={onCancel} className={quietBtn}>
          Cancel
        </button>
      </div>
    </form>
  );
};

// ---------- Run and comparison ----------

export const RunProgress: React.FC<{ run: Run }> = ({ run }) => {
  const p = runProgress(run);
  const st = runStatus(run.status);
  return (
    <div className={`${card} space-y-2`} data-run={run.status}>
      <div className="flex flex-wrap items-center gap-2">
        <Badge tone={st.tone}>{st.label}</Badge>
        <span className="text-xs text-zinc-300">{p.text}</span>
        {run.status === 'running' && <Loader2 size={13} className="text-sky-400 animate-spin motion-reduce:animate-none" aria-hidden="true" />}
      </div>
      {run.status === 'running' && (
        <div className="h-1.5 rounded-full bg-zinc-800 overflow-hidden" role="progressbar" aria-label="Run progress" aria-valuemin={0} aria-valuemax={p.total} aria-valuenow={p.done}>
          <div className="h-full bg-sky-500 transition-all" style={{ width: `${p.total ? (p.done / p.total) * 100 : 0}%` }} />
        </div>
      )}
      {run.status === 'interrupted' && (
        <p className="flex items-start gap-1.5 text-xs text-rose-200">
          <AlertTriangle size={13} className="mt-0.5 shrink-0" aria-hidden="true" />
          {INTERRUPTED_TEXT}
        </p>
      )}
      <ul className="space-y-1">
        {run.branches.map((b) => {
          const bs = branchStatus(b.status);
          return (
            <li key={b.key} className="flex flex-wrap items-center gap-x-2 gap-y-0.5 text-xs">
              <Badge tone={bs.tone}>{bs.label}</Badge>
              <span className="text-zinc-200 break-words min-w-0">{b.label}</span>
              {b.status === 'failed' && (b.error || b.error_code) && <span className="text-rose-300 break-words min-w-0 basis-full sm:basis-auto">{b.error || b.error_code}</span>}
            </li>
          );
        })}
      </ul>
      {run.cleanup_errors.length > 0 && <p className="text-[11px] text-amber-200 break-words">Cleanup: {run.cleanup_errors.join('; ')}</p>}
    </div>
  );
};

const FigureRow: React.FC<{ f: OptionReport['measured'][number] }> = ({ f }) => (
  <li className="py-1.5 border-t border-zinc-800/80 first:border-t-0">
    <div className="flex items-center gap-2 text-xs">
      <span className="text-zinc-400 flex-1 min-w-0">{metricLabel(f.metric)}</span>
      <span className="font-mono text-zinc-100 tabular-nums">{formatValue(f.value, f.unit)}</span>
      <KindBadge tag={f.tag} />
    </div>
    {f.tag === 'verified' && (
      <details className="mt-1">
        <summary className="cursor-pointer text-[11px] text-zinc-500 hover:text-zinc-300">How it was measured</summary>
        <code className="block mt-1 p-2 rounded bg-zinc-950 border border-zinc-800 text-[10px] text-zinc-300 whitespace-pre-wrap break-all">{f.command}</code>
      </details>
    )}
  </li>
);

const CostRow: React.FC<{ f: OptionReport['cost'] }> = ({ f }) => (
  <div className="pt-2 space-y-1">
    <div className="flex items-center gap-2 text-xs">
      <span className="text-zinc-400 flex-1 min-w-0">{metricLabel(f.metric)}</span>
      {f.tag === 'speculation' ? <span className="text-zinc-400 italic">unknown</span> : <span className="font-mono text-zinc-100 tabular-nums">about {formatValue(f.value, f.unit)}</span>}
      <KindBadge tag={f.tag} />
    </div>
    {f.tag === 'estimate' && (
      <p className="flex flex-wrap items-center gap-x-1.5 gap-y-0.5 text-[11px] text-zinc-500">
        <span>Prices as of {f.as_of} from</span>
        {safeHttps(f.source_url) ? (
          <a href={f.source_url} target="_blank" rel="noopener noreferrer" className="inline-flex items-center gap-0.5 text-sky-300 hover:text-sky-200 underline underline-offset-2">
            {sourceHost(f.source_url)}
            <ExternalLink size={10} aria-hidden="true" />
            <span className="sr-only">(opens in a new tab)</span>
          </a>
        ) : (
          <span className="break-all">{f.source_url}</span>
        )}
        {f.stale && <Badge tone="rose">Price may be out of date</Badge>}
      </p>
    )}
    {f.tag === 'speculation' && <p className="text-[11px] text-zinc-500">{f.note.charAt(0).toUpperCase() + f.note.slice(1)}.</p>}
  </div>
);

export const OptionCard: React.FC<{ o: OptionReport; leaders: string[] }> = ({ o, leaders }) => {
  const bs = branchStatus(o.status);
  return (
    <li className={`${card} space-y-2 min-w-0`}>
      <div className="flex flex-wrap items-center gap-2">
        <h4 className="text-sm font-medium text-zinc-100 break-words min-w-0">{o.label}</h4>
        {o.status !== 'ok' && <Badge tone={bs.tone}>{bs.label}</Badge>}
        {leaders.map((l) => (
          <Badge key={l} tone="sky">
            {l}
          </Badge>
        ))}
      </div>
      {o.status === 'failed' && <p className="text-xs text-rose-300 break-words">{o.error || o.error_code || 'This option failed.'}</p>}
      {o.build_network_used && <p className="text-[11px] text-amber-200">The build step had internet.</p>}
      {o.measured.length > 0 ? (
        <ul>{o.measured.map((f) => <FigureRow key={f.metric} f={f} />)}</ul>
      ) : (
        <p className="text-xs text-zinc-500">Nothing measured{o.status === 'failed' ? ' — the test didn’t finish.' : '.'}</p>
      )}
      <CostRow f={o.cost} />
      {o.notes.length > 0 && (
        <ul className="pt-2 space-y-1.5">
          {o.notes.map((n, i) => (
            <li key={i} className="text-xs">
              <div className="flex items-center gap-2">
                <span className="text-zinc-400 flex-1">{noteLabel(n.criterion)}</span>
                <KindBadge tag={n.tag} />
              </div>
              <p className="mt-0.5 text-zinc-300 break-words">{n.text}</p>
            </li>
          ))}
        </ul>
      )}
    </li>
  );
};

/** The comparison. Leaders show only when the report gives them; every figure keeps its label. */
export const ComparisonReport: React.FC<{ report: Report }> = ({ report }) => {
  const byOption = (key: string) => report.leaders.filter((l) => l.option === key).map((l) => leaderLabel(l.criterion));
  const label = (key: string) => report.options.find((o) => o.key === key)?.label ?? key;
  return (
    <section aria-label="Comparison" className="space-y-3">
      <h3 className="text-xs font-semibold text-zinc-300">Comparison</h3>
      {report.leaders.length > 0 && (
        <ul className="flex flex-col sm:flex-row sm:flex-wrap gap-2" aria-label="Leaders">
          {report.leaders.map((l) => (
            <li key={l.criterion} className="flex flex-wrap items-center gap-2 px-3 py-2 rounded-lg border border-zinc-800 bg-zinc-900/60 text-xs">
              <span className="text-zinc-400">{leaderLabel(l.criterion)}:</span>
              <span className="text-zinc-100 font-medium break-words">{label(l.option)}</span>
              <span className="font-mono text-zinc-300">{l.criterion === 'cost' ? 'about ' : ''}{formatValue(l.value, l.unit)}</span>
              <KindBadge tag={l.basis} />
            </li>
          ))}
        </ul>
      )}
      <ul className="grid grid-cols-1 md:grid-cols-2 gap-3">
        {report.options.map((o) => (
          <OptionCard key={o.key} o={o} leaders={byOption(o.key)} />
        ))}
      </ul>
      {report.caveats.length > 0 && (
        <div className="rounded-lg border border-amber-500/20 bg-amber-500/5 p-3">
          <p className="text-xs font-medium text-amber-200 mb-1">Keep in mind</p>
          <ul className="list-disc pl-4 space-y-0.5 text-[11px] text-amber-100/80">
            {report.caveats.map((c, i) => (
              <li key={i} className="break-words">
                {c}
              </li>
            ))}
          </ul>
        </div>
      )}
    </section>
  );
};

export const ScenarioDetail: React.FC<{
  scenario: Scenario;
  runs: Run[] | null;
  current: { run: Run; report: Report } | null;
  starting: boolean;
  /** Another run is going (409 busy, or the server says so). */
  busy: boolean;
  notice: { text: string; error: boolean } | null;
  onStart: () => void;
  onPickRun: (id: string) => void;
  onEdit: () => void;
  onDelete: () => void;
}> = ({ scenario, runs, current, starting, busy, notice, onStart, onPickRun, onEdit, onDelete }) => {
  const live = current?.run.status === 'running';
  const runPick = useId();
  return (
    <div className="space-y-4">
      <div className="space-y-1">
        <h3 className="text-base font-semibold text-zinc-100 break-words">{scenario.title}</h3>
        {scenario.question && <p className="text-xs text-zinc-400 break-words">{scenario.question}</p>}
        <p className="text-[11px] text-zinc-500 break-words">
          {scenario.options.map((o) => o.label).join(' · ')}
          {scenario.build_network && ' · build has internet'}
        </p>
      </div>
      <div className="flex flex-wrap gap-2">
        <button type="button" onClick={onStart} disabled={starting || live || busy} className={mainBtn}>
          {starting ? <Loader2 size={13} className="animate-spin motion-reduce:animate-none" aria-hidden="true" /> : <Play size={13} aria-hidden="true" />}
          {live ? 'Running…' : 'Start a run'}
        </button>
        <button type="button" onClick={onEdit} disabled={live} className={quietBtn}>
          <Pencil size={12} aria-hidden="true" /> Edit
        </button>
        <button type="button" onClick={onDelete} disabled={live} className={quietBtn}>
          <Trash2 size={12} aria-hidden="true" /> Delete
        </button>
      </div>
      <p className="text-[11px] text-zinc-500">Each option runs in a sealed-off container on this server, one after another. No cloud account is used and nothing is spent.</p>

      <div role="status" aria-live="polite">
        {busy && !live && <Notice text="Another run is going right now. You can start this one when it finishes." error />}
        {notice && <Notice text={notice.text} error={notice.error} />}
      </div>

      {runs && runs.length > 1 && (
        <div className="flex flex-wrap items-center gap-2">
          <label htmlFor={runPick} className="text-[11px] text-zinc-400">
            Run
          </label>
          <select id={runPick} className={`${field} !w-auto max-w-full`} value={current?.run.id ?? ''} onChange={(e) => onPickRun(e.target.value)}>
            {runs.map((r) => (
              <option key={r.id} value={r.id}>
                {new Date(r.started_at).toLocaleString(undefined, { day: 'numeric', month: 'short', hour: '2-digit', minute: '2-digit' })} · {runStatus(r.status).label}
              </option>
            ))}
          </select>
        </div>
      )}

      {runs && runs.length === 0 && !current && <p className="text-xs text-zinc-500">No runs yet. Start one to measure each option.</p>}

      {current && (
        <>
          <RunProgress run={current.run} />
          {current.report.options.some((o) => o.measured.length > 0 || o.status !== 'pending') && <ComparisonReport report={current.report} />}
        </>
      )}
    </div>
  );
};

// ---------- Page ----------

type Screen = { kind: 'list' } | { kind: 'edit'; scenario: Scenario | null } | { kind: 'scenario'; id: string };

export interface ParallelViewProps {
  screen: Screen;
  scenarios: Scenario[] | null;
  loading: boolean;
  error: string | null;
  needsToken?: boolean;
  body: React.ReactNode;
  onRefresh: () => void;
  onBack?: () => void;
  onList: () => void;
}

/** The frame: header, sign-in and load errors; `body` is the current screen. */
export const ParallelView: React.FC<ParallelViewProps> = ({ screen, scenarios, loading, error, needsToken = false, body, onRefresh, onBack, onList }) => (
  <div className="flex-1 flex flex-col h-full bg-zinc-950 overflow-hidden">
    <div className={`h-14 border-b border-zinc-800/80 ${onBack ? 'px-2' : 'px-4'} sm:px-6 flex items-center justify-between gap-3 bg-zinc-900/30 backdrop-blur-sm select-none`}>
      {onBack && (
        <button
          type="button"
          onClick={onBack}
          aria-label="Back to chats"
          className="shrink-0 p-2 -ml-0.5 -mr-2 rounded-lg text-zinc-300 hover:text-zinc-100 hover:bg-zinc-800 focus-visible:outline focus-visible:outline-2 focus-visible:outline-sky-500"
        >
          <ChevronLeft size={18} />
        </button>
      )}
      <div className="min-w-0 flex-1">
        <h2 className="text-sm font-semibold text-zinc-100 truncate">Parallel Universe</h2>
        <p className="text-[11px] text-zinc-400 truncate mt-0.5">Try ways to run your app side by side</p>
      </div>
      <button
        type="button"
        onClick={onRefresh}
        disabled={loading || needsToken}
        aria-label="Refresh"
        className="inline-flex items-center gap-1.5 px-2.5 py-1 rounded-lg text-xs text-zinc-300 border border-zinc-700/60 hover:bg-zinc-800 disabled:opacity-60 focus-visible:outline focus-visible:outline-2 focus-visible:outline-sky-500"
      >
        <RefreshCw size={12} className={loading ? 'animate-spin motion-reduce:animate-none' : ''} aria-hidden="true" />
        <span className="hidden sm:inline">Refresh</span>
      </button>
    </div>

    <div className="flex-1 overflow-y-auto overflow-x-hidden p-4 sm:p-6" aria-busy={loading}>
      <div className="max-w-3xl mx-auto space-y-5">
        {screen.kind !== 'list' && (
          <button type="button" onClick={onList} className="inline-flex items-center gap-1 -ml-1 min-h-[32px] px-1 rounded text-xs text-zinc-400 hover:text-zinc-200 focus-visible:outline focus-visible:outline-2 focus-visible:outline-sky-500">
            <ChevronLeft size={14} aria-hidden="true" /> All scenarios
          </button>
        )}

        {needsToken && (
          <div role="alert" className="px-3 py-2 rounded-lg border border-amber-500/30 bg-amber-500/10 flex flex-wrap items-center gap-2 text-xs text-amber-100">
            <span>Only you can use Parallel Universe. Confirm it’s you to open it.</span>
            <button type="button" onClick={requestPersonToken} className="ml-auto px-2 py-0.5 rounded border border-amber-500/40 hover:bg-amber-500/10">
              Confirm it’s you
            </button>
          </div>
        )}

        {error && !needsToken && (
          <div role="alert" className="px-3 py-2 rounded-lg border border-rose-500/20 bg-rose-500/5 flex items-center gap-2 text-xs text-rose-300">
            <span>{error}</span>
            <button type="button" onClick={onRefresh} className="ml-auto shrink-0 px-2 py-0.5 rounded border border-rose-500/30 hover:bg-rose-500/10">
              Retry
            </button>
          </div>
        )}

        {!scenarios && loading && (
          <p className="flex items-center gap-2 text-xs text-zinc-400">
            <Loader2 size={14} className="animate-spin motion-reduce:animate-none" aria-hidden="true" /> Loading…
          </p>
        )}

        {scenarios && !needsToken && body}
      </div>
    </div>
  </div>
);

const POLL_MS = 2000;

/** Loads scenarios and prices, saves the form, starts runs and follows them. */
export const ParallelPage: React.FC<{ onBack?: () => void }> = ({ onBack }) => {
  const [screen, setScreen] = useState<Screen>({ kind: 'list' });
  const [scenarios, setScenarios] = useState<Scenario[] | null>(null);
  const [prices, setPrices] = useState<Price[] | null>(null);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [needsToken, setNeedsToken] = useState(false);
  const [saving, setSaving] = useState(false);
  const [saveError, setSaveError] = useState<string | null>(null);
  const [runs, setRuns] = useState<Run[] | null>(null);
  const [current, setCurrent] = useState<{ run: Run; report: Report } | null>(null);
  const [starting, setStarting] = useState(false);
  const [busy, setBusy] = useState(false);
  const [notice, setNotice] = useState<{ text: string; error: boolean } | null>(null);
  // A deleted scenario leaves the list at once; the delete is sent once Undo has had its chance
  const [removing, setRemoving] = useState<Scenario | null>(null);
  const removal = useUndoableDelete(() => {
    setScenarios((list) => (list ?? []).filter((x) => x.id !== removing?.id));
    setRemoving(null);
  });
  const alive = useRef(true);

  const fail = (e: unknown) => {
    if (!alive.current) return;
    if (e instanceof ParallelError && e.status === 401) setNeedsToken(true);
    else setError(e instanceof Error ? e.message : 'Something went wrong.');
  };

  const load = useCallback(async () => {
    setLoading(true);
    try {
      const list = await parallelApi.scenarios();
      if (!alive.current) return;
      setScenarios(list);
      setError(null);
      setNeedsToken(false);
      parallelApi.prices().then((p) => alive.current && setPrices(p.prices), () => {});
    } catch (e) {
      fail(e);
    } finally {
      if (alive.current) setLoading(false);
    }
  }, []);

  useEffect(() => {
    alive.current = true;
    load();
    const again = () => load();
    window.addEventListener(PERSON_SIGNED_IN_EVENT, again);
    return () => {
      alive.current = false;
      window.removeEventListener(PERSON_SIGNED_IN_EVENT, again);
    };
  }, [load]);

  const openRun = useCallback(async (id: string) => {
    try {
      const r = await parallelApi.run(id);
      if (alive.current) setCurrent(r);
    } catch (e) {
      if (alive.current) setNotice({ text: `Couldn’t load that run: ${e instanceof Error ? e.message : 'something went wrong.'}`, error: true });
    }
  }, []);

  const loadRuns = useCallback(
    async (scenarioId: string, pick = true) => {
      try {
        const r = await parallelApi.runs(scenarioId);
        if (!alive.current) return;
        setRuns(r.runs);
        setBusy(r.running && !r.runs.some((x) => x.status === 'running'));
        if (pick && r.runs[0]) openRun(r.runs[0].id);
      } catch (e) {
        if (alive.current) setNotice({ text: `Couldn’t load runs: ${e instanceof Error ? e.message : 'something went wrong.'}`, error: true });
      }
    },
    [openRun],
  );

  const scenarioId = screen.kind === 'scenario' ? screen.id : null;
  useEffect(() => {
    setRuns(null);
    setCurrent(null);
    setNotice(null);
    setBusy(false);
    if (scenarioId) loadRuns(scenarioId);
  }, [scenarioId, loadRuns]);

  // While the shown run is going, look again every couple of seconds.
  const runningId = current?.run.status === 'running' ? current.run.id : null;
  useEffect(() => {
    if (!runningId) return;
    const t = window.setInterval(async () => {
      try {
        const r = await parallelApi.run(runningId);
        if (!alive.current) return;
        setCurrent(r);
        if (r.run.status !== 'running') {
          setNotice(r.run.status === 'finished' ? { text: 'Run finished.', error: false } : { text: 'The run was interrupted.', error: true });
          if (scenarioId) loadRuns(scenarioId, false);
        }
      } catch {
        // the next tick tries again
      }
    }, POLL_MS);
    return () => window.clearInterval(t);
  }, [runningId, scenarioId, loadRuns]);

  const save = async (body: ScenarioBody) => {
    if (screen.kind !== 'edit') return;
    setSaving(true);
    setSaveError(null);
    try {
      const s = screen.scenario ? await parallelApi.update(screen.scenario, body) : await parallelApi.create(body);
      if (!alive.current) return;
      setScenarios((list) => [s, ...(list ?? []).filter((x) => x.id !== s.id)]);
      playSfx('click');
      setScreen({ kind: 'scenario', id: s.id });
    } catch (e) {
      if (e instanceof ParallelError && e.status === 401) setNeedsToken(true);
      else if (alive.current) setSaveError(e instanceof Error ? e.message : 'Something went wrong.');
    } finally {
      if (alive.current) setSaving(false);
    }
  };

  const start = async (s: Scenario) => {
    setStarting(true);
    setNotice(null);
    try {
      const { run } = await parallelApi.start(s.id);
      if (!alive.current) return;
      setBusy(false);
      setRuns((list) => [run, ...(list ?? [])]);
      openRun(run.id);
    } catch (e) {
      if (!alive.current) return;
      if (e instanceof ParallelError && e.busy) setBusy(true);
      else setNotice({ text: `Couldn’t start a run: ${e instanceof Error ? e.message : 'something went wrong.'}`, error: true });
    } finally {
      if (alive.current) setStarting(false);
    }
  };

  const remove = (s: Scenario) => {
    setRemoving(s);
    removal.start(() => parallelApi.remove(s.id));
    setScreen({ kind: 'list' });
  };
  const undoRemove = () => {
    removal.undo();
    setRemoving(null);
  };
  const listed = (scenarios ?? []).filter((x) => !(removal.pending && x.id === removing?.id));

  const shownScenario = scenarioId ? (scenarios ?? []).find((s) => s.id === scenarioId) ?? null : null;
  let body: React.ReactNode = null;
  if (screen.kind === 'list') {
    body = (
      <>
        {removal.pending && removing && <UndoNotice text={`Deleted “${removing.title}”.`} onUndo={undoRemove} className="mb-3" />}
        {removal.error && <Notice text={`Couldn’t delete it: ${removal.error}`} error />}
        <ScenarioList scenarios={listed} onOpen={(s) => setScreen({ kind: 'scenario', id: s.id })} onNew={() => (setSaveError(null), setScreen({ kind: 'edit', scenario: null }))} />
      </>
    );
  } else if (screen.kind === 'edit') {
    body = (
      <ScenarioForm
        key={screen.scenario?.id ?? 'new'}
        initial={screen.scenario ? draftFrom(screen.scenario) : emptyScenario()}
        editing={!!screen.scenario}
        prices={prices}
        saving={saving}
        serverError={saveError}
        onSave={save}
        onCancel={() => setScreen(screen.scenario ? { kind: 'scenario', id: screen.scenario.id } : { kind: 'list' })}
      />
    );
  } else if (shownScenario) {
    body = (
      <>
        <ScenarioDetail
          scenario={shownScenario}
          runs={runs}
          current={current}
          starting={starting}
          busy={busy}
          notice={notice}
          onStart={() => start(shownScenario)}
          onPickRun={openRun}
          onEdit={() => (setSaveError(null), setScreen({ kind: 'edit', scenario: shownScenario }))}
          onDelete={() => remove(shownScenario)}
        />
      </>
    );
  } else {
    body = <p className="text-xs text-zinc-500">That scenario no longer exists.</p>;
  }

  return (
    <ParallelView
      screen={screen}
      scenarios={scenarios}
      loading={loading}
      error={error}
      needsToken={needsToken}
      body={body}
      onRefresh={() => (load(), scenarioId && loadRuns(scenarioId))}
      onBack={onBack}
      onList={() => { setScreen({ kind: 'list' }); load(); }}
    />
  );
};
