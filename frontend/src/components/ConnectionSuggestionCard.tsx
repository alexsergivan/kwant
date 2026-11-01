import React, { useEffect, useRef, useState } from 'react';
import { AlertTriangle, CheckCircle2, ExternalLink, Link2, Loader2, Plug } from 'lucide-react';
import { ModernAvatar } from './ModernAvatar';
import { inDesktopApp } from '../externalLinks';
import {
  CardState,
  ConnectApi,
  ConnectionSuggestionMeta,
  connectedNow,
  defaultConnectApi,
  initialState,
  startConnect,
} from '../connectionSuggestions';

const focusRing = 'focus-visible:outline focus-visible:outline-2 focus-visible:outline-offset-2';
const mainBtn = `inline-flex items-center gap-1.5 min-h-[40px] sm:min-h-[34px] px-3.5 rounded-xl text-sm font-medium bg-sky-600 text-white hover:bg-sky-500 disabled:opacity-60 ${focusRing} focus-visible:outline-sky-300`;
const quietBtn = `inline-flex items-center gap-1.5 min-h-[40px] sm:min-h-[34px] px-3 rounded-xl text-sm text-zinc-300 border border-zinc-700 hover:bg-zinc-800 ${focusRing} focus-visible:outline-sky-400`;

export interface ConnectionSuggestionViewProps {
  meta: ConnectionSuggestionMeta;
  messageId: string;
  kwantName: string;
  kwantId?: string;
  state: CardState;
  /** The Mac app: sign-ins open in the default browser, not a new tab. */
  mac?: boolean;
  onConnect: () => void;
  onOpenConnections?: () => void;
  flash?: boolean;
}

