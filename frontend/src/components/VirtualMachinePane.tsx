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
  Hand,
  Crosshair,
  Maximize2,
} from 'lucide-react';
import { playSfx } from '../audio/chiptune';
import { BOT_BUSY_TEXT, HELD_BY_PERSON_TEXT, useBrowserStream } from '../useBrowserStream';
import { ComputerModeChip, ComputerModeError, LocalBrowserNote, OpenWindowButton, SignInLine, useComputerMode } from './ComputerModeSwitch';
import { browserFooterLabel, browserOwnerWord, isBlankPage, showsLocalWindow } from '../computerMode';
import { paneStatus } from '../browserPanel';
import { ModernAvatar } from './ModernAvatar';
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
import { choosePage, pageTeam, shownPage, stepPage, trailTime, type ReplyPage } from '../browserPages';
import { useBrowserPages } from '../useBrowserPages';
import { BrowserPageStrip } from './BrowserPageStrip';

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
  /** `page`: the page shown (0.9.3), so the address bar drives that page. */
  onNavigate?: (url: string, page?: string | null) => void;
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
  /** 0.9.3: show this page of the team's browser (a card's Sign in / Solve); `n` goes up per ask. */
  pagePick?: { page: string; n: number } | null;
  /** Bot names by id, for the page strip. */
  botNames?: Record<string, string>;
  /** 0.9.8: the browser's owner has a turn running now (header "Browsing now", pill "Live"). */
  running?: boolean;
  /** 0.9.8: when that turn ended, for "Finished 10:46 PM"; null with no run since the app opened (Idle). */
  finishedAt?: string | null;
  /** 0.9.8: the bot browsing ("Ben's browser") when the page on screen isn't a bot's job page. */
  browsingBot?: string | null;
  /** 0.9.8: the pages the reply opened (the tapped chip's, else the running turn's), newest first. */
  trail?: ReplyPage[];
  /** 0.9.8: the person used the panel (a tab, Point, Take over, the trail, Enlarge, the address bar). */
  onTouch?: () => void;
  /** 0.9.8: what keeps a bot-opened panel open after the turn: control, Point picks waiting to be sent. */
  onUsage?: (u: { control: boolean; pointPending: boolean }) => void;
  /** 0.9.8: playing the close animation (the panel closes itself in a moment). */
  closing?: boolean;
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

