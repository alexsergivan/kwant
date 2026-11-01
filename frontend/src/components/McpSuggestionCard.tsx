import React, { useState } from 'react';
import { AlertTriangle, CheckCircle2, ExternalLink, Globe, KeyRound, Loader2, MinusCircle, Monitor, Plug } from 'lucide-react';
import { ModernAvatar } from './ModernAvatar';
import { Suggestion, SuggestionMeta, failureText, inputKey, inputLabel, installPayload, runsLine, suggestionView } from '../mcpSuggestions';
import { inDesktopApp } from '../externalLinks';

export interface McpSuggestionCardProps {
  meta: SuggestionMeta;
  messageId: string;
  kwantName: string;
  kwantId?: string;
  /** Live record from `mcp_suggestion_updated` or the list, when known; wins over `meta`. */
  live?: Suggestion | null;
  /** Resolves to an error message on failure; `signIn` true when the server still needs a sign-in. */
  onInstall: (suggestionId: string, body: { env?: Record<string, string>; headers?: Record<string, string> }) => Promise<{ error?: string; signIn?: boolean }>;
  onDismiss: (suggestionId: string) => Promise<string | void>;
  /** Opens the server's sign-in; resolves to an error message, or nothing once signed in. */
  onSignIn: (serverId: string) => Promise<string | void>;
  /** The install said this server needs a sign-in that hasn't happened yet. */
  needsSignIn?: boolean;
  flash?: boolean;
}

/**
 * A bot's suggestion to add a tool server, in the chat. Says what it is, why the bot wants it and
 * exactly what would run (or which address it connects to); the person types any keys and installs.
 * A failed install keeps the form and its values so they can fix a key and try again.
 */
