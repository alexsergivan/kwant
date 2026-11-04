import { useEffect, useState } from 'react';
import { Reply, ThumbsDown, ThumbsUp, X } from 'lucide-react';
import { Reaction, ReplyRef, nextReaction, reactionLabels } from '../reactReply';

// Thumbs and Reply on a bot message, the "Replying to" chip in the composer, and the quote in the
// person's bubble. Look and states: react-reply/design.md (Mira); behaviour: contract.md §1 and §3.

/** Ghost buttons, quieter than the Copy chip; 44 px on phones, 36 px from `sm`. */
export const reactClass =
  'inline-flex items-center justify-center gap-1.5 min-h-11 min-w-11 sm:min-h-9 sm:min-w-9 px-2 rounded-lg border border-transparent text-xs font-medium text-zinc-400 hover:text-zinc-100 hover:bg-white/[0.06] focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-sky-400 transition-colors duration-150 motion-reduce:transition-none cursor-pointer';
const SELECTED: Record<Reaction, string> = {
  up: '!text-sky-200 bg-sky-400/15 !border-sky-400/40 hover:!bg-sky-400/20 hover:!text-sky-100',
  down: '!text-zinc-100 bg-white/[0.10] !border-white/20 hover:!bg-white/[0.14]',
};

/** A short note that removes itself, for "Not saved" and "not loaded here". */
function useBriefNote(ms = 4000) {
  const [note, setNote] = useState<{ text: string; n: number } | null>(null);
  useEffect(() => {
    if (!note) return;
    const t = setTimeout(() => setNote(null), ms);
    return () => clearTimeout(t);
  }, [note, ms]);
  return [note?.text ?? null, (text: string) => setNote((p) => ({ text, n: (p?.n ?? 0) + 1 }))] as const;
}

export function ReactionButtons({
  messageId,
  reaction,
  asks,
  onReact,
}: {
  messageId: string;
  reaction: Reaction | null;
  /** This is the bot's newest message and it asks something: a thumb is a yes or no. */
  asks: boolean;
  /** Resolves false when the save failed (the parent has already rolled the reaction back). */
  onReact: (reaction: Reaction | null) => Promise<boolean> | void;
}) {
  // The pop plays on the icon of the thumb just selected; a new key replays it on the next select.
  const [pop, setPop] = useState<{ which: Reaction; n: number } | null>(null);
  const [failed, showFailed] = useBriefNote();
  const labels = reactionLabels(asks);

  const press = async (which: Reaction) => {
    const next = nextReaction(reaction, which);
    if (next) setPop((p) => ({ which: next, n: (p?.n ?? 0) + 1 }));
    else setPop(null);
    const ok = await onReact(next);
    if (ok === false) {
      setPop(null);
      showFailed('Not saved. Try again.');
    }
  };

  const thumb = (which: Reaction) => {
    const on = reaction === which;
    const Icon = which === 'up' ? ThumbsUp : ThumbsDown;
    const popping = pop?.which === which;
    return (
      <button
        type="button"
        aria-pressed={on}
        aria-label={labels[which]}
        title={on ? `${labels[which]} · click to undo` : labels[which]}
        data-reaction={which}
        onClick={() => press(which)}
        className={`${reactClass} ${on ? SELECTED[which] : ''} ${popping ? 'kw-react-pop' : ''} ${which === 'down' ? 'kw-react-down' : ''}`}
      >
        <Icon key={popping ? pop.n : 0} size={15} strokeWidth={2} aria-hidden="true" fill={on ? 'currentColor' : 'none'} />
      </button>
    );
  };

  return (
    <>
      <div role="group" aria-label="React to this reply" className="flex items-center gap-1" data-message-reactions={messageId}>
        {thumb('up')}
        {thumb('down')}
      </div>
      {failed && (
        <span role="status" className="text-meta text-bad">
          {failed}
        </span>
      )}
    </>
  );
}

