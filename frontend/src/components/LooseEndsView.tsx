import React, { useCallback, useEffect, useId, useRef, useState } from 'react';
import {
  AlertTriangle,
  CalendarClock,
  Check,
  CheckCircle2,
  ChevronLeft,
  Copy,
  ExternalLink,
  GitBranch,
  Loader2,
  Mail,
  MessageSquare,
  Plus,
  Quote,
  RefreshCw,
  Search,
  Trash2,
  X,
} from 'lucide-react';
import { PERSON_SIGNED_IN_EVENT, requestPersonToken } from '../auth';
import { playSfx } from '../audio/chiptune';
import { storeApi } from '../passwordStore';
import { BitwardenLoginForm, usePasswordStore } from './PasswordStorePanel';
import {
  ACTION_LABELS,
  CASE_TABS,
  CaseAction,
  CaseTab,
  ImapFields,
  LooseCase,
  LooseEndsError,
  LooseSource,
  NewSource,
  Tone,
  actionDoneText,
  actionsFor,
  ageText,
  caseChat,
  deadlineText,
  draftCopyText,
  draftOpen,
  githubSource,
  imapSource,
  isLoopbackHost,
  kindLabel,
  looseApi,
  safeHref,
  shortDate,
  sourceLabel,
  sourceStatusText,
  tabOf,
} from '../looseEnds';

const TONES: Record<Tone, string> = {
  amber: 'border-amber-500/30 bg-amber-500/10 text-amber-200',
  emerald: 'border-emerald-500/30 bg-emerald-500/10 text-emerald-200',
  rose: 'border-rose-500/30 bg-rose-500/10 text-rose-200',
  sky: 'border-sky-500/30 bg-sky-500/10 text-sky-200',
  zinc: 'border-zinc-700 bg-zinc-800 text-zinc-300',
};

const SOURCE_ICON: Record<string, React.ReactNode> = {
  imap: <Mail size={12} aria-hidden="true" />,
  github: <GitBranch size={12} aria-hidden="true" />,
  kwant: <MessageSquare size={12} aria-hidden="true" />,
};

const actionBtn =
  'inline-flex items-center gap-1.5 min-h-[36px] sm:min-h-[30px] px-3 py-1 rounded-lg text-xs font-medium transition-colors disabled:opacity-50 focus-visible:outline focus-visible:outline-2 focus-visible:outline-sky-500';
const quietBtn = `${actionBtn} border border-zinc-700 bg-zinc-800/80 text-zinc-100 hover:bg-zinc-700 hover:border-zinc-600`;
const field =
  'w-full min-h-[36px] px-2.5 py-1.5 bg-zinc-950 border border-zinc-800 rounded-lg text-xs text-zinc-100 placeholder:text-zinc-600 focus:outline-none focus:border-sky-500';

const NOTHING_SENT = 'Draft only — copy it and send it yourself. Nothing here is ever sent for you.';

/** Snooze lengths offered on a card. */
const SNOOZES: Array<{ days: number; label: string }> = [
  { days: 3, label: '3 days' },
  { days: 7, label: '1 week' },
  { days: 30, label: '1 month' },
];

export interface LooseCaseCardProps {
  c: LooseCase;
  busy: boolean;
  now?: number;
  onAction: (c: LooseCase, action: CaseAction, body?: unknown) => void;
  /** Opens a bot thread in the app; without it a case shows no chat link. */
  onOpenChat?: (kwantId: string, threadId: string) => void;
  /** Tests open a choice. */
  initialChoice?: 'snooze' | 'dismiss' | null;
}

/** One loose end: who, why it stalled, the evidence, the draft and what to do with it. Generic
 * over the source, so a letter or a bank statement can use the same card. */