export const McpSuggestionCard: React.FC<McpSuggestionCardProps> = ({
  meta,
  messageId,
  kwantName,
  kwantId,
  live,
  onInstall,
  onDismiss,
  onSignIn,
  needsSignIn,
  flash,
}) => {
  const view = suggestionView(meta, live);
  const [values, setValues] = useState<Record<string, string>>({});
  const [busy, setBusy] = useState<'install' | 'dismiss' | 'signin' | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [dismissError, setDismissError] = useState<string | null>(null);
  const [signInError, setSignInError] = useState<string | null>(null);

  const headingId = `mcp-s-${messageId}`;
  const pending = view.status === 'pending' && busy !== 'install';
  const installing = view.status === 'installing' || busy === 'install';
  const failure = error ?? (view.status === 'pending' ? view.last_error : null);
  const runs = runsLine(view);
  const local = view.runs_on_your_computer || view.transport === 'stdio';

  const install = async () => {
    if (!pending || busy) return;
    const { body, missing } = installPayload(view.inputs, values);
    if (missing.length) {
      setError(`Fill in ${missing.join(', ')} first.`);
      return;
    }
    setBusy('install');
    setError(null);
    const res = await onInstall(view.suggestion_id, body);
    setBusy(null);
    if (res.error) setError(res.error);
    else setValues({});
  };

  const dismiss = async () => {
    if (!pending || busy) return;
    setBusy('dismiss');
    setDismissError(null);
    const err = await onDismiss(view.suggestion_id);
    setBusy(null);
    if (err) setDismissError(err);
  };

  const signIn = async () => {
    if (!view.server_id || busy) return;
    setBusy('signin');
    setSignInError(null);
    const err = await onSignIn(view.server_id);
    setBusy(null);
    if (err) setSignInError(err);
  };

  const border =
    view.status === 'installed' ? 'border-emerald-500/30' : view.status === 'dismissed' ? 'border-zinc-800' : failure ? 'border-rose-500/40' : 'border-sky-500/35';

  return (
    <section data-message-id={messageId} aria-labelledby={headingId} className={`flex items-start gap-3.5 w-full max-w-5xl rounded-xl ${flash ? 'kw-flash' : ''}`}>
      <ModernAvatar role={kwantName} name={kwantName} size={34} kwantId={kwantId} />
      <div className={`flex-1 min-w-0 max-w-2xl rounded-2xl rounded-tl-sm border bg-zinc-900/60 shadow-sm ${border}`}>
        <div className="px-4 pt-3 pb-2 space-y-1.5">
          <span className="flex items-center gap-1.5 text-[11px] font-medium text-zinc-400">
            <Plug size={12} className="text-sky-300" aria-hidden="true" />
            <span className="text-zinc-200">{kwantName}</span> suggests a tool
          </span>
          <div className="flex flex-wrap items-baseline gap-x-2 gap-y-1">
            <h3 id={headingId} className="text-sm font-semibold text-zinc-100 break-words leading-snug">
              {view.title}
            </h3>
            {view.version && <span className="text-[11px] text-zinc-500">v{view.version}</span>}
          </div>
          {view.description && <p className="text-sm text-zinc-300 break-words leading-relaxed">{view.description}</p>}
          {view.reason && (
            <p className="text-sm text-zinc-300 break-words leading-relaxed">
              <span className="text-zinc-500">Why: </span>
              {view.reason}
            </p>
          )}
        </div>

        <div className="mx-4 mb-2 rounded-xl border border-zinc-800 bg-zinc-950/70 px-3 py-2 space-y-1.5">
          <div className="flex flex-wrap items-center gap-2 text-[11px]">
            {local ? (
              <span className="inline-flex items-center gap-1 rounded-full border border-amber-500/40 bg-amber-500/10 px-2 py-0.5 font-medium text-amber-200">
                <Monitor size={11} aria-hidden="true" /> Runs on your computer
              </span>
            ) : (
              <span className="inline-flex items-center gap-1 rounded-full border border-sky-500/30 bg-sky-500/10 px-2 py-0.5 font-medium text-sky-200">
                <Globe size={11} aria-hidden="true" /> Online service
              </span>
            )}
            {view.repository_url && (
              <a
                href={view.repository_url}
                target="_blank"
                rel="noopener noreferrer"
                className="inline-flex items-center gap-1 text-zinc-400 hover:text-zinc-200 underline-offset-2 hover:underline rounded focus-visible:outline focus-visible:outline-2 focus-visible:outline-sky-400"
              >
                Source code <ExternalLink size={11} aria-hidden="true" />
                <span className="sr-only">(opens in a new tab)</span>
              </a>
            )}
          </div>
          <p className="text-[11px] text-zinc-500">{local ? 'Installing runs this command:' : 'Installing connects to:'}</p>
          <code className="block text-xs text-zinc-200 font-mono break-all whitespace-pre-wrap">{runs || 'Unknown'}</code>
          {(view.fixed_env.length > 0 || view.fixed_headers.length > 0) && (
            <p className="text-[11px] text-zinc-500 break-words">
              Also sets: {[...view.fixed_env, ...view.fixed_headers].join(', ')}
            </p>
          )}
        </div>

        {view.status === 'pending' || installing ? (
          <form
            className="px-4 pb-2 space-y-2.5"
            autoComplete="off"
            onSubmit={(e) => {
              e.preventDefault();
              install();
            }}
          >
            {view.inputs.map((i) => {
              const key = inputKey(i);
              const id = `${headingId}-${key}`;
              const { label, code, note } = inputLabel(i);
              return (
                <div key={key} className="space-y-1">
                  <label htmlFor={id} className="flex flex-wrap items-center gap-1.5 text-xs text-zinc-300">
                    {i.secret && <KeyRound size={11} className="text-zinc-500" aria-hidden="true" />}
                    <span className={code ? 'font-mono' : 'font-medium'}>{label}</span>
                    {!i.required && <span className="text-zinc-500">(optional)</span>}
                  </label>
                  {(i.description || note) && (
                    <p id={`${id}-hint`} className="text-[11px] text-zinc-500 break-words">
                      {[i.description, note].filter(Boolean).join(' · ')}
                    </p>
                  )}
                  <input
                    id={id}
                    type={i.secret ? 'password' : 'text'}
                    autoComplete={i.secret ? 'new-password' : 'off'}
                    spellCheck={false}
                    data-1p-ignore
                    data-lpignore="true"
                    required={i.required}
                    aria-describedby={i.description || note ? `${id}-hint` : undefined}
                    aria-invalid={!!failure || undefined}
                    disabled={installing || busy !== null}
                    value={values[key] ?? ''}
                    placeholder={i.placeholder && !i.secret ? i.placeholder : i.secret ? 'Paste it here' : ''}
                    onChange={(e) => setValues((v) => ({ ...v, [key]: e.target.value }))}
                    className="w-full min-h-[40px] sm:min-h-[34px] rounded-lg bg-zinc-950 border border-zinc-800 px-2.5 py-1.5 text-sm text-zinc-100 placeholder-zinc-600 focus:outline-none focus:border-sky-500/60 disabled:opacity-60"
                  />
                </div>
              );
            })}
            {failure && (
              <p role="alert" className="flex items-start gap-1.5 rounded-lg border border-rose-500/30 bg-rose-500/10 px-2.5 py-1.5 text-xs text-rose-200 break-words">
                <AlertTriangle size={13} className="mt-px shrink-0" aria-hidden="true" />
                <span>It didn’t install. {failureText(failure, view.inputs.length > 0)}</span>
              </p>
            )}
            <div className="flex flex-wrap items-center gap-2 pt-0.5">
              <button
                type="submit"
                disabled={busy !== null || installing}
                className="inline-flex items-center justify-center gap-1.5 min-h-[40px] sm:min-h-[34px] px-3.5 rounded-xl text-sm font-medium bg-sky-500 text-zinc-950 hover:bg-sky-400 disabled:bg-zinc-800 disabled:text-zinc-500 focus-visible:outline focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-sky-400"
              >
                {installing && <Loader2 size={13} className="animate-spin motion-reduce:animate-none" aria-hidden="true" />}
                {installing ? 'Installing and testing…' : failure ? 'Try again' : 'Install'}
              </button>
              <button
                type="button"
                onClick={dismiss}
                disabled={busy !== null || installing}
                className="min-h-[40px] sm:min-h-[34px] px-3 rounded-xl text-sm text-zinc-300 border border-zinc-700 hover:bg-zinc-800 disabled:opacity-50 focus-visible:outline focus-visible:outline-2 focus-visible:outline-zinc-500"
              >
                {busy === 'dismiss' ? 'Saying no…' : 'Not now'}
              </button>
            </div>
          </form>
        ) : null}

        <div className="px-4 pb-3 pt-0.5 text-[11px] space-y-1.5">
          <p role="status" aria-live="polite" className="min-w-0">
            {view.status === 'installed' ? (
              needsSignIn ? (
                <span className="inline-flex items-center gap-1.5 text-amber-200">
                  <KeyRound size={13} aria-hidden="true" /> Installed. Sign in to {view.title} so {kwantName} can use it.
                </span>
              ) : (
                <span className="inline-flex items-center gap-1.5 text-emerald-300">
                  <CheckCircle2 size={13} aria-hidden="true" /> Ready. {kwantName} can use it from its next reply.
                </span>
              )
            ) : view.status === 'dismissed' ? (
              <span className="inline-flex items-center gap-1.5 text-zinc-400">
                <MinusCircle size={13} aria-hidden="true" /> Not installed. {kwantName} carries on without it.
              </span>
            ) : installing ? (
              <span className="sr-only">Installing and testing…</span>
            ) : (
              <span className="text-zinc-500">Only you can install it. Keys you type are saved hidden and never appear in the chat.</span>
            )}
          </p>
          {view.status === 'installed' && needsSignIn && view.server_id && (
            <div className="flex flex-wrap items-center gap-2">
              <button
                type="button"
                onClick={signIn}
                disabled={busy !== null}
                className="inline-flex items-center gap-1.5 min-h-[40px] sm:min-h-[34px] px-3.5 rounded-xl text-sm font-medium bg-amber-500 text-zinc-950 hover:bg-amber-400 disabled:opacity-60 focus-visible:outline focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-amber-400"
              >
                {busy === 'signin' && <Loader2 size={13} className="animate-spin motion-reduce:animate-none" aria-hidden="true" />}
                {busy === 'signin' ? (inDesktopApp() ? 'Finish signing in in your browser…' : 'Waiting for sign-in…') : signInError ? 'Try signing in again' : 'Sign in'}
              </button>
              {signInError && (
                <span role="alert" className="text-rose-300 break-words">
                  {signInError}
                </span>
              )}
            </div>
          )}
          {dismissError && (
            <p role="alert" className="text-rose-300 break-words">
              {dismissError}
            </p>
          )}
        </div>
      </div>
    </section>
  );
};
