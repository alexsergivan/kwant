// 33k long-chat-loads-earlier: the top of a chat's message list. While earlier messages exist, a quiet full-width
// "Earlier messages" button loads them (on click, or by itself once it scrolls into view; ChatView watches it). At
// the chat's true start a quiet line says so, with the date of the first message.
import React from 'react';
import { Loader2 } from 'lucide-react';
import { dateText } from '../clock';

export type EarlierState = 'more' | 'loading' | 'start';

export const EarlierButton = React.forwardRef<HTMLButtonElement, { loading: boolean; onLoad: () => void }>(({ loading, onLoad }, ref) => (
  <div data-earlier-messages="" className="mx-auto mb-1 flex w-full max-w-4xl justify-center">
    <button
      ref={ref}
      type="button"
      // Stays focusable while it loads (aria-disabled, not disabled), so keyboard focus never drops to the page.
      aria-disabled={loading || undefined}
      onClick={() => {
        if (!loading) onLoad();
      }}
      className="flex w-full min-h-[44px] sm:min-h-9 items-center justify-center gap-1.5 rounded-xl border border-dashed border-zinc-700/70 px-3 text-xs text-zinc-300 hover:bg-zinc-800/50 hover:text-zinc-100 aria-disabled:cursor-default aria-disabled:hover:bg-transparent transition-colors motion-reduce:transition-none focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-sky-400"
    >
      {loading && <Loader2 size={12} className="animate-spin motion-reduce:animate-none" aria-hidden="true" />}
      {loading ? 'Loading earlier messages' : 'Earlier messages'}
    </button>
  </div>
));
EarlierButton.displayName = 'EarlierButton';

/** "Start of your chat with Ada · Oct 3, 2026". Focusable from script only, so focus can land here from the button. */
export const ChatStartLine = React.forwardRef<HTMLParagraphElement, { title: string; firstAt: string | null | undefined }>(({ title, firstAt }, ref) => {
  const date = dateText(firstAt, { year: true });
  return (
    <p ref={ref} tabIndex={-1} data-chat-start="" className="mx-auto mb-1 w-full max-w-4xl px-3 py-2 text-center text-xs text-zinc-400 break-words focus:outline-none">
      {`Start of your chat with ${title}${date ? ` · ${date}` : ''}`}
    </p>
  );
});
ChatStartLine.displayName = 'ChatStartLine';