export const LooseCaseCard: React.FC<LooseCaseCardProps> = ({ c, busy, now = Date.now(), onAction, onOpenChat, initialChoice = null }) => {
  const id = useId();
  const [choice, setChoice] = useState<'snooze' | 'dismiss' | null>(initialChoice);
  const [copied, setCopied] = useState<'ok' | 'failed' | null>(null);
  const kind = kindLabel(c.kind);
  const who = c.counterpart_name || c.counterpart;
  const due = c.deadline ? deadlineText(c.deadline, now) : null;
  const link = safeHref(c.link);
  const chat = onOpenChat ? caseChat(c) : null;
  const open = c.draft ? draftOpen(c.draft) : null;

  const copy = async () => {
    if (!c.draft) return;
    try {
      await navigator.clipboard.writeText(draftCopyText(c.draft));
      setCopied('ok');
      playSfx('click');
    } catch {
      setCopied('failed');
    }
  };

  const press = (a: CaseAction) => {
    if (a === 'snooze' || a === 'dismiss') {
      setChoice((cur) => (cur === a ? null : a));
      return;
    }
    setChoice(null);
    onAction(c, a);
  };

  return (
    <li className="rounded-xl border border-zinc-800 bg-zinc-900/60" aria-labelledby={`${id}-t`} data-case={c.id}>
      <div className="p-3 sm:p-4 space-y-3">
        <div className="flex flex-wrap items-center gap-1.5 text-[11px]">
          <span className="inline-flex items-center gap-1 px-1.5 py-px rounded-full border border-zinc-700 bg-zinc-800 text-zinc-300">
            {SOURCE_ICON[c.source] ?? <Search size={12} aria-hidden="true" />}
            {sourceLabel(c.source)}
          </span>
          <span className={`px-1.5 py-px rounded-full border ${TONES[kind.tone]}`}>{kind.label}</span>
          {c.status === 'snoozed' && c.snoozed_until && <span className={`px-1.5 py-px rounded-full border ${TONES.zinc}`}>Snoozed until {shortDate(c.snoozed_until)}</span>}
          {c.status === 'waiting_reply' && <span className={`px-1.5 py-px rounded-full border ${TONES.sky}`}>You followed up</span>}
          {c.status === 'closed' && <span className={`px-1.5 py-px rounded-full border ${TONES.emerald}`}>Done</span>}
          {c.status === 'dismissed' && <span className={`px-1.5 py-px rounded-full border ${TONES.zinc}`}>Dismissed</span>}
          <time dateTime={c.last_activity_at} title={new Date(c.last_activity_at).toLocaleString()} className="ml-auto shrink-0 text-zinc-500">
            {ageText(c.age_days)}
          </time>
        </div>

        <div className="min-w-0">
          <h4 id={`${id}-t`} className="text-sm font-medium text-zinc-100 break-words">
            {c.title}
          </h4>
          {who && (
            <p className="text-xs text-zinc-400 break-words">
              With {c.counterpart_name && c.counterpart && c.counterpart_name !== c.counterpart ? `${c.counterpart_name} (${c.counterpart})` : who}
            </p>
          )}
        </div>

        <p className="text-xs text-zinc-300 break-words">{c.why}</p>

        {(due || c.reference) && (
          <div className="flex flex-wrap gap-x-4 gap-y-1 text-[11px]">
            {due && (
              <span className={`inline-flex items-center gap-1 ${due.passed ? 'text-rose-300' : 'text-zinc-300'}`}>
                <CalendarClock size={12} aria-hidden="true" /> {due.text}
              </span>
            )}
            {c.reference && <span className="text-zinc-400 break-all">Reference: {c.reference}</span>}
          </div>
        )}

        {c.evidence.length > 0 && (
          <div className="space-y-1.5">
            <p className="text-[11px] font-semibold text-zinc-500 uppercase tracking-wider">Evidence</p>
            {c.evidence.map((e, i) => (
              <figure key={i} className="rounded-lg border border-zinc-800 bg-zinc-950/60 px-3 py-2">
                <blockquote className="flex gap-2 text-xs text-zinc-200 whitespace-pre-wrap break-words">
                  <Quote size={12} className="mt-0.5 shrink-0 text-zinc-600" aria-hidden="true" />
                  <span>{e.quote}</span>
                </blockquote>
                <figcaption className="mt-1 text-[11px] text-zinc-500 break-words">
                  {e.outbound ? 'You' : e.from}, <time dateTime={e.at}>{shortDate(e.at)}</time>
                </figcaption>
              </figure>
            ))}
            {link && (
              <a href={link} target="_blank" rel="noopener noreferrer" className="inline-flex items-center gap-1 text-[11px] text-sky-300 hover:text-sky-200 underline-offset-2 hover:underline">
                <ExternalLink size={12} aria-hidden="true" /> Open the original
              </a>
            )}
          </div>
        )}

        {chat && (
          <button
            type="button"
            onClick={() => onOpenChat?.(chat.kwantId, chat.threadId)}
            className="inline-flex min-h-[32px] items-center gap-1 text-[11px] text-sky-300 hover:text-sky-200 underline-offset-2 hover:underline"
          >
            <MessageSquare size={12} aria-hidden="true" /> Open chat
          </button>
        )}

        {c.draft && (c.status === 'open' || c.status === 'waiting_reply') && (
          <div className="rounded-lg border border-sky-500/20 bg-sky-500/5 p-3 space-y-2">
            <div className="flex flex-wrap items-baseline justify-between gap-2">
              <p className="text-[11px] font-semibold text-sky-200">
                Draft {c.draft.to ? `to ${c.draft.to}` : ''}
                {c.draft.generated_by === 'model' && <span className="ml-1 font-normal text-zinc-500">(written by a model)</span>}
              </p>
            </div>
            <p className="text-[11px] text-amber-200" data-nothing-sent>
              {NOTHING_SENT}
            </p>
            {c.draft.subject && <p className="text-xs text-zinc-300 break-words">Subject: {c.draft.subject}</p>}
            <pre className="max-h-48 overflow-y-auto whitespace-pre-wrap break-words font-sans text-xs text-zinc-100 bg-zinc-950/60 border border-zinc-800 rounded-lg p-2.5 select-text">
              {c.draft.body}
            </pre>
            <div className="flex flex-wrap items-center gap-2">
              <button type="button" onClick={copy} className={quietBtn}>
                {copied === 'ok' ? <Check size={13} aria-hidden="true" /> : <Copy size={13} aria-hidden="true" />}
                {copied === 'ok' ? 'Copied' : 'Copy draft'}
              </button>
              {open && (
                <a href={open.href} target={open.href.startsWith('mailto:') ? undefined : '_blank'} rel="noopener noreferrer" className={quietBtn}>
                  <ExternalLink size={13} aria-hidden="true" /> {open.label}
                </a>
              )}
              <span role="status" className="text-[11px] text-zinc-400">
                {copied === 'failed' ? 'Couldn’t copy here: select the text above and copy it.' : ''}
              </span>
            </div>
          </div>
        )}

        <div role="group" aria-label={`What to do with: ${c.title}`} className="flex flex-wrap gap-2">
          {actionsFor(c).map((a) => (
            <button
              key={a}
              type="button"
              disabled={busy}
              aria-expanded={a === 'snooze' || a === 'dismiss' ? choice === a : undefined}
              onClick={() => press(a)}
              className={a === 'done' ? `${actionBtn} bg-emerald-600/90 hover:bg-emerald-500 text-white` : quietBtn}
            >
              {a === 'done' && <CheckCircle2 size={13} aria-hidden="true" />}
              {a === 'draft' && <RefreshCw size={13} aria-hidden="true" />}
              {ACTION_LABELS[a]}
            </button>
          ))}
          {busy && (
            <span className="inline-flex items-center gap-1.5 text-[11px] text-zinc-400">
              <Loader2 size={12} className="animate-spin motion-reduce:animate-none" aria-hidden="true" /> Saving…
            </span>
          )}
        </div>

        {choice === 'snooze' && (
          <div role="group" aria-label="Snooze for" className="flex flex-wrap items-center gap-2 text-xs">
            <span className="text-zinc-400">Bring it back in</span>
            {SNOOZES.map((s) => (
              <button key={s.days} type="button" disabled={busy} className={quietBtn} onClick={() => (setChoice(null), onAction(c, 'snooze', { days: s.days }))}>
                {s.label}
              </button>
            ))}
          </div>
        )}
        {choice === 'dismiss' && (
          <div role="group" aria-label="Dismiss" className="flex flex-wrap items-center gap-2 text-xs">
            <button type="button" disabled={busy} className={quietBtn} onClick={() => (setChoice(null), onAction(c, 'dismiss', { scope: 'thread' }))}>
              Just this one
            </button>
            {c.counterpart && (
              <button type="button" disabled={busy} className={quietBtn} onClick={() => (setChoice(null), onAction(c, 'dismiss', { scope: 'sender' }))}>
                Everything from {c.counterpart_name || c.counterpart}
              </button>
            )}
          </div>
        )}
      </div>
    </li>
  );
};

