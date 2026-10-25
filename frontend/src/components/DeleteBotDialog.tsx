import React, { useEffect, useRef, useState } from 'react';
import { AlertTriangle, Download, Loader2, Trash2 } from 'lucide-react';
import { BotUsage, botDeleteApi, deleteBlockedReason, leadsNote, lossLines } from '../botDelete';
import { downloadExport, trapFocusTarget } from '../imports';

interface DeleteBotDialogProps {
  kwant: { id: string; name: string };
  /** The sidebar's "Working on …" line when the bot is busy. */
  busy?: string | null;
  onClose: () => void;
  /** Called once the server has deleted it. */
  onDeleted: (id: string) => void;
}

const FOCUSABLE = 'button:not([disabled]), a[href]';

/** Confirms deleting a bot: names what goes with it, and offers Export first. */
export const DeleteBotDialog: React.FC<DeleteBotDialogProps> = ({ kwant, busy, onClose, onDeleted }) => {
  const [usage, setUsage] = useState<BotUsage | null>(null);
  const [loaded, setLoaded] = useState(false);
  const [deleting, setDeleting] = useState(false);
  const [exporting, setExporting] = useState(false);
  const [exported, setExported] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const boxRef = useRef<HTMLDivElement>(null);
  const cancelRef = useRef<HTMLButtonElement>(null);

  useEffect(() => {
    let live = true;
    botDeleteApi.usage(kwant.id).then((u) => {
      if (!live) return;
      setUsage(u);
      setLoaded(true);
    });
    return () => {
      live = false;
    };
  }, [kwant.id]);

  // Start on Cancel: the safe choice.
  useEffect(() => cancelRef.current?.focus(), []);

  const blocked = deleteBlockedReason(kwant.name, busy, usage);

  const onKeyDown = (e: React.KeyboardEvent) => {
    if (e.key === 'Escape') {
      e.stopPropagation();
      if (!deleting) onClose();
      return;
    }
    if (e.key !== 'Tab' || !boxRef.current) return;
    const items = Array.from(boxRef.current.querySelectorAll<HTMLElement>(FOCUSABLE));
    const to = trapFocusTarget(items.length, items.indexOf(document.activeElement as HTMLElement), e.shiftKey);
    if (to !== null) {
      e.preventDefault();
      items[to]?.focus();
    }
  };

  const doExport = async () => {
    setExporting(true);
    setError(null);
    try {
      const file = await downloadExport(kwant.id);
      setExported(file);
    } catch (err) {
      setError(err instanceof Error ? err.message : 'Couldn’t export the bot.');
    }
    setExporting(false);
  };

  const doDelete = async () => {
    if (blocked || deleting) return;
    setDeleting(true);
    setError(null);
    const err = await botDeleteApi.remove(kwant.id, kwant.name);
    setDeleting(false);
    if (err) {
      setError(err);
      return;
    }
    onDeleted(kwant.id);
  };

  const btn =
    'inline-flex items-center justify-center gap-1.5 min-h-[40px] sm:min-h-[34px] px-3.5 rounded-lg text-xs font-medium transition-colors disabled:opacity-50 disabled:cursor-not-allowed focus-visible:outline focus-visible:outline-2 focus-visible:outline-offset-2';

  return (
    <div className="kw-dialog kw-dialog-bottom fixed inset-0 z-[60] bg-black/70 backdrop-blur-sm flex items-center justify-center p-4" onKeyDown={onKeyDown}>
      <div
        ref={boxRef}
        role="alertdialog"
        aria-modal="true"
        aria-labelledby="delete-bot-title"
        aria-describedby="delete-bot-desc"
        className="kw-dialog-panel bg-zinc-900 border border-rose-500/30 rounded-2xl max-w-md w-full max-h-[92vh] overflow-y-auto p-4 sm:p-5 text-zinc-100 shadow-2xl space-y-4"
      >
        <div className="flex items-start gap-2.5">
          <div className="w-8 h-8 shrink-0 rounded-lg bg-rose-500/10 border border-rose-500/25 flex items-center justify-center text-rose-300">
            <Trash2 size={15} aria-hidden="true" />
          </div>
          <div className="min-w-0">
            <h3 id="delete-bot-title" className="text-sm font-semibold break-words">
              Delete {kwant.name}?
            </h3>
            <p id="delete-bot-desc" className="text-xs text-zinc-400">
              This removes the bot and everything below for good. It can’t be undone.
            </p>
          </div>
        </div>

        <ul aria-label="What gets deleted" aria-busy={!loaded} className="list-disc pl-5 text-xs text-zinc-200 space-y-0.5">
          {lossLines(usage).map((l) => (
            <li key={l}>{l}</li>
          ))}
        </ul>
        <p className="text-[11px] text-zinc-500">
          {leadsNote(usage) ?? 'Teams it’s in lose it as a member.'} Other bots and their chats stay, and so do its team-chat messages and working folder.
        </p>

        <div className="rounded-xl border border-zinc-800 bg-zinc-950/70 p-3 space-y-2">
          <p className="text-xs text-zinc-300">Want a copy first? Export saves its instructions, memory, skills and routines to a file you can import later.</p>
          <button type="button" onClick={doExport} disabled={exporting || deleting} className={`${btn} text-zinc-100 border border-zinc-700 hover:bg-zinc-800 focus-visible:outline-zinc-400`}>
            {exporting ? <Loader2 size={13} className="animate-spin motion-reduce:animate-none" aria-hidden="true" /> : <Download size={13} aria-hidden="true" />}
            {exporting ? 'Exporting…' : `Export ${kwant.name}`}
          </button>
          {exported && (
            <p role="status" className="text-[11px] text-emerald-300 break-words">
              Saved {exported}.
            </p>
          )}
        </div>

        {blocked && (
          <p role="status" className="flex items-start gap-1.5 text-xs text-amber-200">
            <AlertTriangle size={13} className="mt-px shrink-0" aria-hidden="true" />
            <span>{blocked}</span>
          </p>
        )}
        {error && (
          <p role="alert" className="flex items-start gap-1.5 rounded-lg border border-rose-500/30 bg-rose-500/10 px-2.5 py-1.5 text-xs text-rose-200 break-words">
            <AlertTriangle size={13} className="mt-px shrink-0" aria-hidden="true" />
            <span>{error}</span>
          </p>
        )}

        <div className="flex flex-wrap items-center justify-end gap-2 pt-1">
          <button ref={cancelRef} type="button" onClick={onClose} disabled={deleting} className={`${btn} text-zinc-300 hover:text-zinc-100 hover:bg-zinc-800 focus-visible:outline-zinc-400`}>
            Cancel
          </button>
          <button
            type="button"
            onClick={doDelete}
            disabled={!!blocked || deleting}
            title={blocked ?? undefined}
            className={`${btn} text-white bg-rose-600 hover:bg-rose-500 focus-visible:outline-rose-400`}
          >
            {deleting ? <Loader2 size={13} className="animate-spin motion-reduce:animate-none" aria-hidden="true" /> : <Trash2 size={13} aria-hidden="true" />}
            {deleting ? 'Deleting…' : `Delete ${kwant.name}`}
          </button>
        </div>
      </div>
    </div>
  );
};
