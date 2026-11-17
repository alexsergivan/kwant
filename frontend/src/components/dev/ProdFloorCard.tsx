import React, { useCallback, useEffect, useReducer, useRef, useState } from 'react';
import { Database, Folder, Target, X, Check, Terminal } from 'lucide-react';
import type { BackupResult, FloorBackup, FloorResolveOpts, ProdFloor } from '../../dev/types';
import { parseBackupResult, parseBackupState } from '../../dev/types';
import { allowReady, floorLed, floorReducer, floorSentence, floorSubline, floorView, floorWho, initialFloorUi, type RowTone } from '../../dev/logic';
import { devApi, devStore } from '../../devMode';
import { personFetch } from '../../auth';

export type FloorDecision = 'allow_once' | 'deny';

export interface ProdFloorCardProps {
  id: string;
  floor: ProdFloor;
  bot: string | null;
  command: string;
  cwd: string | null;
  /** `opts.withoutBackup`: the person confirmed "Allow without a backup" on a deploy kwant only guessed (22f). */
  onResolve: (id: string, decision: FloorDecision, opts?: FloorResolveOpts) => void;
  disabled?: boolean;
  /** Takes the backup; defaults to `POST /api/spaces/:space_id/dev/backup`. */
  onBackup?: (spaceId: string) => Promise<BackupResult>;
  /** Saves the space's backup command; defaults to the Dev mode settings' own PUT. */
  onSetCommand?: (spaceId: string, command: string) => Promise<void>;
  /** Re-reads the backup state (same shape as the card's `backup`); null when the server can't say. */
  onLoadBackup?: (spaceId: string) => Promise<FloorBackup | null>;
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

/** The Dev mode settings' PUT, so the settings card sees the new command too; `on` stays as it is. */
export async function putBackupCommand(spaceId: string, command: string): Promise<void> {
  const s = await devApi.put(spaceId, { on: devStore.get(spaceId)?.on ?? true, backup_command: command });
  devStore.set(spaceId, s);
}

/** Juno's 22f route: the card's current backup state. null on an older server (404) or a shape it doesn't know. */
export async function getBackupState(spaceId: string): Promise<FloorBackup | null> {
  const res = await personFetch(`/api/spaces/${encodeURIComponent(spaceId)}/dev/backup`);
  if (!res.ok) return null;
  return parseBackupState(await res.json().catch(() => null));
}

/** The longest backup command the server keeps (`BACKUP_COMMAND_CHARS`). */
const COMMAND_MAX = 2000;

const ROW: Record<RowTone, string> = {
  ok: 'border-teal-300/35 bg-teal-300/10 text-zinc-100',
  warn: 'border-amber-300/35 bg-amber-300/10 text-zinc-100',
  bad: 'border-rose-300/40 bg-rose-300/10 text-zinc-100',
  neutral: 'border-hairline bg-zinc-950/40 text-zinc-400',
  info: 'border-hairline bg-zinc-950/40 text-sky-200',
};
const LEAD: Record<RowTone, string> = { ok: 'text-ok', warn: 'text-warn', bad: 'text-bad', neutral: 'text-zinc-300', info: 'text-sky-200' };

const BTN = 'flex items-center justify-center gap-1.5 rounded-lg px-3 py-1.5 text-xs font-medium transition-colors min-h-[44px] sm:min-h-0 max-sm:w-full focus-visible:outline focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-sky-400 disabled:cursor-not-allowed disabled:opacity-50 aria-disabled:opacity-50 aria-disabled:cursor-not-allowed';
const AMBER = `${BTN} bg-amber-500 text-zinc-950 shadow-sm hover:bg-amber-400`;
const PLAIN = `${BTN} bg-zinc-800 text-zinc-300 hover:bg-zinc-700`;
const QUIET = `${BTN} text-zinc-300 hover:bg-white/[.06] hover:text-zinc-100`;
const FIELD = 'w-full rounded-lg border border-white/[0.14] bg-zinc-950/60 px-2.5 py-2 font-mono text-[12px] text-zinc-100 placeholder:text-zinc-500 focus:border-sky-400/60 focus:outline-none focus-visible:outline focus-visible:outline-2 focus-visible:outline-offset-1 focus-visible:outline-sky-400 max-sm:text-[16px]';

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
export const ProdFloorCard: React.FC<ProdFloorCardProps> = ({
  id,
  floor,
  bot,
  command,
  cwd,
  onResolve,
  disabled = false,
  onBackup = postBackup,
  onSetCommand = putBackupCommand,
  onLoadBackup = getBackupState,
  now: fixedNow,
}) => {
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
  const cmdRef = useRef<HTMLInputElement>(null);
  const skipRef = useRef<HTMLInputElement>(null);
  const skipOpenerRef = useRef<HTMLButtonElement>(null);

  // 22f: the inline backup command (no_command) and the one-step confirm for Allow without a backup.
  const [cmd, setCmd] = useState<{ open: boolean; value: string; saving: boolean; error: string | null }>({ open: false, value: '', saving: false, error: null });
  const [skip, setSkip] = useState<{ open: boolean; sure: boolean }>({ open: false, sure: false });
  /** The command was just saved from this card: the row says so until the backup starts. */
  const [justSaved, setJustSaved] = useState(false);

  // The server's backup state, re-read on focus and after a backup or a new command, so a backup taken (or a
  // command set) elsewhere counts without a reload. The server checks freshness again at answer time.
  const spaceId = floor.space_id;
  const busyRef = useRef(false);
  busyRef.current = view.busy || cmd.saving;
  const refresh = useCallback(async () => {
    if (!spaceId) return null;
    try {
      const b = await onLoadBackup(spaceId);
      if (b) send({ type: 'live', backup: b, now: Date.now() });
      return b;
    } catch {
      return null;
    }
  }, [spaceId, onLoadBackup]);
  useEffect(() => {
    if (!spaceId || fixedNow !== undefined) return;
    const onFocus = () => {
      if (!busyRef.current && document.visibilityState !== 'hidden') void refresh();
    };
    window.addEventListener('focus', onFocus);
    document.addEventListener('visibilitychange', onFocus);
    return () => {
      window.removeEventListener('focus', onFocus);
      document.removeEventListener('visibilitychange', onFocus);
    };
  }, [spaceId, fixedNow, refresh]);

  // `fromSecondary`: the secondary "Back up first" unmounts once the backup starts, so focus goes back to the
  // main button (which turns into Allow in place) when the backup is done.
  const takeBackup = async (fromSecondary = false) => {
    if (off || !floor.space_id) return;
    send({ type: 'backup_start' });
    setJustSaved(false);
    try {
      const result = await onBackup(floor.space_id);
      send({ type: 'backup_done', result, now: Date.now() });
      setTick(Date.now());
      void refresh();
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

  const openCommand = () => {
    if (off) return;
    setSkip({ open: false, sure: false });
    setCmd((c) => ({ ...c, open: true, error: null }));
    window.setTimeout(() => cmdRef.current?.focus(), 0);
  };
  const closeCommand = () => {
    setCmd((c) => ({ ...c, open: false, error: null }));
    window.setTimeout(() => mainRef.current?.focus(), 0);
  };
  const saveCommand = async (e?: React.FormEvent) => {
    e?.preventDefault();
    const value = cmd.value.trim();
    if (!spaceId || cmd.saving) return;
    if (!value) {
      setCmd((c) => ({ ...c, error: 'Type the command that takes the backup.' }));
      cmdRef.current?.focus();
      return;
    }
    setCmd((c) => ({ ...c, saving: true, error: null }));
    try {
      await onSetCommand(spaceId, value);
    } catch (err) {
      setCmd((c) => ({ ...c, saving: false, error: err instanceof Error ? err.message : 'Couldn’t save the command.' }));
      window.setTimeout(() => cmdRef.current?.focus(), 0);
      return;
    }
    const b = await refresh();
    // An older server can't re-read the state: the command is saved, so there is simply no backup yet.
    if (!b || b.state === 'no_command') send({ type: 'live', backup: { state: 'none', age_s: null, bytes: null, path: null, error: null }, now: Date.now() });
    setCmd({ open: false, value: '', saving: false, error: null });
    setJustSaved(true);
    // The main button is Back up first now, in the same place: focus goes there.
    window.setTimeout(() => mainRef.current?.focus(), 0);
  };

  const openSkip = () => {
    if (off) return;
    setCmd((c) => ({ ...c, open: false, error: null }));
    setSkip({ open: true, sure: false });
    window.setTimeout(() => skipRef.current?.focus(), 0);
  };
  const closeSkip = () => {
    setSkip({ open: false, sure: false });
    window.setTimeout(() => (skipOpenerRef.current ?? mainRef.current)?.focus(), 0);
  };
  const allowWithoutBackup = () => {
    if (off || !skip.sure || !view.skip) return;
    onResolve(id, 'allow_once', { withoutBackup: true });
  };
  const showSkip = view.skip && skip.open;
  const showCmd = view.main === 'set_command' && cmd.open;
  const cmdId = `floor-${id}-cmd`;
  const skipId = `floor-${id}-skip`;

  const main =
    view.main === 'allow'
      ? { label: allowLabel, icon: <Check size={14} aria-hidden="true" />, onClick: allow }
      : view.main === 'backup'
        ? { label: 'Back up first', icon: <Database size={14} aria-hidden="true" />, onClick: () => void takeBackup() }
        : view.main === 'retry'
          ? { label: 'Try the backup again', icon: <Database size={14} aria-hidden="true" />, onClick: () => void takeBackup() }
          : view.main === 'set_command'
            ? { label: 'Set backup command', icon: <Terminal size={14} aria-hidden="true" />, onClick: openCommand }
            : null;

  return (
    <div
      ref={cardRef}
      data-approval-card={id}
      data-prod-floor={floor.kind}
      data-floor-case={view.case}
      role="group"
      aria-labelledby={headingId}
      className="dv-floor my-3 max-w-2xl rounded-xl text-zinc-100 shadow-lg backdrop-blur-sm"
    >
      {/* The hazard tape is .dv-floor::before (4 px, only on the floor card). */}
      <div className="p-3 sm:px-4 sm:pb-4 sm:pt-3.5">
      <div className="flex min-w-0 items-center gap-2">
        <span className="dv-led" data-s={view.tone === 'ok' ? 'ok' : 'warn'} data-floor-led aria-hidden="true" />
        <h3 id={headingId} className="min-w-0 text-meta text-zinc-400 [overflow-wrap:anywhere]">
          <span className="font-mono text-[11px] font-bold uppercase tracking-[0.1em] text-amber-300">Production floor</span>
          <span className="sr-only">: </span>
          <span className="before:content-['·_'] before:text-zinc-500" data-floor-who>
            {' '}
            {floorWho(floor, bot ?? 'A bot')}
          </span>
        </h3>
      </div>
      {why && <p className="mt-2 text-[16px] font-semibold leading-snug text-zinc-50 [overflow-wrap:anywhere]" data-floor-why>{why}</p>}
      <p className={`${why ? 'mt-0.5' : 'mt-2'} mb-2.5 text-[13px] text-zinc-300`}>{floorSubline(floor)}</p>

      <div className="dv-cmd mb-3" data-floor-cmd>
        {cwd && (
          <div className="bar" data-floor-cwd>
            <Folder size={12} aria-hidden="true" className="shrink-0" />
            <span className="sr-only">In folder:</span>
            <span className="min-w-0 break-all">{cwd}</span>
          </div>
        )}
        <pre className="max-h-64 overflow-y-auto">
          <span className="ps" aria-hidden="true">$ </span>
          {command || '(empty command)'}
        </pre>
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
        <span className="dv-led mt-[5px]" data-s={floorLed(view.tone, view.case)} data-backup-led aria-hidden="true" />
        <p className="min-w-0 [overflow-wrap:anywhere]">
          {view.lead && <span className={`font-semibold ${LEAD[view.tone]}`}>{view.lead}</span>}
          {justSaved && view.case === 'blocked' ? ' Command saved. Press Back up first to take one.' : view.text}
          {view.file && (
            <>
              {' · '}
              <code className="font-mono text-[11px] text-zinc-300">{view.file}</code>
            </>
          )}
        </p>
      </div>

      {showCmd && (
        <form data-floor-command onSubmit={(e) => void saveCommand(e)} className="mb-3 rounded-lg border border-white/[0.14] bg-zinc-950/40 p-3">
          <label htmlFor={cmdId} className="mb-1 block text-xs font-medium text-zinc-200">
            Backup command
          </label>
          <input
            ref={cmdRef}
            id={cmdId}
            value={cmd.value}
            maxLength={COMMAND_MAX}
            onChange={(e) => setCmd((c) => ({ ...c, value: e.target.value, error: null }))}
            onKeyDown={(e) => {
              if (e.key === 'Escape') {
                e.preventDefault();
                closeCommand();
              }
            }}
            placeholder="./scripts/backup.sh"
            spellCheck={false}
            autoCapitalize="off"
            autoComplete="off"
            aria-invalid={cmd.error ? true : undefined}
            aria-describedby={`${cmdId}-help${cmd.error ? ` ${cmdId}-err` : ''}`}
            readOnly={cmd.saving}
            className={FIELD}
          />
          <p id={`${cmdId}-help`} className="mt-1 text-[11px] leading-snug text-zinc-400 [overflow-wrap:anywhere]">
            Runs in the bot’s folder when you press Back up first. It should print the backup file’s path. For example:{' '}
            <code className="font-mono text-zinc-300">ssh root@server 'pg_dump app &gt; /srv/b.sql &amp;&amp; echo /srv/b.sql'</code>
          </p>
          {cmd.error && (
            <p id={`${cmdId}-err`} role="alert" className="mt-1 text-[11px] text-rose-300 [overflow-wrap:anywhere]">
              {cmd.error}
            </p>
          )}
          <div className="mt-2 flex flex-wrap justify-end gap-2 max-sm:flex-col-reverse max-sm:items-stretch">
            <button type="button" onClick={closeCommand} className={QUIET}>
              Cancel
            </button>
            <button type="submit" data-floor-command-save aria-disabled={cmd.saving || undefined} aria-busy={cmd.saving || undefined} className={AMBER}>
              <Check size={14} aria-hidden="true" />
              {cmd.saving ? 'Saving…' : 'Save command'}
            </button>
          </div>
        </form>
      )}

      {showSkip && (
        <div data-floor-skip role="group" aria-labelledby={`${skipId}-h`} className="mb-3 rounded-lg border border-amber-300/35 bg-amber-300/[.07] p-3">
          <p id={`${skipId}-h`} className="text-xs font-semibold text-zinc-100">
            Allow without a backup?
          </p>
          <p className="mt-0.5 text-[11px] leading-snug text-zinc-300">If this deploy does change the database, there will be no copy to go back to.</p>
          <label htmlFor={skipId} className="mt-2 flex min-h-[44px] cursor-pointer items-center gap-2 text-xs text-zinc-100 sm:min-h-0">
            <input
              ref={skipRef}
              id={skipId}
              type="checkbox"
              checked={skip.sure}
              onChange={(e) => setSkip((k) => ({ ...k, sure: e.target.checked }))}
              onKeyDown={(e) => {
                if (e.key === 'Escape') {
                  e.preventDefault();
                  closeSkip();
                }
              }}
              className="h-4 w-4 shrink-0 accent-amber-500"
            />
            This deploy doesn’t change the database
          </label>
          <div className="mt-2 flex flex-wrap justify-end gap-2 max-sm:flex-col-reverse max-sm:items-stretch">
            <button type="button" onClick={closeSkip} className={QUIET}>
              Cancel
            </button>
            <button type="button" data-floor-skip-allow aria-disabled={!skip.sure || off || undefined} onClick={allowWithoutBackup} className={`${AMBER} aria-disabled:hover:bg-amber-600`}>
              <Check size={14} aria-hidden="true" />
              Allow without a backup
            </button>
          </div>
        </div>
      )}

      {/* With Allow without a backup there are three buttons: the note gets its own line so Deny never wraps alone. */}
      <div data-approval-actions className={`flex flex-wrap items-center gap-2 max-sm:flex-col max-sm:items-stretch ${view.skip ? 'justify-end' : ''}`}>
        {!showSkip && <p className={`text-[11px] text-zinc-400 ${view.skip ? 'basis-full' : 'sm:mr-auto'}`}>{view.note}</p>}
        {main && !showCmd && (
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
            // While the confirm is open its own button is the one amber action.
            className={showSkip ? PLAIN : AMBER}
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
        {view.skip && !skip.open && (
          <button ref={skipOpenerRef} type="button" data-floor-skip-open aria-disabled={off || undefined} onClick={openSkip} className={PLAIN}>
            Allow without a backup
          </button>
        )}
        <button type="button" disabled={off} onClick={() => !off && onResolve(id, 'deny')} className={PLAIN}>
          <X size={14} aria-hidden="true" />
          Deny
        </button>
      </div>
      </div>
    </div>
  );
};