/**
 * A login can't be saved on this server while approvals are off (or Bitwarden is locked): connect
 * or unlock Bitwarden here, move any passwords already on this server, then save again.
 */
export const NeedsBitwarden: React.FC<{ message: string; locked: boolean; onReady: () => void; onCancel: () => void }> = ({ message, locked, onReady, onCancel }) => {
  const store = usePasswordStore(true);
  const [step, setStep] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const s = store.status;
  const mode: 'connect' | 'unlock' = s?.store === 'bitwarden' && s.bitwarden ? 'unlock' : 'connect';

  const finish = async (local: number) => {
    setError(null);
    try {
      if (local > 0) await storeApi.migrate();
      onReady();
    } catch (e) {
      setError(e instanceof Error ? e.message : 'Couldn’t move the passwords.');
    }
  };

  return (
    <div role="alert" className="rounded-xl border border-amber-500/40 bg-amber-500/10 p-3 space-y-2" data-needs-bitwarden>
      <p className="text-xs text-amber-100">{locked ? 'Bitwarden is locked. Unlock it to save this login there.' : message}</p>
      <p className="text-[11px] text-amber-100/80">Your login stays in this form until it’s saved. Nothing was saved yet.</p>
      {!step ? (
        <div className="flex flex-wrap gap-2">
          <button type="button" onClick={() => setStep(true)} disabled={!s} className={`${actionBtn} bg-sky-500 hover:bg-sky-400 text-zinc-950 font-semibold`}>
            {mode === 'unlock' ? 'Unlock Bitwarden' : 'Move to Bitwarden'}
          </button>
          <button type="button" onClick={onCancel} className={quietBtn}>
            Not now
          </button>
        </div>
      ) : (
        s && (
          <BitwardenLoginForm
            mode={mode}
            status={s}
            onCancel={() => setStep(false)}
            onDone={(next) => {
              store.set(next);
              setStep(false);
              finish(next.local_count);
            }}
          />
        )
      )}
      {error && <p className="text-[11px] text-rose-300">{error}</p>}
    </div>
  );
};