export function ReplyButton({ botName, active, onReply }: { botName: string; active: boolean; onReply: () => void }) {
  return (
    <button
      type="button"
      onClick={onReply}
      className={`${reactClass} ${active ? '!text-sky-200' : ''}`}
      aria-label={`Reply to ${botName}`}
      title="Reply to this message"
      data-reply-button
    >
      <Reply size={15} strokeWidth={2} aria-hidden="true" />
      <span>Reply</span>
    </button>
  );
}

/** The first row of the composer while a reply is being written. */
export function ReplyChip({
  botName,
  excerpt,
  onOpen,
  onCancel,
}: {
  botName: string;
  excerpt: string;
  onOpen: () => void;
  onCancel: () => void;
}) {
  return (
    <div
      data-reply-chip
      aria-live="polite"
      className="basis-full min-w-0 flex items-center gap-2 pl-2.5 pr-1 py-1 rounded-xl border-l-2 border-sky-400 bg-white/[0.04] kw-fade-in motion-reduce:animate-none"
    >
      <Reply size={13} aria-hidden="true" className="shrink-0 text-sky-300" />
      <button
        type="button"
        onClick={onOpen}
        title={excerpt}
        className="min-w-0 flex-1 flex items-baseline gap-1 text-left text-meta rounded focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-sky-400 cursor-pointer"
      >
        <span className="shrink-0 text-zinc-400">Replying to</span>
        <span className="shrink-0 max-w-[40%] truncate font-semibold text-zinc-100">{botName}:</span>
        <span className="min-w-0 truncate text-zinc-300">{excerpt}</span>
      </button>
      <button
        type="button"
        onClick={onCancel}
        aria-label="Cancel reply"
        title="Cancel reply (Esc)"
        className="shrink-0 flex items-center justify-center h-11 w-11 sm:h-7 sm:w-7 rounded-lg text-zinc-400 hover:text-zinc-100 hover:bg-white/[0.08] focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-sky-400 cursor-pointer"
      >
        <X size={14} aria-hidden="true" />
      </button>
    </div>
  );
}

const quoteBox = 'mb-2 flex w-full min-w-0 items-center gap-1.5 rounded-lg border-l-2 border-sky-400/70 bg-white/[0.04] pl-2 pr-2.5 py-1 text-left text-meta';

/** The one-line quote at the top of the person's bubble. */
export function ReplyQuote({
  reply,
  botName,
  onOpen,
}: {
  reply: ReplyRef;
  botName: string;
  /** False when the original isn't loaded in this chat; the quote then says so instead of scrolling. */
  onOpen: () => boolean;
}) {
  const [note, showNote] = useBriefNote();
  if (reply.missing)
    return (
      <div data-reply-quote={reply.id} className={quoteBox}>
        <Reply size={12} aria-hidden="true" className="shrink-0 text-sky-300" />
        <span className="italic text-zinc-300">Original message was deleted</span>
      </div>
    );
  const excerpt = reply.excerpt ?? '';
  return (
    <>
      <button
        type="button"
        data-reply-quote={reply.id}
        onClick={() => {
          if (!onOpen()) showNote("That message isn't loaded here.");
        }}
        aria-label={`Replying to ${botName}${excerpt ? `: ${excerpt}` : ''}. Show the original`}
        className={`${quoteBox} min-h-11 sm:min-h-0 hover:bg-white/[0.07] focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-sky-400 transition-colors duration-150 motion-reduce:transition-none cursor-pointer`}
      >
        <Reply size={12} aria-hidden="true" className="shrink-0 text-sky-300" />
        <span className="shrink-0 max-w-[40%] truncate font-semibold text-zinc-100">{botName}</span>
        {excerpt && <span className="min-w-0 truncate text-zinc-300">{excerpt}</span>}
      </button>
      {note && (
        <span role="status" className="block -mt-1 mb-2 text-meta text-zinc-300">
          {note}
        </span>
      )}
    </>
  );
}
