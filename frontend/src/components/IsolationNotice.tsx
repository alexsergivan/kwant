import React, { useRef, useState } from 'react';
import { Check, FolderOpen, KeyRound, Loader2, Monitor, PauseCircle, Pencil, Play, RotateCcw, ShieldAlert } from 'lucide-react';
import { RetryState, RetryTarget, runRetryOnce } from '../interruptions';
import {
  CONTINUE_TEXT,
  IsolationFailedMeta,
  NoticeAction,
  RequestLimitMeta,
  SAFETY,
  SafetyKind,
  isolationCard,
  requestLimitText,
} from '../isolationNotice';

const BOX = 'w-full max-w-5xl rounded-xl border border-amber-500/30 bg-amber-950/20 px-3.5 py-2.5 sm:px-4';
const BTN =
  'shrink-0 inline-flex items-center justify-center gap-1.5 min-h-[44px] sm:min-h-[36px] px-3 py-1.5 rounded-lg text-xs font-medium border transition-colors cursor-pointer disabled:cursor-default disabled:opacity-70 focus-visible:outline focus-visible:outline-2';
const PRIMARY = `${BTN} text-amber-100 bg-amber-500/15 border-amber-500/40 hover:bg-amber-500/25 disabled:hover:bg-amber-500/15 focus-visible:outline-amber-400`;
const SECONDARY = `${BTN} text-zinc-200 border-zinc-700 hover:bg-zinc-800/70 focus-visible:outline-sky-500`;

const SafetyLine: React.FC<{ kind: SafetyKind }> = ({ kind }) => {
  if (!kind) return null;
  const [a, name, b] = SAFETY[kind];
  return (
    <p data-isolation-safety={kind} className="mt-1.5 text-xs leading-[1.5] text-zinc-400">
      {a}
      {name && <span className="font-medium text-zinc-300">{name}</span>}
      {b}
    </p>
  );
};

export interface IsolationNoticeProps {
  messageId: string;
  meta: IsolationFailedMeta;
  /** The saved text: for a code with no row, the server's own words come from here. */
  text?: string | null;
  /** Only on the newest notice the person hasn't written past (`retryTargets`). */
  retry?: RetryTarget | null;
  onRetry?: (target: RetryTarget) => void | Promise<unknown>;
  onOpenComputerSettings?: () => void;
  onOpenAccounts?: () => void;
  onChangeTeamFolder?: (teamId: string) => void;
  onEditBot?: (botName: string) => void;
  flash?: boolean;
}

