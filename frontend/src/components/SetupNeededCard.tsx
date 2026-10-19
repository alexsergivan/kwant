import React from 'react';
import { AlertTriangle, CheckCircle2, KeyRound } from 'lucide-react';
import { parseMetaObject } from '../questions';

/** A turn nothing could run (message meta kind `setup_needed`, from src/drivers/engines.rs). */
export interface SetupMeta {
  reason: 'no_account' | 'host_off';
}

export function parseSetupMeta(raw: unknown): SetupMeta | null {
  const m = parseMetaObject(raw);
  if (!m || m.kind !== 'setup_needed') return null;
  return { reason: m.reason === 'host_off' ? 'host_off' : 'no_account' };
}

interface Props {
  meta: SetupMeta;
  botName: string;
  spaceName: string;
  /** Whether something can run now; once it can, the card says to send the message again. */
  canRun: boolean | null;
  onOpenAccounts: () => void;
  flash?: boolean;
}

/** Shown in place of the bot's reply when no account could run its turn, with a way to fix it. */
export const SetupNeededCard: React.FC<Props> = ({ meta, botName, spaceName, canRun, onOpenAccounts, flash }) => {
  const fixed = canRun === true;
  const detail = fixed
    ? `An account works in ${spaceName} now. Send your message again to get a reply.`
    : meta.reason === 'host_off'
      ? `This machine is off in ${spaceName} and no other account is on. Turn one on, or add an account.`
      : `No account can run bots in ${spaceName} yet. Sign in with Claude, Codex or Grok, or add an API key.`;
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
        {!fixed && (
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
