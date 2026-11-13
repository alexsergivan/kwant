import React, { useEffect, useId, useState, useSyncExternalStore } from 'react';
import { AlertTriangle, ArrowUpCircle, CheckCircle2, ExternalLink, Loader2, RotateCw, X } from 'lucide-react';
import { BannerView, UpdateController, appUpdateController, bannerView, dismissVersion, dismissedVersion } from '../appUpdate';
import { MarkdownRenderer } from './MarkdownRenderer';
import { hasWhatsNew } from './WhatsNewSheet';
import { openWhatsNew } from '../whatsNewSheet';

const PHONE_QUERY = '(max-width: 767px)';
const UPDATED_NOTE_MS = 10000;

/** The shared update state; starts the one poller on first use. */
export function useAppUpdate(ctrl: UpdateController = appUpdateController()) {
  const model = useSyncExternalStore(ctrl.subscribe, ctrl.get, ctrl.get);
  useEffect(() => ctrl.start(), [ctrl]);
  return { model, ctrl };
}

interface ViewProps {
  view: BannerView;
  onUpdate: () => void;
  onDismiss: () => void;
  /** "See what's new" on the "Updated to" bar, when there's a What's new sheet for that version. */
  onWhatsNew?: () => void;
}

const btn =
  'inline-flex min-h-[32px] items-center gap-1.5 rounded-lg px-3 text-xs font-medium focus-visible:outline focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-sky-400';

/** The slim bar itself, without the polling: what tests and screenshots render. */
export const AppUpdateBannerView: React.FC<ViewProps> = ({ view, onUpdate, onDismiss, onWhatsNew }) => {
  const [notesOpen, setNotesOpen] = useState(false);
  const notesId = useId();
  const notes = view.kind === 'available' || view.kind === 'error' ? view.notes.trim() : '';
  const tone =
    view.kind === 'error'
      ? 'border-amber-500/30 bg-amber-500/10'
      : view.kind === 'updated'
        ? 'border-emerald-500/30 bg-emerald-500/10'
        : 'border-sky-500/30 bg-sky-500/10';

  return (
    <aside aria-label="App update" data-update-banner={view.kind} className={`shrink-0 border-b ${tone} text-xs text-zinc-100`}>
      <div className="flex flex-wrap items-center gap-x-3 gap-y-2 px-4 py-2">
        <p className="flex min-w-0 flex-1 items-center gap-2" role="status" aria-live="polite">
          {view.kind === 'available' && <ArrowUpCircle size={15} className="shrink-0 text-sky-300" aria-hidden="true" />}
          {(view.kind === 'downloading' || view.kind === 'restarting') && (
            <Loader2 size={15} className="shrink-0 animate-spin text-sky-300 motion-reduce:animate-none" aria-hidden="true" />
          )}
          {view.kind === 'error' && <AlertTriangle size={15} className="shrink-0 text-amber-300" aria-hidden="true" />}
          {view.kind === 'updated' && <CheckCircle2 size={15} className="shrink-0 text-emerald-300" aria-hidden="true" />}
          <span className="min-w-0">
            {view.kind === 'available' && (
              <>
                <span className="font-medium">kwant bots {view.version} is ready.</span>
                <span className="text-zinc-400"> Your chats and bots stay as they are.</span>
              </>
            )}
            {view.kind === 'downloading' && (
              <span className="font-medium">
                Downloading kwant bots {view.version}…{view.progress != null && ` ${Math.round(view.progress * 100)}%`}
              </span>
            )}
            {view.kind === 'restarting' && <span className="font-medium">Restarting the app to finish the update…</span>}
            {view.kind === 'error' && (
              <>
                <span className="font-medium">kwant bots {view.version} didn’t install.</span>
                <span className="text-zinc-300"> {view.error} This version keeps working.</span>
              </>
            )}
            {view.kind === 'updated' && <span className="font-medium">Updated to kwant bots {view.version}.</span>}
          </span>
        </p>

        {(view.kind === 'available' || view.kind === 'error') && (
          <div className="flex shrink-0 flex-wrap items-center gap-2">
            {notes && (
              <button
                type="button"
                aria-expanded={notesOpen}
                aria-controls={notesId}
                onClick={() => setNotesOpen((v) => !v)}
                className={`${btn} text-sky-200 hover:bg-sky-500/15`}
              >
                What’s new
              </button>
            )}
            {view.kind === 'error' && (
              <a href={view.url} target="_blank" rel="noreferrer" className={`${btn} text-zinc-200 hover:bg-zinc-800`}>
                <ExternalLink size={13} aria-hidden="true" /> Download manually
              </a>
            )}
            <button type="button" onClick={onUpdate} className={`${btn} bg-sky-600 text-white hover:bg-sky-500`}>
              {view.kind === 'error' ? <RotateCw size={13} aria-hidden="true" /> : <ArrowUpCircle size={13} aria-hidden="true" />}
              {view.kind === 'error' ? 'Try again' : 'Update and restart'}
            </button>
          </div>
        )}

        {view.kind === 'downloading' && (
          <div
            role="progressbar"
            aria-label={`Downloading kwant bots ${view.version}`}
            aria-valuemin={0}
            aria-valuemax={100}
            aria-valuenow={view.progress == null ? undefined : Math.round(view.progress * 100)}
            className="h-1.5 w-32 shrink-0 overflow-hidden rounded-full bg-zinc-800"
          >
            <div
              className={`h-full bg-sky-400 transition-[width] motion-reduce:transition-none ${view.progress == null ? 'w-1/3 animate-pulse' : ''}`}
              style={view.progress == null ? undefined : { width: `${Math.round(view.progress * 100)}%` }}
            />
          </div>
        )}

        {view.kind === 'updated' && onWhatsNew && (
          <button
            type="button"
            onClick={onWhatsNew}
            data-see-whats-new
            className={`${btn} min-h-[28px] h-7 border border-emerald-300/35 px-2.5 text-emerald-200 hover:bg-emerald-500/15 hover:text-emerald-50`}
          >
            See what’s new
          </button>
        )}

        {(view.kind === 'available' || view.kind === 'updated') && (
          <button
            type="button"
            onClick={onDismiss}
            aria-label={view.kind === 'available' ? `Hide until the next version` : 'Close'}
            title={view.kind === 'available' ? 'Hide until the next version' : undefined}
            className="-my-1 flex h-8 w-8 shrink-0 items-center justify-center rounded-lg text-zinc-400 hover:bg-zinc-800 hover:text-zinc-100 focus-visible:outline focus-visible:outline-2 focus-visible:outline-sky-400"
          >
            <X size={15} />
          </button>
        )}
      </div>
      {notes && (
        <div id={notesId} hidden={!notesOpen} className="max-h-60 overflow-y-auto border-t border-zinc-800/80 px-4 py-3 text-zinc-300 select-text">
          <MarkdownRenderer content={notes} />
        </div>
      )}
    </aside>
  );
};

