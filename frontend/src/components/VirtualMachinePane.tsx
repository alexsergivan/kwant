import React, { useState, useRef, useEffect, useCallback, useMemo } from 'react';
import {
  X,
  Globe,
  Terminal,
  RefreshCw,
  ExternalLink,
  Cpu,
  Key,
  MousePointerClick,
  Loader2,
  Laptop,
  Puzzle,
  CheckCircle2,
  Eye,
  Hand,
  Crosshair,
} from 'lucide-react';
import { playSfx } from '../audio/chiptune';
import { BOT_BUSY_TEXT, HELD_BY_PERSON_TEXT, useBrowserStream } from '../useBrowserStream';
import { ComputerModeChip, ComputerModeError, LocalBrowserNote, OpenWindowButton, useComputerMode } from './ComputerModeSwitch';
import { browserFooterLabel, isBlankPage, showsLocalWindow } from '../computerMode';
import { loginMatchesPage } from '../savedPasswords';
import { PendingLoginPrompt, SavedPasswordsPanel, useSavedPasswords } from './SavedPasswordsPanel';
import { usePasswordStore } from './PasswordStorePanel';
import { BrowserControlOverlay } from './BrowserControlOverlay';
import { BrowserTakeoverStage } from './BrowserTakeoverStage';
import { HumanStep, HumanStepKind } from '../humanSteps';
import type { SwitchEntry } from '../watchLive';
import { BrowserSwitcherMenu, BrowserSwitcherRow } from './WatchLive';
import { showOnce, showOnceStore, useShowOnce } from '../showOnce';
import { RecordStepsButton, RecordingStrip } from './ShowOnce';
import { PointLayer, PointSink } from './PointLayer';
import { POINT_COPY, footerText } from '../point';

const PHONE_QUERY = '(max-width: 639px)';

/** While the bot finishes a step, taking over is retried this often, for this long (no backend change). */
export const TAKEOVER_RETRY_MS = 1000;
export const TAKEOVER_RETRY_FOR_MS = 10_000;

/** "@Bot needs you …" for a step holding this browser (Mira's spec §3 C). */
export const STEP_NEED: Record<HumanStepKind, string> = {
  sign_in: 'needs you to sign in',
  two_factor: 'needs you to enter a code',
  captcha: 'needs you to solve a puzzle',
  other: 'needs you in the browser',
};

/** The stage's primary "Done" for each kind (captcha keeps its auto-continue, so it has none). */
export const STEP_DONE: Record<Exclude<HumanStepKind, 'captcha'>, string> = {
  sign_in: 'Done, I’m signed in',
  two_factor: 'Done, I’m in',
  other: 'Done',
};

/** Below 640px taking control opens the page full screen. */
function useNarrow(): boolean {
  const [narrow, setNarrow] = useState(() => typeof window !== 'undefined' && !!window.matchMedia?.(PHONE_QUERY).matches);
  useEffect(() => {
    const mq = window.matchMedia?.(PHONE_QUERY);
    if (!mq) return;
    const on = () => setNarrow(mq.matches);
    mq.addEventListener('change', on);
    return () => mq.removeEventListener('change', on);
  }, []);
  return narrow;
}

export interface TerminalLogEntry {
  command: string;
  output: string;
  timestamp?: string;
}

export interface VirtualMachinePaneProps {
  isOpen: boolean;
  onClose: () => void;
  teamId?: string;
  teamName?: string;
  activeUrl?: string;
  pageTitle?: string;
  screenshotBase64?: string | null;
  isRunning?: boolean;
  browserError?: string | null;
  terminalLogs?: TerminalLogEntry[];
  onRefresh?: () => void;
  onRestartVm?: () => void;
  onNavigate?: (url: string) => void;
  /** Some bot in the space runs without asking (shown next to saved passwords). */
  approvalsOff?: boolean;
  /** The CAPTCHA this bot's browser is stuck on (open, or closed a moment ago). */
  captchaStep?: HumanStep | null;
  /** Goes up each time the person asks to solve it: take control and, on a phone, go full screen. */
  solveRequest?: number;
  /** An open sign-in, code or other step this browser is held for: the stage offers its Done. */
  humanStep?: HumanStep | null;
  /** W3: chats whose browser is active right now; with 2 or more the header becomes a switcher. */
  switcher?: { entries: SwitchEntry[]; current: string; onPick: (owner: string) => void } | null;
  /** Answers that step as done; resolves to an error message on failure. */
  onStepDone?: (id: string) => Promise<string | void> | void;
  /** Show it once: the single bot this browser belongs to (not set for teams and channels). */
  showOnceBot?: { id: string; name: string };
  /** G3 Point at it: where picks go (the composer). Without it there's no Point button. */
  point?: PointSink;
}

type PaneTab = 'browser' | 'terminal' | 'vault';

// On a phone, going back to the list unmounts the chat while the pane stays open, so the
// chosen tab lives here, per bot, or it would fall back to Browser on the way back.
const chosenTabs = new Map<string, PaneTab>();

/** The tab the pane last showed for this bot or team, for when it opens again. */
export function chosenPaneTab(teamId: string | undefined): PaneTab {
  return (teamId && chosenTabs.get(teamId)) || 'browser';
}