/** Isolation stopped a reply (fail closed): why, what's safe, and the one or two ways on. */
export const IsolationNotice: React.FC<IsolationNoticeProps> = ({
  messageId,
  meta,
  text,
  retry,
  onRetry,
  onOpenComputerSettings,
  onOpenAccounts,
  onChangeTeamFolder,
  onEditBot,
  flash,
}) => {
  const card = isolationCard(meta, text);
  const [state, setState] = useState<RetryState>('idle');
  const lock = useRef<RetryState>('idle');
  const headingId = `iso-${messageId}`;
  const reasonId = `iso-r-${messageId}`;

  const button = (a: NoticeAction, primary: boolean) => {
    const cls = primary ? PRIMARY : SECONDARY;
    switch (a) {
      case 'retry':
        if (!retry || !onRetry) return null;
        return (
          <button
            key={a}
            type="button"
            data-isolation-action="retry"
            onClick={() => void runRetryOnce(lock, retry, onRetry, setState)}
            disabled={state !== 'idle'}
            aria-describedby={headingId}
            className={cls}
          >
            {state === 'sending' ? (
              <Loader2 size={13} className="animate-spin motion-reduce:animate-none" aria-hidden="true" />
            ) : state === 'sent' ? (
              <Check size={13} aria-hidden="true" />
            ) : (
              <RotateCcw size={13} aria-hidden="true" />
            )}
            <span>{state === 'sending' ? 'Sending…' : state === 'sent' ? 'Sent again' : 'Retry'}</span>
          </button>
        );
      case 'computer':
        if (!onOpenComputerSettings) return null;
        return (
          <button key={a} type="button" data-isolation-action="computer" onClick={onOpenComputerSettings} className={cls}>
            <Monitor size={13} aria-hidden="true" />
            <span>Computer settings</span>
          </button>
        );
      case 'accounts':
        if (!onOpenAccounts) return null;
        return (
          <button key={a} type="button" data-isolation-action="accounts" onClick={onOpenAccounts} className={cls}>
            <KeyRound size={13} aria-hidden="true" />
            <span>Accounts</span>
          </button>
        );
      case 'change_folder':
        if (!onChangeTeamFolder || !meta.teamId) return null;
        return (
          <button key={a} type="button" data-isolation-action="change_folder" onClick={() => onChangeTeamFolder(meta.teamId!)} className={cls}>
            <FolderOpen size={13} aria-hidden="true" />
            <span>Change folder</span>
          </button>
        );
      case 'edit_bot':
        if (!onEditBot) return null;
        return (
          <button key={a} type="button" data-isolation-action="edit_bot" onClick={() => onEditBot(meta.botName)} className={cls}>
            <Pencil size={13} aria-hidden="true" />
            <span>Edit {meta.botName}</span>
          </button>
        );
    }
  };

  // The first button that can show is primary, so Retry leads when there's no Change folder.
  let primaryTaken = false;
  const buttons = card.actions
    .map((a) => {
      const canShow =
        a === 'retry'
          ? !!retry && !!onRetry
          : a === 'computer'
            ? !!onOpenComputerSettings
            : a === 'accounts'
              ? !!onOpenAccounts
              : a === 'change_folder'
                ? !!onChangeTeamFolder && !!meta.teamId
                : !!onEditBot;
      if (!canShow) return null;
      const el = button(a, !primaryTaken);
      primaryTaken = true;
      return el;
    })
    .filter(Boolean);

  return (
    <section
      data-message-id={messageId}
      data-isolation-notice={card.row}
      aria-labelledby={headingId}
      aria-describedby={reasonId}
      className={`${BOX} ${flash ? 'kw-flash' : ''}`}
    >
      <div className="flex items-start gap-2">
        <ShieldAlert size={15} className="mt-0.5 shrink-0 text-amber-400" aria-hidden="true" />
        <div className="min-w-0 flex-1">
          <p id={headingId} className="text-sm font-semibold text-amber-100 break-words">
            {card.first}
          </p>
          <p id={reasonId} className="mt-0.5 text-sm leading-[1.5] text-amber-200 break-words">
            {card.reason.map((s, i) =>
              s.folder ? (
                <span key={i} data-isolation-folder className="font-mono text-[13px] text-amber-100 [overflow-wrap:anywhere]">
                  {s.text}
                </span>
              ) : (
                <React.Fragment key={i}>{s.text}</React.Fragment>
              ),
            )}
          </p>
          <SafetyLine kind={card.safety} />
          {buttons.length > 0 && <div className="mt-2.5 flex flex-wrap gap-2">{buttons}</div>}
        </div>
      </div>
    </section>
  );
};

export interface RequestLimitNoticeProps {
  messageId: string;
  meta: RequestLimitMeta;
  /** Only on the newest notice the person hasn't written past; without it there's no Continue. */
  canContinue?: boolean;
  onSend?: (text: string) => void | Promise<unknown>;
  flash?: boolean;
}

/** Notice 5: a reply paused at its request limit; Continue starts a new reply with a fresh limit. */
export const RequestLimitNotice: React.FC<RequestLimitNoticeProps> = ({ messageId, meta, canContinue, onSend, flash }) => {
  const copy = requestLimitText(meta);
  const [state, setState] = useState<RetryState>('idle');
  const lock = useRef<RetryState>('idle');
  const headingId = `limit-${messageId}`;
  const show = !!canContinue && !!onSend;
  return (
    <section data-message-id={messageId} data-request-limit aria-labelledby={headingId} className={`${BOX} ${flash ? 'kw-flash' : ''}`}>
      <div className="flex items-start gap-2">
        <PauseCircle size={15} className="mt-0.5 shrink-0 text-amber-400" aria-hidden="true" />
        <div className="min-w-0 flex-1">
          <p id={headingId} className="text-sm font-semibold text-amber-100 break-words">
            {copy.first}
          </p>
          <p className="mt-0.5 text-sm leading-[1.5] text-amber-200 break-words">{copy.body}</p>
          {show && (
            <div className="mt-2.5 flex flex-wrap gap-2">
              <button
                type="button"
                data-request-limit-action="continue"
                onClick={() => void runRetryOnce(lock, { text: CONTINUE_TEXT, attachments: [] }, (t) => onSend!(t.text), setState)}
                disabled={state !== 'idle'}
                aria-describedby={headingId}
                className={PRIMARY}
              >
                {state === 'sending' ? (
                  <Loader2 size={13} className="animate-spin motion-reduce:animate-none" aria-hidden="true" />
                ) : state === 'sent' ? (
                  <Check size={13} aria-hidden="true" />
                ) : (
                  <Play size={13} aria-hidden="true" />
                )}
                <span>{state === 'sending' ? 'Sending…' : state === 'sent' ? 'Sent' : 'Continue'}</span>
              </button>
            </div>
          )}
        </div>
      </div>
    </section>
  );
};
