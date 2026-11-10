// G3 (0.9.0) Point at it: the layer over the watch view while Point is on. Hover (or a tap on a
// phone) outlines the element under the pointer, arrows walk the tree, a click / Enter / Add to
// message picks it. A pick becomes an unsent upload in the composer (through `sink`).
// Design: release-090/designs/g3-point.md. Wire: release-090/g3-contract.md rev 2.
import React, { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import {
  ARROW_DIRS,
  FAIL_BAR_MS,
  HOVER_THROTTLE_MS,
  LIMIT_BAR_MS,
  PICKED_FLASH_MS,
  POINT_COPY,
  PointEl,
  PointError,
  PointPick,
  SELF_REFRESH_MS,
  addedText,
  containFit,
  parsePointEl,
  parsePointPick,
  pickRefusal,
  pickedCountText,
  spokenText,
  tagPlacement,
  tagText,
  toPage,
  toView,
} from '../point';

/** Where picks go: the composer's upload list. */
export interface PointSink {
  /** Picks already in the composer (capturing ones included). */
  picks: number;
  /** Files the composer can still take (10 in all). */
  filesLeft: number;
  /** A capturing chip appears; returns its key. */
  start(label: string, n: number, opts: { fromMouse: boolean }): string;
  /** The capture finished: the chip becomes the pick. */
  done(key: string, pick: PointPick): void;
  /** The capture failed: the chip goes away. */
  fail(key: string): void;
}

export interface PointLayerProps {
  request: (payload: Record<string, unknown>) => Promise<Record<string, any>>;
  /** The page's viewport in CSS px (the frame's width and height), or null before the first frame. */
  page: { width: number; height: number } | null;
  /** The newest frame's id: a new one re-reads the selected element (`dir:"self"`). */
  frameId: number | null;
  pageUrl?: string;
  phone: boolean;
  sink: PointSink;
  /** Esc (or the phone's Back to chat, through onBackToChat) */
  onExit: () => void;
  onBackToChat?: () => void;
}

type Bar = { kind: 'fail' | 'limit' | 'waiting'; text: string } | null;
type Sheet = 'none' | 'select' | 'added';

function useReducedMotion(): boolean {
  const [reduced, setReduced] = useState(() => typeof window !== 'undefined' && !!window.matchMedia?.('(prefers-reduced-motion: reduce)').matches);
  useEffect(() => {
    const mq = window.matchMedia?.('(prefers-reduced-motion: reduce)');
    if (!mq) return;
    const on = () => setReduced(mq.matches);
    mq.addEventListener?.('change', on);
    return () => mq.removeEventListener?.('change', on);
  }, []);
  return reduced;
}

const codeOf = (e: unknown) => (e instanceof PointError ? e.code : 'failed');

export const PointLayer: React.FC<PointLayerProps> = ({ request, page, frameId, pageUrl, phone, sink, onExit, onBackToChat }) => {
  const viewRef = useRef<HTMLDivElement>(null);
  const [size, setSize] = useState<{ width: number; height: number }>({ width: 0, height: 0 });
  const [sel, setSel] = useState<PointEl | null>(null);
  const [picked, setPicked] = useState<string | null>(null);
  const [bar, setBar] = useState<Bar>(null);
  const [said, setSaid] = useState('');
  const [sheet, setSheet] = useState<Sheet>('none');
  const reduced = useReducedMotion();

  // Only the newest hit test / neighbour answer is drawn
  const tok = useRef(0);
  const lastPos = useRef<{ x: number; y: number } | null>(null);
  const lastHover = useRef(0);
  const hoverTimer = useRef<ReturnType<typeof setTimeout> | null>(null);
  const sinkRef = useRef(sink);
  sinkRef.current = sink;
  const selRef = useRef(sel);
  selRef.current = sel;
  const alive = useRef(true);
  useEffect(() => () => {
    alive.current = false;
    if (hoverTimer.current) clearTimeout(hoverTimer.current);
  }, []);

  // Measure the view the page image is drawn in
  useEffect(() => {
    const el = viewRef.current;
    if (!el) return;
    const measure = () => setSize({ width: el.clientWidth, height: el.clientHeight });
    measure();
    if (typeof ResizeObserver === 'undefined') return;
    const ro = new ResizeObserver(measure);
    ro.observe(el);
    return () => ro.disconnect();
  }, []);

  // Keyboard users land in the view when Point turns on
  useEffect(() => {
    viewRef.current?.focus({ preventScroll: true });
  }, []);

  const fit = useMemo(() => (page ? containFit(size, page) : null), [size, page]);

  // Bars clear themselves (the waiting bar stays until an answer comes)
  useEffect(() => {
    if (!bar || bar.kind === 'waiting') return;
    const t = setTimeout(() => setBar(null), bar.kind === 'fail' ? FAIL_BAR_MS : LIMIT_BAR_MS);
    return () => clearTimeout(t);
  }, [bar]);

  // Teal after a pick: 600 ms, or until the selection changes under reduced motion
  useEffect(() => {
    if (!picked || reduced) return;
    const t = setTimeout(() => setPicked(null), PICKED_FLASH_MS);
    return () => clearTimeout(t);
  }, [picked, reduced]);

  const select = useCallback((el: PointEl | null, announce = true) => {
    setSel((prev) => {
      if (prev?.handle !== el?.handle) setPicked(null);
      return el;
    });
    if (el && announce) setSaid(spokenText(el));
  }, []);

  const onError = useCallback((e: unknown, loud: boolean) => {
    const code = codeOf(e);
    if (code === 'loading') setBar({ kind: 'waiting', text: POINT_COPY.waiting });
    else if (code === 'bad_request') console.error('point: bad request', e);
    else if (loud && code !== 'unsupported') setBar({ kind: 'fail', text: POINT_COPY.failed });
    if (code === 'nothing' || code === 'stale') select(null, false);
  }, [select]);

  /** Hit test at a view point; `then` runs with the element when it's still the newest answer. */
  const hit = useCallback((vx: number, vy: number, loud: boolean, then?: (el: PointEl) => void) => {
    if (!fit || !page) return;
    const p = toPage(fit, page, vx, vy);
    if (!p) {
      if (loud) setBar({ kind: 'fail', text: POINT_COPY.failed });
      select(null, false);
      return;
    }
    lastPos.current = { x: vx, y: vy };
    const t = ++tok.current;
    request({ action: 'point_at', x: p.x, y: p.y }).then(
      (m) => {
        if (!alive.current || t !== tok.current) return;
        const el = parsePointEl(m.el);
        if (!el) return onError(new PointError('failed'), loud);
        setBar((b) => (b?.kind === 'waiting' ? null : b));
        select(el);
        then?.(el);
      },
      (e) => {
        if (!alive.current || t !== tok.current) return;
        onError(e, loud);
      },
    );
  }, [fit, page, request, select, onError]);

  const neighbour = useCallback((dir: string) => {
    const cur = selRef.current;
    const t = ++tok.current;
    request(cur ? { action: 'point_neighbour', handle: cur.handle, dir } : { action: 'point_neighbour', dir: 'centre' }).then(
      (m) => {
        if (!alive.current || t !== tok.current) return;
        const el = parsePointEl(m.el, m.edge ?? m.el?.edge);
        if (!el) return onError(new PointError('failed'), true);
        setBar((b) => (b?.kind === 'waiting' ? null : b));
        // Nothing that way: keep the selection and say nothing new
        if (el.edge && cur && el.handle === cur.handle) return;
        select(el);
      },
      (e) => {
        if (!alive.current || t !== tok.current) return;
        if (codeOf(e) === 'stale' && lastPos.current) return hit(lastPos.current.x, lastPos.current.y, false);
        onError(e, true);
      },
    );
  }, [request, select, onError, hit]);

  const pick = useCallback((el: PointEl, fromMouse: boolean) => {
    const s = sinkRef.current;
    const refusal = pickRefusal(s.picks, s.filesLeft);
    if (refusal) {
      setBar({ kind: 'limit', text: refusal });
      setSaid(refusal);
      return;
    }
    const n = s.picks + 1;
    const key = s.start(el.label, n, { fromMouse });
    setPicked(el.handle);
    setSaid(addedText(el.label));
    if (phone) setSheet('added');
    request({ action: 'point_pick', handle: el.handle, n }).then(
      (m) => {
        const p = parsePointPick(m.pick);
        if (!p) {
          sinkRef.current.fail(key);
          if (alive.current) setBar({ kind: 'fail', text: POINT_COPY.failed });
          return;
        }
        sinkRef.current.done(key, p);
      },
      (e) => {
        sinkRef.current.fail(key);
        if (!alive.current) return;
        setPicked(null);
        if (phone) setSheet('none');
        const code = codeOf(e);
        if (code === 'bad_request') console.error('point: bad request', e);
        setBar({ kind: 'fail', text: POINT_COPY.failed });
        if (code === 'stale' && lastPos.current) hit(lastPos.current.x, lastPos.current.y, false);
      },
    );
  }, [request, phone, hit]);

  // A new frame while something is selected: re-read its box, at most every 250 ms
  const lastSelf = useRef(0);
  const selfTimer = useRef<ReturnType<typeof setTimeout> | null>(null);
  useEffect(() => {
    if (!sel || frameId === null) return;
    const run = () => {
      selfTimer.current = null;
      const cur = selRef.current;
      if (!cur) return;
      lastSelf.current = Date.now();
      const t = tok.current;
      request({ action: 'point_neighbour', handle: cur.handle, dir: 'self' }).then(
        (m) => {
          if (!alive.current || t !== tok.current || selRef.current?.handle !== cur.handle) return;
          const el = parsePointEl(m.el);
          if (el) setSel(el);
        },
        (e) => {
          if (!alive.current || t !== tok.current) return;
          if (codeOf(e) === 'stale') {
            if (lastPos.current && !phone) hit(lastPos.current.x, lastPos.current.y, false);
            else select(null, false);
          }
        },
      );
    };
    if (selfTimer.current) return;
    const wait = Math.max(0, SELF_REFRESH_MS - (Date.now() - lastSelf.current));
    selfTimer.current = setTimeout(run, wait);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [frameId]);
  useEffect(() => () => {
    if (selfTimer.current) clearTimeout(selfTimer.current);
  }, []);

  // The page moved on: whatever was selected belongs to the old one
  const firstUrl = useRef(true);
  useEffect(() => {
    if (firstUrl.current) {
      firstUrl.current = false;
      return;
    }
    tok.current++;
    select(null, false);
    setSheet('none');
  }, [pageUrl, select]);

  const local = (e: { clientX: number; clientY: number }) => {
    const r = viewRef.current!.getBoundingClientRect();
    return { x: e.clientX - r.left, y: e.clientY - r.top };
  };

  const onPointerMove = (e: React.PointerEvent) => {
    if (phone || e.pointerType !== 'mouse') return;
    const { x, y } = local(e);
    lastPos.current = { x, y };
    const fire = () => {
      hoverTimer.current = null;
      lastHover.current = Date.now();
      const p = lastPos.current;
      if (p) hit(p.x, p.y, false);
    };
    if (hoverTimer.current) return;
    const since = Date.now() - lastHover.current;
    if (since >= HOVER_THROTTLE_MS) fire();
    else hoverTimer.current = setTimeout(fire, HOVER_THROTTLE_MS - since);
  };

  const outline = sel && fit && page ? toView(fit, page, sel.box) : null;

  const onClick = (e: React.MouseEvent) => {
    const { x, y } = local(e);
    if (phone) {
      hit(x, y, true, () => setSheet('select'));
      return;
    }
    const cur = selRef.current;
    if (cur && outline && x >= outline.x && x <= outline.x + outline.width && y >= outline.y && y <= outline.y + outline.height) {
      pick(cur, true);
      return;
    }
    hit(x, y, true, (el) => pick(el, true));
  };

  const onKeyDown = (e: React.KeyboardEvent) => {
    if (e.target !== e.currentTarget) return;
    const dir = ARROW_DIRS[e.key];
    if (dir) {
      e.preventDefault();
      neighbour(selRef.current ? dir : 'centre');
      return;
    }
    if (e.key === 'Enter' || e.key === ' ') {
      e.preventDefault();
      if (selRef.current) pick(selRef.current, false);
      return;
    }
    if (e.key === 'Escape') {
      e.preventDefault();
      e.stopPropagation();
      if (sheet !== 'none') setSheet('none');
      else onExit();
    }
  };

  const teal = !!sel && picked === sel.handle;
  const tag = sel ? tagText(sel) : '';
  const place = outline ? tagPlacement(outline, size.height) : 'above';
  const topText = !page ? POINT_COPY.waiting : phone ? POINT_COPY.barPhone : POINT_COPY.barDesktop;

  return (
    <div
      ref={viewRef}
      role="application"
      tabIndex={0}
      aria-label={POINT_COPY.viewName}
      data-point-layer=""
      data-point-state={teal ? 'picked' : sel ? 'selected' : 'idle'}
      onPointerMove={onPointerMove}
      onPointerLeave={() => {
        if (hoverTimer.current) clearTimeout(hoverTimer.current);
        hoverTimer.current = null;
      }}
      onClick={onClick}
      onKeyDown={onKeyDown}
      className="absolute inset-0 z-10 cursor-crosshair rounded-xl ring-2 ring-inset ring-violet-500/70 focus:outline-none focus-visible:ring-violet-300"
    >
      <div className="absolute top-0 inset-x-0 h-7 px-2.5 flex items-center justify-between gap-2 bg-[rgb(14_10_33/0.88)] text-[11.5px] text-zinc-100 pointer-events-none rounded-t-xl">
        <span className="truncate font-semibold">{bar?.kind === 'waiting' ? bar.text : topText}</span>
        {!phone && <span className="shrink-0 text-zinc-400">{POINT_COPY.keys}</span>}
      </div>

      {outline && outline.width > 0 && outline.height > 0 && (
        <>
          <div
            data-point-outline=""
            className={`absolute pointer-events-none rounded-[3px] border-2 ${
              teal ? 'border-teal-300 bg-teal-300/15' : 'border-violet-400 bg-violet-400/10'
            } motion-safe:transition-[left,top,width,height] motion-safe:duration-75`}
            style={{ left: outline.x, top: outline.y, width: outline.width, height: outline.height }}
          />
          <span
            data-point-tag=""
            className={`absolute pointer-events-none max-w-[calc(100%-8px)] truncate h-[18px] px-1.5 leading-[18px] rounded font-mono text-[11px] ${
              teal ? 'bg-teal-300 text-teal-950' : 'bg-violet-500 text-white'
            }`}
            style={{
              left: Math.max(4, Math.min(outline.x, size.width - 4 - 160)),
              top: place === 'above' ? outline.y - 20 : place === 'below' ? outline.y + outline.height + 2 : outline.y + 2,
            }}
          >
            {teal ? addedText(sel!.label) : tag}
          </span>
        </>
      )}

      {bar && bar.kind !== 'waiting' && (
        <div
          role="alert"
          data-point-bar={bar.kind}
          className={`absolute bottom-2 inset-x-2 px-2.5 py-1.5 rounded-lg text-xs pointer-events-none ${
            bar.kind === 'fail' ? 'border border-rose-500/40 bg-rose-950/90 text-rose-200' : 'border border-amber-400/40 bg-[#2a1f08]/90 text-amber-200'
          }`}
        >
          {bar.text}
        </div>
      )}

      <span className="sr-only" aria-live="polite" data-point-live="">
        {said}
      </span>

      {phone && sheet !== 'none' && (
        <div
          role="dialog"
          aria-label={sheet === 'added' ? POINT_COPY.added : tag}
          data-point-sheet={sheet}
          onClick={(e) => e.stopPropagation()}
          className="fixed inset-x-0 bottom-0 z-[70] px-4 pt-3 pb-[calc(12px+env(safe-area-inset-bottom))] bg-[#14102b] border-t border-[#3f3868] rounded-t-2xl shadow-2xl cursor-default"
        >
          {sheet === 'select' && sel ? (
            <>
              <p className="font-mono text-[12px] text-zinc-100 truncate">{tag}</p>
              {sel.text && <p className="mt-0.5 text-[12px] text-zinc-400 truncate">{sel.text}</p>}
              <div className="mt-3 flex gap-2">
                <button
                  type="button"
                  onClick={() => neighbour('parent')}
                  className="flex-1 h-11 rounded-[10px] border border-[#3f3868] text-zinc-100 text-sm font-semibold focus:outline-none focus-visible:ring-2 focus-visible:ring-violet-300"
                >
                  {POINT_COPY.wider}
                </button>
                <button
                  type="button"
                  onClick={() => pick(sel, false)}
                  className="flex-1 h-11 rounded-[10px] bg-violet-600 text-white text-sm font-semibold focus:outline-none focus-visible:ring-2 focus-visible:ring-violet-300"
                >
                  {POINT_COPY.add}
                </button>
              </div>
            </>
          ) : (
            <>
              <p className="text-sm font-semibold text-teal-200">{POINT_COPY.added}</p>
              <p className="mt-0.5 text-[12px] text-zinc-300">{pickedCountText(sink.picks)}</p>
              <div className="mt-3 flex gap-2">
                <button
                  type="button"
                  onClick={() => {
                    setSheet('none');
                    select(null, false);
                  }}
                  className="flex-1 h-11 rounded-[10px] border border-[#3f3868] text-zinc-100 text-sm font-semibold focus:outline-none focus-visible:ring-2 focus-visible:ring-violet-300"
                >
                  {POINT_COPY.keep}
                </button>
                <button
                  type="button"
                  onClick={() => (onBackToChat ?? onExit)()}
                  className="flex-1 h-11 rounded-[10px] bg-violet-600 text-white text-sm font-semibold focus:outline-none focus-visible:ring-2 focus-visible:ring-violet-300"
                >
                  {POINT_COPY.back}
                </button>
              </div>
            </>
          )}
        </div>
      )}
    </div>
  );
};