// 0.9.3: the page the person picked, per team, for the same reason as the tab: the pane remounts.
// `at` is when it was picked, so a page list asked for before that can't call it closed.
const pickedPages = new Map<string, { page: string; at: number }>();
let handledPagePick = 0;

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
  pagePick = null,
  botNames,
  running = false,
  finishedAt = null,
  browsingBot = null,
  trail = [],
  onTouch,
  onUsage,
  closing = false,
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

  // 0.9.3: which of the team's pages to show. The person's pick wins while it exists; else the newest
  // open step's page; else the server's default. A new step only marks its tab (Ada's brief, §3).
  const pages = useBrowserPages(teamId, isOpen && !isWindow);
  const [picked, setPickedState] = useState<{ page: string; at: number } | null>(() => (teamId && pickedPages.get(teamId)) || null);
  const setPicked = useCallback(
    (page: string | null) => {
      const next = page ? { page, at: Date.now() } : null;
      if (teamId) {
        if (next) pickedPages.set(teamId, next);
        else pickedPages.delete(teamId);
      }
      setPickedState(next);
    },
    [teamId],
  );
  // A card's Sign in / Solve, or a step notification: that step's page, if it is this team's
  useEffect(() => {
    if (!pagePick || pagePick.n <= handledPagePick || !teamId || pageTeam(pagePick.page) !== teamId) return;
    handledPagePick = pagePick.n;
    setPicked(pagePick.page);
    pages.reload();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [pagePick?.n, teamId]);
  const pageKeys = useMemo(() => pages.list?.pages.map((p) => p.page) ?? null, [pages.list]);
  // A list asked for before the pick can't know the picked page yet
  const knownPages = pageKeys && (!picked || (pages.list?.at ?? 0) >= picked.at) ? pageKeys : null;
  const openStep = humanStep?.status === 'open' ? humanStep : captchaStep?.status === 'open' ? captchaStep : null;
  const chosenPage = choosePage({ picked: picked?.page ?? null, stepPage: stepPage(openStep), pages: knownPages });
  // The picked page closed (idle, or the cap made room): forget it, so it never comes back by itself
  useEffect(() => {
    if (picked && knownPages && !knownPages.includes(picked.page)) setPicked(null);
  }, [picked, knownPages, setPicked]);
  const pageOnScreen = shownPage(chosenPage, pages.list, teamId);
  const pageInfo = pages.list?.pages.find((p) => p.page === pageOnScreen) ?? null;
  // A step or CAPTCHA is only this view's when it holds the page on screen
  const onScreen = (s: HumanStep | null | undefined) => (s && stepPage(s) === pageOnScreen ? s : null);
  humanStep = onScreen(humanStep);
  captchaStep = onScreen(captchaStep);
  const botName = useCallback((id: string) => botNames?.[id], [botNames]);
  // Who the page on screen belongs to: its bot on a job page, else the team ("Architect needs you to sign in")
  const pageBot = (pageInfo?.job && pageInfo.kwant_id && botNames?.[pageInfo.kwant_id]) || teamName;
  // 0.9.8 header and footer: "Ben's browser", or "The team's browser" when no bot owns the page on screen
  const whoBrowses = (pageInfo?.job && pageInfo.kwant_id && botNames?.[pageInfo.kwant_id]) || browsingBot || null;
  const whose = browserOwnerWord(whoBrowses);
  const paneTitle = `${whose.charAt(0).toUpperCase()}${whose.slice(1)} browser`;
  const touch = useCallback(() => onTouch?.(), [onTouch]);

  // Interactive Remote Control State
  const stream = useBrowserStream(teamId, isOpen && activeTab === 'browser' && !isWindow && !computer.windowPending, chosenPage);
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
  const pageUrl = stream.location ? stream.location.url : chosenPage ? pageInfo?.url ?? '' : activeUrl;
  // The same for the title: a new page's empty title must not fall back to the previous page's.
  const shownTitle = stream.location ? stream.location.title : chosenPage ? pageInfo?.title ?? '' : pageTitle;
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
    // On the chosen page only: a card's Solve switches page and asks for control in one go
    if (!wantsControl || !connected || stream.page !== chosenPage) return;
    setWantsControl(false);
    if (!controlling) send({ action: 'take_control' });
  }, [wantsControl, connected, controlling, send, stream.page, chosenPage]);

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
    // Taking over a page picks it: a step on another page must not pull the view (and the control) away
    if (!picked && pageKeys && pageKeys.length >= 2 && pageOnScreen) setPicked(pageOnScreen);
    setActiveTab('browser');
    setFullScreen(true);
    if (connected) send({ action: 'take_control' });
    else setWantsControl(true);
  }, [connected, send, picked, pageKeys, pageOnScreen, setPicked]);

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

  // 0.9.8 D: a panel the bot opened stays open while the person controls it or has picks to send
  const pointPending = (point?.picks ?? 0) > 0;
  useEffect(() => onUsage?.({ control: isControlMode, pointPending }), [onUsage, isControlMode, pointPending]);

  // 0.9.8 C: Enlarge shows the same frame big, view only
  const [enlarged, setEnlarged] = useState(false);
  const enlargeClose = useRef<HTMLButtonElement>(null);
  useEffect(() => {
    if (!enlarged) return;
    enlargeClose.current?.focus();
    const onKey = (e: KeyboardEvent) => {
      if (e.key === 'Escape') {
        e.stopPropagation();
        setEnlarged(false);
      }
    };
    window.addEventListener('keydown', onKey, true);
    return () => window.removeEventListener('keydown', onKey, true);
  }, [enlarged]);
  useEffect(() => {
    if (enlarged && !frameSrc) setEnlarged(false);
  }, [enlarged, frameSrc]);

  // 0.9.8 E: the phone sheet takes focus to its ✕ and gives it back (to the chip) when it closes
  const closeButton = useRef<HTMLButtonElement>(null);
  useEffect(() => {
    if (!isOpen || !narrow) return;
    const before = document.activeElement as HTMLElement | null;
    closeButton.current?.focus();
    return () => {
      if (before && before.isConnected && before !== document.body) before.focus();
    };
  }, [isOpen, narrow]);
  // Dragging the grabber down more than 96 px closes the sheet; a tap does nothing
  const [drag, setDrag] = useState<{ y0: number; dy: number } | null>(null);
  const dragging = drag && drag.dy > 0 ? drag.dy : 0;

  if (!isOpen) return null;
  const switching = !!switcher && switcher.entries.length >= 2;
  const status = paneStatus({ running, finishedAt });
  const live = status.kind === 'browsing';
  const finished = status.kind === 'finished';
  const close = () => {
    playSfx('click');
    onClose();
  };
  const statusLine = (
    <p className="text-[11px] text-zinc-400 truncate flex items-center gap-1" aria-live="polite" data-pane-status={status.kind}>
      {live && <span aria-hidden="true" className="shrink-0 w-[7px] h-[7px] rounded-full bg-emerald-400" />}
      <span className="truncate">
        {status.text} · @{teamName}
      </span>
    </p>
  );
  const pointButtonEl =
    pointShown || (point && takeover && !isWindow) ? (
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
          touch();
          setPointing((p) => !p);
        }}
        className={`${narrow ? 'h-11 flex-1 justify-center' : 'h-8'} px-3 rounded-[10px] inline-flex items-center gap-1.5 text-[13px] font-semibold whitespace-nowrap cursor-pointer disabled:cursor-not-allowed disabled:opacity-50 focus:outline-none focus-visible:ring-2 focus-visible:ring-sky-300 border ${
          pointOn ? 'bg-violet-600 border-violet-400 text-white ring-[3px] ring-violet-500/25' : 'border-[#3f3868] text-zinc-200 hover:bg-white/5'
        }`}
      >
        <Crosshair size={14} aria-hidden="true" />
        {POINT_COPY.button}
      </button>
    ) : null;

  return (
    // 0.9.8 E: on a phone the panel is a 70 % bottom sheet over a scrim; the chat stays readable above it.
    <>
    {narrow && (
      <div
        aria-hidden="true"
        data-vm-scrim=""
        onClick={close}
        className={`fixed inset-0 z-[59] bg-[rgb(5_3_14/0.42)] transition-opacity duration-150 motion-reduce:transition-none ${closing ? 'opacity-0' : 'opacity-100'}`}
      />
    )}
    <div
      data-vm-pane=""
      data-pane-closing={closing || undefined}
      role={narrow ? 'dialog' : undefined}
      aria-modal={narrow ? true : undefined}
      aria-label={narrow ? paneTitle : undefined}
      style={dragging ? { transform: `translateY(${dragging}px)`, transition: 'none' } : undefined}
      className={`${
        narrow
          ? `fixed inset-x-0 bottom-0 z-[60] h-[70dvh] rounded-t-[20px] border-t border-[#3a3266] bg-[#0d0a1d] shadow-[0_-12px_40px_rgba(0,0,0,.5)] pb-[env(safe-area-inset-bottom)] overflow-hidden animate-in slide-in-from-bottom duration-200 ${closing ? 'translate-y-full opacity-0' : ''}`
          : `w-96 border-l border-zinc-800 h-full z-30 bg-zinc-950 shadow-2xl animate-in slide-in-from-right duration-200 ${closing ? 'translate-x-4 opacity-0' : ''}`
      } flex flex-col select-none transition-[transform,opacity] ease-out duration-150 motion-reduce:transition-none motion-reduce:animate-none`}
    >
      {narrow && (
        <div
          data-vm-grabber=""
          aria-hidden="true"
          className="shrink-0 pt-2 pb-1 touch-none cursor-grab"
          onPointerDown={(e) => {
            (e.target as HTMLElement).setPointerCapture?.(e.pointerId);
            setDrag({ y0: e.clientY, dy: 0 });
          }}
          onPointerMove={(e) => drag && setDrag({ y0: drag.y0, dy: e.clientY - drag.y0 })}
          onPointerUp={() => {
            const far = !!drag && drag.dy > 96;
            setDrag(null);
            if (far) close();
          }}
          onPointerCancel={() => setDrag(null)}
        >
          <div className="mx-auto w-9 h-1 rounded-full bg-[#4b4478]" />
        </div>
      )}
      {/* Header: whose browser it is and whether it's browsing now (0.9.8 C) */}
      <div className={`${narrow ? 'h-[52px]' : 'h-14 bg-zinc-900/60'} border-b border-zinc-800 px-4 flex items-center justify-between gap-2`}>
        <div className="flex items-center gap-2 min-w-0">
          <ModernAvatar role={whoBrowses ?? teamName} name={whoBrowses ?? teamName} size={24} modelBadge={false} />
          <div className="min-w-0">
            {switching && !narrow ? (
              <BrowserSwitcherMenu
                entries={switcher!.entries}
                current={switcher!.current}
                onPick={(owner) => {
                  touch();
                  switcher!.onPick(owner);
                }}
                disabled={isControlMode}
                title={paneTitle}
                sub={statusLine}
              />
            ) : (
              <>
                <div className="text-[13px] font-semibold text-zinc-100 truncate" data-pane-title="">{paneTitle}</div>
                {statusLine}
              </>
            )}
          </div>
        </div>

        <div className="flex items-center gap-1 shrink-0">
          <ComputerModeChip api={computer} />
          <OpenWindowButton api={computer} />
          <button
            ref={closeButton}
            type="button"
            onClick={close}
            aria-label="Close"
            title="Close"
            className={`${narrow ? 'w-11 h-11' : 'w-[30px] h-[30px]'} inline-flex items-center justify-center rounded-md text-zinc-400 hover:text-zinc-100 hover:bg-zinc-800 transition-colors cursor-pointer focus:outline-none focus-visible:ring-2 focus-visible:ring-sky-300`}
          >
            <X size={narrow ? 18 : 15} aria-hidden="true" />
          </button>
        </div>
      </div>
      {switching && narrow && <BrowserSwitcherRow entries={switcher!.entries} current={switcher!.current} onPick={(owner) => { touch(); switcher!.onPick(owner); }} disabled={isControlMode} />}

      {computer.state.error && (
        <div className="px-3 pt-2">
          <ComputerModeError error={computer.state.error} onDismiss={computer.dismissError} />
        </div>
      )}

      {/* Tabs */}
      <div className={`flex border-b border-zinc-800 bg-zinc-900/30 text-xs font-medium ${narrow ? 'h-10 shrink-0' : ''}`}>
        <button
          type="button"
          onClick={() => {
            playSfx('click');
            touch();
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
            touch();
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
            touch();
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
          <div className="flex-1 min-h-0 flex flex-col p-3 gap-2 overflow-y-auto" onScroll={touch} data-pane-body="">
            {pages.list && (
              <BrowserPageStrip
                pages={pages.list.pages}
                current={pageOnScreen}
                botName={botName}
                disabled={isControlMode}
                narrow={narrow}
                onPick={(page) => {
                  playSfx('click');
                  touch();
                  setPicked(page);
                }}
              />
            )}
            {/* Browser Address Bar (not on a phone: the trail's host says where it is, typing a URL is a Take over job) */}
            {!narrow && (
            <div className="flex items-center gap-1.5 p-1.5 bg-zinc-900 border border-zinc-800 rounded-lg text-xs">
              <Globe size={13} className="text-zinc-400 flex-shrink-0" />
              <input
                type="text"
                readOnly={!onNavigate}
                value={inputUrl}
                onChange={(e) => {
                  touch();
                  setInputUrl(e.target.value);
                }}
                onKeyDown={(e) => {
                  if (e.key === 'Enter' && onNavigate && inputUrl.trim()) {
                    let target = inputUrl.trim();
                    if (!target.startsWith('http://') && !target.startsWith('https://')) {
                      target = `https://${target}`;
                    }
                    if (isControlMode) stream.send({ action: 'navigate', url: target }, true);
                    else onNavigate(target, chosenPage);
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
            )}

            {shownTitle && !narrow && (
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
                <span className="flex-1">{solvedNote ? `Solved, ${pageBot} continues` : `Solve the puzzle, ${pageBot} continues by itself`}</span>
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
                    <p className="text-[13px] leading-[18px] text-zinc-100">@{pageBot} is finishing a step, try again in a moment.</p>
                  ) : (
                    <>
                      <p className="text-[13px] leading-[18px] text-zinc-100">
                        <span className="font-semibold">@{pageBot} is finishing a step.</span> Taking over as soon as it’s done.
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
            <SignInLine signIn={stream.signIn} api={computer} />
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
                    data-frame-finished={finished || undefined}
                    className={`w-full h-full object-contain ${finished ? 'saturate-[.6] brightness-[.82]' : ''}`}
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
                  {!pointOn && !needsYou && !waitingForBot && !waitBot?.gaveUp && (live || finished) && (
                    <span data-pane-pill={live ? 'live' : 'finished'} className="absolute top-2 left-2 h-[22px] px-2 inline-flex items-center gap-1.5 rounded-full bg-[rgb(14_10_33/0.88)] text-[11px] text-zinc-200 pointer-events-none group-hover:opacity-0 group-focus-within:opacity-0 kw-veil-in">
                      {live && <span aria-hidden="true" className="w-1.5 h-1.5 rounded-full bg-emerald-400" />}
                      {live ? 'Live' : 'Finished'}
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
                          <Hand size={14} aria-hidden="true" /> @{pageBot} {needsYou}
                        </span>
                      )}
                      <span
                        className={`inline-flex items-center gap-1.5 px-3.5 h-9 rounded-[10px] text-sm font-semibold ${
                          waitingForBot ? 'bg-[#2c2552] text-[#a39dcc]' : 'bg-control text-control-ink'
                        }`}
                      >
                        {waitingForBot ? (
                          <>Waiting for @{pageBot}…</>
                        ) : (
                          <>
                            <MousePointerClick size={15} aria-hidden="true" /> {waitBot?.gaveUp ? 'Try again' : 'Take over'}
                          </>
                        )}
                      </span>
                      {!needsYou && !waitingForBot && !waitBot?.gaveUp && <span className="text-xs text-zinc-200">Click anywhere to take over. @{pageBot} pauses.</span>}
                    </button>
                  )}
                  {!pointOn && !isInteracting && (
                    <button
                      type="button"
                      aria-label="Enlarge"
                      title="Enlarge"
                      data-pane-enlarge=""
                      onClick={() => {
                        playSfx('click');
                        touch();
                        setEnlarged(true);
                      }}
                      className="absolute top-2 right-2 z-10 w-7 h-7 inline-flex items-center justify-center rounded-lg bg-[rgb(14_10_33/0.88)] text-zinc-200 hover:text-white focus:outline-none focus-visible:ring-2 focus-visible:ring-sky-300"
                    >
                      <Maximize2 size={14} aria-hidden="true" />
                    </button>
                  )}
                </>
              ) : (
                <div className="text-center p-6 text-zinc-500 text-xs">
                  <Globe size={32} className="mx-auto mb-2 opacity-40 text-zinc-500" />
                  <p>{browserReady ? 'Browser idle' : 'Browser unavailable'}</p>
                  <p className="text-[10px] text-zinc-500 mt-1">{browserReady ? 'Enter a URL above to open a page' : isLocal ? 'Check that Chrome is installed, then use Restart browser below' : 'Start Docker, then use Restart browser below'}</p>
                </div>
              )}
            </div>

            {/* 0.9.8 C: Point at it and Take over under the preview, both secondary; Hand back as today */}
            {!isWindow && (pointButtonEl || stream.connected || isControlMode) && (
              <div className="flex gap-2 shrink-0" data-pane-actions="">
                {pointButtonEl}
                <button
                  type="button"
                  onClick={() => {
                    if (isControlMode) handBack();
                    else {
                      touch();
                      takeOver();
                    }
                  }}
                  disabled={!stream.connected || waitingForBot}
                  title={isControlMode ? 'Hand the browser back to the bot' : `Take over ${whose} browser`}
                  data-takeover-button
                  className={`${narrow ? 'h-11 flex-1 justify-center' : 'h-8'} px-3 rounded-[10px] inline-flex items-center gap-1.5 text-[13px] font-semibold whitespace-nowrap cursor-pointer disabled:cursor-not-allowed disabled:opacity-50 focus:outline-none focus-visible:ring-2 focus-visible:ring-sky-300 border border-[#3f3868] ${
                    isControlMode ? 'bg-[rgb(44_37_82/0.8)] text-zinc-100' : 'text-zinc-200 hover:bg-white/5'
                  }`}
                >
                  {isControlMode ? <Hand size={14} aria-hidden="true" /> : <MousePointerClick size={14} aria-hidden="true" />}
                  {isControlMode ? 'Hand back' : 'Take over'}
                </button>
              </div>
            )}

            {/* 0.9.8 B: the pages this reply opened, newest first; a row opens the page in a new tab */}
            {trail.length > 0 && (
              <section aria-labelledby="pane-trail-head" data-pane-trail="" className="shrink-0">
                <h3 id="pane-trail-head" className="mt-1 mb-1.5 text-[11px] font-medium uppercase tracking-[0.08em] text-zinc-400">
                  Pages this reply · {trail.length}
                </h3>
                <ol className="rounded-xl border border-[#2c2552] overflow-hidden">
                  {trail.map((p, i) => {
                    const now = live && i === 0;
                    return (
                      <li key={p.url} className={i > 0 ? 'border-t border-[#2c2552]' : ''}>
                        <a
                          href={p.url}
                          target="_blank"
                          rel="noopener noreferrer"
                          onClick={touch}
                          title={p.url}
                          className={`grid grid-cols-[44px_1fr_auto] items-center gap-2.5 px-2.5 py-2 ${narrow ? 'min-h-11' : 'min-h-12'} hover:bg-white/[0.04] focus:outline-none focus-visible:ring-2 focus-visible:ring-inset focus-visible:ring-sky-300 ${now ? 'bg-emerald-400/5' : ''}`}
                        >
                          <span className="text-xs text-zinc-400 tabular-nums">{trailTime(p.at)}</span>
                          <span className="min-w-0">
                            <span className="block text-[13px] text-zinc-100 truncate">{p.title}</span>
                            <span className="block text-xs text-zinc-400 truncate">{p.host}</span>
                          </span>
                          {now ? (
                            <span className="inline-flex items-center gap-1 text-[11px] font-semibold text-emerald-400">
                              <span aria-hidden="true" className="w-1.5 h-1.5 rounded-full bg-emerald-400" /> Now
                            </span>
                          ) : (
                            <ExternalLink size={14} aria-hidden="true" className="text-zinc-500" />
                          )}
                        </a>
                      </li>
                    );
                  })}
                </ol>
              </section>
            )}
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
                      onNavigate(origin, chosenPage);
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

      {enlarged && frameSrc && (
        <div
          role="dialog"
          aria-modal="true"
          aria-label={`${paneTitle}, enlarged`}
          data-pane-enlarged=""
          className="fixed inset-0 z-[80] flex items-center justify-center bg-[rgb(5_3_14/0.78)]"
          onClick={() => setEnlarged(false)}
        >
          <div className="relative max-w-[1024px] w-[calc(100vw-48px)]" onClick={(e) => e.stopPropagation()}>
            <img src={frameSrc} alt={`${paneTitle}, enlarged`} draggable={false} className={`w-full h-auto rounded-xl border border-white/10 ${finished ? 'saturate-[.6] brightness-[.82]' : ''}`} />
            <button
              ref={enlargeClose}
              type="button"
              aria-label="Close"
              onClick={() => setEnlarged(false)}
              className="absolute top-2 right-2 w-9 h-9 inline-flex items-center justify-center rounded-lg bg-[rgb(14_10_33/0.88)] text-zinc-200 hover:text-white focus:outline-none focus-visible:ring-2 focus-visible:ring-sky-300"
            >
              <X size={16} aria-hidden="true" />
            </button>
          </div>
        </div>
      )}

      {showFullScreen && (
        <BrowserControlOverlay
          botName={pageBot}
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
          belowBar={<><SignInLine signIn={stream.signIn} api={computer} />{teamId ? <RecordingStrip stage={teamId} phone /> : null}</>}
          stream={stream}
          step={stepForStage}
          controlling={stream.controlling}
          resizeIgnored={stream.resizeIgnored}
          restoreSize={stream.restoreViewport}
        />
      )}

      {showStage && (
        <BrowserTakeoverStage
          botName={pageBot}
          othersBrowsing={switching}
          pageUrl={pageUrl}
          pageTitle={shownTitle}
          stream={stream}
          image={frameSrc}
          step={stepForStage}
          notice={
            solvedNote
              ? { tone: 'success', text: `Solved, ${pageBot} continues` }
              : captchaOpen
                ? { tone: 'warn', text: `Solve the puzzle, ${pageBot} continues by itself` }
                : null
          }
          onNavigate={onNavigate ? (url) => onNavigate(url, chosenPage) : undefined}
          onHandBack={handBackOrStop}
          onRetryControl={takeOver}
          extraAction={recordSteps}
          handBackQuiet={recording}
          belowBar={<><SignInLine signIn={stream.signIn} api={computer} />{teamId ? <RecordingStrip stage={teamId} /> : null}</>}
        />
      )}

      {/* Footer Status */}
      <div className="h-10 shrink-0 px-2.5 border-t border-zinc-800 bg-zinc-900/60 flex items-center justify-between gap-2 text-xs text-zinc-400">
        <div className="flex items-center gap-1.5 min-w-0" data-browser-footer>
          {isLocal ? (
            <Laptop size={12} className={browserReady || isWindow ? 'text-amber-300/80' : 'text-zinc-500'} />
          ) : (
            <Cpu size={12} className={browserReady ? 'text-emerald-400' : 'text-zinc-500'} />
          )}
          <span className="truncate">{browserFooterLabel(computer.state.settings, browserReady, { botName: whoBrowses, browsing: live })}</span>
        </div>
        {pointOn && point ? (
          <span data-point-footer="" className="min-w-0 flex-1 px-2 text-center leading-snug text-zinc-300 [overflow-wrap:anywhere]">
            {/* Wraps only between "Pointing · n picked" and "· Esc to stop" in a narrow pane */}
            {(() => {
              const [state, count, keys] = footerText(point.picks).split(' · ');
              return (
                <>
                  <span className="whitespace-nowrap">{`${state} · ${count}`}</span>{' '}
                  <span className="whitespace-nowrap">{`· ${keys}`}</span>
                </>
              );
            })()}
          </span>
        ) : point && takeover && !isWindow ? (
          <span data-point-footer="" className="min-w-0 flex-1 px-2 text-center leading-snug text-zinc-400">{POINT_COPY.handBack}</span>
        ) : null}
        <div className="flex items-center gap-3 shrink-0">
          {onRefresh && !narrow && (
            <button
              type="button"
              onClick={() => {
                playSfx('click');
                onRefresh();
                passwords.reload();
              }}
              title="Refresh the picture and saved logins"
              className="hover:text-zinc-200 text-zinc-400 transition-colors cursor-pointer focus:outline-none focus-visible:ring-2 focus-visible:ring-sky-300 rounded"
            >
              Refresh
            </button>
          )}
          {onRestartVm && !isWindow && (
            <button
              type="button"
              onClick={() => {
                playSfx('click');
                onRestartVm();
              }}
              className={`hover:text-zinc-200 text-zinc-400 flex items-center gap-1 transition-colors cursor-pointer focus:outline-none focus-visible:ring-2 focus-visible:ring-sky-300 rounded ${narrow ? 'min-h-11' : ''}`}
            >
              <RefreshCw size={11} aria-hidden="true" />
              <span>Restart browser</span>
            </button>
          )}
        </div>
      </div>
    </div>
    </>
  );
};
