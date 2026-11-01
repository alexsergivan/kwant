import React, { useId, useState } from 'react';
import { ChevronRight } from 'lucide-react';
import { localClock } from '../clock';

export interface ReplyFacts {
  /** "Claude · Opus 5.5": the engine and model the reply ran on. */
  ranOn?: string | null;
  account?: string | null;
  tokens?: { label: string; detail: string } | null;
}

export interface ReplyMetaProps extends ReplyFacts {
  name: string;
  /** The message's created_at; no time is shown without one (a reply still streaming). */
  at?: string | null;
  /** Drawn before the name on phones, where the avatar column is hidden. */
  avatar?: React.ReactNode;
  /** Whether a step failed: the Details toggle says so while it's folded. */
  failed?: boolean;
  now?: number;
  /** Steps and the reasoning item; shown with the facts when Details is open. */
  children?: React.ReactNode;
}

const hasContent = (n: React.ReactNode) => React.Children.toArray(n).length > 0;

/** A bot reply's header: one line, "Name · 3:42 PM", and a Details toggle holding the rest. */
export const ReplyMeta: React.FC<ReplyMetaProps> = ({ name, at, avatar, ranOn, account, tokens, failed = false, now, children }) => {
  const [open, setOpen] = useState(false);
  const panelId = useId();
  const time = localClock(at, now);
  const facts: Array<[string, React.ReactNode]> = [];
  if (ranOn) facts.push(['Model', ranOn]);
  if (account) facts.push(['Account', account]);
  if (tokens) facts.push(['Tokens', <span title={tokens.detail}>{tokens.label}<span className="text-zinc-500"> · {tokens.detail}</span></span>]);
  const hasDetails = facts.length > 0 || hasContent(children);
  return (
    <div className="w-full max-w-full min-w-0">
      <div className="flex items-center gap-2 mb-1.5 text-[11px] text-zinc-400 font-medium max-w-full min-w-0 whitespace-nowrap">
        {avatar && <span className="sm:hidden shrink-0 inline-flex">{avatar}</span>}
        <span className="text-zinc-200 shrink-0">{name}</span>
        {time && (
          <span className="text-zinc-500 truncate min-w-0" data-reply-time="">
            <span aria-hidden="true">· </span>
            {time}
          </span>
        )}
        {hasDetails && (
          <button
            type="button"
            onClick={() => setOpen((o) => !o)}
            aria-expanded={open}
            aria-controls={open ? panelId : undefined}
            className={`shrink-0 inline-flex items-center gap-0.5 rounded px-1 -mx-0.5 min-h-[24px] text-[11px] hover:text-zinc-200 hover:bg-zinc-900 transition-colors focus-visible:outline focus-visible:outline-2 focus-visible:outline-sky-400 ${
              failed && !open ? 'text-rose-300' : 'text-zinc-500'
            }`}
          >
            {failed && !open ? 'Details · a step failed' : 'Details'}
            <ChevronRight size={12} className={`transition-transform motion-reduce:transition-none ${open ? 'rotate-90' : ''}`} aria-hidden="true" />
          </button>
        )}
      </div>
      {open && hasDetails && (
        <div id={panelId} className="mb-2 w-full max-w-2xl">
          {facts.length > 0 && (
            <dl className="mb-2 grid grid-cols-[auto_1fr] gap-x-3 gap-y-0.5 text-[11px]">
              {facts.map(([k, v]) => (
                <React.Fragment key={k}>
                  <dt className="text-zinc-500">{k}</dt>
                  <dd className="text-zinc-300 min-w-0 break-words">{v}</dd>
                </React.Fragment>
              ))}
            </dl>
          )}
          {children}
        </div>
      )}
    </div>
  );
};