const EMPTY_IMAP: ImapFields = { label: '', host: '', port: '', security: 'tls', username: '', folders: '', accept_invalid_certs: false };

/** Add an IMAP mailbox or GitHub. The password or token is read from its field at submit and never kept in state. */
export const AddSourceForm: React.FC<{ onAdded: (s: LooseSource) => void; onCancel: () => void }> = ({ onAdded, onCancel }) => {
  const id = useId();
  const [kind, setKind] = useState<'imap' | 'github'>('imap');
  const [imap, setImap] = useState<ImapFields>(EMPTY_IMAP);
  const [ghLabel, setGhLabel] = useState('');
  const [ghBase, setGhBase] = useState('');
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [vault, setVault] = useState<{ message: string; locked: boolean } | null>(null);
  const secretRef = useRef<HTMLInputElement>(null);
  const local = isLoopbackHost(imap.host);

  const submit = async (e?: React.FormEvent) => {
    e?.preventDefault();
    const secret = secretRef.current?.value ?? '';
    const req: NewSource | { error: string } = kind === 'imap' ? imapSource(imap, secret) : githubSource(ghLabel, ghBase, secret);
    if ('error' in req) {
      setError(req.error);
      return;
    }
    setBusy(true);
    setError(null);
    setVault(null);
    try {
      const r = await looseApi.addSource(req);
      if (secretRef.current) secretRef.current.value = '';
      playSfx('coin');
      onAdded(r.source);
    } catch (err) {
      if (err instanceof LooseEndsError && (err.needsBitwarden || err.locked)) setVault({ message: err.message, locked: err.locked });
      else setError(err instanceof Error ? err.message : 'Couldn’t add it.');
    } finally {
      setBusy(false);
    }
  };

  const set = <K extends keyof ImapFields>(k: K, v: ImapFields[K]) => setImap((f) => ({ ...f, [k]: v }));
  const label = (htmlFor: string, text: string) => (
    <label htmlFor={htmlFor} className="block text-[11px] text-zinc-400 mb-1">
      {text}
    </label>
  );

  return (
    <form onSubmit={submit} className="rounded-xl border border-zinc-800 bg-zinc-900/60 p-3 sm:p-4 space-y-3" aria-labelledby={`${id}-h`}>
      <div className="flex items-center justify-between gap-2">
        <h4 id={`${id}-h`} className="text-xs font-semibold text-zinc-200">
          Add a source
        </h4>
        <button type="button" onClick={onCancel} aria-label="Close" className="p-1.5 rounded-lg text-zinc-400 hover:text-zinc-100 hover:bg-zinc-800">
          <X size={14} />
        </button>
      </div>
      <div role="radiogroup" aria-label="Kind of source" className="grid grid-cols-2 gap-2">
        {(['imap', 'github'] as const).map((k) => (
          <button
            key={k}
            type="button"
            role="radio"
            aria-checked={kind === k}
            onClick={() => (setKind(k), setError(null), setVault(null))}
            className={`${actionBtn} justify-center border ${kind === k ? 'border-sky-500/50 bg-sky-500/10 text-sky-200' : 'border-zinc-700 bg-zinc-800/60 text-zinc-300 hover:bg-zinc-800'}`}
          >
            {SOURCE_ICON[k]} {k === 'imap' ? 'Email (IMAP)' : 'GitHub'}
          </button>
        ))}
      </div>

      {kind === 'imap' ? (
        <div className="grid grid-cols-1 sm:grid-cols-2 gap-3">
          <div className="sm:col-span-2">
            {label(`${id}-host`, 'Mail server')}
            <input id={`${id}-host`} className={field} value={imap.host} onChange={(e) => set('host', e.target.value)} placeholder="imap.gmail.com" autoComplete="off" spellCheck={false} />
          </div>
          <div>
            {label(`${id}-sec`, 'Connection')}
            <select id={`${id}-sec`} className={field} value={imap.security} onChange={(e) => set('security', e.target.value as ImapFields['security'])}>
              <option value="tls">Encrypted (TLS, port 993)</option>
              <option value="starttls">STARTTLS (port 143)</option>
              <option value="plain" disabled={!local}>
                Not encrypted (this computer only)
              </option>
            </select>
          </div>
          <div>
            {label(`${id}-port`, 'Port')}
            <input id={`${id}-port`} className={field} inputMode="numeric" value={imap.port} onChange={(e) => set('port', e.target.value.replace(/\D/g, ''))} placeholder={imap.security === 'tls' ? '993' : '143'} />
          </div>
          <div>
            {label(`${id}-user`, 'Login')}
            <input id={`${id}-user`} className={field} value={imap.username} onChange={(e) => set('username', e.target.value)} placeholder="you@example.com" autoComplete="off" spellCheck={false} />
          </div>
          <div>
            {label(`${id}-secret`, 'Password or app password')}
            <input id={`${id}-secret`} ref={secretRef} type="password" className={field} autoComplete="new-password" />
          </div>
          <div>
            {label(`${id}-label`, 'Name (optional)')}
            <input id={`${id}-label`} className={field} value={imap.label} onChange={(e) => set('label', e.target.value)} placeholder="Work mail" />
          </div>
          <div>
            {label(`${id}-folders`, 'Folders (optional)')}
            <input id={`${id}-folders`} className={field} value={imap.folders} onChange={(e) => set('folders', e.target.value)} placeholder="INBOX, Sent" />
          </div>
          {local && (
            <label className="sm:col-span-2 flex items-center gap-2 text-[11px] text-zinc-300">
              <input type="checkbox" checked={imap.accept_invalid_certs} onChange={(e) => set('accept_invalid_certs', e.target.checked)} />
              Accept this computer’s own certificate (Proton Bridge)
            </label>
          )}
        </div>
      ) : (
        <div className="grid grid-cols-1 sm:grid-cols-2 gap-3">
          <div className="sm:col-span-2">
            {label(`${id}-tok`, 'Token (read-only access to issues and pull requests is enough)')}
            <input id={`${id}-tok`} ref={secretRef} type="password" className={field} autoComplete="new-password" />
          </div>
          <div>
            {label(`${id}-glabel`, 'Name (optional)')}
            <input id={`${id}-glabel`} className={field} value={ghLabel} onChange={(e) => setGhLabel(e.target.value)} placeholder="GitHub" />
          </div>
          <div>
            {label(`${id}-base`, 'API address (GitHub Enterprise only)')}
            <input id={`${id}-base`} className={field} value={ghBase} onChange={(e) => setGhBase(e.target.value)} placeholder="https://api.github.com" spellCheck={false} />
          </div>
        </div>
      )}

      <p className="text-[11px] text-zinc-500">Loose ends only reads this source. It tests the connection first and saves nothing if the test fails. The login goes to your password vault.</p>

      {vault && <NeedsBitwarden message={vault.message} locked={vault.locked} onReady={() => submit()} onCancel={() => setVault(null)} />}
      {error && (
        <p role="alert" className="flex items-start gap-1.5 text-xs text-rose-300 break-words">
          <AlertTriangle size={13} className="mt-px shrink-0" aria-hidden="true" /> {error}
        </p>
      )}

      <div className="flex flex-wrap justify-end gap-2">
        <button type="button" onClick={onCancel} className={quietBtn}>
          Cancel
        </button>
        <button type="submit" disabled={busy} className={`${actionBtn} bg-sky-500 hover:bg-sky-400 text-zinc-950 font-semibold`}>
          {busy ? <Loader2 size={13} className="animate-spin motion-reduce:animate-none" aria-hidden="true" /> : <Plus size={13} aria-hidden="true" />}
          {busy ? 'Testing…' : 'Test and add'}
        </button>
      </div>
    </form>
  );
};