/** "kwant X.Y.Z is ready" across the top of the Mac app. Renders nothing off the Mac app and on
 * phone-sized screens (the update runs on the Mac, so a phone gets the kwant card in Settings only). */
export const AppUpdateBanner: React.FC = () => {
  const { model, ctrl } = useAppUpdate();
  const [dismissed, setDismissed] = useState(dismissedVersion);
  const [phone, setPhone] = useState(() => !!window.matchMedia?.(PHONE_QUERY).matches);

  useEffect(() => {
    const mq = window.matchMedia?.(PHONE_QUERY);
    const onMq = () => setPhone(!!mq?.matches);
    mq?.addEventListener('change', onMq);
    return () => mq?.removeEventListener('change', onMq);
  }, []);

  const updated = model.phase === 'updated';
  useEffect(() => {
    if (!updated) return;
    const t = window.setTimeout(() => ctrl.clearUpdated(), UPDATED_NOTE_MS);
    return () => window.clearTimeout(t);
  }, [updated, ctrl]);

  const view = bannerView(model, dismissed);
  if (!view || phone) return null;
  return (
    <AppUpdateBannerView
      key={`${view.kind}-${view.version}`}
      view={view}
      onUpdate={() => void ctrl.install()}
      onWhatsNew={view.kind === 'updated' && hasWhatsNew(view.version) ? () => openWhatsNew(view.version) : undefined}
      onDismiss={() => {
        if (view.kind === 'updated') return ctrl.clearUpdated();
        dismissVersion(view.version);
        setDismissed(view.version);
      }}
    />
  );
};
