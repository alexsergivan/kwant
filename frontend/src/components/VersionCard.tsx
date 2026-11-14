// Settings › kwant: the running version, whether a newer one is out, and What's new.
// One card everywhere: the Mac app installs the update; elsewhere Update says how (one command).
import React, { useEffect, useId, useState } from 'react';
import { AlertTriangle, ArrowUpCircle, Check, CheckCircle2, ChevronRight, Copy, ExternalLink, Loader2, RotateCw } from 'lucide-react';
import { type HowToUpdate, LATEST_RELEASE_URL, type UpdateModel, type VersionLine, checkedAgo, versionLine } from '../appUpdate';
import { useAppUpdate } from './AppUpdateBanner';
import { WhatsNewMark } from './WhatsNewSheet';
import { RELEASES_URL, type WhatsNewEntry, entrySummary, openWhatsNew, shortDate } from '../whatsNewSheet';
import { whatsNewFor } from '../whatsNewContent';

const focus = 'focus-visible:outline focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-sky-400';
const quiet = `inline-flex min-h-[28px] items-center gap-1 rounded-md px-1.5 text-xs font-medium text-sky-300 hover:bg-sky-500/[0.12] hover:text-sky-200 max-md:min-h-[44px] ${focus}`;
const primary = `inline-flex min-h-[32px] items-center gap-1.5 rounded-lg bg-sky-600 px-3 text-xs font-semibold text-white hover:bg-sky-500 max-md:min-h-[44px] ${focus}`;
const ghost = `inline-flex min-h-[32px] items-center gap-1.5 rounded-lg border border-zinc-700 px-3 text-xs font-medium text-zinc-200 hover:bg-zinc-800 max-md:min-h-[44px] ${focus}`;

/** Where the How to update command runs, in plain words. */
const HOW_WHERE: Record<HowToUpdate['kind'], string> = {
  docker: 'Run this in the folder with your docker-compose.yml:',
  binary: 'Run this on the server kwant bots runs on:',
  source: 'Run this in your kwant bots folder:',
  windows: 'Download the new installer and run it. Your chats and bots stay as they are.',
};

const HowToUpdatePanel: React.FC<{ id: string; how: HowToUpdate | null; version: string; url: string }> = ({ id, how, version, url }) => {
  const [copied, setCopied] = useState(false);
  useEffect(() => {
    if (!copied) return;
    const t = window.setTimeout(() => setCopied(false), 2000);
    return () => window.clearTimeout(t);
  }, [copied]);
  const command = how?.command ?? null;
  return (
    <div id={id} data-how-to-update={how?.kind ?? 'none'} className="mt-3 rounded-lg border border-zinc-800 bg-zinc-900/70 p-3">
      <p className="m-0 text-xs font-semibold text-zinc-200">How to update</p>
      <p className="mb-0 mt-1 text-xs text-zinc-400">{how ? HOW_WHERE[how.kind] : 'Get the new version from its release page.'}</p>
      {command && (
        <div className="mt-2 flex items-start gap-2">
          <pre className="m-0 min-w-0 flex-1 whitespace-pre-wrap break-all rounded-md bg-black/40 px-2.5 py-2 font-mono text-[11.5px] leading-[18px] text-zinc-100 select-text">
            {command}
          </pre>
          <button
            type="button"
            onClick={() => {
              void navigator.clipboard?.writeText(command).then(
                () => setCopied(true),
                () => setCopied(false),
              );
            }}
            className={`${ghost} shrink-0`}
            aria-label={copied ? 'Copied' : 'Copy the command'}
          >
            {copied ? <Check size={13} aria-hidden="true" /> : <Copy size={13} aria-hidden="true" />}
            {copied ? 'Copied' : 'Copy'}
          </button>
        </div>
      )}
      {!command && (
        <a href={url} target="_blank" rel="noreferrer" className={`${primary} mt-2`}>
          <ExternalLink size={13} aria-hidden="true" /> Download kwant bots {version}
        </a>
      )}
      {command && <p className="mb-0 mt-2 text-[11px] text-zinc-500">Your chats and bots stay as they are.</p>}
    </div>
  );
};

/** "What's new in X ›" in the What's new card's look: the current version's sheet. */
const WhatsNewButton: React.FC<{ entry: WhatsNewEntry; onPick: (version: string) => void }> = ({ entry, onPick }) => (
  <button
    type="button"
    onClick={() => onPick(entry.version)}
    data-whats-new-current={entry.version}
    className={`mt-3 flex min-h-[56px] w-full items-center gap-3 rounded-[10px] bg-sky-500/10 py-2 pl-2.5 pr-3 text-left shadow-[inset_0_0_0_1px_rgb(157_143_255/.25)] hover:bg-sky-500/[0.16] max-md:min-h-[60px] ${focus}`}
  >
    <span className="grid h-9 w-9 shrink-0 place-items-center rounded-[9px] bg-surface-overlay shadow-[inset_0_0_0_1px_rgb(255_255_255/.14)]">
      <WhatsNewMark size={24} />
    </span>
    <span className="min-w-0 flex-1">
      <b className="block text-[13.5px] font-semibold leading-5 text-zinc-100">What’s new in {entry.version}</b>
      <span className="block text-xs leading-[18px] text-zinc-400">
        {shortDate(entry.date)} · {entrySummary(entry)}
      </span>
    </span>
    <ChevronRight size={15} className="shrink-0 text-sky-300" aria-hidden="true" />
  </button>
);

