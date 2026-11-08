import React, { useCallback, useEffect, useId, useLayoutEffect, useMemo, useRef, useState } from 'react';
import { createPortal } from 'react-dom';
import { ArrowLeft, ArrowRight, Check, Globe, Hand, Keyboard, Loader2, Lock, MousePointerClick, RotateCw } from 'lucide-react';
import {
  Size,
  buttonName,
  createFrameCoalescer,
  createKeyboard,
  createWheelCoalescer,
  isMacPlatform,
  mouseModifiers,
  pagePoint,
  resizeFor,
  sizeDrifted,
  stageLayout,
  typeMessages,
  wheelPixels,
} from '../browserInput';
import { PageInput, Pt, createGestureTracker } from '../browserGestures';
import { domainOf } from '../humanSteps';
import type { BrowserStream } from '../useBrowserStream';
import { BOT_BUSY_TEXT } from '../useBrowserStream';
import { GHOST_BUTTON, PRIMARY_BUTTON, PageDialogModal, SelectPopover } from './BrowserPopups';
import { ModernAvatar } from './ModernAvatar';
import { sendPageInput } from './BrowserControlOverlay';

/** How long the stage waits for its size to settle before asking the page to match it. */
export const RESIZE_DEBOUNCE_MS = 150;
/** How long a page may differ from the asked-for size before the stage asks again, and how often. */
export const SIZE_DRIFT_MS = 600;
const SIZE_RESENDS = 3;
/** "Relaying…" shows only for a discrete action still waiting after this long, so quick ones never flash. */
export const RELAYING_DELAY_MS = 300;

const WIDE_QUERY = '(min-width: 1024px)';
function useWide(): boolean {
  const [wide, setWide] = useState(() => typeof window !== 'undefined' && !!window.matchMedia?.(WIDE_QUERY).matches);
  useEffect(() => {
    const mq = window.matchMedia?.(WIDE_QUERY);
    if (!mq) return;
    const on = () => setWide(mq.matches);
    mq.addEventListener('change', on);
    return () => mq.removeEventListener('change', on);
  }, []);
  return wide;
}

/** The address split for display: host bright, path and query muted. */
export function splitAddress(raw: string): { secure: boolean; host: string; rest: string } {
  try {
    const u = new URL(raw);
    if (u.protocol !== 'http:' && u.protocol !== 'https:') return { secure: false, host: raw, rest: '' };
    const rest = `${u.pathname === '/' && !u.search && !u.hash ? '' : u.pathname}${u.search}${u.hash}`;
    return { secure: u.protocol === 'https:', host: u.host, rest };
  } catch {
    return { secure: false, host: raw, rest: '' };
  }
}

export interface BrowserTakeoverStageProps {
  botName: string;
  pageUrl: string;
  pageTitle?: string;
  stream: BrowserStream;
  /** The page image as a data URL, or null before the first frame. */
  image: string | null;
  /** An open sign-in (or code, or other) step for this browser: answering it also hands back. */
  step?: { doneLabel: string; onDone: () => Promise<string | void> | void } | null;
  /** The bot's avatar sprite, when known. */
  avatarRole?: string;
  /** Opens an address while the bot still has the browser (not used while in control). */
  onNavigate?: (url: string) => void;
  /** A CAPTCHA hint, or that it was solved. */
  notice?: { tone: 'warn' | 'success'; text: string } | null;
  onHandBack: () => void;
  onRetryControl: () => void;
  /** Another action before Hand back, such as Show it once's Record steps. */
  extraAction?: React.ReactNode;
  /** Hand back steps aside (ghost) while something else holds the bar, such as a recording. */
  handBackQuiet?: boolean;
  /** A strip under the top bar, such as the Recording bar. */
  belowBar?: React.ReactNode;
  /**
   * F4: other chats are browsing too. The panel's switcher is under this stage while you're in
   * control, so its "Hand back first" can't be reached; Hand back says it instead.
   */
  othersBrowsing?: boolean;
}

const BUTTON_BIT: Record<number, number> = { 0: 1, 1: 4, 2: 2 };

/**
 * Taking over a bot's browser on a computer or tablet: the page fills a large stage over the app,
 * drawn 1:1 with the page resized to match. Mouse, wheel and keyboard go to the page as they are;
 * the top bar keeps the address, history and "Hand back".
 */
