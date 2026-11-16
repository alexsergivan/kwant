import React, { useEffect, useRef, useState } from 'react';
import { AlertTriangle, CheckCircle2, KeyRound, RefreshCw } from 'lucide-react';
import { parseMetaObject } from '../questions';

/** A turn nothing could run (message meta kind `setup_needed`, from src/drivers/engines.rs).
 *  `ollama_down`: the bot is pinned to Ollama and Ollama isn't answering at `url`. */
export interface SetupMeta {
  reason: 'no_account' | 'host_off' | 'ollama_down';
  url?: string;
}

export function parseSetupMeta(raw: unknown): SetupMeta | null {
  const m = parseMetaObject(raw);
  if (!m || m.kind !== 'setup_needed') return null;
  if (m.reason === 'ollama_down') return { reason: 'ollama_down', url: typeof m.url === 'string' && m.url ? m.url : undefined };
  return { reason: m.reason === 'host_off' ? 'host_off' : 'no_account' };
}

/** The `ollama_down` detail line; `stillDown` once a Re-check has found Ollama still not answering. */
export function ollamaDownDetail(url: string | undefined, stillDown: boolean): string {
  return `Ollama ${stillDown ? 'still ' : ''}isn't answering at ${url ?? 'the address in Settings'}. Start it, then try again.`;
}

interface Props {
  meta: SetupMeta;
  botName: string;
  spaceName: string;
  /** Whether something can run now; once it can, the card says to send the message again. */
  canRun: boolean | null;
  onOpenAccounts: () => void;
  /** Whether Ollama answers now (null while unknown): an `ollama_down` card is fixed only by Ollama. */
  ollamaReady?: boolean | null;
  /** Checks the engines again (an `ollama_down` card's button). An `ollama_down` card never offers
   *  Accounts: an account can't start Ollama. */
  onRecheck?: () => void;
  rechecking?: boolean;
  flash?: boolean;
}

/** Shown in place of the bot's reply when no account could run its turn, with a way to fix it. */
export const SetupNeededCard: React.FC<Props> = ({ meta, botName, spaceName, canRun, onOpenAccounts, ollamaReady, onRecheck, rechecking, flash }) => {
  const ollama = meta.reason === 'ollama_down';
  const fixed = ollama ? ollamaReady === true : canRun === true;
  // A Re-check that ends with Ollama still down says so, or the press looks like it did nothing.
  const [stillDown, setStillDown] = useState(false);
  const wasRechecking = useRef(rechecking);
  useEffect(() => {
    if (wasRechecking.current && !rechecking && ollamaReady !== true) setStillDown(true);
    wasRechecking.current = rechecking;
  }, [rechecking, ollamaReady]);
  const detail = fixed
    ? ollama
      ? 'Ollama is answering now. Send your message again to get a reply.'
      : `An account works in ${spaceName} now. Send your message again to get a reply.`
    : ollama
      ? ollamaDownDetail(meta.url, stillDown)
      : meta.reason === 'host_off'
        ? `This computer's AI accounts are off in ${spaceName} and no other account is on. Turn one on, or add an account.`
        : `No account can run bots in ${spaceName} yet. Sign in with Claude, ChatGPT or Grok, or add an API key.`;
  const recheck = ollama && !!onRecheck;
  return (
    <div className={`flex items-start gap-3.5 w-full max-w-5xl rounded-xl ${flash ? 'kw-flash' : ''}`} data-setup-card={meta.reason}>
      <div
        className={`w-[34px] h-[34px] flex-shrink-0 rounded-full flex items-center justify-center border ${
          fixed ? 'bg-emerald-500/10 border-emerald-500/40' : 'bg-amber-500/10 border-amber-500/40'
        }`}
        aria-hidden="true"
      >
        {fixed ? <CheckCircle2 size={16} className="text-emerald-300" /> : <AlertTriangle size={16} className="text-amber-300" />}
      </div>
      <div
        role="status"
        className={`min-w-0 flex-1 max-w-xl rounded-2xl rounded-tl-sm border p-4 ${
          fixed ? 'border-zinc-800 bg-zinc-900/60' : 'border-amber-500/30 bg-amber-500/5'
        }`}
      >
        <p className="text-sm font-medium text-zinc-100">{fixed ? `${botName} can answer now` : `${botName} can't answer yet`}</p>
        <p className="mt-1 text-xs text-zinc-400 leading-relaxed">{detail}</p>
        {!fixed && recheck && (
          <button
            type="button"
            onClick={onRecheck}
            disabled={rechecking}
            data-setup-recheck
            className="mt-3 inline-flex items-center gap-1.5 px-3 py-1.5 rounded-lg text-xs font-medium text-white bg-sky-600 hover:bg-sky-500 disabled:opacity-60 focus-visible:outline focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-sky-300"
          >
            <RefreshCw size={13} aria-hidden="true" className={rechecking ? 'animate-spin motion-reduce:animate-none' : undefined} />
            {rechecking ? 'Checking…' : 'Re-check'}
          </button>
        )}
        {!fixed && !ollama && (
          <button
            type="button"
            onClick={onOpenAccounts}
            className="mt-3 inline-flex items-center gap-1.5 px-3 py-1.5 rounded-lg text-xs font-medium text-white bg-sky-600 hover:bg-sky-500 focus-visible:outline focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-sky-300"
          >
            <KeyRound size={13} aria-hidden="true" />
            Add an account
          </button>
        )}
      </div>
    </div>
  );
};
