import React, { useEffect, useReducer, useRef, useState } from 'react';
import { Database, Loader2, ShieldAlert, Target, X, Check } from 'lucide-react';
import type { BackupResult, ProdFloor } from '../../dev/types';
import { parseBackupResult } from '../../dev/types';
import { allowReady, floorHeader, floorReducer, floorSentence, floorSubline, floorView, initialFloorUi, type RowTone } from '../../dev/logic';

export type FloorDecision = 'allow_once' | 'deny';

export interface ProdFloorCardProps {
  id: string;
  floor: ProdFloor;
  bot: string | null;
  command: string;
  cwd: string | null;
  onResolve: (id: string, decision: FloorDecision) => void;
  disabled?: boolean;
  /** Takes the backup; defaults to `POST /api/spaces/:space_id/dev/backup`. */
  onBackup?: (spaceId: string) => Promise<BackupResult>;
  /** A fixed clock for screenshots and tests; the card ticks on its own otherwise. */
  now?: number;
}

/** Lin's route: 200 with {ok, error} for a run, 409 {error} when there is no command or one is running. */
export async function postBackup(spaceId: string): Promise<BackupResult> {
  const res = await fetch(`/api/spaces/${encodeURIComponent(spaceId)}/dev/backup`, { method: 'POST' });
  const body = await res.json().catch(() => null);
  if (!res.ok) {
    const err = body && typeof body === 'object' && typeof (body as { error?: unknown }).error === 'string' ? (body as { error: string }).error : `HTTP ${res.status}`;
    throw new Error(err);
  }
  return parseBackupResult(body);
}

const ROW: Record<RowTone, string> = {
  ok: 'border-teal-300/35 bg-teal-300/10 text-zinc-100',
  warn: 'border-amber-300/35 bg-amber-300/10 text-zinc-100',
  bad: 'border-rose-300/40 bg-rose-300/10 text-zinc-100',
  neutral: 'border-hairline bg-zinc-950/40 text-zinc-400',
  info: 'border-hairline bg-zinc-950/40 text-sky-200',
};
const LEAD: Record<RowTone, string> = { ok: 'text-ok', warn: 'text-warn', bad: 'text-bad', neutral: 'text-zinc-300', info: 'text-sky-200' };

const BTN = 'flex items-center justify-center gap-1.5 rounded-lg px-3 py-1.5 text-xs font-medium transition-colors min-h-[44px] sm:min-h-0 max-sm:w-full focus-visible:outline focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-sky-400 disabled:cursor-not-allowed disabled:opacity-50 aria-disabled:opacity-50 aria-disabled:cursor-not-allowed';
const AMBER = `${BTN} bg-amber-600 text-white shadow-sm hover:bg-amber-500`;
const PLAIN = `${BTN} bg-zinc-800 text-zinc-300 hover:bg-zinc-700`;

/** "kwant.jumpin.games · production": the place, then a warn chip for the word "production". */
const BlastValue: React.FC<{ label: string; value: string }> = ({ label, value }) => {
  const m = label === 'Where' ? value.match(/^(.*?)\s*·\s*production$/i) : null;
  if (!m) return <>{value}</>;
  return (
    <>
      {m[1]} <span className="kw-chip kw-chip-warn ml-1 align-middle">production</span>
    </>
  );
};

