import React from 'react';
import { ArchiveRestore, Eye, Trash2, TriangleAlert } from 'lucide-react';
import { type Idea, failedLine, peekingText, since } from '../../ideas';
import { BotFace, EditIdea, IdeaMenu, IdeaText, InfoLine, Spinner, btn } from './parts';

export type SlipBusy = 'look' | 'restore' | 'edit' | null;

export interface IdeaSlipProps {
  idea: Idea;
  /** The name for the idea's kwant_id (peeking, or who wrote the take); null when unknown. */
  botName: string | null;
  botNames: string[];
  now: number;
  busy?: SlipBusy;
  /** A 409 or error line under the slip. */
  notice?: { tone: 'sky' | 'rose'; text: string } | null;
  editing?: boolean;
  editError?: string | null;
  onLook?: () => void;
  onEdit?: (from: 'button' | 'menu') => void;
  onSaveEdit?: (text: string) => void;
  onCancelEdit?: () => void;
  onDelete?: () => void;
  onRestore?: () => void;
  menuRef?: React.Ref<HTMLButtonElement>;
}

/** One idea on paper: In the jar (new and failed), Being peeked at, and Later. Never shows `idea.error`. */
export const IdeaSlip: React.FC<IdeaSlipProps> = ({ idea, botName, botNames, now, busy = null, notice, editing = false, editError, onLook, onEdit, onSaveEdit, onCancelEdit, onDelete, onRestore, menuRef }) => {
  const { status } = idea;
  const peeking = status === 'peeking';
  const failed = status === 'failed';
  const later = status === 'later';
  const textId = `idea-${idea.id}-text`;
  const border = peeking ? 'border-[rgb(157_143_255/.45)]' : failed ? 'border-[rgb(253_164_175/.35)]' : 'border-[rgb(63_56_104/.6)]';
  const surface = later ? 'bg-[#1d1840]/[0.35]' : 'bg-[#1d1840]';
  // Busy buttons stay focusable (aria-disabled, M3).
  const off = busy !== null;
  const guard = (fn: (() => void) | undefined) => () => {
    if (!off) fn?.();
  };
  const lookBtn = (
    <button type="button" onClick={guard(onLook)} aria-disabled={off || undefined} className={btn.secondary} data-action="look">
      {busy === 'look' ? <Spinner /> : <Eye size={14} aria-hidden="true" />}
      {busy === 'look' ? 'Asking…' : 'Look now'}
    </button>
  );
  return (
    <article aria-labelledby={textId} className={`kw-slip rounded-xl border ${border} ${surface} px-3.5 py-3 flex flex-col gap-2 min-w-0`} data-idea-id={idea.id} data-status={status}>
      {editing ? (
        <EditIdea initial={idea.text} hasTake={!!idea.take} busy={busy === 'edit'} error={editError ?? null} onSave={(t) => onSaveEdit?.(t)} onCancel={() => onCancelEdit?.()} />
      ) : (
        <div className="flex items-start gap-2 pr-3">
          <div className="flex-1 min-w-0">
            <IdeaText id={textId} text={idea.text} botNames={botNames} className={`text-[15px] leading-[22px] sm:text-[14px] sm:leading-5 ${later ? 'text-[#bcb7de]' : 'text-[#f7f6ff]'}`} />
          </div>
          {!peeking && (
            <IdeaMenu
              label="More for this idea"
              buttonRef={menuRef}
              disabled={busy !== null}
              items={[
                { key: 'edit', label: 'Edit', onSelect: () => onEdit?.('menu') },
                { key: 'delete', label: 'Delete', onSelect: () => onDelete?.(), danger: true },
              ]}
            />
          )}
        </div>
      )}
      {!editing && (
        <div className="flex flex-wrap items-center gap-x-2 gap-y-2 text-[12px] leading-4 text-[#8d87b8]">
          {peeking ? (
            <>
              <BotFace kwantId={idea.kwant_id} name={botName ?? 'Bot'} size={20} peeking />
              <span className="min-w-0 flex-1" data-peeking-text>
                {peekingText(botName, idea.peeked_at, now)}
              </span>
              <button type="button" disabled className={btn.secondary} aria-disabled="true">
                <Spinner />
                Peeking…
              </button>
            </>
          ) : failed ? (
            <>
              <span className="kw-chip kw-chip-bad !text-[11px]" data-failed-chip>
                <TriangleAlert size={12} aria-hidden="true" />
                Couldn’t read this one
              </span>
              <span className="min-w-0 flex-1">Try Look now</span>
              {lookBtn}
            </>
          ) : later ? (
            <>
              <span className="min-w-0 flex-1">{idea.take ? 'Has a take' : droppedText(idea.created_at, now)}</span>
              <button type="button" onClick={guard(onRestore)} aria-disabled={off || undefined} className={btn.secondary} data-action="restore">
                {busy === 'restore' ? <Spinner /> : <ArchiveRestore size={14} aria-hidden="true" />}
                Restore
              </button>
              <button type="button" onClick={guard(onDelete)} aria-disabled={off || undefined} className={btn.quiet} data-action="delete">
                <Trash2 size={14} aria-hidden="true" />
                Delete
              </button>
            </>
          ) : (
            <>
              <span className="min-w-0 flex-1">{droppedText(idea.created_at, now)}</span>
              {lookBtn}
            </>
          )}
        </div>
      )}
      {failed && !editing && (
        <p className="text-[12px] leading-4 text-[#a39dcc]" data-failed-line>
          {failedLine(botName)}
        </p>
      )}
      {notice && (
        <InfoLine tone={notice.tone} className="!text-[12px]">
          {notice.text}
        </InfoLine>
      )}
    </article>
  );
};

export function droppedText(createdAt: string, now: number): string {
  const s = since(createdAt, now);
  if (!s) return 'In the jar';
  return s === 'just now' ? 'Dropped just now' : s === 'yesterday' || /\d{1,2} \w{3}|\w{3} \d/.test(s) ? `Dropped ${s}` : `Dropped ${s} ago`;
}
