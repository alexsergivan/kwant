import React, { useCallback, useEffect, useId, useRef, useState } from 'react';
import {
  AlertTriangle,
  CalendarClock,
  Check,
  CheckCircle2,
  ChevronLeft,
  Copy,
  ExternalLink,
  FileText,
  GitBranch,
  GitMerge,
  Bot,
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
import { SAVE_FIRST, SaveGate, saveGate, storeApi } from '../passwordStore';
import { approvalModeApi } from '../approvalMode';
import { BitwardenLoginForm, usePasswordStore } from './PasswordStorePanel';
import { AddLetterForm, LetterPage } from './LetterView';
import {
  ACTION_LABELS,
  CASE_TABS,
  CaseAction,
  CaseTab,
  ImapFields,
  LooseCase,
  LooseEndsError,
  LooseOwner,
  LooseOwnerChange,
  LooseSource,
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
  FORGE_CHOICES,
  FORGE_TOKEN_HINT,
  ForgeChoice,
  cantTellForge,
  cantTellText,
  forgeConnectedText,
  forgeHost,
  forgeSource,
  forgeTakenText,
  sourceKindText,
  isLoopbackHost,
  kindLabel,
  looseApi,
  ownerChange,
  ownerSummary,
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
  forge: <GitMerge size={12} aria-hidden="true" />,
  kwant: <MessageSquare size={12} aria-hidden="true" />,
  document: <FileText size={12} aria-hidden="true" />,
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
  /** Opens a letter case (`source: "document"`) with its pages and fields. */
  onOpenLetter?: (caseId: string) => void;
  /** Tests open a choice. */
  initialChoice?: 'snooze' | 'dismiss' | null;
}

/** One loose end: who, why it stalled, the evidence, the draft and what to do with it. Generic
 * over the source, so a letter or a bank statement can use the same card. */
export const LooseCaseCard: React.FC<LooseCaseCardProps> = ({ c, busy, now = Date.now(), onAction, onOpenChat, onOpenLetter, initialChoice = null }) => {
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

        {c.source === 'document' && onOpenLetter && (
          <button type="button" onClick={() => onOpenLetter(c.id)} className={`${actionBtn} bg-sky-500 hover:bg-sky-400 text-zinc-950 font-semibold`} data-open-letter>
            <FileText size={13} aria-hidden="true" /> Open letter
          </button>
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
 * A login can't be saved on this server unless every space and bot asks before running commands
 * (or Bitwarden is locked): connect
 * or unlock Bitwarden here, move any passwords already on this server, then save again.
 */
export const NeedsBitwarden: React.FC<{
  message: string;
  locked: boolean;
  /** `acceptRisk`: the person chose Save it on this server (0.7.3 A2, when the server says `can_accept`). */
  onReady: (acceptRisk?: boolean) => void;
  onCancel: () => void;
  canAccept?: boolean;
}> = ({ message, locked, onReady, onCancel, canAccept = false }) => {
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
          {canAccept && !locked && (
            <button type="button" onClick={() => onReady(true)} className={`${actionBtn} bg-sky-500 hover:bg-sky-400 text-zinc-950 font-semibold`} data-needs-bitwarden-save>
              {SAVE_FIRST.save}
            </button>
          )}
          <button type="button" onClick={() => setStep(true)} disabled={!s} className={`${actionBtn} ${canAccept && !locked ? 'border border-zinc-600 bg-zinc-800/70 text-zinc-100 hover:bg-zinc-700' : 'bg-sky-500 hover:bg-sky-400 text-zinc-950 font-semibold'}`}>
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

/**
 * The card before the token field (0.7.3 A3). With this server's store while some bot runs without
 * asking, it's a warning, not a wall: Save it on this server (sends accept_risk with the save, once per
 * server), Use Bitwarden, or Make every bot ask first. A locked Bitwarden offers Unlock only.
 * `onReady(note, acceptRisk)` shows the token field.
 */
export const SaveFirst: React.FC<{
  gate: Exclude<SaveGate, 'open'>;
  onReady: (note: string | null, acceptRisk?: boolean) => void;
  /** 0.7.6: the card's reason line for another token (Jira names its own scoped token). Defaults to the GitHub line. */
  why?: string;
}> = ({ gate, onReady, why }) => {
  const id = useId();
  const store = usePasswordStore(true, true);
  const [step, setStep] = useState(false);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const locked = gate === 'locked';
  const box = useRef<HTMLDivElement>(null);
  // Use/Unlock Bitwarden opens its form in place: focus goes to its first field, not back to the page
  useEffect(() => {
    if (!step) return undefined;
    const h = window.setTimeout(() => box.current?.querySelector<HTMLElement>('input:not([disabled]), select:not([disabled])')?.focus(), 0);
    return () => window.clearTimeout(h);
  }, [step, !!store.status]);

  const ask = async () => {
    setBusy(true);
    setError(null);
    try {
      await approvalModeApi.turnAllOn();
      const next = await storeApi.peek();
      store.set(next);
      if (saveGate(next) === 'open') onReady(SAVE_FIRST.asked);
      else setError(SAVE_FIRST.askLeft);
    } catch (e) {
      setError(e instanceof Error ? e.message : 'Couldn’t change approvals.');
    } finally {
      setBusy(false);
    }
  };

  return (
    <div ref={box} role="group" aria-labelledby={`${id}-t`} className={`sm:col-span-2 rounded-xl border p-3 space-y-2 ${step ? 'border-white/10 bg-white/[0.03]' : 'border-amber-500/40 bg-amber-500/10'}`} data-save-first={gate} data-save-first-step={step || undefined}>
      <p id={`${id}-t`} className="flex items-start gap-1.5 text-xs font-medium text-amber-100">
        {!step && <AlertTriangle size={13} className="mt-px shrink-0" aria-hidden="true" />}
        {locked ? SAVE_FIRST.lockedTitle : SAVE_FIRST.title}
      </p>
      {!locked && <p className="text-[11px] text-amber-100/80 leading-snug" data-save-first-why>{why ?? SAVE_FIRST.why}</p>}
      {!step ? (
        <div className="flex flex-wrap gap-2">
          {!locked && (
            <button type="button" onClick={() => onReady(null, true)} className={`${actionBtn} w-full sm:w-auto justify-center bg-sky-500 hover:bg-sky-400 text-zinc-950 font-semibold`} data-save-first-save data-land>
              {SAVE_FIRST.save}
            </button>
          )}
          <button
            type="button"
            onClick={() => setStep(true)}
            disabled={!store.status}
            className={`${actionBtn} w-full sm:w-auto justify-center ${locked ? 'bg-sky-500 hover:bg-sky-400 text-zinc-950 font-semibold' : 'border border-zinc-600 bg-zinc-800/70 text-zinc-100 hover:bg-zinc-700'}`}
            data-save-first-bitwarden
            data-land={locked || undefined}
          >
            {locked ? SAVE_FIRST.unlock : SAVE_FIRST.bitwarden}
          </button>
          {!locked && (
            <button type="button" onClick={ask} disabled={busy} aria-describedby={`${id}-scope`} className={`${quietBtn} w-full sm:w-auto justify-center`} data-save-first-ask>
              {busy && <Loader2 size={13} className="animate-spin motion-reduce:animate-none" aria-hidden="true" />}
              {busy ? SAVE_FIRST.asking : SAVE_FIRST.ask}
            </button>
          )}
          {!locked && (
            <p id={`${id}-scope`} className="basis-full text-[11px] text-zinc-400" data-save-first-scope>
              {SAVE_FIRST.askScope}
            </p>
          )}
        </div>
      ) : (
        store.status && (
          <BitwardenLoginForm
            mode={locked ? 'unlock' : 'connect'}
            status={store.status}
            plainTitle
            onCancel={() => setStep(false)}
            onDone={async (next) => {
              store.set(next);
              setStep(false);
              try {
                if (next.local_count > 0) await storeApi.migrate();
                onReady(SAVE_FIRST.connected);
              } catch (e) {
                setError(e instanceof Error ? e.message : 'Couldn’t move the passwords.');
              }
            }}
          />
        )
      )}
      {error && <p role="alert" className="text-[11px] text-rose-300">{error}</p>}
    </div>
  );
};

const EMPTY_IMAP: ImapFields = { label: '', host: '', port: '', security: 'tls', username: '', folders: '', accept_invalid_certs: false };

/** The GitHub login's hint under the token field: the same words wherever it's connected. */
export const GITHUB_TOKEN_HINT = 'Read-only access to issues and pull requests is enough. To post reviews from Dev mode, give it Pull requests: Read and write.';
/** 0.7.4: what a GitLab or Gitea login is for. */
export const FORGE_NOTE = 'Dev mode reads reviews and CI with this login. It’s tested first, and nothing is saved if the test fails.';
/** Where else this connection is used (0.7.3 B1): one GitHub login for Loose ends and Dev mode. */
export const GITHUB_ALSO_DEV = 'Also used by Dev mode';

export type SourceKind = 'imap' | 'github' | 'forge';
const KIND_TEXT: Record<SourceKind, string> = { imap: 'Email (IMAP)', github: 'GitHub', forge: 'GitLab, Gitea or Forgejo' };

const KindPicker: React.FC<{ kind: SourceKind; onKind: (k: SourceKind) => void }> = ({ kind, onKind }) => (
  <div role="radiogroup" aria-label="Kind of source" className="grid grid-cols-1 sm:grid-cols-3 gap-2">
    {(['imap', 'github', 'forge'] as const).map((k) => (
      <button
        key={k}
        type="button"
        role="radio"
        aria-checked={kind === k}
        onClick={() => onKind(k)}
        className={`${actionBtn} justify-center border ${kind === k ? 'border-sky-500/50 bg-sky-500/10 text-sky-200' : 'border-zinc-700 bg-zinc-800/60 text-zinc-300 hover:bg-zinc-800'}`}
      >
        {SOURCE_ICON[k]} {KIND_TEXT[k]}
      </button>
    ))}
  </div>
);

/**
 * The one GitHub connect form (0.7.3 B1): Loose ends' Add a source > GitHub, and every Connect GitHub in
 * Dev mode (in a popover or a sheet). It reads the password store first; when the token would sit on this
 * server while a bot runs without asking, the A3 card comes before the token field. The token is read from
 * its field at submit and never kept in state.
 */
export const GitHubConnectForm: React.FC<{
  /** `test`: the save's test answer (a forge's `{login, forge}`), for the line after saving. */
  onAdded: (s: LooseSource, test?: unknown) => void;
  onCancel: () => void;
  /** Focus the first control (the token, or the card's first button) once this turns true. */
  land?: boolean;
  /** Scroll into view on landing (a page); a popover or sheet is already in view. */
  scroll?: boolean;
  heading?: string;
  /** Above the fields: Loose ends' kind picker. */
  above?: React.ReactNode;
  /** The line above the buttons. */
  note?: string;
  /** Test hook: start past the store check. */
  initialGate?: SaveGate;
  /** 0.7.4 (G4): a GitLab, Gitea or Forgejo connection instead, prefilled from a repo row's Connect GitLab. */
  forge?: { url?: string; choice?: ForgeChoice | null };
}> = ({ onAdded, onCancel, land = false, scroll = true, heading = 'Connect GitHub', above, note, initialGate, forge }) => {
  const id = useId();
  const isForge = !!forge;
  const [address, setAddress] = useState(forge?.url ?? '');
  const [choice, setChoice] = useState<ForgeChoice | null>(forge?.choice ?? null);
  // "Which is it?" stays folded unless the server couldn't tell (or the person opens it).
  const [choiceOpen, setChoiceOpen] = useState(false);
  const [cantTell, setCantTell] = useState<string | null>(null);
  const choiceRef = useRef<HTMLDivElement>(null);
  const [ghLabel, setGhLabel] = useState('');
  const [ghBase, setGhBase] = useState('');
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [vault, setVault] = useState<{ message: string; locked: boolean; canAccept: boolean } | null>(null);
  const secretRef = useRef<HTMLInputElement>(null);
  const formRef = useRef<HTMLFormElement>(null);
  // The store is read before the token field shows ('checking' until it answers; a failed read
  // leaves the form open, and the 409 card still catches it).
  const [gate, setGate] = useState<SaveGate | 'checking'>(initialGate ?? 'checking');
  const [ready, setReady] = useState<string | null>(null);
  const acceptRisk = useRef(false);
  useEffect(() => {
    if (initialGate) return undefined;
    let live = true;
    storeApi.peek().then(
      (st) => live && setGate(saveGate(st)),
      () => live && setGate('open'),
    );
    return () => {
      live = false;
    };
  }, [initialGate]);
  // 0.7.3 C6: Bitwarden connected or unlocked in Settings or another tab, or approvals changed there,
  // unblocks this form when the person comes back to it: the store is read again on focus and on the
  // tab turning visible, while the card is up.
  const blocked = gate === 'store' || gate === 'locked';
  useEffect(() => {
    if (!blocked) return undefined;
    let live = true;
    const recheck = () => {
      if (typeof document !== 'undefined' && document.visibilityState === 'hidden') return;
      storeApi.peek().then(
        (st) => {
          if (!live) return;
          const next = saveGate(st);
          if (next === 'open') setReady(st.store === 'bitwarden' ? SAVE_FIRST.connected : !st.approvals_off ? SAVE_FIRST.asked : null);
          setGate((g) => (g === 'store' || g === 'locked' ? next : g));
        },
        () => undefined,
      );
    };
    window.addEventListener('focus', recheck);
    document.addEventListener('visibilitychange', recheck);
    return () => {
      live = false;
      window.removeEventListener('focus', recheck);
      document.removeEventListener('visibilitychange', recheck);
    };
  }, [blocked]);
  // Land: bring the form into view and focus its first control: the token, or the card's first way on.
  // Again after a way on is taken.
  const landed = useRef<string | null>(null);
  useEffect(() => {
    if (!land || gate === 'checking') return undefined;
    if (landed.current === gate) return undefined;
    // Marked only once focus is in: a cancelled frame (StrictMode's second run, a re-render) tries
    // again, and a way on that renders a frame later (the store panel loading) is waited for.
    let h = 0;
    let tries = 0;
    const step = () => {
      const form = formRef.current;
      const target = form && (gate === 'open' ? (isForge && !address.trim() ? form.querySelector<HTMLElement>('[data-forge-url]') : secretRef.current) : form.querySelector<HTMLElement>('[data-land]:not([disabled])'));
      if (!form || !target) {
        if (++tries < 30) h = window.setTimeout(step, 50);
        return;
      }
      landed.current = gate;
      if (scroll) form.scrollIntoView?.({ block: 'start', behavior: 'auto' });
      target.focus({ preventScroll: true });
    };
    h = window.setTimeout(step, 0);
    return () => window.clearTimeout(h);
  }, [land, gate, scroll]);

  const submit = async (e?: React.FormEvent, accept = acceptRisk.current) => {
    e?.preventDefault();
    const token = secretRef.current?.value ?? '';
    const req = isForge ? forgeSource(ghLabel, address, choice, token) : githubSource(ghLabel, ghBase, token);
    if ('error' in req) {
      setError(req.error);
      return;
    }
    setBusy(true);
    setError(null);
    setVault(null);
    setCantTell(null);
    try {
      const r = await looseApi.addSource(req, accept);
      if (secretRef.current) secretRef.current.value = '';
      playSfx('coin');
      onAdded(r.source, r.test);
    } catch (err) {
      if (err instanceof LooseEndsError && (err.needsBitwarden || err.locked)) setVault({ message: err.message, locked: err.locked, canAccept: err.canAccept });
      else if (isForge && cantTellForge(err)) {
        // The server's own words, then the choice opens with its first option in focus.
        setCantTell(cantTellText((err as Error).message));
        setChoiceOpen(true);
        window.setTimeout(() => choiceRef.current?.querySelector<HTMLElement>('[role="radio"]')?.focus(), 0);
      } else if (isForge && err instanceof LooseEndsError && err.status === 409) setError(forgeTakenText(forgeHost(String((req as { config: Record<string, unknown> }).config.url ?? ''))));
      else setError(err instanceof Error ? err.message : 'Couldn’t add it.');
    } finally {
      setBusy(false);
    }
  };

  return (
    <form ref={formRef} onSubmit={submit} className="rounded-xl border border-zinc-800 bg-zinc-900/60 p-3 sm:p-4 space-y-3 scroll-mt-4" aria-labelledby={`${id}-h`} data-add-source={isForge ? 'forge' : 'github'} data-github-connect={isForge ? undefined : true} data-forge-connect={isForge || undefined}>
      <div className="flex items-center justify-between gap-2">
        <h4 id={`${id}-h`} className="text-xs font-semibold text-zinc-200">
          {heading}
        </h4>
        <button type="button" onClick={onCancel} aria-label="Close" className="p-1.5 rounded-lg text-zinc-400 hover:text-zinc-100 hover:bg-zinc-800">
          <X size={14} />
        </button>
      </div>
      {above}
      {gate === 'checking' ? (
        <p className="flex items-center gap-2 text-xs text-zinc-400" data-save-checking>
          <Loader2 size={13} className="animate-spin motion-reduce:animate-none" aria-hidden="true" /> Checking where the token goes…
        </p>
      ) : gate !== 'open' ? (
        <SaveFirst
          gate={gate}
          onReady={(n, accept) => {
            if (accept) acceptRisk.current = true;
            setReady(n);
            setGate('open');
          }}
        />
      ) : (
        <div className="grid grid-cols-1 sm:grid-cols-2 gap-3">
          {ready && (
            <p role="status" className="sm:col-span-2 text-xs text-emerald-300" data-save-ready>
              {ready}
            </p>
          )}
          {isForge && (
            <div className="sm:col-span-2">
              <label htmlFor={`${id}-url`} className="block text-[11px] text-zinc-400 mb-1">
                Server address
              </label>
              <input
                id={`${id}-url`}
                className={field}
                value={address}
                onChange={(e) => setAddress(e.target.value)}
                placeholder="https://gitlab.example.com"
                inputMode="url"
                autoComplete="off"
                spellCheck={false}
                data-forge-url
              />
            </div>
          )}
          <div className="sm:col-span-2">
            <label htmlFor={`${id}-tok`} className="block text-[11px] text-zinc-400 mb-1">
              Token
            </label>
            <input id={`${id}-tok`} ref={secretRef} type="password" className={field} autoComplete="new-password" aria-describedby={`${id}-tok-hint`} data-github-token={isForge ? undefined : true} data-forge-token={isForge || undefined} />
            <p id={`${id}-tok-hint`} className="mt-1 text-[11px] text-zinc-400">
              {isForge ? FORGE_TOKEN_HINT : GITHUB_TOKEN_HINT}
            </p>
            {ghBase.trim() && (
              <p className="mt-1 text-[11px] text-zinc-400" data-ghe-token-hint>
                Make the token on your Enterprise server: Settings → Developer settings → Personal access tokens.
              </p>
            )}
          </div>
          {isForge && (
            <div className="sm:col-span-2" ref={choiceRef} data-forge-choice={choiceOpen ? 'open' : 'closed'}>
              {cantTell && (
                <p role="alert" className="mb-2 flex items-start gap-1.5 text-xs text-amber-200 break-words" data-forge-cant-tell>
                  <AlertTriangle size={13} className="mt-px shrink-0" aria-hidden="true" /> {cantTell} Pick which it is, then try again.
                </p>
              )}
              {!choiceOpen ? (
                <button type="button" onClick={() => setChoiceOpen(true)} aria-expanded={false} className="text-[11px] text-sky-300 hover:text-sky-200 underline-offset-2 hover:underline" data-forge-choice-open>
                  {choice ? `${FORGE_CHOICES.find((c) => c.value === choice)?.label}. Change` : 'Which is it? (optional)'}
                </button>
              ) : (
                <>
                  <p id={`${id}-which`} className="block text-[11px] text-zinc-400 mb-1">
                    Which is it?{cantTell ? '' : ' (optional)'}
                  </p>
                  <div role="radiogroup" aria-labelledby={`${id}-which`} className="grid grid-cols-3 gap-2">
                    {FORGE_CHOICES.map((c) => (
                      <button
                        key={c.value}
                        type="button"
                        role="radio"
                        aria-checked={choice === c.value}
                        onClick={() => {
                          setChoice(choice === c.value && !cantTell ? null : c.value);
                        }}
                        className={`${actionBtn} justify-center border ${choice === c.value ? 'border-sky-500/50 bg-sky-500/10 text-sky-200' : 'border-zinc-700 bg-zinc-800/60 text-zinc-300 hover:bg-zinc-800'}`}
                        data-forge-pick={c.value}
                      >
                        {c.label}
                      </button>
                    ))}
                  </div>
                </>
              )}
            </div>
          )}
          <div className={isForge ? 'sm:col-span-2' : undefined}>
            <label htmlFor={`${id}-glabel`} className="block text-[11px] text-zinc-400 mb-1">
              Name (optional)
            </label>
            <input id={`${id}-glabel`} className={field} value={ghLabel} onChange={(e) => setGhLabel(e.target.value)} placeholder={isForge ? forgeHost(/^[a-z]+:\/\//i.test(address.trim()) ? address : `https://${address.trim()}`) || 'Our GitLab' : 'GitHub'} />
          </div>
          {!isForge && (
          <div>
            <label htmlFor={`${id}-base`} className="block text-[11px] text-zinc-400 mb-1">
              GitHub Enterprise address (leave empty for github.com)
            </label>
            <input id={`${id}-base`} className={field} value={ghBase} onChange={(e) => setGhBase(e.target.value)} placeholder="https://github.yourcompany.com" spellCheck={false} />
          </div>
          )}
        </div>
      )}

      <p className="text-[11px] text-zinc-500">{note ?? (isForge ? FORGE_NOTE : 'Loose ends and Dev mode share this login. It’s tested first, and nothing is saved if the test fails.')}</p>

      {vault && <NeedsBitwarden message={vault.message} locked={vault.locked} canAccept={vault.canAccept} onReady={(accept) => submit(undefined, accept || acceptRisk.current)} onCancel={() => setVault(null)} />}
      {error && (
        <p role="alert" className="flex items-start gap-1.5 text-xs text-rose-300 break-words">
          <AlertTriangle size={13} className="mt-px shrink-0" aria-hidden="true" /> {error}
        </p>
      )}

      <div className="flex flex-wrap justify-end gap-2">
        <button type="button" onClick={onCancel} className={quietBtn}>
          Cancel
        </button>
        <button type="submit" disabled={busy || gate !== 'open'} className={`${actionBtn} bg-sky-500 hover:bg-sky-400 text-zinc-950 font-semibold disabled:opacity-50`}>
          {busy ? <Loader2 size={13} className="animate-spin motion-reduce:animate-none" aria-hidden="true" /> : <Plus size={13} aria-hidden="true" />}
          {busy ? 'Testing…' : 'Test and add'}
        </button>
      </div>
    </form>
  );
};

/** Add an IMAP mailbox or GitHub. GitHub is the shared connect form; the IMAP password is read from its field at submit and never kept in state. */
export const AddSourceForm: React.FC<{
  onAdded: (s: LooseSource, test?: unknown) => void;
  onCancel: () => void;
  initialKind?: SourceKind;
  /** Dev mode's Connect (flow F5): land on the first field once `land` is true. */
  connect?: boolean;
  land?: boolean;
}> = ({ onAdded, onCancel, initialKind = 'imap', connect = false, land = false }) => {
  const [kind, setKind] = useState<SourceKind>(initialKind);
  // Choosing GitHub (or GitLab) in the picker lands on its first control (the token, or the card), like Connect does.
  const [picked, setPicked] = useState(false);
  const onKind = (k: SourceKind) => {
    setKind(k);
    setPicked(k !== 'imap');
  };
  if (kind === 'forge')
    return <GitHubConnectForm key="forge" forge={{}} heading="Add a source" above={<KindPicker kind={kind} onKind={onKind} />} onAdded={onAdded} onCancel={onCancel} land={picked} />;
  if (kind === 'github')
    return <GitHubConnectForm key="github" heading="Add a source" above={<KindPicker kind={kind} onKind={onKind} />} onAdded={onAdded} onCancel={onCancel} land={(connect && land) || picked} />;
  return <ImapSourceForm picker={<KindPicker kind={kind} onKind={onKind} />} onAdded={onAdded} onCancel={onCancel} />;
};

const ImapSourceForm: React.FC<{ picker: React.ReactNode; onAdded: (s: LooseSource) => void; onCancel: () => void }> = ({ picker, onAdded, onCancel }) => {
  const id = useId();
  const [imap, setImap] = useState<ImapFields>(EMPTY_IMAP);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [vault, setVault] = useState<{ message: string; locked: boolean; canAccept: boolean } | null>(null);
  const secretRef = useRef<HTMLInputElement>(null);
  const local = isLoopbackHost(imap.host);

  const submit = async (e?: React.FormEvent, accept = false) => {
    e?.preventDefault();
    const req = imapSource(imap, secretRef.current?.value ?? '');
    if ('error' in req) {
      setError(req.error);
      return;
    }
    setBusy(true);
    setError(null);
    setVault(null);
    try {
      const r = await looseApi.addSource(req, accept);
      if (secretRef.current) secretRef.current.value = '';
      playSfx('coin');
      onAdded(r.source);
    } catch (err) {
      if (err instanceof LooseEndsError && (err.needsBitwarden || err.locked)) setVault({ message: err.message, locked: err.locked, canAccept: err.canAccept });
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
    <form onSubmit={submit} className="rounded-xl border border-zinc-800 bg-zinc-900/60 p-3 sm:p-4 space-y-3 scroll-mt-4" aria-labelledby={`${id}-h`} data-add-source="imap">
      <div className="flex items-center justify-between gap-2">
        <h4 id={`${id}-h`} className="text-xs font-semibold text-zinc-200">
          Add a source
        </h4>
        <button type="button" onClick={onCancel} aria-label="Close" className="p-1.5 rounded-lg text-zinc-400 hover:text-zinc-100 hover:bg-zinc-800">
          <X size={14} />
        </button>
      </div>
      {picker}
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

      <p className="text-[11px] text-zinc-500">Loose ends only reads this source. It tests the connection first and saves nothing if the test fails. The login goes to your password vault.</p>

      {vault && <NeedsBitwarden message={vault.message} locked={vault.locked} canAccept={vault.canAccept} onReady={(accept) => submit(undefined, accept)} onCancel={() => setVault(null)} />}
      {error && (
        <p role="alert" className="flex items-start gap-1.5 text-xs text-rose-300 break-words">
          <AlertTriangle size={13} className="mt-px shrink-0" aria-hidden="true" /> {error}
        </p>
      )}

      <div className="flex flex-wrap justify-end gap-2">
        <button type="button" onClick={onCancel} className={quietBtn}>
          Cancel
        </button>
        <button type="submit" disabled={busy} className={`${actionBtn} bg-sky-500 hover:bg-sky-400 text-zinc-950 font-semibold disabled:opacity-50`}>
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
  /** Tests open the form; Dev mode's Connect opens it on GitHub (flow F5). */
  initialAdding?: boolean;
  initialKind?: SourceKind;
  /** Called after a source is added, with it. */
  onAdded?: (s: LooseSource) => void;
  /** The page above has finished loading: Connect lands on the GitHub form now (B3), not before the cases push it down. */
  settled?: boolean;
}> = ({ sources, error, onChanged, initialAdding = false, initialKind, onAdded, settled = true }) => {
  const [adding, setAdding] = useState(initialAdding);
  const connect = initialAdding && initialKind === 'github';
  const [busy, setBusy] = useState<string | null>(null);
  const [confirmRemove, setConfirmRemove] = useState<string | null>(null);
  const [note, setNote] = useState<{ text: string; error: boolean } | null>(null);

  const run = async (s: LooseSource, what: 'test' | 'toggle' | 'remove') => {
    setBusy(s.id);
    setNote(null);
    try {
      if (what === 'test') {
        const r = await looseApi.testSource(s.id);
        setNote({ text: s.kind === 'forge' ? `${s.label}: ${forgeConnectedText(r?.test, forgeHost(String(s.config?.url ?? '')))}` : `${s.label}: connected.`, error: false });
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
          initialKind={initialKind}
          connect={connect}
          land={connect && settled}
          onCancel={() => setAdding(false)}
          onAdded={(s, test) => {
            setAdding(false);
            setNote({ text: s.kind === 'forge' ? forgeConnectedText(test, forgeHost(String(s.config?.url ?? ''))) : `Added ${s.label}. It’s read at the next scan.`, error: false });
            onChanged();
            onAdded?.(s);
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
                      {sourceKindText(s)}
                      {s.builtin ? ' · built in' : ''} · {st.text}
                      {(s.kind === 'github' || s.kind === 'forge') && <span data-github-also-dev> · {s.kind === 'forge' ? 'Used by Dev mode' : GITHUB_ALSO_DEV}</span>}
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

/** Which bot prepares drafts when a review or a letter deadline comes up, in this space. */
export const LooseOwnerView: React.FC<{
  owner: LooseOwner | null;
  bots: Array<{ id: string; name: string }>;
  error: string | null;
  busy: boolean;
  onChange: (next: LooseOwnerChange) => void;
}> = ({ owner, bots, error, busy, onChange }) => {
  const id = useId();
  const lead = owner && !owner.kwant_id ? owner.owner : null;
  return (
    <section aria-labelledby={`${id}-h`} className="space-y-2">
      <h3 id={`${id}-h`} className="text-[11px] font-semibold text-zinc-500 uppercase tracking-wider">
        Who prepares drafts
      </h3>
      {!owner ? (
        error ? (
          <p className="text-xs text-rose-300">Couldn’t load this setting: {error}</p>
        ) : (
          <p className="flex items-center gap-2 text-xs text-zinc-400">
            <Loader2 size={13} className="animate-spin motion-reduce:animate-none" aria-hidden="true" /> Loading…
          </p>
        )
      ) : (
        <div className="rounded-xl border border-zinc-800 bg-zinc-900/60 p-3 space-y-3" aria-busy={busy}>
          <label className="flex items-start gap-2.5 text-xs text-zinc-200 cursor-pointer">
            <input
              type="checkbox"
              checked={owner.enabled}
              disabled={busy}
              onChange={(e) => onChange(ownerChange(owner, { enabled: e.target.checked }))}
              className="mt-0.5 h-4 w-4 shrink-0 accent-sky-500"
            />
            <span>When a review finds loose ends or a letter deadline comes up, a bot starts preparing drafts and next steps.</span>
          </label>
          <div className="flex flex-wrap items-center gap-2 sm:pl-6">
            <Bot size={14} className="text-sky-400 shrink-0" aria-hidden="true" />
            <label htmlFor={`${id}-bot`} className="text-xs text-zinc-400">
              Bot
            </label>
            <select
              id={`${id}-bot`}
              value={owner.kwant_id ?? ''}
              disabled={busy || !owner.enabled}
              onChange={(e) => onChange(ownerChange(owner, { kwant_id: e.target.value || null }))}
              className={`${field} w-auto min-w-0 max-w-full flex-1 sm:flex-none sm:min-w-[14rem]`}
            >
              <option value="">{lead ? `The space lead (${lead.name})` : 'The space lead'}</option>
              {bots.map((b) => (
                <option key={b.id} value={b.id}>
                  {b.name}
                </option>
              ))}
            </select>
            {busy && <Loader2 size={13} className="animate-spin motion-reduce:animate-none text-zinc-400" aria-label="Saving" />}
          </div>
          <p className={`text-[11px] sm:pl-6 ${owner.enabled ? 'text-zinc-400' : 'text-zinc-500'}`} data-owner-summary>
            {ownerSummary(owner)}
          </p>
          <div role="status" aria-live="polite">
            {error && <p className="text-xs text-rose-300 sm:pl-6">Couldn’t save: {error}</p>}
          </div>
        </div>
      )}
    </section>
  );
};

/** Loads and saves the owner setting for the current space. */
export const LooseOwnerSetting: React.FC = () => {
  const [owner, setOwner] = useState<LooseOwner | null>(null);
  const [bots, setBots] = useState<Array<{ id: string; name: string }>>([]);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  useEffect(() => {
    let cancelled = false;
    looseApi.owner().then(
      (o) => !cancelled && setOwner(o),
      (e) => !cancelled && setError(e instanceof Error ? e.message : 'something went wrong.'),
    );
    looseApi.bots().then(
      (list) => !cancelled && setBots(Array.isArray(list) ? list.map((k) => ({ id: k.id, name: k.name })) : []),
      () => undefined,
    );
    return () => {
      cancelled = true;
    };
  }, []);
  const save = async (next: LooseOwnerChange) => {
    if (busy) return;
    setBusy(true);
    setError(null);
    try {
      setOwner(await looseApi.setOwner(next));
    } catch (e) {
      setError(e instanceof Error ? e.message : 'something went wrong.');
    } finally {
      setBusy(false);
    }
  };
  return <LooseOwnerView owner={owner} bots={bots} error={error} busy={busy} onChange={save} />;
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
  /** "Who prepares drafts": the per-space owner setting. */
  owner?: React.ReactNode;
  onTab: (t: CaseTab) => void;
  onRefresh: () => void;
  onScan: () => void;
  onAction: (c: LooseCase, action: CaseAction, body?: unknown) => void;
  onOpenChat?: (kwantId: string, threadId: string) => void;
  onOpenLetter?: (caseId: string) => void;
  /** "Add a letter": a button, or the open form. */
  addLetter?: React.ReactNode;
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
  owner = null,
  onTab,
  onRefresh,
  onScan,
  onAction,
  onOpenChat,
  onOpenLetter,
  addLetter = null,
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

          {cases && addLetter}

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
                      <LooseCaseCard key={c.id} c={c} busy={busy === c.id} now={now} onAction={onAction} onOpenChat={onOpenChat} onOpenLetter={onOpenLetter} />
                    ))}
                  </ul>
                )}
              </div>
            </>
          )}

          {!needsToken && owner}
          {!needsToken && sources}
        </div>
      </div>
    </div>
  );
};

export const AddLetterButton: React.FC<{ onClick: () => void }> = ({ onClick }) => (
  <div className="flex flex-wrap items-center gap-x-3 gap-y-1 rounded-xl border border-dashed border-zinc-700 p-3">
    <button type="button" onClick={onClick} className={`${actionBtn} bg-sky-500 hover:bg-sky-400 text-zinc-950 font-semibold`} data-add-letter-button>
      <FileText size={13} aria-hidden="true" /> Add a letter
    </button>
    <p className="text-[11px] text-zinc-500 min-w-0 flex-1">Photograph or upload a bill or form to get its deadline and a reply draft. Nothing is sent.</p>
  </div>
);

/** Loads cases and sources, runs the card actions and scans. */
export const LooseEndsPage: React.FC<{
  onBack?: () => void;
  onOpenChat?: (kwantId: string, threadId: string) => void;
  /** Dev mode's Connect (flow F5): open the add form on GitHub, and say when a GitHub source is added. */
  connectGitHub?: boolean;
  onGitHubAdded?: () => void;
}> = ({ onBack, onOpenChat, connectGitHub = false, onGitHubAdded }) => {
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
  const [letter, setLetter] = useState<string | null>(null);
  const [addingLetter, setAddingLetter] = useState(false);
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

  if (letter)
    return (
      <LetterPage
        caseId={letter}
        onBack={() => (setLetter(null), load())}
        onDeleted={(title) => {
          setLetter(null);
          setNotice({ text: `Deleted “${title}”, its pages and everything read from it.`, error: false });
          load();
        }}
      />
    );

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
      onOpenLetter={(id) => (setNotice(null), setLetter(id))}
      addLetter={
        addingLetter ? (
          <AddLetterForm
            onCancel={() => setAddingLetter(false)}
            onCreated={(id) => {
              setAddingLetter(false);
              setNotice(null);
              setLetter(id);
            }}
          />
        ) : (
          <AddLetterButton onClick={() => setAddingLetter(true)} />
        )
      }
      onBack={onBack}
      owner={<LooseOwnerSetting />}
      sources={
        <LooseSources
          sources={sources}
          error={sourcesError}
          onChanged={() => (loadSources(), load())}
          initialAdding={connectGitHub}
          initialKind={connectGitHub ? 'github' : undefined}
          onAdded={(s) => s.kind === 'github' && onGitHubAdded?.()}
          settled={!loading || cases !== null || error !== null}
        />
      }
    />
  );
};