/** Where loose ends are looked for: bot chats, mailboxes and GitHub. */
export const LooseSources: React.FC<{
  sources: LooseSource[] | null;
  error: string | null;
  onChanged: () => void;
  /** Tests open the form. */
  initialAdding?: boolean;
}> = ({ sources, error, onChanged, initialAdding = false }) => {
  const [adding, setAdding] = useState(initialAdding);
  const [busy, setBusy] = useState<string | null>(null);
  const [confirmRemove, setConfirmRemove] = useState<string | null>(null);
  const [note, setNote] = useState<{ text: string; error: boolean } | null>(null);

  const run = async (s: LooseSource, what: 'test' | 'toggle' | 'remove') => {
    setBusy(s.id);
    setNote(null);
    try {
      if (what === 'test') {
        await looseApi.testSource(s.id);
        setNote({ text: `${s.label}: connected.`, error: false });
      } else if (what === 'toggle') {
        await looseApi.setEnabled(s.id, !s.enabled);
      } else {
        const r = await looseApi.removeSource(s.id);
        setConfirmRemove(null);
        setNote({ text: `Removed ${s.label}, its saved login and ${r.cases_removed} ${r.cases_removed === 1 ? 'case' : 'cases'}.`, error: false });
      }
      onChanged();
    } catch (e) {
      setNote({ text: `${s.label}: ${e instanceof Error ? e.message : 'something went wrong.'}`, error: true });
    } finally {
      setBusy(null);
    }
  };

  return (
    <section aria-labelledby="loose-sources" className="space-y-2">
      <div className="flex items-center justify-between gap-2">
        <h3 id="loose-sources" className="text-[11px] font-semibold text-zinc-500 uppercase tracking-wider">
          Where to look
        </h3>
        {!adding && (
          <button type="button" onClick={() => setAdding(true)} className={quietBtn}>
            <Plus size={13} aria-hidden="true" /> Add a source
          </button>
        )}
      </div>
      {error && <p className="text-xs text-rose-300">{error}</p>}
      <div role="status" aria-live="polite">
        {note && <p className={`text-xs ${note.error ? 'text-rose-300' : 'text-emerald-300'}`}>{note.text}</p>}
      </div>
      {adding && (
        <AddSourceForm
          onCancel={() => setAdding(false)}
          onAdded={(s) => {
            setAdding(false);
            setNote({ text: `Added ${s.label}. It’s read at the next scan.`, error: false });
            onChanged();
          }}
        />
      )}
      {sources && (
        <ul className="space-y-2">
          {sources.map((s) => {
            const st = sourceStatusText(s);
            return (
              <li key={s.id} className="rounded-xl border border-zinc-800 bg-zinc-900/40 p-3 space-y-2">
                <div className="flex items-start gap-2 min-w-0">
                  <span className="mt-0.5 text-zinc-400">{SOURCE_ICON[s.kind] ?? <Search size={12} aria-hidden="true" />}</span>
                  <div className="min-w-0 flex-1">
                    <p className="text-xs font-medium text-zinc-100 break-words">{s.builtin ? 'Your bot chats, tasks and questions' : s.label}</p>
                    <p className={`text-[11px] break-words ${st.error ? 'text-rose-300' : 'text-zinc-500'}`}>
                      {sourceLabel(s.kind)}
                      {s.builtin ? ' · built in' : ''} · {st.text}
                    </p>
                  </div>
                </div>
                {!s.builtin && (
                  <div className="flex flex-wrap gap-2">
                    <button type="button" disabled={busy !== null} onClick={() => run(s, 'test')} className={quietBtn}>
                      {busy === s.id ? <Loader2 size={13} className="animate-spin motion-reduce:animate-none" aria-hidden="true" /> : null} Test
                    </button>
                    <button type="button" disabled={busy !== null} aria-pressed={s.enabled} onClick={() => run(s, 'toggle')} className={quietBtn}>
                      {s.enabled ? 'Pause' : 'Resume'}
                    </button>
                    {confirmRemove === s.id ? (
                      <>
                        <button type="button" disabled={busy !== null} onClick={() => run(s, 'remove')} className={`${actionBtn} bg-rose-600 hover:bg-rose-500 text-white`}>
                          <Trash2 size={13} aria-hidden="true" /> Remove it and its cases
                        </button>
                        <button type="button" onClick={() => setConfirmRemove(null)} className={quietBtn}>
                          Keep
                        </button>
                      </>
                    ) : (
                      <button type="button" disabled={busy !== null} onClick={() => setConfirmRemove(s.id)} className={`${actionBtn} text-rose-300 hover:bg-rose-500/10`}>
                        <Trash2 size={13} aria-hidden="true" /> Remove
                      </button>
                    )}
                  </div>
                )}
              </li>
            );
          })}
        </ul>
      )}
    </section>
  );
};