export const BrowserTakeoverStage: React.FC<BrowserTakeoverStageProps> = ({ botName, pageUrl, pageTitle, stream, image, step, avatarRole, onNavigate, notice, onHandBack, onRetryControl, extraAction, handBackQuiet = false, belowBar, othersBrowsing = false }) => {
  const { send, controlling, frame, acknowledgeFrame } = stream;
  const stageRef = useRef<HTMLDivElement>(null);
  const imgRef = useRef<HTMLImageElement>(null);
  const sinkRef = useRef<HTMLTextAreaElement>(null);
  const urlRef = useRef<HTMLInputElement>(null);
  const actionRef = useRef<HTMLButtonElement>(null);
  const wide = useWide();
  // Where keystrokes go: the page, or the top bar (then the outline dims and a hint says so)
  const [pageFocused, setPageFocused] = useState(true);
  const [stage, setStage] = useState<Size>({ width: 0, height: 0 });
  const [url, setUrl] = useState(pageUrl);
  const [urlEditing, setUrlEditing] = useState(false);
  const [stepBusy, setStepBusy] = useState(false);
  const [stepError, setStepError] = useState<string | null>(null);
  const mac = useMemo(() => isMacPlatform(), []);

  useEffect(() => {
    if (!urlEditing) setUrl(pageUrl);
  }, [pageUrl, urlEditing]);

  // The page's size in CSS px; the image may carry dpr times as many pixels
  const page: Size = { width: frame?.width || 1280, height: frame?.height || 800 };
  const layout = stageLayout(stage, page);
  const pageRef = useRef(page);
  pageRef.current = page;
  const controllingRef = useRef(controlling);
  controllingRef.current = controlling;
  const sendRef = useRef(send);
  sendRef.current = send;

  // The app behind can't scroll while this is up
  useEffect(() => {
    const html = document.documentElement;
    const before = html.style.overflow;
    html.style.overflow = 'hidden';
    return () => {
      html.style.overflow = before;
    };
  }, []);

  useLayoutEffect(() => {
    const el = stageRef.current;
    if (!el) return;
    const measure = () => setStage({ width: el.clientWidth, height: el.clientHeight });
    measure();
    const ro = new ResizeObserver(measure);
    ro.observe(el);
    return () => ro.disconnect();
  }, []);

  // Ask the page to be exactly the stage's size, once it stops changing
  const lastResize = useRef('');
  const requested = useRef<{ width: number; height: number } | null>(null);
  const resends = useRef(0);
  useEffect(() => {
    if (!controlling) {
      lastResize.current = '';
      requested.current = null;
      resends.current = 0;
      return;
    }
    const msg = resizeFor(stage, window.devicePixelRatio);
    if (!msg) return;
    const key = `${msg.width}x${msg.height}@${msg.dpr}`;
    if (key === lastResize.current) return;
    const t = window.setTimeout(() => {
      lastResize.current = key;
      requested.current = { width: msg.width, height: msg.height };
      resends.current = 0;
      sendRef.current(msg);
    }, lastResize.current ? RESIZE_DEBOUNCE_MS : 0);
    return () => window.clearTimeout(t);
  }, [controlling, stage.width, stage.height]);

  // A cross-site navigation can put the page back to its own size: ask again, a few times at most
  useEffect(() => {
    if (!controlling || !sizeDrifted(requested.current, page)) {
      if (controlling && requested.current) resends.current = 0;
      return;
    }
    if (resends.current >= SIZE_RESENDS) return;
    const t = window.setTimeout(() => {
      const msg = resizeFor(stage, window.devicePixelRatio);
      if (!msg || !sizeDrifted(requested.current, pageRef.current)) return;
      resends.current += 1;
      sendRef.current(msg);
    }, SIZE_DRIFT_MS);
    return () => window.clearTimeout(t);
  }, [controlling, page.width, page.height, pageUrl]);

  const focusPage = useCallback(() => sinkRef.current?.focus({ preventScroll: true }), []);
  useEffect(() => {
    focusPage();
  }, [focusPage]);

  const toPage = useCallback((clientX: number, clientY: number, clampToEdge: boolean) => {
    const img = imgRef.current;
    if (!img) return null;
    const r = img.getBoundingClientRect();
    return pagePoint(clientX, clientY, r, pageRef.current, clampToEdge);
  }, []);

  // Mouse: moves once a frame, presses and releases as they happen, wheel steps summed per frame
  const heldButtons = useRef(0);
  // Where the pointer last was on the page, for releases that happen off the stage
  const lastPoint = useRef<{ x: number; y: number } | null>(null);
  const lastTouch = useRef(-Infinity);
  const raf = useCallback((fn: () => void) => {
    requestAnimationFrame(fn);
  }, []);
  const moves = useMemo(
    () =>
      createFrameCoalescer<{ x: number; y: number; buttons: number; modifiers: number }>(raf, (m) => {
        const button = m.buttons & 1 ? 'left' : m.buttons & 2 ? 'right' : m.buttons & 4 ? 'middle' : 'none';
        sendRef.current({ action: 'mouse', type: 'move', x: m.x, y: m.y, button, buttons: m.buttons, click_count: 0, modifiers: m.modifiers });
      }),
    [raf]
  );
  const wheel = useMemo(() => createWheelCoalescer(raf, (m) => sendRef.current({ ...m })), [raf]);
  const keyboard = useMemo(() => createKeyboard(mac, (m) => sendRef.current(m as unknown as Record<string, unknown>)), [mac]);

  const popupOpen = !!stream.select || !!stream.dialog;
  const popupRef = useRef(popupOpen);
  popupRef.current = popupOpen;

  useEffect(() => {
    const el = stageRef.current;
    if (!el) return;
    const inPopup = (t: EventTarget | null) => !!(t as Element | null)?.closest?.('[data-browser-popup]');

    const onDown = (e: MouseEvent) => {
      if (inPopup(e.target)) return;
      // A touch also fires mouse events after it; the touch path already sent it
      if (e.timeStamp - lastTouch.current < 800) return;
      e.preventDefault();
      focusPage();
      if (!controllingRef.current) {
        // The first click takes the browser back; it isn't passed to the page
        onRetryControl();
        return;
      }
      const p = toPage(e.clientX, e.clientY, false);
      if (!p) return;
      moves.flush();
      lastPoint.current = p;
      heldButtons.current |= BUTTON_BIT[e.button] ?? 0;
      sendRef.current({ action: 'mouse', type: 'down', x: p.x, y: p.y, button: buttonName(e.button), buttons: e.buttons || heldButtons.current, click_count: Math.max(1, e.detail || 1), modifiers: mouseModifiers(e, mac) });
    };
    const onMove = (e: MouseEvent) => {
      if (!controllingRef.current || popupRef.current) return;
      if (e.timeStamp - lastTouch.current < 800) return;
      const held = heldButtons.current !== 0;
      if (!held && !el.contains(e.target as Node)) return;
      const p = toPage(e.clientX, e.clientY, held);
      if (!p) return;
      lastPoint.current = p;
      moves.push({ ...p, buttons: held ? e.buttons || heldButtons.current : 0, modifiers: mouseModifiers(e, mac) });
    };
    const onUp = (e: MouseEvent) => {
      const bit = BUTTON_BIT[e.button] ?? 0;
      if (!(heldButtons.current & bit)) return;
      heldButtons.current &= ~bit;
      const p = toPage(e.clientX, e.clientY, true);
      if (!p) return;
      lastPoint.current = p;
      moves.flush();
      sendRef.current({ action: 'mouse', type: 'up', x: p.x, y: p.y, button: buttonName(e.button), buttons: heldButtons.current, click_count: Math.max(1, e.detail || 1), modifiers: mouseModifiers(e, mac) });
    };
    const onContext = (e: MouseEvent) => {
      if (!inPopup(e.target)) e.preventDefault();
    };
    const onWheel = (e: WheelEvent) => {
      if (inPopup(e.target)) return;
      e.preventDefault();
      if (!controllingRef.current) return;
      const p = toPage(e.clientX, e.clientY, true);
      if (!p) return;
      wheel.add(p, wheelPixels(e, pageRef.current.height), mouseModifiers(e, mac));
    };
    const onBlurWindow = () => {
      // Released outside the window: this window never sees that release, so send it now, or the
      // page still holds the button and the next press gives it an extra mouseup and click
      const held = heldButtons.current;
      if (!held) return;
      heldButtons.current = 0;
      const p = lastPoint.current;
      if (!controllingRef.current || !p) return;
      moves.flush();
      let left = held;
      for (const [button, bit] of Object.entries(BUTTON_BIT)) {
        if (!(held & bit)) continue;
        left &= ~bit;
        sendRef.current({ action: 'mouse', type: 'up', x: p.x, y: p.y, button: buttonName(Number(button)), buttons: left, click_count: 1, modifiers: 0 });
      }
    };
    el.addEventListener('mousedown', onDown);
    window.addEventListener('mousemove', onMove);
    window.addEventListener('mouseup', onUp);
    el.addEventListener('contextmenu', onContext);
    el.addEventListener('wheel', onWheel, { passive: false });
    window.addEventListener('blur', onBlurWindow);
    return () => {
      el.removeEventListener('mousedown', onDown);
      window.removeEventListener('mousemove', onMove);
      window.removeEventListener('mouseup', onUp);
      el.removeEventListener('contextmenu', onContext);
      el.removeEventListener('wheel', onWheel);
      window.removeEventListener('blur', onBlurWindow);
      moves.drop();
    };
  }, [focusPage, mac, moves, onRetryControl, toPage, wheel]);

  // Touch on a tablet: tap clicks, one finger drags, two fingers scroll (the phone's gestures, no zoom)
  const tracker = useMemo(
    () =>
      createGestureTracker({
        toPage: (p: Pt) => toPage(p.x, p.y, false),
        toPageClamped: (p: Pt) => toPage(p.x, p.y, true) ?? { x: 0, y: 0 },
        send: (input: PageInput) => sendPageInput(sendRef.current, input),
        now: () => performance.now(),
        later: (fn, ms) => {
          const t = window.setTimeout(fn, ms);
          return () => window.clearTimeout(t);
        },
      }),
    [toPage]
  );

  // Keyboard: a hidden, focused text area takes every key (and IME, and paste) for the page
  useEffect(() => {
    const sink = sinkRef.current;
    if (!sink) return;
    const onKey = (e: KeyboardEvent) => {
      // Nothing reaches the app's own shortcuts (Escape, Cmd+K, Ctrl+1…) while typing into the page
      e.stopPropagation();
      if (e.type === 'keydown' && e.key === 'F6') {
        e.preventDefault();
        (actionRef.current ?? urlRef.current)?.focus();
        return;
      }
      if (!controllingRef.current) {
        if (e.type === 'keydown' && !e.isComposing) e.preventDefault();
        return;
      }
      if (keyboard.handle(e)) e.preventDefault();
    };
    const onPaste = (e: ClipboardEvent) => {
      e.preventDefault();
      e.stopPropagation();
      if (!controllingRef.current) return;
      const text = e.clipboardData?.getData('text/plain') ?? '';
      for (const m of typeMessages(text)) sendRef.current(m, true);
    };
    const onCompositionEnd = (e: CompositionEvent) => {
      if (controllingRef.current && e.data) for (const m of typeMessages(e.data)) sendRef.current(m, true);
      sink.value = '';
    };
    // Text that came without a key we could send (dictation, an on-screen keyboard)
    const onInput = (e: Event) => {
      const ie = e as InputEvent;
      if (ie.isComposing) return;
      if (controllingRef.current && ie.inputType === 'insertText' && ie.data) for (const m of typeMessages(ie.data)) sendRef.current(m, true);
      sink.value = '';
    };
    const onBlur = () => {
      keyboard.releaseAll();
      // Switching to another window keeps the page as the target; focus comes back to it
      window.setTimeout(() => {
        if (document.hasFocus() && document.activeElement !== sink) setPageFocused(false);
      }, 0);
    };
    const onFocus = () => setPageFocused(true);
    sink.addEventListener('focus', onFocus);
    sink.addEventListener('keydown', onKey);
    sink.addEventListener('keyup', onKey);
    sink.addEventListener('paste', onPaste);
    sink.addEventListener('compositionend', onCompositionEnd);
    sink.addEventListener('input', onInput);
    sink.addEventListener('blur', onBlur);
    return () => {
      sink.removeEventListener('keydown', onKey);
      sink.removeEventListener('keyup', onKey);
      sink.removeEventListener('paste', onPaste);
      sink.removeEventListener('compositionend', onCompositionEnd);
      sink.removeEventListener('input', onInput);
      sink.removeEventListener('blur', onBlur);
      sink.removeEventListener('focus', onFocus);
      keyboard.releaseAll();
    };
  }, [keyboard]);

  // A new frame is drawn in the same image; acknowledge it once it has loaded
  const frameId = useRef<number | null>(null);
  frameId.current = frame?.id ?? null;
  const onFrameLoaded = useCallback(() => {
    if (frameId.current !== null) acknowledgeFrame(frameId.current);
  }, [acknowledgeFrame]);

  const navigate = (raw: string) => {
    let target = raw.trim();
    if (!target) return;
    if (!/^https?:\/\//i.test(target)) target = `https://${target}`;
    setUrlEditing(false);
    if (controlling) send({ action: 'navigate', url: target }, true);
    else onNavigate?.(target);
    focusPage();
  };
  const history = (direction: 'back' | 'forward' | 'reload') => {
    send({ action: 'history', direction }, true);
    focusPage();
  };

  const answerStep = async () => {
    if (!step) return;
    setStepBusy(true);
    setStepError(null);
    try {
      const err = await step.onDone();
      if (err) {
        setStepError(err);
        return;
      }
      onHandBack();
    } finally {
      setStepBusy(false);
    }
  };


  const domain = domainOf(pageUrl) || pageUrl || 'the page';
  const address = splitAddress(urlEditing ? pageUrl : url || pageUrl);
  const anchor = stream.select
    ? {
        left: layout.left + stream.select.rect.x * layout.scale,
        top: layout.top + stream.select.rect.y * layout.scale,
        width: stream.select.rect.width * layout.scale,
        height: stream.select.rect.height * layout.scale,
      }
    : null;
  const showHistory = !stream.unsupported.has('history');
  const error = stream.error;

  // "Relaying…" only for a discrete action still waiting after a moment, so quick ones never flash
  const [relaying, setRelaying] = useState(false);
  useEffect(() => {
    if (!stream.busy) {
      setRelaying(false);
      return;
    }
    const t = window.setTimeout(() => setRelaying(true), RELAYING_DELAY_MS);
    return () => window.clearTimeout(t);
  }, [stream.busy]);

  // The app behind can't be reached while the stage is up
  useEffect(() => {
    const root = document.getElementById('root');
    if (!root || root.hasAttribute('inert')) return;
    root.setAttribute('inert', '');
    return () => root.removeAttribute('inert');
  }, []);

  const focusOut = controlling && !pageFocused && !popupOpen;
  const outline = controlling ? (focusOut ? 'rgb(124 196 255 / 0.45)' : '#7cc4ff') : 'rgb(255 255 255 / 0.14)';
  const hintId = useId();
  const switchHintId = useId();

  const coarse = '[@media(pointer:coarse)]:h-11';
  const navButton = `shrink-0 inline-flex items-center justify-center w-8 h-8 [@media(pointer:coarse)]:w-11 ${coarse} rounded-lg text-zinc-200 hover:bg-white/[0.06] aria-disabled:text-[#5a5383] aria-disabled:hover:bg-transparent aria-disabled:cursor-default focus:outline-none focus-visible:ring-2 focus-visible:ring-[#b8adff]`;
  const actionButton = `shrink-0 inline-flex items-center gap-1.5 h-8 ${coarse} px-3 rounded-[10px] text-[13px] font-semibold disabled:opacity-60`;

  const who = (
    <div className="shrink-0 w-[248px] max-w-[50vw] min-w-0 flex items-center gap-2.5" data-stage-who>
      <span className="relative shrink-0 w-7 h-7">
        <ModernAvatar role={avatarRole || botName} name={botName} size={28} />
        {controlling && (
          // The bot waits: its Thinking dots, held still
          <span aria-hidden="true" className="absolute -top-1 -right-1.5 flex gap-[2px] px-[3px] py-[2px] rounded-full bg-[#0e0a21] border border-white/[0.14]">
            {[0, 1, 2].map((i) => (
              <span key={i} className="w-[3px] h-[3px] rounded-full bg-zinc-100" />
            ))}
          </span>
        )}
      </span>
      <div className="min-w-0">
        <div className="text-sm leading-[18px] font-semibold text-zinc-100 truncate">{botName}</div>
        <div className="flex items-center gap-1.5 text-xs leading-4 whitespace-nowrap min-w-0" role="status" aria-live="polite">
          {controlling ? (
            <>
              <span className="shrink-0 w-2 h-2 rounded-full bg-control" aria-hidden="true" />
              <span className="shrink-0 font-semibold text-control">You’re in control</span>
              <span className="min-w-0 truncate text-[#a39dcc]">· @{botName} waits</span>
            </>
          ) : (
            <>
              <Loader2 size={12} className="shrink-0 text-[#9d8fff] animate-spin motion-reduce:animate-none" aria-hidden="true" />
              <span className="text-[#a39dcc]">Taking control…</span>
            </>
          )}
        </div>
      </div>
    </div>
  );

  const nav = showHistory && (
    <div className="shrink-0 flex items-center gap-1" data-stage-nav>
      {(
        [
          ['back', 'Back', <ArrowLeft key="b" size={16} aria-hidden="true" />],
          ['forward', 'Forward', <ArrowRight key="f" size={16} aria-hidden="true" />],
          ['reload', 'Reload', <RotateCw key="r" size={16} aria-hidden="true" />],
        ] as const
      ).map(([dir, label, icon]) => (
        <button
          key={dir}
          type="button"
          className={navButton}
          onClick={() => controlling && history(dir)}
          aria-disabled={!controlling || undefined}
          aria-label={label}
          title={label}
        >
          {icon}
        </button>
      ))}
    </div>
  );

  const urlField = (
    <div
      className={`relative flex-1 min-w-0 ${wide ? 'max-w-[760px]' : ''} h-9 [@media(pointer:coarse)]:h-10 flex items-center gap-2 px-3 rounded-[10px] bg-[#120e26] border border-white/[0.14] focus-within:border-transparent focus-within:ring-2 focus-within:ring-[#b8adff]`}
      data-stage-url
    >
      {address.secure ? <Lock size={13} className="shrink-0 text-[#8d87b8]" aria-hidden="true" /> : <Globe size={13} className="shrink-0 text-[#8d87b8]" aria-hidden="true" />}
      <div className="relative flex-1 min-w-0 h-full">
        <input
          ref={urlRef}
          type="text"
          value={url}
          aria-label="Address"
          title={pageTitle || pageUrl}
          spellCheck={false}
          autoCapitalize="off"
          autoCorrect="off"
          onFocus={(e) => {
            setUrlEditing(true);
            e.currentTarget.select();
          }}
          onBlur={() => setUrlEditing(false)}
          onChange={(e) => setUrl(e.target.value)}
          onKeyDown={(e) => {
            if (e.key === 'Enter') {
              e.preventDefault();
              navigate(url);
            } else if (e.key === 'Escape') {
              e.preventDefault();
              e.stopPropagation();
              setUrl(pageUrl);
              focusPage();
            }
          }}
          className={`absolute inset-0 w-full bg-transparent font-mono text-[13px] focus:outline-none ${urlEditing ? 'text-zinc-100' : 'text-transparent'}`}
        />
        {!urlEditing && (
          <span aria-hidden="true" className="absolute inset-0 flex items-center font-mono text-[13px] whitespace-nowrap overflow-hidden pointer-events-none">
            <span className="shrink-0 text-zinc-100">{address.host}</span>
            <span className="min-w-0 truncate text-[#8d87b8]">{address.rest}</span>
          </span>
        )}
      </div>
    </div>
  );

  const actions = (
    <div className="shrink-0 ml-auto flex items-center gap-2" data-stage-actions>
      {extraAction}
      <button
        ref={actionRef}
        type="button"
        onClick={onHandBack}
        title={othersBrowsing ? 'Hand back first to switch browser' : undefined}
        aria-describedby={othersBrowsing ? switchHintId : undefined}
        className={`${actionButton} ${step || handBackQuiet ? GHOST_BUTTON : PRIMARY_BUTTON}`}
        data-primary={(!step && !handBackQuiet) || undefined}
      >
        <Hand size={15} aria-hidden="true" /> Hand back
      </button>
      {othersBrowsing && (
        <span id={switchHintId} className="sr-only">
          Hand back first to switch browser
        </span>
      )}
      {step && (
        <button type="button" onClick={answerStep} disabled={stepBusy} className={`${actionButton} ${PRIMARY_BUTTON}`} data-primary>
          {stepBusy ? <Loader2 size={15} className="animate-spin motion-reduce:animate-none" aria-hidden="true" /> : <Check size={15} aria-hidden="true" />}
          {step.doneLabel}
        </button>
      )}
    </div>
  );

  return createPortal(
    <div className={`fixed inset-0 z-[70] flex bg-[rgb(5_3_15/0.72)] kw-scrim-in ${wide ? 'p-4' : ''}`} data-browser-takeover>
      <div
        role="dialog"
        aria-modal="true"
        aria-label={`${botName}'s browser, you're in control`}
        aria-describedby={hintId}
        data-escape-layer
        data-control-outline={focusOut ? 'dim' : controlling ? 'on' : 'off'}
        className={`relative flex-1 min-w-0 flex flex-col overflow-hidden bg-[#0a0719] text-zinc-100 kw-stage-in ${wide ? 'rounded-[16px]' : ''}`}
        style={wide ? { boxShadow: `0 0 0 2px ${outline}, 0 24px 80px rgb(0 0 0 / 0.6)` } : undefined}
        onKeyDown={(e) => {
          // Keys in the top bar stay in the top bar; F6 and Escape go back to the page
          e.stopPropagation();
          if (e.key === 'F6' || e.key === 'Escape') {
            e.preventDefault();
            focusPage();
          }
        }}
      >
        <p id={hintId} className="sr-only">
          Press F6 to reach Hand back.
        </p>
        <div
          className={`shrink-0 bg-[#0e0a21] border-b border-white/[0.07] ${wide ? 'h-14 pl-[14px] pr-3 flex items-center gap-3' : 'px-3 py-2 flex flex-col gap-2'}`}
          data-stage-bar
        >
          {wide ? (
            <>
              {who}
              {nav}
              {urlField}
              {actions}
            </>
          ) : (
            <>
              <div className="flex items-center gap-3 min-w-0">
                {who}
                {actions}
              </div>
              <div className="flex items-center gap-2 min-w-0">
                {nav}
                {urlField}
              </div>
            </>
          )}
        </div>

        {belowBar}
        {notice && !error && !stepError && (
          <div
            role="status"
            aria-live="polite"
            className={`shrink-0 px-3 py-1.5 text-xs border-b ${
              notice.tone === 'success' ? 'border-[rgb(94_234_212/0.2)] bg-[rgb(94_234_212/0.08)] text-[#5eead4]' : 'border-[rgb(252_211_77/0.2)] bg-[rgb(252_211_77/0.08)] text-[#fcd34d]'
            }`}
          >
            {notice.text}
          </div>
        )}
        {(error || stepError) && (
          <div role="alert" className="shrink-0 px-3 py-1.5 flex items-center gap-2 text-xs border-b border-rose-500/20 bg-rose-950/40 text-rose-200">
            <span className="flex-1">{stepError || error}</span>
            {!controlling && error === BOT_BUSY_TEXT && (
              <button type="button" onClick={onRetryControl} className={`px-2 h-7 rounded-md text-xs ${PRIMARY_BUTTON}`}>
                Try again
              </button>
            )}
          </div>
        )}

        <div
          ref={stageRef}
          data-browser-stage
          className="relative flex-1 min-h-0 overflow-hidden bg-[#0a0719] select-none"
          style={{ touchAction: 'none', cursor: controlling ? stream.cursor || 'default' : 'progress' }}
          onPointerDown={(e) => {
            if (e.pointerType !== 'touch' || popupOpen) return;
            lastTouch.current = e.timeStamp;
            focusPage();
            if (!controlling) {
              onRetryControl();
              return;
            }
            e.currentTarget.setPointerCapture?.(e.pointerId);
            tracker.down(e.pointerId, { x: e.clientX, y: e.clientY }, e.isPrimary);
          }}
          onPointerMove={(e) => {
            if (e.pointerType !== 'touch') return;
            lastTouch.current = e.timeStamp;
            tracker.move(e.pointerId, { x: e.clientX, y: e.clientY });
          }}
          onPointerUp={(e) => {
            if (e.pointerType !== 'touch') return;
            lastTouch.current = e.timeStamp;
            tracker.up(e.pointerId, { x: e.clientX, y: e.clientY });
          }}
          onPointerCancel={(e) => e.pointerType === 'touch' && tracker.cancel(e.pointerId)}
        >
          {image ? (
            <img
              ref={imgRef}
              src={image}
              alt={`${botName}'s page: ${domain}`}
              draggable={false}
              onLoad={onFrameLoaded}
              onError={onFrameLoaded}
              data-browser-page
              className="absolute max-w-none pointer-events-none"
              style={{ left: layout.left, top: layout.top, width: layout.width, height: layout.height }}
            />
          ) : (
            <div className="absolute inset-0 flex items-center justify-center text-[#a39dcc]">
              <Loader2 size={20} className="animate-spin motion-reduce:animate-none" aria-hidden="true" />
              <span className="sr-only">Waiting for the browser</span>
            </div>
          )}

          {focusOut && (
            <>
              <div className="absolute inset-0 bg-[rgb(10_7_25/0.18)] pointer-events-none" aria-hidden="true" />
              <div
                className="absolute top-3 left-1/2 -translate-x-1/2 h-7 px-3 inline-flex items-center gap-1.5 rounded-full bg-[rgb(14_10_33/0.92)] border border-white/[0.14] text-xs text-zinc-100 whitespace-nowrap pointer-events-none"
                data-focus-hint
              >
                <Keyboard size={13} aria-hidden="true" /> Click the page to type into it
              </div>
            </>
          )}

          {relaying && (
            <div className="absolute top-3 right-3 h-6 px-2.5 inline-flex items-center gap-1.5 rounded-full bg-[rgb(14_10_33/0.9)] border border-white/[0.14] text-xs text-zinc-200 pointer-events-none" data-relaying>
              <Loader2 size={12} className="text-[#9d8fff] animate-spin motion-reduce:animate-none" aria-hidden="true" />
              Relaying…
            </div>
          )}

          {!controlling && (
            <div className="absolute inset-0 flex items-center justify-center pointer-events-none">
              <span className="inline-flex items-center gap-2 px-4 h-10 rounded-full bg-[rgb(14_10_33/0.92)] border border-white/[0.14] text-sm">
                {error === BOT_BUSY_TEXT ? (
                  <>
                    <MousePointerClick size={15} aria-hidden="true" /> Click to try again
                  </>
                ) : (
                  <>
                    <Loader2 size={15} className="text-[#9d8fff] animate-spin motion-reduce:animate-none" aria-hidden="true" /> Taking control…
                  </>
                )}
              </span>
            </div>
          )}

          {stream.select && controlling && (
            <SelectPopover
              key={JSON.stringify(stream.select.rect)}
              request={stream.select}
              anchor={anchor}
              stage={stage}
              onPick={(indexes) => {
                send(stream.select!.multiple ? { action: 'select_option', indexes } : { action: 'select_option', index: indexes[0] }, true);
                stream.clearSelect();
                focusPage();
              }}
              onClose={() => {
                stream.clearSelect();
                focusPage();
              }}
            />
          )}
          {stream.dialog && (
            <PageDialogModal
              key={`${stream.dialog.kind}:${stream.dialog.message}`}
              dialog={stream.dialog}
              host={domain}
              onAnswer={(accept, text) => {
                send(text !== undefined ? { action: 'dialog', accept, prompt_text: text } : { action: 'dialog', accept }, true);
                stream.clearDialog();
                focusPage();
              }}
            />
          )}

          <textarea
            ref={sinkRef}
            aria-label={`Type into ${domain}. F6 moves to Hand back.`}
            aria-keyshortcuts="F6"
            autoCapitalize="off"
            autoCorrect="off"
            autoComplete="off"
            spellCheck={false}
            data-browser-keys
            className="absolute left-0 top-0 w-px h-px p-0 border-0 opacity-0 resize-none overflow-hidden"
            style={{ caretColor: 'transparent' }}
            tabIndex={0}
          />
        </div>
        {!wide && <div aria-hidden="true" className="absolute inset-0 pointer-events-none z-[60]" style={{ boxShadow: `inset 0 0 0 2px ${outline}` }} />}
      </div>
    </div>,
    document.body
  );
};
