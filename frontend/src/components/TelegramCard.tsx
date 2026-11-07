// Connections › Optional › Telegram (0.7.8, design §G): the catalogue card, the setup sheet with
// pairing, the connected card with its switches, Allow approvals and Disconnect.
import React, { useCallback, useEffect, useId, useRef, useState } from 'react';
import { createPortal } from 'react-dom';
import { AlertTriangle, Check, Clock, Copy, ExternalLink, Loader2, Lock, Plus, Power, RefreshCw, Send, WifiOff, X } from 'lucide-react';
import { useEscapeLayer } from '../escape';
import {
  COPY,
  KINDS,
  KindKey,
  PairLink,
  TG_EVENT,
  TelegramError,
  TelegramEventDetail,
  TelegramStatus,
  announceAt,
  countdown,
  hhmm,
  pairedName,
  parseStatus,
  qrSource,
  safeLink,
  shortDate,
  shortLink,
  statusLine,
  telegramApi,
  tokenErrorText,
} from '../telegram';

export type TelegramApi = typeof telegramApi;

const focusRing = 'focus-visible:outline focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-sky-500';
const actionBtn = `inline-flex items-center justify-center gap-1.5 min-h-[44px] sm:min-h-[32px] px-3 py-1 rounded-lg text-xs font-medium transition-colors disabled:opacity-50 ${focusRing}`;
const quietBtn = `${actionBtn} border border-zinc-700 bg-zinc-800/80 text-zinc-100 hover:bg-zinc-700 hover:border-zinc-600`;
const primaryBtn = `${actionBtn} text-white bg-sky-600 hover:bg-sky-500 shadow-sm`;
const dangerGhost = `${actionBtn} border border-rose-500/40 text-rose-200 hover:bg-rose-500/10`;
const dangerBtn = `${actionBtn} text-white bg-rose-600 hover:bg-rose-500`;
const textBtn = `inline-flex items-center min-h-[44px] sm:min-h-0 px-1 rounded text-xs font-medium text-sky-300 hover:text-sky-200 hover:underline underline-offset-2 ${focusRing}`;
const spin = 'animate-spin motion-reduce:animate-none';

/** Sheets, dialogs and toasts sit over the page, outside the card list. */
const Over: React.FC<{ children: React.ReactNode }> = ({ children }) => (typeof document === 'undefined' ? <>{children}</> : createPortal(children, document.body));

const Glyph: React.FC = () => (
  <span className="tg-glyph" aria-hidden="true">
    <Send size={16} />
  </span>
);

/** The G7 privacy paragraph: always visible, never collapsed. */
export const PrivacyNote: React.FC = () => (
  <p className="flex items-start gap-2 px-3 py-2 rounded-lg border border-amber-500/30 bg-amber-500/10 text-xs text-amber-100" data-tg-privacy>
    <AlertTriangle size={14} className="mt-px shrink-0 text-amber-300" aria-hidden="true" />
    <span>
      <b className="font-semibold text-amber-50">{COPY.privacyLead}</b> {COPY.privacyRest}
    </span>
  </p>
);

/** 32×18 switch; the row around it is the 44 px target on phone. */
const Switch: React.FC<{ on: boolean; labelledBy: string; describedBy?: string; armed?: boolean; disabled?: boolean; onToggle: () => void }> = ({
  on,
  labelledBy,
  describedBy,
  armed,
  disabled,
  onToggle,
}) => (
  <button
    type="button"
    role="switch"
    aria-checked={on}
    aria-labelledby={labelledBy}
    aria-describedby={describedBy}
    disabled={disabled}
    onClick={onToggle}
    className={`tg-sw ${on ? 'on' : ''} ${armed ? 'armed' : ''} ${focusRing}`}
    data-switch={labelledBy}
  >
    <i aria-hidden="true" />
  </button>
);

const KindList: React.FC<{ kinds: TelegramStatus['kinds']; compact?: boolean; busy?: KindKey | null; onToggle: (k: KindKey) => void }> = ({ kinds, compact, busy, onToggle }) => {
  const id = useId();
  return (
    <ul className={`tg-kinds ${compact ? 'compact' : ''}`}>
      {KINDS.map((k) => (
        <li key={k.key}>
          <Switch on={kinds[k.key]} labelledBy={`${id}-${k.key}`} describedBy={compact ? undefined : `${id}-${k.key}-h`} disabled={busy === k.key} onToggle={() => onToggle(k.key)} />
          <div className="min-w-0">
            <b id={`${id}-${k.key}`} className="block text-xs font-medium text-zinc-100">
              {k.title}
            </b>
            {!compact && (
              <small id={`${id}-${k.key}-h`} className="block text-[11px] text-zinc-400">
                {k.help}
              </small>
            )}
          </div>
        </li>
      ))}
    </ul>
  );
};