export interface LooseEndsViewProps {
  cases: LooseCase[] | null;
  loading: boolean;
  running: boolean;
  error: string | null;
  /** The person closed "Confirm it's you": show how to try again. */
  needsToken?: boolean;
  notice?: { text: string; error: boolean } | null;
  busy?: string | null;
  tab: CaseTab;
  now?: number;
  nextReviewAt?: string | null;
  sources: React.ReactNode;
  onTab: (t: CaseTab) => void;
  onRefresh: () => void;
  onScan: () => void;
  onAction: (c: LooseCase, action: CaseAction, body?: unknown) => void;
  onOpenChat?: (kwantId: string, threadId: string) => void;
  onBack?: () => void;
}

/** The Loose ends page: stalled things with evidence and a draft, plus where kwant looks. */
export const LooseEndsView: React.FC<LooseEndsViewProps> = ({
  cases,
  loading,
  running,
  error,
  needsToken = false,
  notice = null,
  busy = null,
  tab,
  now = Date.now(),
  nextReviewAt = null,
  sources,
  onTab,
  onRefresh,
  onScan,
  onAction,
  onOpenChat,
  onBack,
}) => {
  const counts = CASE_TABS.reduce((acc, t) => ({ ...acc, [t.key]: (cases ?? []).filter((c) => tabOf(c.status) === t.key).length }), {} as Record<CaseTab, number>);
  const shown = (cases ?? []).filter((c) => tabOf(c.status) === tab);
  const current = CASE_TABS.find((t) => t.key === tab)!;

  return (
    <div className="flex-1 flex flex-col h-full bg-zinc-950 overflow-hidden">
      <div className={`h-14 border-b border-zinc-800/80 ${onBack ? 'px-2' : 'px-4'} sm:px-6 flex items-center justify-between gap-3 bg-zinc-900/30 backdrop-blur-sm select-none`}>
        {onBack && (
          <button
            type="button"
            onClick={onBack}
            aria-label="Back to chats"
            className="shrink-0 p-2 -ml-0.5 -mr-2 rounded-lg text-zinc-300 hover:text-zinc-100 hover:bg-zinc-800 focus-visible:outline focus-visible:outline-2 focus-visible:outline-sky-500"
          >
            <ChevronLeft size={18} />
          </button>
        )}
        <div className="min-w-0 flex-1">
          <h2 className="text-sm font-semibold text-zinc-100 truncate">Loose ends</h2>
          <p className="text-[11px] text-zinc-400 truncate mt-0.5">
            {running ? 'Looking for loose ends…' : nextReviewAt ? `Next weekly review ${shortDate(nextReviewAt)}` : 'Things you started and never finished'}
          </p>
        </div>
        <button
          type="button"
          onClick={onScan}
          disabled={running || loading || needsToken}
          className="inline-flex items-center gap-1.5 px-2.5 py-1 rounded-lg text-xs text-zinc-300 border border-zinc-700/60 hover:bg-zinc-800 disabled:opacity-60 focus-visible:outline focus-visible:outline-2 focus-visible:outline-sky-500"
        >
          <RefreshCw size={12} className={running ? 'animate-spin motion-reduce:animate-none' : ''} aria-hidden="true" />
          {running ? 'Scanning…' : 'Scan now'}
        </button>
      </div>

      <div className="flex-1 overflow-y-auto p-4 sm:p-6" aria-busy={loading || running}>
        <div className="max-w-3xl mx-auto space-y-5">
          <p className="text-[11px] text-zinc-500">Your bots read your conversations without changing them and never send anything. Each loose end comes with a draft you send yourself.</p>

          {needsToken && (
            <div role="alert" className="px-3 py-2 rounded-lg border border-amber-500/30 bg-amber-500/10 flex flex-wrap items-center gap-2 text-xs text-amber-100">
              <span>Only you can see loose ends. Confirm it’s you to open them.</span>
              <button type="button" onClick={requestPersonToken} className="ml-auto px-2 py-0.5 rounded border border-amber-500/40 hover:bg-amber-500/10">
                Confirm it’s you
              </button>
            </div>
          )}

          {error && !needsToken && (
            <div role="alert" className="px-3 py-2 rounded-lg border border-rose-500/20 bg-rose-500/5 flex items-center gap-2 text-xs text-rose-300">
              <span>Couldn’t load loose ends: {error}</span>
              <button type="button" onClick={onRefresh} className="ml-auto px-2 py-0.5 rounded border border-rose-500/30 hover:bg-rose-500/10">
                Retry
              </button>
            </div>
          )}

          <div role="status" aria-live="polite">
            {notice && (
              <p className={`px-3 py-2 rounded-lg border text-xs ${notice.error ? 'border-amber-500/30 bg-amber-500/10 text-amber-200' : 'border-emerald-500/30 bg-emerald-500/10 text-emerald-200'}`}>
                {notice.text}
              </p>
            )}
          </div>

          {!cases && loading && (
            <p className="flex items-center gap-2 text-xs text-zinc-400">
              <Loader2 size={14} className="animate-spin motion-reduce:animate-none" aria-hidden="true" /> Loading…
            </p>
          )}

          {cases && (
            <>
              <div role="tablist" aria-label="Loose ends by status" className="flex gap-1 overflow-x-auto -mx-1 px-1">
                {CASE_TABS.map((t) => (
                  <button
                    key={t.key}
                    type="button"
                    role="tab"
                    aria-selected={tab === t.key}
                    onClick={() => onTab(t.key)}
                    className={`shrink-0 min-h-[36px] sm:min-h-[30px] px-3 rounded-lg text-xs font-medium border ${
                      tab === t.key ? 'border-sky-500/40 bg-sky-500/10 text-sky-200' : 'border-transparent text-zinc-400 hover:text-zinc-200 hover:bg-zinc-800'
                    }`}
                  >
                    {t.label} <span className="font-mono text-zinc-500">{counts[t.key]}</span>
                  </button>
                ))}
              </div>

              <div role="tabpanel" aria-label={current.label}>
                {shown.length === 0 ? (
                  <div className="py-12 flex flex-col items-center text-center gap-2" data-empty>
                    <CheckCircle2 size={28} className="text-emerald-400" aria-hidden="true" />
                    <p className="text-sm font-medium text-zinc-200">{tab === 'open' ? 'No loose ends' : `Nothing ${current.label.toLowerCase()}`}</p>
                    <p className="text-xs text-zinc-500 max-w-xs">{current.empty}</p>
                  </div>
                ) : (
                  <ul className="space-y-3">
                    {shown.map((c) => (
                      <LooseCaseCard key={c.id} c={c} busy={busy === c.id} now={now} onAction={onAction} onOpenChat={onOpenChat} />
                    ))}
                  </ul>
                )}
              </div>
            </>
          )}

          {!needsToken && sources}
        </div>
      </div>
    </div>
  );
};

