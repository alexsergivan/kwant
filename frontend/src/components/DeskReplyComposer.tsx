// desk-host-reply-composer (front desk fix #3): in a front desk guest's thread the composer is "Reply to Tom".
// It sends with pocketApi.deskReply (the guest page shows the line as the host's and the guest's wait clears),
// never a bot turn. The server saves each reply into this thread (meta `to_guest`), which ChatView draws as the sent
// line ("You → Tom · 4:31 PM"); until it lands, a local echo stands in. `?view=guest:<id>` says if the guest is gone.
import React, { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import { ArrowUp, BellRing, CircleSlash, Send } from 'lucide-react';
import { pocketApi, PocketApiError } from '../pocketApi';
import { deskReplyNote, goneError, pocketEnded, sentLabel, handoffChipText, type DeskGuest, type DeskSentLine } from '../deskThread';

export type DeskGone = 'ended' | 'left' | null;
const NO_LINES: DeskSentLine[] = [];

export interface DeskThreadState {
  /** The local echo of a reply on its way, until the server's saved copy is in the thread. */
  lines: DeskSentLine[];
  gone: DeskGone;
  send: (text: string) => Promise<boolean>;
  sending: boolean;
  error: string | null;
}

/** Reads whether the guest can still be reached when the thread opens and whenever its messages change; `send` replies. */
export function useDeskThread(guest: DeskGuest | null, messageCount: number): DeskThreadState {
  const code = guest?.code ?? null;
  const guestId = guest?.guestId ?? null;
  const [echo, setEcho] = useState<DeskSentLine | null>(null);
  const [gone, setGone] = useState<DeskGone>(null);
  const [sending, setSending] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const live = useRef(0);

  const goneFrom = (e: unknown): DeskGone => (e instanceof PocketApiError && e.code === 'no_such_player' ? 'left' : 'ended');

  const load = useCallback(async () => {
    if (!code || !guestId) return;
    const mine = ++live.current;
    try {
      const read = await pocketApi.deskGuest(code, guestId);
      if (mine !== live.current) return;
      setGone(pocketEnded(read.pocket) ? 'ended' : null);
    } catch (e) {
      if (mine !== live.current) return;
      // Only a gone pocket or guest changes the composer; a dropped connection keeps it (the send says so).
      if (goneError(e)) setGone(goneFrom(e));
    }
  }, [code, guestId]);

  // A different thread: start clean.
  useEffect(() => {
    setEcho(null);
    setGone(null);
    setError(null);
  }, [code, guestId]);

  useEffect(() => {
    void load();
  }, [load, messageCount]);

  // The saved reply is in the thread (a new message): the sent echo steps aside for it.
  const seen = useRef(messageCount);
  const count = useRef(messageCount);
  count.current = messageCount;
  useEffect(() => {
    if (messageCount === seen.current) return;
    seen.current = messageCount;
    setEcho((e) => (e && !e.sending ? null : e));
  }, [messageCount]);

  const send = useCallback(
    async (text: string) => {
      const body = text.trim();
      if (!code || !guestId || !body || sending) return false;
      const at = new Date().toISOString();
      const before = count.current;
      setEcho({ id: `echo-${at}`, text: body, at, asked_at: at, sending: true });
      setSending(true);
      setError(null);
      try {
        const read = await pocketApi.deskReply(code, guestId, body);
        // The saved copy may already be here (its event can beat this answer).
        setEcho(count.current !== before ? null : { id: `echo-${at}`, text: body, at, asked_at: at });
        if (pocketEnded(read.pocket)) setGone('ended');
        void load();
        return true;
      } catch (e) {
        setEcho(null);
        if (goneError(e)) setGone(goneFrom(e));
        else setError(e instanceof Error && e.message ? e.message : 'That didn’t send. Try again.');
        return false;
      } finally {
        setSending(false);
      }
    },
    [code, guestId, sending, load],
  );

  // One array per echo, never a new one per render: ChatView's rows and its bottom pin key off it.
  const lines = useMemo(() => (echo ? [echo] : NO_LINES), [echo]);
  return { lines, gone, send, sending, error };
}

/** One line the host sent the guest, on the host's side of the thread. */
export const DeskSentRow: React.FC<{ line: DeskSentLine; guestName: string }> = ({ line, guestName }) => (
  <div className="w-full max-w-5xl ml-auto flex flex-col items-end gap-1.5" data-desk-sent={line.sending ? 'sending' : 'sent'}>
    <div className="flex items-center gap-1.5 text-[11px] font-medium text-zinc-400">
      <Send size={11} aria-hidden="true" className="text-info" />
      <span>{line.sending ? `Sending to ${guestName}…` : sentLabel(guestName, line.at)}</span>
    </div>
    <div
      className={`max-w-[85%] sm:max-w-2xl px-3.5 py-3 sm:px-5 sm:py-4 rounded-2xl rounded-tr-sm border border-sky-700/50 bg-sky-950/40 text-sm leading-relaxed text-zinc-100 whitespace-pre-wrap break-words shadow-sm${
        line.sending ? ' opacity-70' : ''
      }`}
    >
      {line.text}
    </div>
  </div>
);

/** The bot's hand-off, in place of the raw `[[person]] …` line: the desk card's and Inbox row's words. */
export const DeskHandoffChip: React.FC<{ reason: string }> = ({ reason }) => (
  <span className="kw-chip mt-2 max-w-full whitespace-normal" data-desk-handoff="">
    <BellRing size={12} aria-hidden="true" className="shrink-0" />
    <span className="min-w-0">{handoffChipText(reason)}</span>
  </span>
);

/** "Reply to Tom": the composer of a front desk guest's thread. */
/** `onSend`: the host's own reply, so the chat follows it to the bottom (as a send in the bot's composer does). */
export const DeskReplyComposer: React.FC<{ guest: DeskGuest; bot: string; desk: DeskThreadState; phone?: boolean; onSend?: () => void }> = ({ guest, bot, desk, phone, onSend }) => {
  const [text, setText] = useState('');
  const ref = useRef<HTMLTextAreaElement>(null);
  const name = guest.guestName;

  useEffect(() => {
    const el = ref.current;
    if (!el) return;
    el.style.height = 'auto';
    el.style.height = `${Math.min(el.scrollHeight, 200)}px`;
  }, [text]);

  if (desk.gone || !guest.code || !guest.guestId) {
    const words = desk.gone === 'left' ? `${name} has left this desk, so a reply can’t reach them.` : desk.gone === 'ended' ? 'This desk has ended, so a reply can’t reach the guest.' : `Replies open once ${name}’s message is here.`;
    return (
      <p className="max-w-4xl mx-auto flex items-center gap-2 px-3 py-3 rounded-2xl border border-hairline bg-surface-raised text-sm text-zinc-400" data-desk-composer="closed" role="status">
        <CircleSlash size={14} aria-hidden="true" className="shrink-0" />
        <span className="min-w-0">{words}</span>
      </p>
    );
  }

  const canSend = !!text.trim() && !desk.sending;
  const submit = async () => {
    if (!canSend) return;
    onSend?.();
    if (await desk.send(text)) {
      setText('');
      ref.current?.focus();
    }
  };

  return (
    <div className="max-w-4xl mx-auto" data-desk-composer="open">
      <p className="mb-1.5 px-1 text-meta text-zinc-400" id="kw-desk-reply-note" data-desk-note="">
        {deskReplyNote(name, bot)}
      </p>
      <form
        aria-label={`Reply to ${name}`}
        onSubmit={(e) => {
          e.preventDefault();
          void submit();
        }}
        className="flex items-end gap-1.5 border rounded-2xl p-2 bg-surface-raised shadow-raised border-hairline-strong focus-within:border-sky-400/50 transition-colors"
      >
        <textarea
          ref={ref}
          data-desk-reply=""
          rows={1}
          value={text}
          onChange={(e) => setText(e.target.value)}
          onKeyDown={(e) => {
            if (e.key === 'Enter' && !e.shiftKey && !e.nativeEvent.isComposing) {
              e.preventDefault();
              void submit();
            }
          }}
          placeholder={`Reply to ${name}`}
          aria-label={`Reply to ${name}`}
          aria-describedby={desk.error ? 'kw-desk-reply-note kw-desk-reply-error' : 'kw-desk-reply-note'}
          aria-invalid={desk.error ? true : undefined}
          className={`flex-1 min-w-0 resize-none bg-transparent px-2.5 py-2 ${phone ? 'text-base' : 'text-sm'} leading-6 text-zinc-100 placeholder:text-zinc-400 focus:outline-none`}
        />
        <button
          type="submit"
          disabled={!canSend}
          aria-label={desk.sending ? `Sending to ${name}` : `Send to ${name}`}
          data-send-ready={canSend ? 'true' : 'false'}
          className={`w-11 h-11 sm:w-9 sm:h-9 rounded-xl flex items-center justify-center shrink-0 focus-visible:outline focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-sky-300 transition-[transform,filter] duration-150 motion-reduce:transition-none ${
            canSend ? 'bg-accent-gradient kw-hover-deepen text-white shadow-glow active:scale-95 motion-reduce:active:scale-100' : 'bg-surface-overlay text-zinc-400 cursor-not-allowed'
          }`}
        >
          <ArrowUp size={17} strokeWidth={2.4} aria-hidden="true" />
        </button>
      </form>
      {desk.error && (
        <p id="kw-desk-reply-error" role="alert" className="mt-1.5 px-1 text-meta text-rose-300" data-desk-error="">
          {desk.error}
        </p>
      )}
    </div>
  );
};
