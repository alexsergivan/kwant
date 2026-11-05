import React, { useId, useState } from 'react';
import { ChevronRight } from 'lucide-react';
import { localClock } from '../clock';
import { RouteInfo, useRouteChip } from './RouteChip';
import { EngineChip } from './Signature';

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
  /** One muted line under the name, e.g. "Claude is out until 03:00 · answered on Codex (GPT-6.1)". */
  note?: string | null;
  /** Why this bot answered: a chip beside the name that opens the full reason (and "Wrong bot?"). */
  route?: RouteInfo | null;
  /** Set after a failover: the engines the reply moved between, with the reason on hover. */
  handoff?: { from: string; to: string; why: string } | null;
  now?: number;
  /** Steps and the reasoning item; shown with the facts when Details is open. */
  children?: React.ReactNode;
}

/** The quiet line for a reply that ran on another account. It wraps rather than clips, so it fits at 390 px. */
export const FailoverNote: React.FC<{ text: string }> = ({ text }) => (
  <p data-failover-note="" className="-mt-0.5 mb-1.5 max-w-full text-xs leading-snug text-zinc-500 break-words">
    {text}
  </p>
);

const hasContent = (n: React.ReactNode) => React.Children.toArray(n).length > 0;

/** A bot reply's header: one line, "Name · 3:42 PM", and a Details toggle holding the rest. */
export const ReplyMeta: React.FC<ReplyMetaProps> = ({ name, at, avatar, ranOn, account, tokens, failed = false, note, route, handoff, now, children }) => {
  const [open, setOpen] = useState(false);
  const why = useRouteChip(route);
  const panelId = useId();
  const time = localClock(at, now);
  const facts: Array<[string, React.ReactNode]> = [];
  if (ranOn) facts.push(['Model', ranOn]);
  if (account) facts.push(['Account', account]);
  if (tokens) facts.push(['Tokens', <span title={tokens.detail}>{tokens.label}<span className="text-zinc-400"> · {tokens.detail}</span></span>]);
  const hasDetails = facts.length > 0 || hasContent(children);
  return (
    <div className="w-full max-w-full min-w-0">
      <div className="mb-1.5 flex min-w-0 max-w-full flex-nowrap sm:flex-wrap items-center gap-x-2 gap-y-1 text-xs text-zinc-400 font-medium">
        {avatar && <span className="sm:hidden shrink-0 inline-flex">{avatar}</span>}
        <span className="shrink-0 text-[13px] font-semibold text-zinc-50">{name}</span>
        {why.chip}
        {(ranOn || handoff) && (
          <EngineChip label={[ranOn, account].filter(Boolean).join(' · ')} short={ranOn ? ranOn.split(' · ').pop() : null} handoff={handoff} />
        )}
        {time && (
          <span className="shrink-0 whitespace-nowrap text-zinc-400" data-reply-time="">
            <span aria-hidden="true" className="sm:hidden">· </span>
            {time}
          </span>
        )}
        {hasDetails && (
          <button
            type="button"
            onClick={() => setOpen((o) => !o)}
            aria-expanded={open}
            aria-controls={open ? panelId : undefined}
            className={`shrink-0 inline-flex items-center gap-0.5 rounded-md px-1.5 min-h-6 text-xs hover:text-zinc-100 hover:bg-zinc-800 transition-colors motion-reduce:transition-none focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-sky-400 ${
              failed && !open ? 'text-rose-300' : 'text-zinc-400'
            }`}
          >
            {failed && !open ? 'Details · a step failed' : 'Details'}
            <ChevronRight size={12} className={`transition-transform motion-reduce:transition-none ${open ? 'rotate-90' : ''}`} aria-hidden="true" />
          </button>
        )}
      </div>
      {why.panel}
      {note && <FailoverNote text={note} />}
      {open && hasDetails && (
        <div id={panelId} className="mb-2 w-full max-w-2xl">
          {facts.length > 0 && (
            <dl className="mb-2 grid grid-cols-[auto_1fr] gap-x-3 gap-y-0.5 text-xs">
              {facts.map(([k, v]) => (
                <React.Fragment key={k}>
                  <dt className="text-zinc-400">{k}</dt>
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