/** The production-floor variant of the approval card (design §6). Never offers "always allow". */
export const ProdFloorCard: React.FC<ProdFloorCardProps> = ({ id, floor, bot, command, cwd, onResolve, disabled = false, onBackup = postBackup, now: fixedNow }) => {
  const [ui, send] = useReducer(floorReducer, floor, initialFloorUi);
  const mountedAt = useRef(fixedNow ?? Date.now()).current;
  const [tick, setTick] = useState(() => Date.now());
  useEffect(() => {
    if (fixedNow !== undefined) return;
    const t = window.setInterval(() => setTick(Date.now()), 1000);
    return () => window.clearInterval(t);
  }, [fixedNow]);
  const now = fixedNow ?? tick;

  const view = floorView(floor, ui, now, mountedAt);
  const isDeploy = floor.kind === 'deploy' || floor.kind === 'deploy_migration';
  const allowLabel = isDeploy ? 'Allow this deploy' : 'Allow';
  const off = disabled || view.busy;
  const headingId = `floor-${id}-h`;
  const why = floorSentence(floor.why);
  const mainRef = useRef<HTMLButtonElement>(null);
  const cardRef = useRef<HTMLDivElement>(null);

  // `fromSecondary`: the secondary "Back up first" unmounts once the backup starts, so focus goes back to the
  // main button (which turns into Allow in place) when the backup is done.
  const takeBackup = async (fromSecondary = false) => {
    if (off || !floor.space_id) return;
    send({ type: 'backup_start' });
    try {
      const result = await onBackup(floor.space_id);
      send({ type: 'backup_done', result, now: Date.now() });
      setTick(Date.now());
    } catch (e) {
      send({ type: 'backup_error', error: e instanceof Error ? e.message : 'no reason given' });
    } finally {
      // Done or failed, the secondary button is gone by now; the main button is Allow on success and
      // "Try the backup again" on failure. Bring focus there only if it was lost with the button (now on
      // <body>) or is still inside this card: never take it from the composer or anywhere else, because
      // after a good backup one Space on that button allows a production deploy.
      if (fromSecondary)
        window.setTimeout(() => {
          const a = document.activeElement;
          if (!a || a === document.body || cardRef.current?.contains(a)) mainRef.current?.focus();
        }, 0);
    }
  };

  const allow = () => {
    // The Allow that replaced Back up first ignores presses for its first 500 ms.
    if (off || !allowReady(ui, Date.now())) return;
    onResolve(id, 'allow_once');
  };

  const main =
    view.main === 'allow'
      ? { label: allowLabel, icon: <Check size={14} aria-hidden="true" />, onClick: allow }
      : view.main === 'backup'
        ? { label: 'Back up first', icon: <Database size={14} aria-hidden="true" />, onClick: () => void takeBackup() }
        : view.main === 'retry'
          ? { label: 'Try the backup again', icon: <Database size={14} aria-hidden="true" />, onClick: () => void takeBackup() }
          : null;

  return (
    <div
      ref={cardRef}
      data-approval-card={id}
      data-prod-floor={floor.kind}
      data-floor-case={view.case}
      role="group"
      aria-labelledby={headingId}
      className="my-3 max-w-2xl rounded-xl border border-amber-500/30 bg-amber-950/20 p-3 text-zinc-100 shadow-lg backdrop-blur-sm sm:p-4"
    >
      <div className="mb-1 flex items-start gap-2 text-xs font-semibold tracking-wide text-amber-400">
        <ShieldAlert size={16} aria-hidden="true" className="mt-px shrink-0" />
        <h3 id={headingId} className="min-w-0 [overflow-wrap:anywhere]">
          {floorHeader(floor, bot ?? 'A bot')}
        </h3>
      </div>
      <p className="mb-3 text-[11px] text-zinc-400 sm:pl-6">{floorSubline(floor)}</p>

      <div className="mb-3 space-y-2 rounded-lg border border-zinc-800 bg-zinc-900/90 p-3 text-xs">
        {why && <p className="text-zinc-300 [overflow-wrap:anywhere]">{why}</p>}
        <pre className="max-h-64 overflow-y-auto whitespace-pre-wrap break-all rounded-md border border-zinc-800 bg-zinc-950/70 p-2 font-mono text-[12px] text-zinc-100">
          {command || '(empty command)'}
        </pre>
        {cwd && (
          <p className="break-all text-[11px] text-zinc-400">
            <span className="text-zinc-500">In folder:</span> <span className="font-mono text-zinc-300">{cwd}</span>
          </p>
        )}
      </div>

      {floor.blast.length > 0 && (
        <section aria-label="Blast radius" className="mb-3 overflow-hidden rounded-xl border border-white/[0.14]">
          <h4 className="flex items-center gap-1.5 border-b border-white/[0.14] bg-zinc-950/30 px-3 py-1.5 text-2xs font-semibold uppercase tracking-wider text-zinc-400">
            <Target size={12} aria-hidden="true" />
            Blast radius
          </h4>
          <dl className="divide-y divide-white/[0.08] text-xs">
            {floor.blast.map(([label, value]) => (
              <div key={label} className="grid grid-cols-1 gap-0.5 px-3 py-2 sm:grid-cols-[120px_1fr] sm:gap-3">
                <dt className="text-zinc-400">{label}</dt>
                <dd className="min-w-0 text-zinc-100 [overflow-wrap:anywhere]">
                  <BlastValue label={label} value={value} />
                </dd>
              </div>
            ))}
          </dl>
        </section>
      )}

      <div data-backup-row={view.case} aria-live="polite" className={`mb-3 flex items-start gap-2 rounded-lg border px-3 py-2 text-xs ${ROW[view.tone]}`}>
        {view.case === 'taking' ? (
          <Loader2 size={14} aria-hidden="true" className="mt-px shrink-0 animate-spin motion-reduce:animate-none" />
        ) : view.tone === 'bad' ? (
          <X size={14} aria-hidden="true" className={`mt-px shrink-0 ${LEAD.bad}`} />
        ) : (
          <Database size={14} aria-hidden="true" className={`mt-px shrink-0 ${LEAD[view.tone]}`} />
        )}
        <p className="min-w-0 [overflow-wrap:anywhere]">
          {view.lead && <span className={`font-semibold ${LEAD[view.tone]}`}>{view.lead}</span>}
          {view.text}
          {view.file && (
            <>
              {' · '}
              <code className="font-mono text-[11px] text-zinc-300">{view.file}</code>
            </>
          )}
        </p>
      </div>

      <div data-approval-actions className="flex flex-wrap items-center gap-2 max-sm:flex-col max-sm:items-stretch">
        <p className="text-[11px] text-zinc-400 sm:mr-auto">{view.note}</p>
        {main && (
          // One element for Back up first, Allow and Try again, so the button changes in place and keeps focus.
          // aria-disabled, never disabled: disabling the focused button sends focus to <body> (B2). The handlers
          // ignore presses while `off`.
          <button
            ref={mainRef}
            key="main"
            type="button"
            data-floor-main={view.main}
            aria-disabled={off || undefined}
            aria-busy={view.busy || undefined}
            onClick={() => !off && main.onClick()}
            className={AMBER}
          >
            {main.icon}
            {main.label}
          </button>
        )}
        {view.secondaryBackup && (
          <button
            type="button"
            // aria-disabled, not disabled: a focused button that turns disabled drops focus to <body> for the
            // whole backup. takeBackup ignores presses while busy.
            aria-disabled={off || undefined}
            onClick={() => void takeBackup(true)}
            className={PLAIN}
          >
            <Database size={14} aria-hidden="true" />
            Back up first
          </button>
        )}
        <button type="button" disabled={off} onClick={() => !off && onResolve(id, 'deny')} className={PLAIN}>
          <X size={14} aria-hidden="true" />
          Deny
        </button>
      </div>
    </div>
  );
};
