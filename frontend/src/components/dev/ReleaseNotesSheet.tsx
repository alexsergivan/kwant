import React, { useRef, useState } from 'react';
import { Check, Copy, Loader2, Sparkles, Trash2, X } from 'lucide-react';
import { MarkdownRenderer } from '../MarkdownRenderer';
import { useEscapeLayer } from '../../escape';
import type { ReleaseNotesSheetProps } from '../../dev/types';

/**
 * The release-notes draft (design §4 "Release notes draft"): a bottom sheet on a phone, a centred panel
 * on desktop. The server answers 202 pending and the ready card comes over SSE, so this shows a waiting
 * state until `card.status` is ready. Save to Notion appears only when the caller passes it (connected).
 */
export const ReleaseNotesSheet: React.FC<ReleaseNotesSheetProps> = ({ card, onSaveToNotion, onDiscard, onClose }) => {
  const rootRef = useRef<HTMLDivElement>(null);
  const [copied, setCopied] = useState(false);
  const [saving, setSaving] = useState<'idle' | 'saving' | 'saved' | 'failed'>('idle');
  const [saveError, setSaveError] = useState<string | null>(null);
  useEscapeLayer(rootRef, true, onClose, { modal: true });

  const ready = card.status === 'ready' && !!card.markdown;
  const labelId = `rn-${card.id}-h`;

  const copy = async () => {
    if (!card.markdown) return;
    try {
      await navigator.clipboard.writeText(card.markdown);
      setCopied(true);
      window.setTimeout(() => setCopied(false), 2000);
    } catch {
      // Clipboard blocked (plain http): the text stays selectable.
    }
  };

  const save = async () => {
    if (!onSaveToNotion || !card.markdown || saving === 'saving') return;
    setSaving('saving');
    setSaveError(null);
    try {
      await onSaveToNotion(card.markdown);
      setSaving('saved');
    } catch (e) {
      setSaving('failed');
      setSaveError(e instanceof Error ? e.message : 'saving didn’t go through');
    }
  };

  return (
    <div
      data-dev-release-notes
      className="kw-dialog kw-dialog-bottom fixed inset-0 z-50 flex items-end justify-center bg-black/70 backdrop-blur-sm sm:items-center sm:p-4"
      onMouseDown={(e) => e.target === e.currentTarget && onClose()}
    >
      <div
        ref={rootRef}
        tabIndex={-1}
        role="dialog"
        aria-modal="true"
        aria-labelledby={labelId}
        className="kw-dialog-panel flex max-h-[92vh] w-full flex-col rounded-t-2xl border border-zinc-800 bg-zinc-900 text-zinc-100 shadow-2xl outline-none sm:max-w-2xl sm:rounded-2xl"
      >
        <div className="flex items-start justify-between gap-3 border-b border-hairline p-4 sm:px-6">
          <div className="min-w-0">
            <h2 id={labelId} className="flex items-center gap-1.5 text-sm font-semibold">
              <Sparkles size={15} aria-hidden="true" className="text-sky-300" />
              Release notes draft
            </h2>
            {card.since_tag && (
              <p className="mt-0.5 text-xs text-zinc-400">
                Since <code className="font-mono text-zinc-300">{card.since_tag}</code>
              </p>
            )}
          </div>
          <button type="button" onClick={onClose} aria-label="Close" className="kw-focus flex h-11 w-11 items-center justify-center rounded-lg text-zinc-400 hover:bg-zinc-800 hover:text-zinc-100 sm:h-8 sm:w-8">
            <X size={18} aria-hidden="true" />
          </button>
        </div>

        <div className="min-h-0 flex-1 overflow-y-auto p-4 sm:px-6" aria-live="polite" aria-busy={!ready || undefined}>
          {card.status === 'pending' && (
            <p className="flex items-center gap-2 text-meta text-zinc-400">
              <Loader2 size={14} aria-hidden="true" className="animate-spin motion-reduce:animate-none" />
              Writing the notes from what landed…
            </p>
          )}
          {card.error && (
            <p role="alert" className="text-meta text-bad">
              Couldn’t write the notes: {card.error}
            </p>
          )}
          {ready && (
            <>
              {card.whats_new && (
                <div className="mb-3 rounded-xl border border-hairline bg-surface-main p-3">
                  <p className="text-2xs font-semibold uppercase tracking-wider text-zinc-400">What’s new</p>
                  <p className="mt-1 text-body text-zinc-100 [overflow-wrap:anywhere]">{card.whats_new}</p>
                </div>
              )}
              <MarkdownRenderer content={card.markdown!} />
            </>
          )}
        </div>

        <div className="flex flex-wrap items-center gap-2 border-t border-hairline p-4 sm:px-6">
          <button type="button" onClick={copy} disabled={!ready} className="kw-btn-primary min-h-[44px] sm:min-h-0 max-sm:flex-1">
            {copied ? <Check size={14} aria-hidden="true" /> : <Copy size={14} aria-hidden="true" />}
            {copied ? 'Copied' : 'Copy'}
          </button>
          {onSaveToNotion && (
            <button type="button" onClick={save} disabled={!ready || saving === 'saving' || saving === 'saved'} aria-busy={saving === 'saving' || undefined} className="kw-btn-ghost min-h-[44px] sm:min-h-0 max-sm:flex-1">
              {saving === 'saved' ? 'Saved to Notion' : saving === 'saving' ? 'Saving…' : 'Save to Notion'}
            </button>
          )}
          <button type="button" onClick={onDiscard} className="kw-btn-ghost min-h-[44px] text-zinc-400 sm:ml-auto sm:min-h-0 max-sm:flex-1">
            <Trash2 size={14} aria-hidden="true" />
            Discard
          </button>
          <span className="sr-only" aria-live="polite">
            {copied ? 'Copied to the clipboard' : ''}
          </span>
          {saveError && (
            <p role="alert" className="w-full text-meta text-bad">
              Couldn’t save to Notion: {saveError}
            </p>
          )}
        </div>
      </div>
    </div>
  );
};
