// Telegram in the chat (design G4): where a message or a decision came from.
import React from 'react';
import { CheckCircle2, Send, ShieldCheck } from 'lucide-react';
import { localClock } from '../clock';

/** The line the chat keeps for an approval allowed in Telegram. No time: kwant stores none. */
export const ALLOWED_FROM_TELEGRAM = '✓ Allowed once from Telegram';

/** "via Telegram": after the time on a message the person sent from Telegram. */
export const ViaTelegram: React.FC = () => (
  <span className="tg-via" data-via-telegram="">
    <Send size={11} aria-hidden="true" />
    via Telegram
  </span>
);

/** "· sent to Telegram too" in a bot reply's time line; just the mark on a phone, where the line is one row. */
export const SentToTelegram: React.FC = () => (
  <span className="shrink-0 inline-flex items-center gap-1 whitespace-nowrap text-zinc-400" data-sent-telegram="">
    <span aria-hidden="true">·</span>
    <span className="max-sm:hidden">sent to Telegram too</span>
    <Send size={11} className="sm:hidden" aria-hidden="true" />
    <span className="sr-only sm:hidden">sent to Telegram too</span>
  </span>
);

/**
 * An approval allowed in Telegram, kept in the chat: "Drafter wanted to use Bash · ✓ Allowed once
 * from Telegram". No time: kwant stores none, so the line reads the same live and after a reload.
 */
export const AllowedFromTelegram: React.FC<{ lead: string }> = ({ lead }) => (
  <div className="tg-done flex items-start gap-2 ml-0 sm:ml-[50px] max-w-3xl px-3 py-2 rounded-xl border border-zinc-800 bg-zinc-900/50 text-xs text-zinc-400" data-resolved-via="telegram">
    <ShieldCheck size={14} className="mt-0.5 shrink-0 text-violet-300" aria-hidden="true" />
    <p className="min-w-0 break-words leading-5">
      {lead} · <b className="font-semibold text-zinc-100">{ALLOWED_FROM_TELEGRAM}</b>
    </p>
  </div>
);

/**
 * A decision that was made in Telegram, kept in the chat: "Drafter asked “Which day?” · You picked
 * Monday · via Telegram · 10:06". `lead` and `outcome` are the record's own words, shown as text.
 */
export const ResolvedViaTelegram: React.FC<{ kind: 'question' | 'approval'; lead: string; outcome: string; at?: string | null }> = ({ kind, lead, outcome, at }) => {
  const time = localClock(at ?? null);
  return (
    <div className="tg-done flex items-start gap-2 ml-0 sm:ml-[50px] max-w-3xl px-3 py-2 rounded-xl border border-zinc-800 bg-zinc-900/50 text-xs text-zinc-400" data-resolved-via="telegram">
      {kind === 'question' ? (
        <CheckCircle2 size={14} className="mt-0.5 shrink-0 text-emerald-400" aria-hidden="true" />
      ) : (
        <ShieldCheck size={14} className="mt-0.5 shrink-0 text-violet-300" aria-hidden="true" />
      )}
      <p className="min-w-0 break-words leading-5">
        {lead} · <b className="font-semibold text-zinc-100">{outcome}</b> <ViaTelegram />
        {time && <> · {time}</>}
      </p>
    </div>
  );
};
