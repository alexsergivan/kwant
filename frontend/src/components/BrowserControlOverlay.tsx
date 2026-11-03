import React, { useCallback, useEffect, useLayoutEffect, useMemo, useRef, useState } from 'react';
import { createPortal } from 'react-dom';
import { CheckCircle2, Hand, Keyboard, Loader2, Maximize, Puzzle } from 'lucide-react';
import {
  Box,
  PAGE_H,
  PAGE_W,
  PageInput,
  Pt,
  Size,
  View,
  clampView,
  createGestureTracker,
  fitView,
  stageToPage,
  stageToPageClamped,
  zoomToBox,
} from '../browserGestures';
import { domainOf } from '../humanSteps';
import { haptic } from '../haptics';
import type { BrowserStream } from '../useBrowserStream';
import { PageDialogModal, SelectPopover } from './BrowserPopups';

/** Sends one gesture to the page: presses, releases, taps and text wait for an ack, moves don't. */
export function sendPageInput(send: (payload: Record<string, unknown>, input?: boolean) => void, input: PageInput) {
  send(input.action === 'scroll' ? input : { ...input, delta_x: 0, delta_y: 0 }, input.action !== 'pointer_move');
}

/** A live page image the person drives with a finger (tap, drag) and zooms with two (pinch, pan). */
function ZoomStage({
  image,
  onFrameDone,
  send,
  target,
  onTap,
  label,
  page = { width: PAGE_W, height: PAGE_H },
}: {
  page?: Size;
  image: string | null;
  onFrameDone: () => void;
  send: (input: PageInput) => void;
  /** A box to zoom to (the CAPTCHA); null shows the whole page. */
  target: Box | null;
  onTap?: () => void;
  label: string;
}) {
  const stageRef = useRef<HTMLDivElement>(null);
  const [stage, setStage] = useState<Size>({ width: 0, height: 0 });
  const [view, setView] = useState<View>({ scale: 0, tx: 0, ty: 0 });
  // 'box' follows the target as it moves or the screen turns; a pinch makes it 'free'
  const [mode, setMode] = useState<'fit' | 'box' | 'free'>(target ? 'box' : 'fit');
  const viewRef = useRef(view);
  viewRef.current = view;
  const stageSize = useRef(stage);
  stageSize.current = stage;

  useLayoutEffect(() => {
    const el = stageRef.current;
    if (!el) return;
    const measure = () => setStage({ width: el.clientWidth, height: el.clientHeight });
    measure();
    const ro = new ResizeObserver(measure);
    ro.observe(el);
    return () => ro.disconnect();
  }, []);

  const targetKey = target ? `${target.x},${target.y},${target.width},${target.height}` : '';
  useEffect(() => {
    if (target) setMode('box');
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [targetKey]);

  useLayoutEffect(() => {
    if (!stage.width || !stage.height) return;
    if (mode === 'box' && target) setView(zoomToBox(target, stage, page));
    else if (mode === 'fit' || (mode === 'box' && !target)) setView(fitView(stage, page));
    else setView((v) => clampView(v, stage, page));
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [stage.width, stage.height, mode, targetKey, page.width, page.height]);
  const pageRef = useRef(page);
  pageRef.current = page;

  const sendRef = useRef(send);
  sendRef.current = send;
  const onTapRef = useRef(onTap);
  onTapRef.current = onTap;
  const tracker = useMemo(
    () =>
      createGestureTracker({
        toPage: (p) => stageToPage(p.x, p.y, viewRef.current, pageRef.current),
        toPageClamped: (p) => stageToPageClamped(p.x, p.y, viewRef.current, pageRef.current),
        send: (input) => {
          if (input.action === 'click') onTapRef.current?.();
          sendRef.current(input);
        },
        view: {
          get: () => viewRef.current,
          set: (v) => {
            setMode('free');
            setView(clampView(v, stageSize.current, pageRef.current));
          },
          zoomed: () => viewRef.current.scale > fitView(stageSize.current, pageRef.current).scale * 1.02,
        },
        now: () => performance.now(),
        later: (fn, ms) => {
          const t = window.setTimeout(fn, ms);
          return () => window.clearTimeout(t);
        },
      }),
    []
  );

  const at = (e: React.PointerEvent): Pt => {
    const r = stageRef.current!.getBoundingClientRect();
    return { x: e.clientX - r.left, y: e.clientY - r.top };
  };

  return (
    <div
      ref={stageRef}
      data-browser-stage
      aria-label={label}
      className="relative flex-1 min-h-0 overflow-hidden bg-black select-none"
      style={{ touchAction: 'none' }}
      onPointerDown={(e) => {
        if (e.pointerType === 'mouse' && e.button !== 0) return;
        e.currentTarget.setPointerCapture?.(e.pointerId);
        tracker.down(e.pointerId, at(e), e.isPrimary);
      }}
      onPointerMove={(e) => tracker.move(e.pointerId, at(e))}
      onPointerUp={(e) => tracker.up(e.pointerId, at(e))}
      onPointerCancel={(e) => tracker.cancel(e.pointerId)}
      onContextMenu={(e) => e.preventDefault()}
    >
      {image && view.scale > 0 ? (
        <img
          src={image}
          alt="The bot's browser"
          draggable={false}
          onLoad={onFrameDone}
          onError={onFrameDone}
          className="absolute left-0 top-0 max-w-none pointer-events-none"
          style={{ width: page.width, height: page.height, transformOrigin: '0 0', transform: `translate(${view.tx}px, ${view.ty}px) scale(${view.scale})` }}
        />
      ) : (
        <div className="absolute inset-0 flex items-center justify-center text-zinc-500">
          <Loader2 size={20} className="animate-spin motion-reduce:animate-none" aria-hidden="true" />
          <span className="sr-only">Waiting for the browser</span>
        </div>
      )}

      <div className="absolute right-3 bottom-3 flex gap-2" style={{ marginBottom: 'env(safe-area-inset-bottom)' }}>
        {target && mode !== 'box' && (
          <button
            type="button"
            onPointerDown={(e) => e.stopPropagation()}
            onClick={() => setMode('box')}
            className="inline-flex items-center gap-1.5 min-h-[40px] px-3 rounded-full bg-zinc-900/90 border border-zinc-700 text-sm text-zinc-100"
          >
            <Puzzle size={14} aria-hidden="true" /> Puzzle
          </button>
        )}
        {mode !== 'fit' && (
          <button
            type="button"
            onPointerDown={(e) => e.stopPropagation()}
            onClick={() => setMode('fit')}
            className="inline-flex items-center gap-1.5 min-h-[40px] px-3 rounded-full bg-zinc-900/90 border border-zinc-700 text-sm text-zinc-100"
          >
            <Maximize size={14} aria-hidden="true" /> Whole page
          </button>
        )}
      </div>
    </div>
  );
}

/** Text typed on the phone's keyboard goes to the page; the field itself stays invisible. */
export function textEdits(before: string, after: string): Array<{ action: 'key'; key: 'Backspace' } | { action: 'type'; text: string }> {
  let same = 0;
  while (same < before.length && same < after.length && before[same] === after[same]) same++;
  const out: Array<{ action: 'key'; key: 'Backspace' } | { action: 'type'; text: string }> = [];
  for (let i = same; i < before.length; i++) out.push({ action: 'key', key: 'Backspace' });
  if (after.length > same) out.push({ action: 'type', text: after.slice(same) });
  return out;
}

const PASSED_KEYS = ['Enter', 'Tab', 'Escape', 'ArrowUp', 'ArrowDown', 'ArrowLeft', 'ArrowRight'];

export interface BrowserControlOverlayProps {
  botName: string;
  pageUrl: string;
  image: string | null;
  onFrameDone: () => void;
  send: (payload: Record<string, unknown>, input?: boolean) => void;
  busy: boolean;
  error: string | null;
  /** After a tap: did it land in a text field (then the keyboard opens). */
  tapFocus: { seq: number; editable: boolean } | null;
  /** A CAPTCHA is open: show the hint and zoom to its box. */
  captcha: { target: Box | null } | null;
  /** The CAPTCHA closed by itself: say so, then the parent closes this. */
  solved: boolean;
  onHandBack: () => void;
  /** The page's viewport size in CSS px (1280×800 unless the desktop stage resized it). */
  page?: Size;
  /** For the page's own <select> lists and alert/confirm/prompt, which the screencast can't show. */
  stream?: Pick<BrowserStream, 'select' | 'dialog' | 'clearSelect' | 'clearDialog'>;
  /** An open sign-in, code or other step for this browser: its Done sits on the status line. */
  step?: { need: string; doneLabel: string; onDone: () => Promise<string | void> | void } | null;
}

/**
 * Taking control on a phone: the page fills the screen with one slim bar on top.
 * One finger taps and drags in the page, two fingers zoom the view, and tapping a text field
 * opens the phone's own keyboard.
 */
export const BrowserControlOverlay: React.FC<BrowserControlOverlayProps> = ({
  botName,
  pageUrl,
  image,
  onFrameDone,
  send,
  busy,
  error,
  tapFocus,
  captcha,
  solved,
  onHandBack,
  page,
  stream,
  step = null,
}) => {
  const keys = useRef<HTMLInputElement>(null);
  const [stepBusy, setStepBusy] = useState(false);
  const [stepError, setStepError] = useState<string | null>(null);
  const answerStep = async () => {
    if (!step || stepBusy) return;
    setStepBusy(true);
    setStepError(null);
    try {
      const err = await step.onDone();
      if (err) setStepError(err);
      else onHandBack();
    } finally {
      setStepBusy(false);
    }
  };
  const typed = useRef('');
  const [keyboard, setKeyboard] = useState(false);

  const openKeyboard = useCallback(() => {
    const el = keys.current;
    if (!el) return;
    el.value = '';
    typed.current = '';
    el.focus({ preventScroll: true });
    setKeyboard(true);
  }, []);

  // A tap that focused a text field in the page brings up the keyboard; one that didn't puts it away
  const lastSeq = useRef(tapFocus?.seq ?? 0);
  useEffect(() => {
    if (!tapFocus || tapFocus.seq === lastSeq.current) return;
    lastSeq.current = tapFocus.seq;
    if (tapFocus.editable) openKeyboard();
    else keys.current?.blur();
  }, [tapFocus, openKeyboard]);

  // The puzzle closing as solved is felt as well as shown
  useEffect(() => {
    if (solved) haptic('success');
  }, [solved]);

  // The page behind can't scroll or zoom while this is up
  useEffect(() => {
    const html = document.documentElement;
    const before = html.style.overflow;
    html.style.overflow = 'hidden';
    return () => {
      html.style.overflow = before;
    };
  }, []);

  const domain = domainOf(pageUrl) || pageUrl || 'Browser';
  const pageInput = useCallback((input: PageInput) => sendPageInput(send, input), [send]);

  return createPortal(
    <div
      role="dialog"
      aria-modal="true"
      aria-label={`${botName}'s browser`}
      data-browser-fullscreen
      className="fixed inset-0 z-[70] flex flex-col bg-black text-zinc-100"
      style={{ paddingLeft: 'env(safe-area-inset-left)', paddingRight: 'env(safe-area-inset-right)' }}
    >
      <div className="shrink-0 bg-[#0e0a21] border-b border-white/[0.07]" style={{ paddingTop: 'env(safe-area-inset-top)' }}>
        <div className="h-12 px-2 flex items-center gap-2">
          <span className={`shrink-0 w-2 h-2 rounded-full bg-control ${busy ? 'animate-pulse motion-reduce:animate-none' : ''}`} aria-hidden="true" />
          <span className="flex-1 min-w-0 truncate font-mono text-xs text-zinc-300" title={pageUrl}>
            {domain}
          </span>
          <button
            type="button"
            onClick={openKeyboard}
            aria-pressed={keyboard}
            aria-label="Keyboard"
            className="shrink-0 inline-flex items-center justify-center w-11 h-11 rounded-lg text-zinc-200 hover:bg-white/[0.06] focus:outline-none focus-visible:ring-2 focus-visible:ring-sky-300"
          >
            <Keyboard size={18} aria-hidden="true" />
          </button>
          <button
            type="button"
            onClick={() => {
              haptic('tap');
              onHandBack();
            }}
            className="shrink-0 inline-flex items-center gap-1.5 h-11 px-3 rounded-[10px] bg-[#6c58e0] hover:bg-[#5845bd] text-white text-sm font-semibold focus:outline-none focus-visible:ring-2 focus-visible:ring-sky-300"
          >
            <Hand size={15} aria-hidden="true" /> Hand back
          </button>
        </div>
        {(solved || captcha || error || stepError) ? (
          <p
            role="status"
            aria-live="polite"
            className={`px-3 pb-2 text-xs ${solved ? 'text-[#5eead4]' : error || stepError ? 'text-rose-300' : 'text-[#fcd34d]'}`}
          >
            {solved ? (
              <span className="inline-flex items-center gap-1.5">
                <CheckCircle2 size={13} aria-hidden="true" /> Solved, {botName} continues
              </span>
            ) : error || stepError ? (
              stepError || error
            ) : (
              <>Solve the puzzle, {botName} continues by itself</>
            )}
          </p>
        ) : step ? (
          <div className="h-11 pl-3 pr-1 flex items-center gap-2 text-xs">
            <span role="status" aria-live="polite" className="flex-1 min-w-0 truncate font-semibold text-[#fcd34d]">
              @{botName} {step.need}
            </span>
            <button
              type="button"
              onClick={answerStep}
              disabled={stepBusy}
              className="shrink-0 inline-flex items-center gap-1.5 h-11 px-2 text-sm font-semibold text-sky-200 disabled:opacity-60 focus:outline-none focus-visible:ring-2 focus-visible:ring-sky-300 rounded-lg"
            >
              {stepBusy ? <Loader2 size={14} className="animate-spin motion-reduce:animate-none" aria-hidden="true" /> : <CheckCircle2 size={14} aria-hidden="true" />}
              {step.doneLabel}
            </button>
          </div>
        ) : null}
      </div>

      <ZoomStage
        image={image}
        onFrameDone={onFrameDone}
        send={pageInput}
        target={captcha?.target ?? null}
        label={`${botName}'s page. Tap to click, drag with one finger, pinch to zoom.`}
        page={page}
      />

      {stream?.select && (
        <SelectPopover
          request={stream.select}
          anchor={null}
          stage={{ width: 0, height: 0 }}
          onPick={(indexes) => {
            send(stream.select!.multiple ? { action: 'select_option', indexes } : { action: 'select_option', index: indexes[0] }, true);
            stream.clearSelect();
          }}
          onClose={stream.clearSelect}
        />
      )}
      {stream?.dialog && (
        <PageDialogModal
          key={`${stream.dialog.kind}:${stream.dialog.message}`}
          dialog={stream.dialog}
          host={domain}
          onAnswer={(accept, text) => {
            send(text !== undefined ? { action: 'dialog', accept, prompt_text: text } : { action: 'dialog', accept }, true);
            stream.clearDialog();
          }}
        />
      )}

      <div className="shrink-0" style={{ height: 'env(safe-area-inset-bottom)' }} />

      <input
        ref={keys}
        type="text"
        aria-label="Type into the page"
        autoCapitalize="off"
        autoCorrect="off"
        autoComplete="off"
        spellCheck={false}
        enterKeyHint="send"
        onBlur={() => setKeyboard(false)}
        onInput={(e) => {
          const now = e.currentTarget.value;
          for (const edit of textEdits(typed.current, now)) send(edit, true);
          typed.current = now;
        }}
        onKeyDown={(e) => {
          if (PASSED_KEYS.includes(e.key)) {
            e.preventDefault();
            send({ action: 'key', key: e.key, code: e.code }, true);
            if (e.key === 'Enter') {
              e.currentTarget.value = '';
              typed.current = '';
            }
          } else if (e.key === 'Backspace' && !e.currentTarget.value) {
            send({ action: 'key', key: 'Backspace', code: 'Backspace' }, true);
          }
        }}
        className="fixed left-0 bottom-0 w-px h-px opacity-0"
        style={{ fontSize: 16 }}
      />
    </div>,
    document.body
  );
};

export { ZoomStage };