// A notification tap switches chats, which mounts a new pane: the request it carries is still new to that one.
let handledSolve = 0;

export function rememberPaneTab(teamId: string | undefined, tab: PaneTab): void {
  if (teamId) chosenTabs.set(teamId, tab);
}

export const VirtualMachinePane: React.FC<VirtualMachinePaneProps> = ({
  isOpen,
  onClose,
  teamId,
  teamName = 'Team',
  activeUrl = '',
  pageTitle = '',
  screenshotBase64,
  isRunning = true,
  browserError,
  terminalLogs = [],
  onRefresh,
  onRestartVm,
  onNavigate,
  approvalsOff = false,
  captchaStep = null,
  solveRequest = 0,
  humanStep = null,
  onStepDone,
  point,
  switcher,
  showOnceBot,
}) => {
  const [activeTab, setActiveTab] = useState<PaneTab>(() => chosenPaneTab(teamId));
  useEffect(() => rememberPaneTab(teamId, activeTab), [teamId, activeTab]);
  const [inputUrl, setInputUrl] = useState(activeUrl);
  // No event fires when the browser catches a login, so a new screenshot or page loads the list again.
  const passwords = useSavedPasswords(teamId, `${activeUrl}|${screenshotBase64?.length ?? 0}|${screenshotBase64?.slice(-32) ?? ''}`);
  const savedCount = passwords.logins.length;
  // Quiet while the pane is closed: a background load mustn't open "Confirm it's you" on page load.
  const passwordStore = usePasswordStore(!!teamId && teamId !== 'default', !isOpen);

  // Local Chrome streams here like the VM, unless the person opened it as a window: then there's
  // nothing to show. Unknown counts as vm.
  const computer = useComputerMode(isOpen);
  const isLocal = computer.state.settings?.mode === 'local';
  const isWindow = showsLocalWindow(computer.state.settings);

  // Interactive Remote Control State
  const stream = useBrowserStream(teamId, isOpen && activeTab === 'browser' && !isWindow && !computer.windowPending);
  const isControlMode = stream.controlling;
  const isInteracting = stream.busy;
  const narrow = useNarrow();
  // Wanted open (asked for, control on its way) or in control: the page takes over the screen
  const [fullScreen, setFullScreen] = useState(false);
  const takeover = (fullScreen || isControlMode) && activeTab === 'browser' && !isWindow && isOpen;
  const showFullScreen = takeover && narrow;
  const showStage = takeover && !narrow;
  const captchaOpen = captchaStep?.status === 'open';
  // Shown for a moment after the CAPTCHA closes by itself, before control goes back to the bot
  const [solvedNote, setSolvedNote] = useState(false);

  // Once the stream reports where the tab is, trust it; before that, the last URL the app heard of.
  const pageUrl = stream.location ? stream.location.url : activeUrl;
  // The same for the title: a new page's empty title must not fall back to the previous page's.
  const shownTitle = stream.location ? stream.location.title : pageTitle;
  const currentScreenshot = isBlankPage(pageUrl) ? null : stream.frame?.image || screenshotBase64;
  const frameSrc = currentScreenshot ? (currentScreenshot.startsWith('data:') ? currentScreenshot : `data:image/jpeg;base64,${currentScreenshot}`) : null;
  // A hidden frame never loads, so acknowledge it here or the stream stalls waiting for the ack.
  const blankFrameId = isBlankPage(pageUrl) ? stream.frame?.id : undefined;
  const { acknowledgeFrame } = stream;
  useEffect(() => {
    if (blankFrameId !== undefined) acknowledgeFrame(blankFrameId);
  }, [blankFrameId, acknowledgeFrame]);
  const controlError = stream.error || (!stream.connected ? browserError : null);
  // A live stream is ready; otherwise trust isRunning only while no error says the service is down.
  const browserReady = stream.connected || (isRunning && !controlError);
  useEffect(() => {
    if (stream.location?.url) setInputUrl(stream.location.url);
  }, [stream.location]);

  useEffect(() => {
    setInputUrl(activeUrl);
  }, [activeUrl]);

  // "Solve it" on the card or a CAPTCHA notification: take control once the stream is up
  const [wantsControl, setWantsControl] = useState(false);
  useEffect(() => {
    if (solveRequest <= handledSolve) return;
    handledSolve = solveRequest;
    setActiveTab('browser');
    setSolvedNote(false);
    setWantsControl(true);
    setFullScreen(true);
  }, [solveRequest]);
  const { connected, controlling, send } = stream;
  useEffect(() => {
    if (!wantsControl || !connected) return;
    setWantsControl(false);
    if (!controlling) send({ action: 'take_control' });
  }, [wantsControl, connected, controlling, send]);

  // The CAPTCHA went away: by itself it says so, then hands the browser back; any other way just closes
  const captchaStatus = captchaStep?.status;
  const lastCaptcha = useRef<{ id?: string; status?: string }>({ id: captchaStep?.id, status: captchaStatus });
  useEffect(() => {
    const was = lastCaptcha.current;
    lastCaptcha.current = { id: captchaStep?.id, status: captchaStatus };
    if (was.status !== 'open' || was.id !== captchaStep?.id || captchaStatus === 'open') return;
    const finish = () => {
      setFullScreen(false);
      setSolvedNote(false);
      if (stream.controlling) send({ action: 'release_control' });
    };
    if (captchaStatus === 'resolved_auto') {
      setSolvedNote(true);
      playSfx('coin');
      const t = setTimeout(finish, 1500);
      return () => clearTimeout(t);
    }
    finish();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [captchaStep?.id, captchaStatus]);

  const handBack = () => {
    playSfx('click');
    setWaitBot(null);
    setFullScreen(false);
    setWantsControl(false);
    if (isControlMode) send({ action: 'release_control' });
  };

  // Show it once (design §S). The recording that runs on this browser, if any
  const so = useShowOnce();
  const soHere = so && teamId && so.stage === teamId ? so : null;
  const recording = soHere?.phase === 'recording';
  // Started from the Skills tab: take over first (Start needs it), then Start once control holds
  const soTaking = soHere?.phase === 'taking';
  useEffect(() => {
    if (!soTaking) return;
    if (controlling) {
      void showOnce.startNow();
      return;
    }
    setActiveTab('browser');
    setFullScreen(true);
    setWantsControl(true);
  }, [soTaking, controlling]);
  // S-B5: a recording left open (reload, closed tab, restart) comes back when Take over opens
  useEffect(() => {
    if (controlling && showOnceBot && teamId && !showOnceStore.get()) void showOnce.resumeOpen(showOnceBot.id, showOnceBot.name, teamId);
  }, [controlling, showOnceBot, teamId]);
  // A27: the review closed after Hand back (or Writing's Close): now the browser goes back
  const handBackRef = useRef<() => void>(() => {});
  handBackRef.current = handBack;
  useEffect(() => showOnceStore.onEnd(({ stage, handBack: hb }) => hb && stage === teamId && handBackRef.current()), [teamId]);
  // Hand back while recording stops first and opens the review; it hands back when that closes
  const handBackOrStop = () => {
    if (recording) {
      void showOnce.stop(true);
      return;
    }
    handBack();
  };
  const recordSteps =
    showOnceBot && teamId && controlling && !soHere ? (
      <RecordStepsButton
        className="shrink-0 inline-flex items-center gap-1.5 h-8 px-3 rounded-[10px] text-[13px] font-semibold border border-[#3f3868] bg-[rgb(44_37_82/0.8)] text-[#ecebfa] hover:bg-[#2c2552] focus-visible:outline focus-visible:outline-2 focus-visible:outline-[#b8adff]"
        onClick={() => {
          playSfx('click');
          void showOnce.record(showOnceBot.id, showOnceBot.name, teamId);
        }}
      />
    ) : null;

  // Take the browser now (a click on the watch view, Take over, or Try again)
  const takeOver = useCallback(() => {
    playSfx('click');
    setWaitBot(null);
    setActiveTab('browser');
    setFullScreen(true);
    if (connected) send({ action: 'take_control' });
    else setWantsControl(true);
  }, [connected, send]);

  // The server let go (timeout, the bot's turn ended it, another tab): the stage closes with it
  const wasControlling = useRef(false);
  useEffect(() => {
    if (wasControlling.current && !controlling) setFullScreen(false);
    wasControlling.current = controlling;
  }, [controlling]);

  // Opening a sign-in or code step's browser takes control the same way Solve it does
  const stepForStage = humanStep && humanStep.status === 'open' && humanStep.kind !== 'captcha' && onStepDone
    ? { doneLabel: STEP_DONE[humanStep.kind], need: STEP_NEED[humanStep.kind], onDone: () => onStepDone(humanStep.id) }
    : null;
  // Any open step holding this browser, the CAPTCHA included: the watch view says so without a hover
  const needsYou = humanStep?.status === 'open' ? STEP_NEED[humanStep.kind] : captchaOpen ? STEP_NEED.captcha : null;

  // The bot is finishing a step: keep asking every second for ten seconds, then offer Try again
  const [waitBot, setWaitBot] = useState<{ since: number; gaveUp: boolean } | null>(null);
  const botBusy = stream.error === BOT_BUSY_TEXT;
  useEffect(() => {
    if (controlling) {
      setWaitBot(null);
      return;
    }
    if (!botBusy) return;
    if (!narrow) setFullScreen(false);
    setWaitBot((w) => w ?? { since: Date.now(), gaveUp: false });
  }, [botBusy, controlling, narrow]);
  useEffect(() => {
    if (!waitBot || waitBot.gaveUp || !botBusy) return;
    if (Date.now() - waitBot.since >= TAKEOVER_RETRY_FOR_MS) {
      setWaitBot({ ...waitBot, gaveUp: true });
      return;
    }
    const t = window.setTimeout(() => send({ action: 'take_control' }), TAKEOVER_RETRY_MS);
    return () => window.clearTimeout(t);
  }, [waitBot, botBusy, send]);
  const waitingForBot = !!waitBot && !waitBot.gaveUp;

  // The person holds this browser in another window: say so once, never retry. Taking over here
  // waits for them to press Hand back there and then Take over here, so a browser never moves without a press.
  const heldElsewhere = stream.error === HELD_BY_PERSON_TEXT && !controlling;
  useEffect(() => {
    if (!heldElsewhere) return;
    setFullScreen(false);
    setWaitBot(null);
  }, [heldElsewhere]);

  // The frame the side pane shows is acknowledged once drawn; one image, its source swapped
  const paneFrameId = useRef<number | null>(null);
  paneFrameId.current = stream.frame?.id ?? null;

  const loginsOnThisPage = useMemo(() => passwords.logins.filter((l) => loginMatchesPage(l, pageUrl)).length, [passwords.logins, pageUrl]);

  // G3 Point at it: shown while a page is on screen and the server knows the point actions
  const [pointing, setPointing] = useState(false);
  const pointButton = useRef<HTMLButtonElement>(null);
  const pointShown = !!point && !isWindow && activeTab === 'browser' && !!frameSrc && stream.connected && !stream.unsupported.has('point');
  const pointOn = pointing && pointShown && !takeover;
  useEffect(() => {
    if (pointing && (!pointShown || takeover)) setPointing(false);
  }, [pointing, pointShown, takeover]);
  const stopPointing = useCallback(() => {
    setPointing(false);
    pointButton.current?.focus();
  }, []);

  if (!isOpen) return null;
  const switching = !!switcher && switcher.entries.length >= 2;

  return (
    // On a phone the panel is the whole screen (0.7.7 nice-to-have 5): a 384 px column beside the chat left the chat a sliver.
    <div
      data-vm-pane=""
      className={`${narrow ? 'fixed inset-0 z-[60] w-full h-[100dvh] pt-[env(safe-area-inset-top)] pb-[env(safe-area-inset-bottom)]' : 'w-96 border-l border-zinc-800 h-full z-30'} bg-zinc-950 flex flex-col shadow-2xl select-none animate-in slide-in-from-right duration-200`}
    >
      {/* Top Header */}
      <div className="h-14 border-b border-zinc-800 px-4 flex items-center justify-between gap-2 bg-zinc-900/60">
        <div className="flex items-center gap-2 min-w-0">
          <div className={`shrink-0 w-2.5 h-2.5 rounded-full ${browserReady ? 'bg-emerald-400 animate-pulse' : 'bg-zinc-500'}`} />
          <div className="min-w-0">
            {switching && !narrow ? (
              <BrowserSwitcherMenu
                entries={switcher!.entries}
                current={switcher!.current}
                onPick={switcher!.onPick}
                disabled={isControlMode}
                title={`@${teamName}`}
                sub={<p className="text-[10px] text-zinc-400 truncate">{isLocal ? 'Your Chrome' : 'Live Chromium'}</p>}
              />
            ) : (
              <>
                <div className="text-xs font-semibold text-zinc-100 truncate" title={`@${teamName}`}>@{teamName}</div>
                <p className="text-[10px] text-zinc-400 truncate">{isLocal ? 'Your Chrome' : 'Live Chromium'}</p>
              </>
            )}
          </div>
        </div>

        <div className="flex items-center gap-1 shrink-0">
          <ComputerModeChip api={computer} />
          <OpenWindowButton api={computer} />

          {pointShown || (point && takeover && !isWindow) ? (
            <button
              ref={pointButton}
              type="button"
              data-point-button=""
              aria-pressed={pointOn}
              aria-label={POINT_COPY.buttonName}
              title={takeover ? POINT_COPY.handBack : POINT_COPY.buttonName}
              disabled={takeover}
              onClick={() => {
                playSfx('click');
                setPointing((p) => !p);
              }}
              className={`${narrow ? 'h-11' : 'h-[30px]'} px-2.5 rounded-[10px] inline-flex items-center gap-1.5 text-[13px] font-semibold whitespace-nowrap cursor-pointer disabled:cursor-not-allowed disabled:opacity-50 focus:outline-none focus-visible:ring-2 focus-visible:ring-violet-300 border ${
                pointOn ? 'bg-violet-600 border-violet-400 text-white ring-[3px] ring-violet-500/25' : 'border-[#3f3868] text-zinc-200 hover:bg-[#2c2552]'
              }`}
            >
              <Crosshair size={14} aria-hidden="true" />
              {POINT_COPY.button}
            </button>
          ) : null}

          {/* Take over: always there while a browser exists, so keyboard users have a way in.
              On a phone with Point showing, the view's own Take over is the way in (no room for both). */}
          {!isWindow && !(narrow && pointShown && !isControlMode) && (
            <button
              type="button"
              onClick={() => (isControlMode ? handBack() : takeOver())}
              disabled={!stream.connected || waitingForBot}
              title={isControlMode ? 'Hand the browser back to the bot' : `Take over @${teamName}'s browser`}
              data-takeover-button
              className={`h-[30px] px-2.5 rounded-[10px] inline-flex items-center gap-1.5 text-[13px] font-semibold whitespace-nowrap cursor-pointer disabled:cursor-not-allowed disabled:opacity-50 focus:outline-none focus-visible:ring-2 focus-visible:ring-sky-300 ${
                isControlMode ? 'border border-[#3f3868] bg-[rgb(44_37_82/0.8)] text-zinc-100' : 'bg-control text-control-ink hover:brightness-110'
              }`}
            >
              {isControlMode ? <Hand size={14} aria-hidden="true" /> : <MousePointerClick size={14} aria-hidden="true" />}
              {isControlMode ? 'Hand back' : 'Take over'}
            </button>
          )}

          <button
            type="button"
            onClick={() => {
              playSfx('click');
              setActiveTab((curr) => (curr === 'vault' ? 'browser' : 'vault'));
            }}
            title={`Saved logins (${savedCount})`}
            aria-label={`Saved logins (${savedCount})`}
            className={`p-1.5 rounded-md transition-colors cursor-pointer relative ${
              activeTab === 'vault'
                ? 'text-sky-400 bg-sky-500/10'
                : 'text-zinc-400 hover:text-zinc-100 hover:bg-zinc-800'
            }`}
          >
            <Key size={13} />
            {savedCount > 0 && (
              <span className="absolute -top-1 -right-1 min-w-3.5 h-3.5 px-0.5 bg-sky-500 text-[9px] font-bold text-zinc-950 rounded-full flex items-center justify-center">
                {savedCount}
              </span>
            )}
          </button>

          {onRefresh && (
            <button
              type="button"
              onClick={() => {
                playSfx('click');
                onRefresh();
                passwords.reload();
              }}
              title="Refresh Screenshot & Passwords"
              className="p-1.5 rounded-md text-zinc-400 hover:text-zinc-100 hover:bg-zinc-800 transition-colors cursor-pointer"
            >
              <RefreshCw size={13} />
            </button>
          )}

          <button
            type="button"
            onClick={() => {
              playSfx('click');
              onClose();
            }}
            title="Close VM Pane"
            className="p-1.5 rounded-md text-zinc-400 hover:text-zinc-100 hover:bg-zinc-800 transition-colors cursor-pointer"
          >
            <X size={15} />
          </button>
        </div>
      </div>
      {switching && narrow && <BrowserSwitcherRow entries={switcher!.entries} current={switcher!.current} onPick={switcher!.onPick} disabled={isControlMode} />}

      {computer.state.error && (
        <div className="px-3 pt-2">
          <ComputerModeError error={computer.state.error} onDismiss={computer.dismissError} />
        </div>
      )}

      {/* Tabs */}
      <div className="flex border-b border-zinc-800 bg-zinc-900/30 text-xs font-medium">
        <button
          type="button"
          onClick={() => {
            playSfx('click');
            setActiveTab('browser');
          }}
          aria-pressed={activeTab === 'browser'}
          className={`flex-1 py-2 flex items-center justify-center gap-1.5 transition-colors border-b-2 cursor-pointer ${
            activeTab === 'browser'
              ? 'border-sky-500 text-sky-400 bg-sky-500/10'
              : 'border-transparent text-zinc-400 hover:text-zinc-200'
          }`}
        >
          <Globe size={13} />
          <span>Browser</span>
        </button>
        <button
          type="button"
          onClick={() => {
            playSfx('click');
            setActiveTab('terminal');
          }}
          aria-pressed={activeTab === 'terminal'}
          className={`flex-1 py-2 flex items-center justify-center gap-1.5 transition-colors border-b-2 cursor-pointer ${
            activeTab === 'terminal'
              ? 'border-sky-500 text-sky-400 bg-sky-500/10'
              : 'border-transparent text-zinc-400 hover:text-zinc-200'
          }`}
        >
          <Terminal size={13} />
          <span>Terminal</span>
        </button>
        <button
          type="button"
          onClick={() => {
            playSfx('click');
            setActiveTab('vault');
          }}
          aria-pressed={activeTab === 'vault'}
          className={`flex-1 py-2 flex items-center justify-center gap-1.5 transition-colors border-b-2 cursor-pointer relative ${
            activeTab === 'vault'
              ? 'border-sky-500 text-sky-400 bg-sky-500/10'
              : 'border-transparent text-zinc-400 hover:text-zinc-200'
          }`}
        >
          <Key size={13} />
          <span>Logins</span>
          {savedCount > 0 && (
            <span className="text-[10px] px-1 py-0.2 rounded-full bg-zinc-800 text-zinc-300 font-mono">
              {savedCount}
            </span>
          )}
        </button>
      </div>

      {teamId && passwords.pending && <PendingLoginPrompt sandboxId={teamId} pending={passwords.pending} onDone={passwords.reload} />}

      {/* Content Area */}
      <div className="flex-1 overflow-hidden flex flex-col">
        {activeTab === 'browser' && isWindow ? (
          <div className="flex-1 flex flex-col p-3 overflow-hidden">
            <LocalBrowserNote api={computer} />
          </div>
        ) : activeTab === 'browser' ? (
          <div className="flex-1 flex flex-col p-3 gap-2 overflow-hidden">
            {/* Browser Address Bar */}
            <div className="flex items-center gap-1.5 p-1.5 bg-zinc-900 border border-zinc-800 rounded-lg text-xs">
              <Globe size={13} className="text-zinc-400 flex-shrink-0" />
              <input
                type="text"
                readOnly={!onNavigate}
                value={inputUrl}
                onChange={(e) => setInputUrl(e.target.value)}
                onKeyDown={(e) => {
                  if (e.key === 'Enter' && onNavigate && inputUrl.trim()) {
                    let target = inputUrl.trim();
                    if (!target.startsWith('http://') && !target.startsWith('https://')) {
                      target = `https://${target}`;
                    }
                    if (isControlMode) stream.send({ action: 'navigate', url: target }, true);
                    else onNavigate(target);
                  }
                }}
                placeholder="https://..."
                className="flex-1 bg-transparent text-zinc-200 font-mono text-[11px] truncate focus:outline-none"
              />

              {loginsOnThisPage > 0 && (
                <button
                  type="button"
                  onClick={() => {
                    playSfx('click');
                    setActiveTab('vault');
                  }}
                  className="flex items-center gap-1 px-1.5 py-0.5 rounded bg-emerald-500/15 text-emerald-400 border border-emerald-500/25 text-[10px] hover:bg-emerald-500/25 transition-colors cursor-pointer flex-shrink-0"
                  title={`${loginsOnThisPage} saved login${loginsOnThisPage === 1 ? '' : 's'} for this address`}
                >
                  <Key size={10} />
                  <span className="font-mono">{loginsOnThisPage} saved</span>
                </button>
              )}

              {teamId && teamId !== 'default' && (
                <a
                  href={`/sandbox/${encodeURIComponent(teamId)}`}
                  target="_blank"
                  rel="noopener"
                  className="text-zinc-400 hover:text-sky-400 p-0.5 flex-shrink-0"
                  title="Open the live sandbox browser in a new tab (shared with your bots)"
                >
                  <ExternalLink size={12} />
                </a>
              )}
            </div>

            {shownTitle && (
              <div className="text-[11px] text-zinc-400 truncate px-1 font-medium">
                {shownTitle}
              </div>
            )}

            {(captchaOpen || solvedNote) && (
              <div
                role="status"
                aria-live="polite"
                className={`flex items-center gap-2 px-2.5 py-1.5 rounded-lg border text-[11px] ${
                  solvedNote ? 'border-emerald-500/30 bg-emerald-950/40 text-emerald-300' : 'border-amber-500/30 bg-amber-950/40 text-amber-200'
                }`}
              >
                {solvedNote ? <CheckCircle2 size={13} aria-hidden="true" /> : <Puzzle size={13} aria-hidden="true" />}
                <span className="flex-1">{solvedNote ? `Solved, ${teamName} continues` : `Solve the puzzle, ${teamName} continues by itself`}</span>
                {!solvedNote && (!isControlMode || narrow) && (
                  <button
                    type="button"
                    onClick={() => {
                      playSfx('click');
                      setWantsControl(true);
                      setFullScreen(true);
                    }}
                    className="px-2 py-1 rounded-md bg-amber-400 text-zinc-950 font-semibold"
                  >
                    Solve it
                  </button>
                )}
              </div>
            )}

            {/* D: the bot is finishing a step; taking over follows as soon as it's done */}
            {waitBot && !isControlMode && (
              <div role="status" aria-live="polite" data-takeover-wait className="flex items-start gap-2.5 px-3 py-2.5 rounded-xl border border-[rgb(157_143_255/0.35)] bg-[#1a1538]">
                {waitBot.gaveUp ? (
                  <Hand size={16} className="shrink-0 mt-0.5 text-[#9d8fff]" aria-hidden="true" />
                ) : (
                  <Loader2 size={16} className="shrink-0 mt-0.5 text-[#9d8fff] animate-spin motion-reduce:animate-none" aria-hidden="true" />
                )}
                <div className="flex-1 min-w-0">
                  {waitBot.gaveUp ? (
                    <p className="text-[13px] leading-[18px] text-zinc-100">@{teamName} is finishing a step, try again in a moment.</p>
                  ) : (
                    <>
                      <p className="text-[13px] leading-[18px] text-zinc-100">
                        <span className="font-semibold">@{teamName} is finishing a step.</span> Taking over as soon as it’s done.
                      </p>
                      <p className="mt-0.5 text-xs text-[#a39dcc]">Trying every second for 10 seconds.</p>
                    </>
                  )}
                </div>
                <button
                  type="button"
                  onClick={() => {
                    setWaitBot(null);
                    stream.clearError();
                  }}
                  aria-label="Close"
                  className="shrink-0 -mr-1 -mt-0.5 w-6 h-6 inline-flex items-center justify-center rounded-md text-[#a39dcc] hover:text-zinc-100 hover:bg-white/[0.06]"
                >
                  <X size={14} aria-hidden="true" />
                </button>
              </div>
            )}

            {/* The person already has this browser in another window: a static note, no spinner, no retry */}
            {heldElsewhere && !isControlMode && (
              <div role="status" aria-live="polite" data-takeover-wait data-takeover-held className="flex items-start gap-2.5 px-3 py-2.5 rounded-xl border border-[rgb(157_143_255/0.35)] bg-[#1a1538]">
                <Hand size={16} className="shrink-0 mt-0.5 text-[#9d8fff]" aria-hidden="true" />
                <p className="flex-1 min-w-0 text-[13px] leading-[18px] text-zinc-100">{HELD_BY_PERSON_TEXT}</p>
                <button
                  type="button"
                  onClick={() => stream.clearError()}
                  aria-label="Close"
                  className="shrink-0 -mr-1 -mt-0.5 w-6 h-6 inline-flex items-center justify-center rounded-md text-[#a39dcc] hover:text-zinc-100 hover:bg-white/[0.06]"
                >
                  <X size={14} aria-hidden="true" />
                </button>
              </div>
            )}

            {controlError && !botBusy && !heldElsewhere && (
              <div role="alert" className="px-2.5 py-2 rounded-lg border border-red-500/30 bg-red-950/40 text-xs text-red-300">
                {controlError}
              </div>
            )}
            <div
              data-browser-watch
              data-watch-state={takeover ? 'stage' : waitingForBot ? 'busy' : needsYou ? 'needs-you' : 'watching'}
              className="group relative w-full aspect-[16/10] shrink-0 border border-white/[0.07] rounded-xl bg-[#0a0719] overflow-hidden flex items-center justify-center"
            >
              {/* Relaying: a discrete action (a navigate from the address bar) */}
              {isInteracting && !takeover && (
                <div className="absolute top-2 right-2 h-6 px-2.5 rounded-full bg-[rgb(14_10_33/0.9)] border border-white/[0.14] text-zinc-200 text-xs flex items-center gap-1.5 z-20 pointer-events-none">
                  <Loader2 size={12} className="animate-spin motion-reduce:animate-none text-[#9d8fff]" aria-hidden="true" />
                  <span>Relaying…</span>
                </div>
              )}

              {takeover ? (
                <div className="text-center p-6 text-[#a39dcc] text-xs space-y-2">
                  <p>{showFullScreen ? 'Showing full screen' : isControlMode ? 'You’re in control in the large view' : 'Taking control…'}</p>
                  <button type="button" onClick={handBack} className="px-2.5 h-8 rounded-[10px] border border-[#3f3868] bg-[rgb(44_37_82/0.8)] text-zinc-100 hover:bg-[#2c2552]">
                    Hand back
                  </button>
                </div>
              ) : frameSrc ? (
                <>
                  <img
                    src={frameSrc}
                    alt={`${teamName}'s browser`}
                    onLoad={() => { if (paneFrameId.current !== null) stream.acknowledgeFrame(paneFrameId.current); }}
                    onError={() => { if (paneFrameId.current !== null) stream.acknowledgeFrame(paneFrameId.current); }}
                    draggable={false}
                    className="w-full h-full object-contain"
                  />
                  {pointOn && point && (
                    <PointLayer
                      request={stream.request}
                      page={stream.frame ? { width: stream.frame.width, height: stream.frame.height } : null}
                      frameId={stream.frame?.id ?? null}
                      pageUrl={pageUrl}
                      phone={narrow}
                      sink={point}
                      onExit={stopPointing}
                      onBackToChat={() => {
                        setPointing(false);
                        onClose();
                      }}
                    />
                  )}
                  {!pointOn && !needsYou && !waitingForBot && !waitBot?.gaveUp && (
                    <span className="absolute top-2 left-2 h-[22px] px-2 inline-flex items-center gap-1 rounded-full bg-[rgb(14_10_33/0.88)] text-[11px] text-zinc-200 pointer-events-none group-hover:opacity-0 group-focus-within:opacity-0 kw-veil-in">
                      <Eye size={12} aria-hidden="true" /> Watching
                    </span>
                  )}
                  {!pointOn && !isWindow && stream.connected && (
                    /* B / C: one click anywhere on the page takes it over (that click isn't passed on) */
                    <button
                      type="button"
                      onClick={waitingForBot ? undefined : takeOver}
                      aria-disabled={waitingForBot || undefined}
                      aria-label={waitingForBot ? `Waiting for ${teamName}` : `Take over ${teamName}'s browser`}
                      className={`absolute inset-0 flex flex-col items-center justify-center gap-2 px-6 text-center focus:outline-none focus-visible:ring-2 focus-visible:ring-inset focus-visible:ring-sky-300 rounded-xl kw-veil-in ${
                        waitingForBot
                          ? 'bg-[rgb(10_7_25/0.8)] cursor-default'
                          : needsYou || waitBot?.gaveUp
                            ? 'bg-[rgb(10_7_25/0.84)] cursor-pointer'
                            : 'bg-[rgb(10_7_25/0.8)] opacity-0 hover:opacity-100 focus-visible:opacity-100 cursor-pointer'
                      }`}
                    >
                      {needsYou && !waitingForBot && (
                        <span className="inline-flex items-center gap-1.5 text-xs font-semibold text-[#fcd34d]">
                          <Hand size={14} aria-hidden="true" /> @{teamName} {needsYou}
                        </span>
                      )}
                      <span
                        className={`inline-flex items-center gap-1.5 px-3.5 h-9 rounded-[10px] text-sm font-semibold ${
                          waitingForBot ? 'bg-[#2c2552] text-[#a39dcc]' : 'bg-control text-control-ink'
                        }`}
                      >
                        {waitingForBot ? (
                          <>Waiting for @{teamName}…</>
                        ) : (
                          <>
                            <MousePointerClick size={15} aria-hidden="true" /> {waitBot?.gaveUp ? 'Try again' : 'Take over'}
                          </>
                        )}
                      </span>
                      {!needsYou && !waitingForBot && !waitBot?.gaveUp && <span className="text-xs text-zinc-200">Click anywhere to take over. @{teamName} pauses.</span>}
                    </button>
                  )}
                </>
              ) : (
                <div className="text-center p-6 text-zinc-500 text-xs">
                  <Globe size={32} className="mx-auto mb-2 opacity-40 text-zinc-500" />
                  <p>{browserReady ? 'Browser idle' : 'Browser unavailable'}</p>
                  <p className="text-[10px] text-zinc-500 mt-1">{browserReady ? 'Enter a URL above to open a page' : isLocal ? 'Check that Chrome is installed, then use Restart Browser below' : 'Start Docker, then use Restart Browser below'}</p>
                </div>
              )}
            </div>
          </div>
        ) : activeTab === 'terminal' ? (
          /* Terminal Tab */
          <div className="flex-1 p-3 overflow-y-auto font-mono text-xs text-zinc-300 space-y-2 bg-zinc-950">
            {terminalLogs.length === 0 ? (
              <div className="text-center text-zinc-600 text-xs py-8">
                <Terminal size={24} className="mx-auto mb-1 opacity-40" />
                <span>No commands executed in container yet</span>
              </div>
            ) : (
              terminalLogs.map((log, index) => (
                <div key={index} className="p-2 bg-zinc-900/60 border border-zinc-800/80 rounded-lg">
                  <div className="flex items-center justify-between text-sky-400 font-semibold text-[11px]">
                    <span>$ {log.command}</span>
                    {log.timestamp && (
                      <span className="text-[9px] text-zinc-500 font-normal">
                        {log.timestamp.includes('T') ? log.timestamp.split('T')[1].split('.')[0] : log.timestamp}
                      </span>
                    )}
                  </div>
                  <pre className="text-[10px] text-zinc-300 mt-1 whitespace-pre-wrap font-mono leading-relaxed">{log.output}</pre>
                </div>
              ))
            )}
          </div>
        ) : (
          /* Saved logins */
          teamId && teamId !== 'default' ? (
            <SavedPasswordsPanel
              sandboxId={teamId}
              state={passwords}
              store={passwordStore}
              approvalsOff={approvalsOff}
              pageUrl={pageUrl}
              onOpen={
                onNavigate
                  ? (origin) => {
                      playSfx('click');
                      onNavigate(origin);
                      setActiveTab('browser');
                    }
                  : undefined
              }
            />
          ) : (
            <p className="flex-1 p-6 text-center text-xs text-zinc-500">Open a bot, team or channel to see its saved logins.</p>
          )
        )}
      </div>

      {showFullScreen && (
        <BrowserControlOverlay
          botName={teamName}
          pageUrl={pageUrl}
          image={frameSrc}
          page={stream.frame ? { width: stream.frame.width, height: stream.frame.height } : undefined}
          onFrameDone={() => {
            if (paneFrameId.current !== null) stream.acknowledgeFrame(paneFrameId.current);
          }}
          send={stream.send}
          busy={isInteracting}
          error={controlError ?? null}
          tapFocus={stream.tapFocus}
          captcha={captchaOpen ? { target: captchaStep?.target ?? null } : null}
          solved={solvedNote}
          onHandBack={handBack}
          interceptHandBack={() => {
            if (!recording) return false;
            void showOnce.stop(true);
            return true;
          }}
          handBackQuiet={recording}
          belowBar={teamId ? <RecordingStrip stage={teamId} phone /> : null}
          stream={stream}
          step={stepForStage}
          controlling={stream.controlling}
          resizeIgnored={stream.resizeIgnored}
          restoreSize={stream.restoreViewport}
        />
      )}

      {showStage && (
        <BrowserTakeoverStage
          botName={teamName}
          othersBrowsing={switching}
          pageUrl={pageUrl}
          pageTitle={shownTitle}
          stream={stream}
          image={frameSrc}
          step={stepForStage}
          notice={
            solvedNote
              ? { tone: 'success', text: `Solved, ${teamName} continues` }
              : captchaOpen
                ? { tone: 'warn', text: `Solve the puzzle, ${teamName} continues by itself` }
                : null
          }
          onNavigate={onNavigate}
          onHandBack={handBackOrStop}
          onRetryControl={takeOver}
          extraAction={recordSteps}
          handBackQuiet={recording}
          belowBar={teamId ? <RecordingStrip stage={teamId} /> : null}
        />
      )}

      {/* Footer Status */}
      <div className="p-2.5 border-t border-zinc-800 bg-zinc-900/60 flex items-center justify-between text-[11px] text-zinc-400">
        <div className="flex items-center gap-1.5" data-browser-footer>
          {isLocal ? (
            <Laptop size={12} className={browserReady || isWindow ? 'text-amber-300/80' : 'text-zinc-500'} />
          ) : (
            <Cpu size={12} className={browserReady ? 'text-emerald-400' : 'text-zinc-500'} />
          )}
          <span>{browserFooterLabel(computer.state.settings, browserReady)}</span>
        </div>
        {pointOn && point ? (
          <span data-point-footer="" className="text-violet-200 font-semibold truncate">{footerText(point.picks)}</span>
        ) : point && takeover && !isWindow ? (
          <span data-point-footer="" className="text-zinc-400 truncate">{POINT_COPY.handBack}</span>
        ) : null}
        {onRestartVm && !isWindow && (
          <button
            type="button"
            onClick={() => {
              playSfx('click');
              onRestartVm();
            }}
            className="hover:text-zinc-200 text-zinc-400 flex items-center gap-1 transition-colors cursor-pointer"
          >
            <RefreshCw size={11} />
            <span>Restart Browser</span>
          </button>
        )}
      </div>
    </div>
  );
};