/** The card itself, drawn from its state; [`ConnectionSuggestionCard`] holds the state. */
export const ConnectionSuggestionView: React.FC<ConnectionSuggestionViewProps> = ({
  meta,
  messageId,
  kwantName,
  kwantId,
  state,
  mac = false,
  onConnect,
  onOpenConnections,
  flash,
}) => {
  const headingId = `conn-s-${messageId}`;
  const { phase } = state;
  const where = mac ? 'your browser' : 'the new tab';
  const border =
    phase === 'connected' ? 'border-emerald-500/30' : phase === 'error' ? 'border-rose-500/40' : phase === 'missing' ? 'border-amber-500/30' : 'border-sky-500/35';
  const again = (
    <button type="button" onClick={onConnect} className={quietBtn} data-connect-again>
      <Link2 size={13} aria-hidden="true" /> Open the sign-in again
    </button>
  );

  return (
    <section
      data-message-id={messageId}
      data-connection-card={phase}
      aria-labelledby={headingId}
      className={`flex items-start gap-3.5 w-full max-w-5xl rounded-xl ${flash ? 'kw-flash' : ''}`}
    >
      <ModernAvatar role={kwantName} name={kwantName} size={34} kwantId={kwantId} />
      <div className={`flex-1 min-w-0 max-w-2xl rounded-2xl rounded-tl-sm border bg-zinc-900/60 shadow-sm ${border}`}>
        <div className="px-4 pt-3 pb-2 space-y-1.5">
          <span className="flex items-center gap-1.5 text-[11px] font-medium text-zinc-400">
            <Plug size={12} className="text-sky-300" aria-hidden="true" />
            <span className="text-zinc-200">{kwantName}</span> suggests a connection
          </span>
          <h3 id={headingId} className="text-sm font-semibold text-zinc-100 break-words leading-snug">
            {meta.name}
          </h3>
          {meta.reason && (
            <p className="text-sm text-zinc-300 break-words leading-relaxed">
              <span className="text-zinc-500">Why: </span>
              {meta.reason}
            </p>
          )}
        </div>

        <div className="px-4 pb-3 space-y-2 text-sm" aria-live="polite">
          {phase === 'connected' ? (
            <p className="flex items-start gap-1.5 text-emerald-200" data-connected>
              <CheckCircle2 size={15} className="mt-0.5 shrink-0" aria-hidden="true" />
              <span>
                <span className="font-medium">Connected.</span> {kwantName} can use {meta.name} from its next turn.
              </span>
            </p>
          ) : phase === 'missing' ? (
            <>
              <p className="flex items-start gap-1.5 text-amber-200 break-words">
                <AlertTriangle size={14} className="mt-0.5 shrink-0" aria-hidden="true" />
                <span>{state.message}</span>
              </p>
              {onOpenConnections && (
                <button type="button" onClick={onOpenConnections} className={mainBtn} data-open-connections>
                  <Plug size={13} aria-hidden="true" /> Open Connections
                </button>
              )}
            </>
          ) : phase === 'waiting' || phase === 'unchecked' ? (
            <>
              <p className="flex items-start gap-1.5 text-zinc-300 break-words">
                {phase === 'waiting' && <Loader2 size={14} className="mt-0.5 shrink-0 animate-spin motion-reduce:animate-none text-sky-300" aria-hidden="true" />}
                <span>
                  {state.link
                    ? `Your browser blocked the new tab. Open the sign-in to connect ${meta.name}.`
                    : phase === 'waiting'
                      ? `Finish signing in to ${meta.name} in ${where}. This updates by itself when you’re done.`
                      : `Finish signing in to ${meta.name} in ${where}. Once you have, its tools reach your bots from their next turn.`}
                </span>
              </p>
              <div className="flex flex-wrap items-center gap-2">
                {state.link ? (
                  <a href={state.link} target="_blank" rel="noopener noreferrer" className={mainBtn} data-sign-in-link>
                    Open the sign-in <ExternalLink size={13} aria-hidden="true" />
                    <span className="sr-only">(opens in a new tab)</span>
                  </a>
                ) : (
                  again
                )}
              </div>
            </>
          ) : phase === 'timed_out' ? (
            <>
              <p className="text-zinc-300 break-words">
                The sign-in hasn’t finished yet. If you closed it, open it again; if you finished, the connection shows in Connections.
              </p>
              <div className="flex flex-wrap items-center gap-2">
                {again}
                {onOpenConnections && (
                  <button type="button" onClick={onOpenConnections} className={quietBtn}>
                    Open Connections
                  </button>
                )}
              </div>
            </>
          ) : (
            <>
              {phase === 'error' && (
                <p role="alert" className="flex items-start gap-1.5 rounded-lg border border-rose-500/30 bg-rose-500/10 px-2.5 py-1.5 text-xs text-rose-200 break-words">
                  <AlertTriangle size={13} className="mt-px shrink-0" aria-hidden="true" />
                  <span>{state.message}</span>
                </p>
              )}
              <div className="flex flex-wrap items-center gap-2">
                <button type="button" onClick={onConnect} disabled={phase === 'opening'} className={mainBtn} data-connect>
                  {phase === 'opening' ? <Loader2 size={13} className="animate-spin motion-reduce:animate-none" aria-hidden="true" /> : <Plug size={13} aria-hidden="true" />}
                  {phase === 'opening' ? 'Opening the sign-in…' : phase === 'error' ? 'Try again' : 'Connect'}
                </button>
              </div>
              <p className="text-[11px] text-zinc-500 break-words">
                You sign in on {meta.name}’s own page, through Composio, which keeps the sign-in on its servers. Nothing connects until you do.
              </p>
            </>
          )}
        </div>
      </div>
    </section>
  );
};

export interface ConnectionSuggestionCardProps {
  meta: ConnectionSuggestionMeta;
  messageId: string;
  kwantName: string;
  kwantId?: string;
  onOpenConnections?: () => void;
  flash?: boolean;
  api?: ConnectApi;
}

/** A bot's offer to connect an app (message meta kind "connection_suggestion") with one-tap Connect. */
export const ConnectionSuggestionCard: React.FC<ConnectionSuggestionCardProps> = ({ api = defaultConnectApi, ...props }) => {
  const [state, setState] = useState<CardState>(() => initialState(props.meta));
  const stopRef = useRef<(() => void) | null>(null);
  const mac = inDesktopApp();

  // An app connected earlier (or from another card) shows as connected when the card appears.
  useEffect(() => {
    if (props.meta.composio !== 'ready') return;
    let live = true;
    connectedNow(api, props.meta.app).then((c) => {
      if (live && c) setState((s) => (s.phase === 'ready' ? { phase: 'connected' } : s));
    });
    return () => {
      live = false;
    };
  }, [api, props.meta.app, props.meta.composio]);

  useEffect(() => () => stopRef.current?.(), []);

  const connect = () => {
    stopRef.current?.();
    stopRef.current = startConnect(props.meta.app, setState, { api, mac });
  };

  return <ConnectionSuggestionView {...props} state={state} mac={mac} onConnect={connect} />;
};