/** Loads cases and sources, runs the card actions and scans. */
export const LooseEndsPage: React.FC<{ onBack?: () => void; onOpenChat?: (kwantId: string, threadId: string) => void }> = ({ onBack, onOpenChat }) => {
  const [cases, setCases] = useState<LooseCase[] | null>(null);
  const [running, setRunning] = useState(false);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [needsToken, setNeedsToken] = useState(false);
  const [notice, setNotice] = useState<{ text: string; error: boolean } | null>(null);
  const [busy, setBusy] = useState<string | null>(null);
  const [tab, setTab] = useState<CaseTab>('open');
  const [sources, setSources] = useState<LooseSource[] | null>(null);
  const [sourcesError, setSourcesError] = useState<string | null>(null);
  const [nextReview, setNextReview] = useState<string | null>(null);
  const alive = useRef(true);

  const loadSources = useCallback(async () => {
    try {
      const r = await looseApi.sources();
      if (alive.current) (setSources(r.sources), setSourcesError(null));
    } catch (e) {
      if (alive.current) setSourcesError(e instanceof Error ? e.message : 'Couldn’t load the sources.');
    }
  }, []);

  const load = useCallback(async () => {
    setLoading(true);
    try {
      const r = await looseApi.cases();
      if (!alive.current) return null;
      setCases(r.cases);
      setRunning(r.running);
      setError(null);
      setNeedsToken(false);
      looseApi.status().then((s) => alive.current && setNextReview(s.next_review_at), () => {});
      loadSources();
      return r;
    } catch (e) {
      if (!alive.current) return null;
      if (e instanceof LooseEndsError && e.status === 401) setNeedsToken(true);
      else setError(e instanceof Error ? e.message : 'Something went wrong.');
      return null;
    } finally {
      if (alive.current) setLoading(false);
    }
  }, [loadSources]);

  useEffect(() => {
    alive.current = true;
    load();
    const again = () => load();
    window.addEventListener(PERSON_SIGNED_IN_EVENT, again);
    return () => {
      alive.current = false;
      window.removeEventListener(PERSON_SIGNED_IN_EVENT, again);
    };
  }, [load]);

  // While a scan runs, look again every few seconds until it's done.
  useEffect(() => {
    if (!running) return;
    const t = window.setInterval(async () => {
      try {
        const s = await looseApi.status();
        if (!s.running) {
          setRunning(false);
          const r = await load();
          if (r) setNotice({ text: 'Scan finished.', error: false });
        }
      } catch {
        // the next tick tries again
      }
    }, 2500);
    return () => window.clearInterval(t);
  }, [running, load]);

  const act = async (c: LooseCase, action: CaseAction, body?: unknown) => {
    if (busy) return;
    setBusy(c.id);
    setNotice(null);
    try {
      const next = await looseApi.act(c.id, action, body);
      setCases((list) => (list ?? []).map((x) => (x.id === next.id ? next : x)));
      setNotice({ text: actionDoneText(action, next), error: false });
      if (action === 'dismiss' && (body as { scope?: string } | undefined)?.scope === 'sender') load();
    } catch (e) {
      setNotice({ text: `${e instanceof Error ? e.message : 'Something went wrong.'} “${c.title}” is unchanged.`, error: true });
    } finally {
      setBusy(null);
    }
  };

  const scan = async () => {
    setNotice(null);
    try {
      await looseApi.run();
      setRunning(true);
    } catch (e) {
      if (e instanceof LooseEndsError && e.status === 409) setRunning(true);
      else setNotice({ text: `Couldn’t start a scan: ${e instanceof Error ? e.message : 'something went wrong.'}`, error: true });
    }
  };

  return (
    <LooseEndsView
      cases={cases}
      loading={loading}
      running={running}
      error={error}
      needsToken={needsToken}
      notice={notice}
      busy={busy}
      tab={tab}
      nextReviewAt={nextReview}
      onTab={setTab}
      onRefresh={load}
      onScan={scan}
      onAction={act}
      onOpenChat={onOpenChat}
      onBack={onBack}
      sources={<LooseSources sources={sources} error={sourcesError} onChanged={() => (loadSources(), load())} />}
    />
  );
};