export interface VersionCardProps {
  model: UpdateModel;
  /** For "Checked 2 min ago". */
  now: number;
  onCheck: () => void;
  onInstall: () => void;
  /** The current version's sheet (bundled). */
  onWhatsNew: (version: string) => void;
  /** A newer version's release notes (not bundled in this app). */
  onReleaseNotes: (version: string, notes: string, date: string | null) => void;
  /** The current version's bundled sheet, if any. */
  entryFor?: (version: string) => WhatsNewEntry | null;
  /** Start with How to update open (screenshots). */
  howOpen?: boolean;
}

/** The card itself, without polling: what tests and screenshots render. */
export const VersionCardView: React.FC<VersionCardProps> = ({ model, now, onCheck, onInstall, onWhatsNew, onReleaseNotes, entryFor = whatsNewFor, howOpen = false }) => {
  const [showHow, setShowHow] = useState(howOpen);
  const howId = useId();
  const line: VersionLine = versionLine(model);
  const current = model.status?.current || model.current || '';
  const entry = current ? entryFor(current) : null;
  const newer = line.kind === 'available' || line.kind === 'install-error' ? line : null;
  const newerSheet = newer && (entryFor(newer.version) || newer.notes.trim()) ? newer : null;

  const status = (() => {
    switch (line.kind) {
      case 'latest':
        return (
          <>
            <span className="inline-flex items-center gap-1.5 font-medium text-zinc-100">
              <CheckCircle2 size={15} className="shrink-0 text-emerald-400" aria-hidden="true" />
              You’re on the latest version
            </span>
            <button type="button" onClick={onCheck} className={quiet}>
              Check again
            </button>
            {line.checkedAt != null && <span className="w-full pl-[21px] text-[11px] text-zinc-500">{checkedAgo(line.checkedAt, now)}</span>}
          </>
        );
      case 'available':
        return (
          <>
            <span className="inline-flex min-w-0 flex-1 items-center gap-1.5 font-medium text-zinc-100">
              <ArrowUpCircle size={15} className="shrink-0 text-sky-300" aria-hidden="true" />
              kwant bots {line.version} is available
            </span>
            <button
              type="button"
              onClick={() => (line.canInstall ? onInstall() : setShowHow((v) => !v))}
              aria-expanded={line.canInstall ? undefined : showHow}
              aria-controls={line.canInstall ? undefined : howId}
              className={primary}
              data-version-update
            >
              <ArrowUpCircle size={13} aria-hidden="true" />
              Update
            </button>
          </>
        );
      case 'checking':
      case 'loading':
        return (
          <span className="inline-flex items-center gap-1.5 text-zinc-300">
            <Loader2 size={15} className="shrink-0 animate-spin text-sky-300 motion-reduce:animate-none" aria-hidden="true" />
            Checking for updates…
          </span>
        );
      case 'downloading':
        return (
          <>
            <span className="inline-flex min-w-0 flex-1 items-center gap-1.5 font-medium text-zinc-100">
              <Loader2 size={15} className="shrink-0 animate-spin text-sky-300 motion-reduce:animate-none" aria-hidden="true" />
              Downloading kwant bots {line.version}…{line.progress != null && ` ${Math.round(line.progress * 100)}%`}
            </span>
            <span
              role="progressbar"
              aria-label={`Downloading kwant bots ${line.version}`}
              aria-valuemin={0}
              aria-valuemax={100}
              aria-valuenow={line.progress == null ? undefined : Math.round(line.progress * 100)}
              className="block h-1.5 w-28 shrink-0 overflow-hidden rounded-full bg-zinc-800"
            >
              <span
                className={`block h-full bg-sky-400 transition-[width] motion-reduce:transition-none ${line.progress == null ? 'w-1/3 animate-pulse' : ''}`}
                style={line.progress == null ? undefined : { width: `${Math.round(line.progress * 100)}%` }}
              />
            </span>
          </>
        );
      case 'restarting':
        return (
          <span className="inline-flex items-center gap-1.5 font-medium text-zinc-100">
            <Loader2 size={15} className="shrink-0 animate-spin text-sky-300 motion-reduce:animate-none" aria-hidden="true" />
            Restarting the app to finish the update…
          </span>
        );
      case 'install-error':
        return (
          <>
            <span className="inline-flex min-w-0 flex-1 items-start gap-1.5 text-zinc-200">
              <AlertTriangle size={15} className="mt-px shrink-0 text-amber-300" aria-hidden="true" />
              <span>
                <span className="font-medium text-zinc-100">kwant bots {line.version} didn’t install.</span> {line.error} This version keeps working.
              </span>
            </span>
            <a href={line.url} target="_blank" rel="noreferrer" className={quiet}>
              Download manually
            </a>
            <button type="button" onClick={onInstall} className={primary}>
              <RotateCw size={13} aria-hidden="true" /> Try again
            </button>
          </>
        );
      case 'check-error': {
        const why = line.error.replace(/^Couldn[’']t check for updates[:.]?\s*/i, '');
        return (
          <>
            <span className="inline-flex min-w-0 items-center gap-1.5 font-medium text-zinc-100">
              <AlertTriangle size={15} className="shrink-0 text-amber-300" aria-hidden="true" />
              Couldn’t check for updates
            </span>
            <button type="button" onClick={onCheck} className={quiet}>
              <RotateCw size={12} aria-hidden="true" /> Try again
            </button>
            {why && <span className="w-full pl-[21px] text-[11px] text-zinc-500">{why}</span>}
          </>
        );
      }
      case 'updated':
        return (
          <span className="inline-flex items-center gap-1.5 font-medium text-zinc-100">
            <CheckCircle2 size={15} className="shrink-0 text-emerald-400" aria-hidden="true" />
            Updated to kwant bots {line.version}
          </span>
        );
      case 'off':
        // Not a dead end: ask again, or get the release by hand
        return (
          <>
            <span className="text-zinc-400">Update checks are turned off here.</span>
            <button
              type="button"
              onClick={() => !line.rechecking && onCheck()}
              aria-disabled={line.rechecking || undefined}
              className={`${quiet} aria-disabled:cursor-default aria-disabled:opacity-70`}
              data-version-recheck
            >
              {line.rechecking ? <Loader2 size={12} className="animate-spin motion-reduce:animate-none" aria-hidden="true" /> : null}
              {line.rechecking ? 'Checking…' : 'Check again'}
            </button>
            <a href={LATEST_RELEASE_URL} target="_blank" rel="noreferrer" className={`${quiet} max-md:-ml-1.5 max-md:basis-full max-md:w-fit`} data-version-latest>
              Get the latest version <ExternalLink size={12} aria-hidden="true" />
            </a>
          </>
        );
    }
  })();

  return (
    <section aria-labelledby="version-card-title" data-version-card={line.kind} className="rounded-xl border border-zinc-800 bg-zinc-950/60 p-4">
      <h3 id="version-card-title" className="m-0 text-sm font-semibold text-zinc-100">
        kwant bots <span className="font-mono text-[13px] font-medium text-zinc-300">{current || 'version unknown'}</span>
      </h3>
      <div role="status" aria-live="polite" className="mt-2 flex flex-wrap items-center gap-x-2 gap-y-1 text-xs" data-version-line>
        {status}
      </div>
      {newerSheet && (
        <button
          type="button"
          onClick={() => (entryFor(newerSheet.version) ? onWhatsNew(newerSheet.version) : onReleaseNotes(newerSheet.version, newerSheet.notes, newerSheet.date))}
          className={`${quiet} -ml-1.5 mt-1`}
          data-whats-new-next={newerSheet.version}
        >
          What’s new in {newerSheet.version} <ChevronRight size={13} aria-hidden="true" />
        </button>
      )}
      {line.kind === 'available' && !line.canInstall && showHow && (
        <HowToUpdatePanel id={howId} how={model.status?.how ?? null} version={line.version} url={line.url} />
      )}
      {entry && <WhatsNewButton entry={entry} onPick={onWhatsNew} />}
      {/* Off already links the latest release; one GitHub link is enough */}
      {line.kind !== 'off' && (
        <a href={RELEASES_URL} target="_blank" rel="noreferrer" className={`mt-2 inline-flex items-center gap-1.5 py-1 text-xs text-sky-300 hover:underline max-md:min-h-[44px] ${focus}`}>
          All releases on GitHub <ExternalLink size={13} aria-hidden="true" />
        </a>
      )}
    </section>
  );
};

/** Ticks once a minute, for "Checked N min ago". */
function useNow(ms = 30_000) {
  const [now, setNow] = useState(() => Date.now());
  useEffect(() => {
    const t = window.setInterval(() => setNow(Date.now()), ms);
    return () => window.clearInterval(t);
  }, [ms]);
  return now;
}

export const VersionCard: React.FC = () => {
  const { model, ctrl } = useAppUpdate();
  const now = useNow();
  const known = model.status !== null;
  // Opening Settings checks again when the last answer is over an hour old, and always when it was off
  useEffect(() => {
    if (known || ctrl.get().unsupported) void ctrl.checkIfStale();
  }, [known, ctrl]);
  return (
    <VersionCardView
      model={model}
      now={now}
      onCheck={() => void ctrl.check()}
      onInstall={() => void ctrl.install()}
      onWhatsNew={(v) => openWhatsNew(v)}
      onReleaseNotes={(v, notes, date) => openWhatsNew(v, { notes, date })}
    />
  );
};