/** A small toast at the bottom of the screen; `action` is an Undo. */
const Toast: React.FC<{ text: string; action?: { label: string; run: () => void } }> = ({ text, action }) => (
  <div className="tg-toast" role="status" aria-live="polite" data-tg-toast>
    <span>{text}</span>
    {action && (
      <button type="button" className={textBtn} onClick={action.run}>
        {action.label}
      </button>
    )}
  </div>
);

// ---------------------------------------------------------------------------------------------
// Setup sheet

type SetupStep = { kind: 'token'; error: { text: string; net: boolean } | null; checking: boolean } | { kind: 'pair' } | { kind: 'paired' };

export interface SetupSheetProps {
  status: TelegramStatus;
  api: TelegramApi;
  /** Where to start: step 1 (also "Fix it" and "Change"), or pairing for a saved token. */
  start: 'token' | 'pair';
  onStatus: (s: TelegramStatus) => void;
  onClose: () => void;
  /** Test and shot seams. */
  now?: () => number;
  initialLink?: PairLink | null;
  initialStep?: SetupStep;
  initialToken?: string;
}

export const SetupSheet: React.FC<SetupSheetProps> = ({ status, api, start, onStatus, onClose, now = Date.now, initialLink = null, initialStep, initialToken = '' }) => {
  const id = useId();
  const rootRef = useRef<HTMLDivElement>(null);
  const tokenRef = useRef<HTMLInputElement>(null);
  const pairedHeadRef = useRef<HTMLHeadingElement>(null);
  const [step, setStep] = useState<SetupStep>(initialStep ?? (start === 'pair' && status.bot ? (status.state === 'paired' ? { kind: 'paired' } : { kind: 'pair' }) : { kind: 'token', error: null, checking: false }));
  const [token, setToken] = useState(initialToken);
  const defaultUrl = typeof location !== 'undefined' && location.protocol === 'https:' ? location.origin : '';
  const [appUrl, setAppUrl] = useState(status.app_url ?? defaultUrl);
  const [link, setLink] = useState<PairLink | null>(initialLink);
  const [linkError, setLinkError] = useState<string | null>(null);
  const [making, setMaking] = useState(false);
  const [expired, setExpired] = useState(false);
  const [tick, setTick] = useState(now());
  const [spoken, setSpoken] = useState('');
  const [toast, setToast] = useState<string | null>(null);
  const [kindBusy, setKindBusy] = useState<KindKey | null>(null);
  const [kindError, setKindError] = useState<string | null>(null);
  const checking = step.kind === 'token' && step.checking;

  useEscapeLayer(rootRef, true, onClose, { modal: true, unsaved: step.kind === 'token' && (token !== '' || checking) });

  const makeLink = useCallback(async () => {
    setMaking(true);
    setLinkError(null);
    try {
      const l = await api.pair();
      setLink(l);
      setExpired(false);
      setTick(now());
      setSpoken('Waiting for Start in Telegram. The link works once, for 10 minutes.');
    } catch (e) {
      if (e instanceof TelegramError && e.code === 'already_paired') {
        onStatus(await api.status().catch(() => status));
        setStep({ kind: 'paired' });
      } else setLinkError(e instanceof Error ? e.message : 'Couldn’t make a pairing link.');
    } finally {
      setMaking(false);
    }
  }, [api, now, onStatus, status]);

  // Entering step 2 without a link (a saved token, or step 1 just passed): make one.
  useEffect(() => {
    if (step.kind === 'pair' && !link && !making && !linkError) void makeLink();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [step.kind]);

  // The countdown: redrawn every second, spoken only at 5:00 and 1:00; expiry swaps the line.
  const left = countdown(link?.expires_at ?? null, tick);
  useEffect(() => {
    if (step.kind !== 'pair' || !link || expired) return;
    const t = window.setInterval(() => setTick(now()), 1000);
    return () => window.clearInterval(t);
  }, [step.kind, link, expired, now]);
  useEffect(() => {
    if (step.kind !== 'pair' || !link || expired) return;
    if (left.secs === 0) setExpired(true);
    else if (announceAt(left.secs)) setSpoken(`The link works for ${left.secs === 300 ? '5 more minutes' : '1 more minute'}.`);
  }, [left.secs, step.kind, link, expired]);
  useEffect(() => {
    if (status.pair_locked && step.kind === 'pair') setExpired(true);
  }, [status.pair_locked, step.kind]);

  // Live events: paired swaps the QR for the teal box; expired dims it.
  useEffect(() => {
    const on = (e: Event) => {
      const d = (e as CustomEvent<TelegramEventDetail>).detail;
      if (d?.type === 'telegram.expired') setExpired(true);
      if (d?.type === 'telegram.paired')
        void api
          .status()
          .then((s) => {
            onStatus(s);
            if (s.state === 'paired') setStep({ kind: 'paired' });
          })
          .catch(() => {});
    };
    window.addEventListener(TG_EVENT, on);
    return () => window.removeEventListener(TG_EVENT, on);
  }, [api, onStatus]);
  useEffect(() => {
    if (step.kind === 'pair' && status.state === 'paired') setStep({ kind: 'paired' });
  }, [status.state, step.kind]);
  useEffect(() => {
    if (step.kind === 'paired') pairedHeadRef.current?.focus();
  }, [step.kind]);
  useEffect(() => {
    if (!toast) return;
    const t = window.setTimeout(() => setToast(null), 4000);
    return () => window.clearTimeout(t);
  }, [toast]);

  const check = async (e: React.FormEvent) => {
    e.preventDefault();
    const t = token.trim();
    if (!t || checking) return;
    setStep({ kind: 'token', error: null, checking: true });
    try {
      const url = appUrl.trim().replace(/\/+$/, '');
      const s = await api.saveToken(t, url || undefined);
      setToken('');
      onStatus(s);
      setLink(null);
      setLinkError(null);
      setStep(s.state === 'paired' ? { kind: 'paired' } : { kind: 'pair' });
    } catch (err) {
      if (err instanceof TelegramError && err.code === 'app_url_not_https') {
        setStep({ kind: 'token', error: { text: err.message, net: false }, checking: false });
        return;
      }
      setStep({ kind: 'token', error: tokenErrorText(err), checking: false });
      tokenRef.current?.focus();
    }
  };

  const change = () => {
    setToken('');
    setLink(null);
    setStep({ kind: 'token', error: null, checking: false });
    window.setTimeout(() => tokenRef.current?.focus());
  };

  const copy = async () => {
    const l = link && safeLink(link.link);
    if (!l) return;
    try {
      await navigator.clipboard.writeText(l);
      setToast(COPY.copied);
    } catch {
      setToast('Couldn’t copy. Use Open in Telegram instead.');
    }
  };

  const toggleKind = async (k: KindKey) => {
    setKindBusy(k);
    setKindError(null);
    try {
      onStatus(await api.patch({ kinds: { [k]: !status.kinds[k] } }));
    } catch (e) {
      setKindError(e instanceof Error ? e.message : 'Couldn’t change that.');
    } finally {
      setKindBusy(null);
    }
  };

  const errId = `${id}-err`;
  const tokenError = step.kind === 'token' ? step.error : null;
  const open = link ? safeLink(link.link) : null;
  const qr = link ? qrSource(link.qr_svg) : null;
  const bot = status.bot;

  const stepHead = (n: number, label: string, done: boolean, right?: React.ReactNode) => (
    <div className="tg-step-h">
      <span className={`tg-n ${done ? 'ok' : ''}`} aria-hidden="true">
        {done ? <Check size={12} /> : n}
      </span>
      <h4 className="text-sm font-medium text-zinc-100">
        {label}
        {done && <span className="sr-only">, done</span>}
      </h4>
      {right}
    </div>
  );

  return (
    <Over>
    <div className="tg-scrim" data-tg-sheet={step.kind}>
      <div ref={rootRef} className="tg-pv" role="dialog" aria-modal="true" aria-labelledby={`${id}-h`} aria-describedby={`${id}-sub`} tabIndex={-1}>
        <div className="tg-pv-head">
          <Glyph />
          <div className="min-w-0 flex-1">
            <h3 id={`${id}-h`} className="text-[15px] font-semibold text-zinc-50">
              Talk to your bots from Telegram
            </h3>
            <p id={`${id}-sub`} className="text-xs text-zinc-400">
              Make your own Telegram bot, paste its token, then pair your Telegram account.
            </p>
          </div>
          <button type="button" className={`tg-icbtn ${focusRing}`} aria-label="Close" onClick={onClose}>
            <X size={16} aria-hidden="true" />
          </button>
        </div>

        <div className="tg-pv-body">
          {step.kind === 'token' ? (
            <>
              <form className="tg-step" onSubmit={check} id={`${id}-f`} noValidate aria-labelledby={`${id}-s1`}>
                <div className="tg-step-h">
                  <span className="tg-n" aria-hidden="true">
                    1
                  </span>
                  <h4 id={`${id}-s1`} className="text-sm font-medium text-zinc-100">
                    Your Telegram bot
                  </h4>
                </div>
                <ol className="tg-how">
                  <li>
                    In Telegram, open <code>@BotFather</code> and send <code>/newbot</code>.
                  </li>
                  <li>
                    Pick a name, then a username that ends in <code>bot</code>.
                  </li>
                  <li>BotFather replies with a token. Paste it here.</li>
                </ol>
                <div className="space-y-1">
                  <label htmlFor={`${id}-tok`} className="block text-xs font-medium text-zinc-200">
                    Bot token
                  </label>
                  <input
                    ref={tokenRef}
                    id={`${id}-tok`}
                    type="password"
                    autoComplete="off"
                    spellCheck={false}
                    autoCapitalize="off"
                    autoCorrect="off"
                    autoFocus
                    value={token}
                    onChange={(e) => {
                      setToken(e.target.value);
                      if (tokenError) setStep({ kind: 'token', error: null, checking: false });
                    }}
                    placeholder="Paste the token from BotFather"
                    disabled={checking}
                    aria-invalid={!!tokenError && !tokenError.net}
                    aria-describedby={tokenError ? errId : `${id}-help`}
                    className={`tg-field ${tokenError && !tokenError.net ? 'bad' : ''}`}
                    data-tg-token
                  />
                  {tokenError ? (
                    <p id={errId} role="alert" className="tg-err" data-tg-error={tokenError.net ? 'net' : 'bad'}>
                      {tokenError.net ? <WifiOff size={13} aria-hidden="true" /> : <X size={13} aria-hidden="true" />}
                      <span>{tokenError.text}</span>
                    </p>
                  ) : (
                    <p id={`${id}-help`} className="text-[11px] text-zinc-400">
                      Saved in the vault. The app never shows it again, not even to you.
                    </p>
                  )}
                </div>
                <div className="space-y-1">
                  <label htmlFor={`${id}-url`} className="block text-xs font-medium text-zinc-200">
                    This app's web address <span className="font-normal text-zinc-500">(optional)</span>
                  </label>
                  <input
                    id={`${id}-url`}
                    type="url"
                    inputMode="url"
                    autoComplete="off"
                    spellCheck={false}
                    value={appUrl}
                    onChange={(e) => setAppUrl(e.target.value)}
                    placeholder="https://kwant.example.com"
                    disabled={checking}
                    aria-describedby={`${id}-url-h`}
                    className="tg-field"
                  />
                  <p id={`${id}-url-h`} className="text-[11px] text-zinc-400">
                    With an https address, Telegram messages get an Open button that comes back here.
                  </p>
                </div>
                <PrivacyNote />
              </form>
              <section className="tg-step off" aria-label="Step 2, Pair your Telegram account, after step 1">
                {stepHead(2, 'Pair your Telegram account', false)}
              </section>
            </>
          ) : (
            <>
              <section className="tg-step done" aria-label="Step 1, Your Telegram bot, done">
                {stepHead(
                  1,
                  'Your Telegram bot',
                  true,
                  <span className="tg-sum">
                    <code>@{bot?.username}</code>
                    {bot?.name ? ` · ${bot.name}` : ''}
                    <button type="button" className={textBtn} onClick={change} aria-label="Change the Telegram bot">
                      Change
                    </button>
                  </span>,
                )}
              </section>
              {step.kind === 'paired' && status.paired ? (
                <section className="tg-step">
                  {stepHead(2, 'Pair your Telegram account', true)}
                  <div className="tg-paired" data-tg-paired>
                    <span className="tg-ok" aria-hidden="true">
                      <Check size={18} />
                    </span>
                    <div className="min-w-0">
                      <h4 ref={pairedHeadRef} tabIndex={-1} className="text-sm font-semibold text-teal-50 focus:outline-none">
                        Paired with {status.paired.name} {status.paired.username && <span className="font-normal text-teal-200/80">@{status.paired.username}</span>}
                      </h4>
                      <p className="text-xs text-teal-100/90">
                        Say hi from Telegram: anything you send goes to #general, and <code>@Name</code> reaches one bot. Send <code>/stop</code> there to unpair.
                      </p>
                    </div>
                  </div>
                  <div className="space-y-1.5">
                    <b className="block text-xs font-medium text-zinc-200">Sent to Telegram</b>
                    <KindList kinds={status.kinds} compact busy={kindBusy} onToggle={toggleKind} />
                    {kindError && (
                      <p role="alert" className="tg-err">
                        <X size={13} aria-hidden="true" />
                        <span>{kindError}</span>
                      </p>
                    )}
                    <p className="text-[11px] text-zinc-400">Change these any time in Connections › Telegram.</p>
                  </div>
                </section>
              ) : (
                <section className="tg-step" aria-labelledby={`${id}-s2`}>
                  <div className="tg-step-h">
                    <span className="tg-n" aria-hidden="true">
                      2
                    </span>
                    <h4 id={`${id}-s2`} className="text-sm font-medium text-zinc-100">
                      Pair your Telegram account
                    </h4>
                  </div>
                  <div className="tg-pair">
                    <div className={`tg-qr ${expired ? 'dim' : ''}`} role="img" aria-label="QR code for the pairing link" data-tg-qr>
                      {qr ? <img src={qr} alt="" width={168} height={168} draggable={false} /> : <span className="tg-qr-empty" />}
                      {expired && (
                        <span className="tg-qr-x" aria-hidden="true">
                          Expired
                        </span>
                      )}
                    </div>
                    <div className="min-w-0 flex-1 space-y-2.5">
                      <p className="text-xs text-zinc-300">
                        Scan it with your phone's camera, or open the link where you use Telegram. Then press <b className="text-zinc-100">Start</b> there.
                      </p>
                      {link && (
                        <p className="tg-link">
                          <code>{shortLink(link.link)}</code>
                        </p>
                      )}
                      <div className="tg-btns">
                        {expired || linkError ? (
                          <button type="button" className={primaryBtn} onClick={() => void makeLink()} disabled={making} data-tg-newlink>
                            {making ? <Loader2 size={13} className={spin} aria-hidden="true" /> : <RefreshCw size={13} aria-hidden="true" />}
                            Make a new link
                          </button>
                        ) : (
                          <>
                            {open ? (
                              <a href={open} target="_blank" rel="noopener noreferrer" className={primaryBtn}>
                                <ExternalLink size={13} aria-hidden="true" />
                                Open in Telegram
                              </a>
                            ) : (
                              <button type="button" className={primaryBtn} disabled>
                                <ExternalLink size={13} aria-hidden="true" />
                                Open in Telegram
                              </button>
                            )}
                            <button type="button" className={quietBtn} onClick={() => void copy()} disabled={!open}>
                              <Copy size={13} aria-hidden="true" />
                              Copy link
                            </button>
                          </>
                        )}
                      </div>
                      {linkError ? (
                        <p role="alert" className="tg-err">
                          <X size={13} aria-hidden="true" />
                          <span>{linkError}</span>
                        </p>
                      ) : expired ? (
                        <p role="status" className="tg-wait bad" data-tg-expired>
                          <Clock size={13} aria-hidden="true" />
                          <span>{status.pair_locked ? 'Too many wrong codes were tried. Make a new link to pair.' : COPY.expired}</span>
                        </p>
                      ) : (
                        <div className="tg-wait" data-tg-wait>
                          <Loader2 size={14} className={spin} aria-hidden="true" />
                          <span aria-hidden="true">
                            {link ? (
                              <>
                                Waiting for Start in Telegram · works once, for <b className="tabular-nums text-zinc-100">{left.text}</b> more
                              </>
                            ) : (
                              'Making a link…'
                            )}
                          </span>
                          <span role="status" className="sr-only">
                            {spoken}
                          </span>
                        </div>
                      )}
                    </div>
                  </div>
                  <p className="text-xs text-zinc-500">
                    Only the first Telegram account that presses Start is paired. Anyone else who messages @{bot?.username ?? 'your bot'}, and every group, gets no reply.
                  </p>
                </section>
              )}
            </>
          )}
        </div>

        <div className="tg-pv-foot">
          {step.kind === 'token' ? (
            <>
              <p className="tg-note">Nothing is saved until Telegram accepts the token.</p>
              <button type="button" className={quietBtn} onClick={onClose}>
                Cancel
              </button>
              <button type="submit" form={`${id}-f`} className={primaryBtn} disabled={!token.trim() || checking} data-tg-check>
                {checking ? (
                  <>
                    <Loader2 size={14} className={spin} aria-hidden="true" />
                    Checking with Telegram…
                  </>
                ) : (
                  'Check token'
                )}
              </button>
            </>
          ) : step.kind === 'paired' ? (
            <>
              <p className="tg-note">Approvals still happen in the app.</p>
              <button type="button" className={primaryBtn} onClick={onClose}>
                Done
              </button>
            </>
          ) : (
            <>
              <p className="tg-note">The bot token is saved. You can pair later from Connections.</p>
              <button type="button" className={quietBtn} onClick={onClose}>
                Close
              </button>
            </>
          )}
        </div>
        {toast && <Toast text={toast} />}
      </div>
    </div>
    </Over>
  );
};

// ---------------------------------------------------------------------------------------------
// Allow approvals: confirm

export const AllowConfirm: React.FC<{ busy?: boolean; onKeep: () => void; onAllow: () => void }> = ({ busy, onKeep, onAllow }) => {
  const id = useId();
  const rootRef = useRef<HTMLDivElement>(null);
  const keepRef = useRef<HTMLButtonElement>(null);
  useEscapeLayer(rootRef, true, onKeep, { modal: true });
  useEffect(() => keepRef.current?.focus(), []);
  return (
    <Over>
    <div className="tg-scrim center" data-tg-allow-confirm>
      <div ref={rootRef} className="tg-confirm" role="alertdialog" aria-modal="true" aria-labelledby={`${id}-h`} aria-describedby={`${id}-p`} tabIndex={-1}>
        <h3 id={`${id}-h`} className="text-[15px] font-semibold text-zinc-50">
          Allow approvals from Telegram?
        </h3>
        <p id={`${id}-p`} className="text-xs text-zinc-300">
          Anyone holding your Telegram account could then approve for your bots. Telegram bot chats aren't end-to-end encrypted.
        </p>
        <ul className="tg-bullets">
          <li>
            Only <b>Allow once</b>. “Always allow” stays in the app.
          </li>
          <li>Never for production connections, payment floors, human steps or Take over. Those still open the app.</li>
          <li>Each approval made there is logged as “from Telegram”.</li>
        </ul>
        <div className="tg-row">
          <button ref={keepRef} type="button" className={quietBtn} onClick={onKeep}>
            Keep it off
          </button>
          <button type="button" className={primaryBtn} onClick={onAllow} disabled={busy}>
            {busy && <Loader2 size={13} className={spin} aria-hidden="true" />}
            Allow from Telegram
          </button>
        </div>
      </div>
    </div>
    </Over>
  );
};

// ---------------------------------------------------------------------------------------------
// Cards

export const CatalogCard: React.FC<{ onSetup: () => void }> = ({ onSetup }) => (
  <li className="rounded-xl border border-zinc-800 bg-zinc-900/60 p-3 sm:p-4 space-y-3" aria-labelledby="tg-card-n" data-catalog="telegram">
    <div className="flex flex-wrap items-start gap-3">
      <Glyph />
      <div className="min-w-0 flex-1 basis-48">
        <div className="flex flex-wrap items-center gap-2">
          <h4 id="tg-card-n" className="text-sm font-medium text-zinc-100">
            Telegram
          </h4>
          <span className="px-1.5 py-px rounded border text-[11px] border-zinc-700 bg-zinc-800 text-zinc-300">Optional</span>
        </div>
        <p className="mt-1 text-xs text-zinc-400">{COPY.description}</p>
      </div>
      <button type="button" className={`${quietBtn} max-sm:w-full`} onClick={onSetup} aria-haspopup="dialog">
        <Plus size={13} aria-hidden="true" /> Set up Telegram
      </button>
    </div>
    <PrivacyNote />
  </li>
);

export interface ConnectedCardProps {
  status: TelegramStatus;
  api: TelegramApi;
  onStatus: (s: TelegramStatus) => void;
  onSetup: (start: 'token' | 'pair') => void;
  onGone: () => void;
  /** Shot seams. */
  initialConfirm?: 'allow' | 'disconnect' | null;
  now?: () => number;
}

export const ConnectedCard: React.FC<ConnectedCardProps> = ({ status, api, onStatus, onSetup, onGone, initialConfirm = null, now = Date.now }) => {
  const id = useId();
  const [confirm, setConfirm] = useState<'allow' | 'disconnect' | null>(initialConfirm);
  const [busy, setBusy] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [toast, setToast] = useState<{ text: string; undo?: boolean } | null>(null);
  const [unpaired, setUnpaired] = useState(false);
  const allowRef = useRef<HTMLButtonElement | null>(null);
  const footRef = useRef<HTMLDivElement>(null);
  const keepRef = useRef<HTMLButtonElement>(null);

  // The Undo toast lasts until `undo_until` (10 s).
  useEffect(() => {
    if (!toast) return;
    const until = toast.undo && status.undo_until ? Date.parse(status.undo_until) - now() : 4000;
    const t = window.setTimeout(() => setToast(null), Math.max(1000, Math.min(10_000, Number.isFinite(until) ? until : 10_000)));
    return () => window.clearTimeout(t);
  }, [toast, status.undo_until, now]);
  useEffect(() => {
    if (confirm === 'disconnect') keepRef.current?.focus();
  }, [confirm]);

  const run = async (key: string, f: () => Promise<TelegramStatus | void>) => {
    setBusy(key);
    setError(null);
    try {
      const s = await f();
      if (s) onStatus(s);
      return true;
    } catch (e) {
      setError(e instanceof Error ? e.message : 'Something went wrong.');
      return false;
    } finally {
      setBusy(null);
    }
  };

  const line = statusLine(status);
  const paired = status.state === 'paired' && status.paired;
  const since = shortDate(status.paired?.since ?? null);
  const allowSince = hhmm(status.allow_since);

  const unpair = async () => {
    if (await run('unpair', () => api.unpair())) {
      setUnpaired(true);
      setToast({ text: `Unpaired ${status.paired?.name ?? 'Telegram'}.`, undo: true });
    }
  };
  const undo = async () => {
    setToast(null);
    if (await run('undo', () => api.undoUnpair())) setUnpaired(false);
  };
  const setAllow = async (on: boolean) => {
    const ok = await run('allow', () => api.patch({ allow_approvals: on }));
    setConfirm(null);
    if (ok || !on) window.setTimeout(() => allowRef.current?.focus());
  };
  const disconnect = async () => {
    if (await run('disconnect', () => api.disconnect())) onGone();
  };

  return (
    <li className="rounded-xl border border-zinc-800 bg-zinc-900/60 p-3 sm:p-4 space-y-3" aria-labelledby={`${id}-n`} data-catalog="telegram" data-tg-state={status.state}>
      <div className="flex items-start gap-3">
        <Glyph />
        <div className="min-w-0 flex-1">
          <div className="flex flex-wrap items-center gap-2">
            <h4 id={`${id}-n`} className="text-sm font-medium text-zinc-100">
              Telegram
            </h4>
            {status.state === 'paired' ? (
              <span className="px-1.5 py-px rounded border text-[11px] border-emerald-500/30 bg-emerald-500/10 text-emerald-200">On</span>
            ) : (
              <span className="px-1.5 py-px rounded border text-[11px] border-zinc-700 bg-zinc-800 text-zinc-300">Not paired</span>
            )}
          </div>
          <p className={`tg-status ${line.tone}`} data-tg-status={line.tone}>
            <span className="tg-dot" aria-hidden="true" />
            <span>
              {line.text}
              {line.tone === 'bad' && (
                <>
                  {' '}
                  <button type="button" className={textBtn} onClick={() => onSetup('token')}>
                    Fix it
                  </button>
                </>
              )}
            </span>
          </p>
        </div>
      </div>

      <dl className="tg-kv">
        <dt>Bot</dt>
        <dd>
          <code>@{status.bot?.username}</code> · token in the vault
        </dd>
        <dt>Paired with</dt>
        <dd>
          {paired ? (
            <>
              {status.paired!.name} {status.paired!.username && <span className="text-zinc-400">@{status.paired!.username}</span>}
              {since ? ` · since ${since}` : ''}{' '}
              <button type="button" className={textBtn} onClick={() => void unpair()} disabled={busy === 'unpair'} aria-label={`Unpair ${pairedName(status.paired!)}`}>
                Unpair
              </button>
            </>
          ) : (
            <>
              {unpaired ? 'Not paired' : 'Not paired yet'} ·{' '}
              <button type="button" className={textBtn} onClick={() => onSetup('pair')} disabled={status.state === 'rejected'} aria-haspopup="dialog" data-tg-pair>
                Pair
              </button>
            </>
          )}
        </dd>
        <dt>Messages go to</dt>
        <dd>#general, answered as in the app</dd>
        <dt>Privacy</dt>
        <dd className="text-amber-200">
          <AlertTriangle size={13} className="inline -mt-0.5 mr-1 text-amber-300" aria-hidden="true" />
          {COPY.privacyLead}
        </dd>
      </dl>

      <div className="tg-sub">
        <h5 className="text-xs font-medium text-zinc-200">Send me “needs you” alerts for</h5>
        <KindList kinds={status.kinds} busy={busy?.startsWith('kind:') ? (busy.slice(5) as KindKey) : null} onToggle={(k) => void run(`kind:${k}`, () => api.patch({ kinds: { [k]: !status.kinds[k] } }))} />
      </div>

      <div className={`tg-sub tg-allow ${status.allow_approvals ? 'on' : ''}`} data-tg-allow={status.allow_approvals ? 'on' : 'off'}>
        <div className="tg-allow-r">
          <span ref={(el) => (allowRef.current = (el?.querySelector('button') as HTMLButtonElement) ?? null)} className="contents">
            <Switch
              on={status.allow_approvals}
              armed={status.allow_approvals}
              labelledBy={`${id}-al`}
              describedBy={`${id}-al-h`}
              disabled={!paired || busy === 'allow'}
              onToggle={() => (status.allow_approvals ? void setAllow(false) : setConfirm('allow'))}
            />
          </span>
          <div className="min-w-0">
            <b id={`${id}-al`} className="block text-xs font-medium text-zinc-100">
              Allow approvals from Telegram
            </b>
            <small id={`${id}-al-h`} className="block text-[11px] text-zinc-400">
              {status.allow_approvals ? (
                <>
                  {allowSince ? `On since ${allowSince}. ` : ''}Only <b className="text-amber-100">Allow once</b> is offered there. Each one is logged as “from Telegram”.
                </>
              ) : paired ? (
                COPY.allowHelpOff
              ) : (
                `${COPY.allowHelpOff} Pair first to change it.`
              )}
            </small>
          </div>
        </div>
        {status.allow_approvals && (
          <p className="tg-never" data-tg-never>
            <Lock size={12} aria-hidden="true" />
            <span>{COPY.never}</span>
          </p>
        )}
      </div>

      {error && (
        <p role="alert" className="tg-err">
          <X size={13} aria-hidden="true" />
          <span>{error}</span>
        </p>
      )}

      {confirm === 'disconnect' ? (
        <div className="tg-inline-confirm" role="group" aria-labelledby={`${id}-dc`} data-tg-disconnect>
          <b id={`${id}-dc`} className="block text-sm font-semibold text-rose-100">
            Disconnect Telegram?
          </b>
          <p className="text-xs text-rose-100/90">
            The token is deleted from the vault{status.paired ? ` and ${status.paired.name} is unpaired` : ''}. Your bot stays in Telegram; delete it with <code>@BotFather</code> if you don't need it.
          </p>
          <div className="tg-row">
            <button ref={keepRef} type="button" className={quietBtn} onClick={() => (setConfirm(null), window.setTimeout(() => footRef.current?.querySelector<HTMLButtonElement>('[data-tg-disconnect-open]')?.focus()))}>
              Keep it
            </button>
            <button type="button" className={dangerBtn} onClick={() => void disconnect()} disabled={busy === 'disconnect'}>
              {busy === 'disconnect' && <Loader2 size={13} className={spin} aria-hidden="true" />}
              Disconnect
            </button>
          </div>
        </div>
      ) : (
        <div ref={footRef} className="tg-foot">
          <button
            type="button"
            className={quietBtn}
            disabled={!paired || busy === 'test'}
            onClick={() => void run('test', () => api.test()).then((ok) => ok && setToast({ text: COPY.testSent }))}
          >
            {busy === 'test' ? <Loader2 size={13} className={spin} aria-hidden="true" /> : <Send size={13} aria-hidden="true" />}
            Send a test message
          </button>
          <button type="button" className={dangerGhost} onClick={() => setConfirm('disconnect')} data-tg-disconnect-open>
            <Power size={13} aria-hidden="true" />
            Disconnect
          </button>
        </div>
      )}

      {confirm === 'allow' && <AllowConfirm busy={busy === 'allow'} onKeep={() => (setConfirm(null), window.setTimeout(() => allowRef.current?.focus()))} onAllow={() => void setAllow(true)} />}
      {toast && (
        <Over>
          <Toast text={toast.text} action={toast.undo && busy !== 'undo' ? { label: 'Undo', run: () => void undo() } : undefined} />
        </Over>
      )}
    </li>
  );
};

// ---------------------------------------------------------------------------------------------
// The whole entry in Connections › Optional

export interface TelegramConnectionProps {
  api?: TelegramApi;
  /** Test and shot seams: a status to start from, and the sheet open. */
  initial?: TelegramStatus | null;
  initialSheet?: Omit<SetupSheetProps, 'status' | 'api' | 'onStatus' | 'onClose'> | null;
  initialConfirm?: 'allow' | 'disconnect' | null;
  now?: () => number;
}

export const TelegramConnection: React.FC<TelegramConnectionProps> = ({ api = telegramApi, initial = null, initialSheet = null, initialConfirm = null, now }) => {
  const [status, setStatus] = useState<TelegramStatus | null>(initial);
  const [loadError, setLoadError] = useState<string | null>(null);
  const [sheet, setSheet] = useState<Omit<SetupSheetProps, 'status' | 'api' | 'onStatus' | 'onClose'> | null>(initialSheet);

  const refresh = useCallback(() => {
    api
      .status()
      .then((s) => (setStatus(s), setLoadError(null)))
      .catch((e) => setLoadError(e instanceof Error ? e.message : 'Couldn’t load Telegram.'));
  }, [api]);
  useEffect(() => {
    if (!initial) refresh();
  }, [initial, refresh]);
  useEffect(() => {
    const on = (e: Event) => {
      if ((e as CustomEvent<TelegramEventDetail>).detail?.type === 'telegram.status') refresh();
    };
    window.addEventListener(TG_EVENT, on);
    return () => window.removeEventListener(TG_EVENT, on);
  }, [refresh]);

  const open = (start: 'token' | 'pair') => setSheet({ start, now });
  const close = () => {
    setSheet(null);
    refresh();
  };

  const s = status ?? parseStatus({ state: 'off' });
  return (
    <>
      {loadError && !status ? (
        <li className="rounded-xl border border-zinc-800 bg-zinc-900/60 p-3 sm:p-4 text-xs text-zinc-300" data-catalog="telegram">
          <span role="alert">Telegram: {loadError}</span>{' '}
          <button type="button" className={textBtn} onClick={refresh}>
            Try again
          </button>
        </li>
      ) : s.state === 'off' ? (
        <CatalogCard onSetup={() => open('token')} />
      ) : (
        <ConnectedCard status={s} api={api} onStatus={setStatus} onSetup={open} onGone={() => setStatus(parseStatus({ state: 'off' }))} initialConfirm={initialConfirm} now={now} />
      )}
      {sheet && <SetupSheet {...sheet} status={s} api={api} onStatus={setStatus} onClose={close} />}
    </>
  );
};
