// The toast after the switch turns Dev mode on (0.7.1, flow F3, Mira's flow copy §3). The person stays
// on their page; this says what Dev mode found and offers the one next step, plus Undo, the only
// one-press way off. role="status", 8 s (held while the pointer or focus is on it or on the switch), or
// until dismissed. Keyboard (K1): the toast sits on the body, so Tab from the switch goes straight to its
// actions, Shift+Tab from the first goes back, and Tab from the last carries on after the switch. Focus is
// never moved without a key press.
// Desktop: bottom-left over the main pane, 360 px. Phone: full width minus 12 px, docked above the chat
// box so it never covers Send (above the tab bar when there is no chat box). A "checking" toast updates
// in place when the status arrives, without restarting the 8 s.

import React, { useEffect, useLayoutEffect, useRef, useState } from 'react';
import { createPortal } from 'react-dom';
import { X } from 'lucide-react';
import { DevModeState, onToast } from '../devMode';

const focusRing = 'focus-visible:outline focus-visible:outline-2 focus-visible:outline-sky-400 focus-visible:outline-offset-2';

export interface DevOnToastProps {
  state: DevModeState;
  onOpenDesk: () => void;
  onConnect?: () => void;
  onTypeRepo: () => void;
  /** Turns Dev mode off; resolves when saved. */
  onUndo: () => Promise<unknown>;
  onClose: () => void;
  /** Phone layout: full width above the chat box. */
  phone?: boolean;
  timeout?: number;
  /** The switch that turned Dev mode on; the toast's actions are its next Tab stops. */
  anchor?: () => HTMLElement | null;
}

const TABBABLE = 'a[href], button:not([disabled]), input:not([disabled]):not([type="hidden"]), select:not([disabled]), textarea:not([disabled]), [tabindex]:not([tabindex="-1"])';

/** The next Tab stop after `from` in page order, skipping `skip` (the toast) and hidden elements. */
export function nextTabStop(from: HTMLElement, skip: HTMLElement | null): HTMLElement | null {
  const all = Array.from(document.querySelectorAll<HTMLElement>(TABBABLE)).filter(
    (el) => el !== from && !skip?.contains(el) && !el.closest('[inert],[aria-hidden="true"]') && !el.closest('[hidden]') && (typeof el.checkVisibility !== 'function' || el.checkVisibility()),
  );
  return all.find((el) => from.compareDocumentPosition(el) & Node.DOCUMENT_POSITION_FOLLOWING) ?? null;
}

const defaultAnchor = () => (typeof document === 'undefined' ? null : document.querySelector<HTMLElement>('[data-dev-switch]'));

/** Keyboard focus, not a mouse click's leftover focus (a clicked switch keeps focus but nobody is there). */
const keyboardFocus = (el: HTMLElement): boolean => {
  if (document.activeElement !== el) return false;
  try {
    return el.matches(':focus-visible');
  } catch {
    return true;
  }
};

/** Bottom offset: 8 px above the chat box if one is on screen, else the shared toast offset. */
function composerGap(): string | null {
  if (typeof document === 'undefined') return null;
  const form = document.querySelector('[data-composer-form]');
  const r = form?.getBoundingClientRect();
  if (!r || !r.height) return null;
  const h = window.visualViewport?.height ?? window.innerHeight;
  return `${Math.max(16, Math.round(h - r.top + 8))}px`;
}

/** Desktop left edge (flow copy §3): 16 px inside the main pane, never over the sidebar. */
function paneLeft(): number {
  if (typeof document === 'undefined') return 16;
  const r = document.querySelector('[data-main-pane]')?.getBoundingClientRect();
  return r && r.width ? Math.round(r.left + 16) : 16;
}

export const DevOnToast: React.FC<DevOnToastProps> = ({ state, onOpenDesk, onConnect, onTypeRepo, onUndo, onClose, phone = false, timeout = 8000, anchor = defaultAnchor }) => {
  const t = onToast(state);
  const [undo, setUndo] = useState<'idle' | 'busy' | 'failed'>('idle');
  const [bottom, setBottom] = useState<string | null>(null);
  const [held, setHeld] = useState(false);
  const [left, setLeft] = useState(16);
  const box = useRef<HTMLDivElement>(null);
  const anchorRef = useRef(anchor);
  anchorRef.current = anchor;
  // The switch holds the toast too: keyboard focus on it, or the pointer over it.
  const [onSwitch, setOnSwitch] = useState({ focus: false, hover: false });

  useEffect(() => {
    const a = anchorRef.current();
    if (!a) return undefined;
    const onKey = (e: KeyboardEvent) => {
      if (e.key !== 'Tab' || e.shiftKey || e.altKey || e.ctrlKey || e.metaKey || e.defaultPrevented) return;
      const first = box.current?.querySelector<HTMLElement>(TABBABLE);
      if (!first) return;
      e.preventDefault();
      first.focus();
    };
    const focus = (v: boolean) => () => setOnSwitch((o) => (o.focus === v ? o : { ...o, focus: v }));
    const hover = (v: boolean) => () => setOnSwitch((o) => (o.hover === v ? o : { ...o, hover: v }));
    const fIn = () => setOnSwitch((o) => ({ ...o, focus: keyboardFocus(a) }));
    const fOut = focus(false);
    const hIn = hover(true);
    const hOut = hover(false);
    a.addEventListener('keydown', onKey);
    a.addEventListener('focus', fIn);
    a.addEventListener('blur', fOut);
    a.addEventListener('pointerenter', hIn);
    a.addEventListener('pointerleave', hOut);
    if (keyboardFocus(a)) setOnSwitch((o) => ({ ...o, focus: true }));
    return () => {
      a.removeEventListener('keydown', onKey);
      a.removeEventListener('focus', fIn);
      a.removeEventListener('blur', fOut);
      a.removeEventListener('pointerenter', hIn);
      a.removeEventListener('pointerleave', hOut);
    };
  }, []);

  // Inside the toast: Shift+Tab from the first action goes back to the switch; Tab from the last goes on
  // to whatever follows the switch, as if the toast sat right after it.
  const onToastKey = (e: React.KeyboardEvent) => {
    if (e.key !== 'Tab' || e.altKey || e.ctrlKey || e.metaKey) return;
    const a = anchorRef.current();
    const stops = Array.from(box.current?.querySelectorAll<HTMLElement>(TABBABLE) ?? []);
    if (!a || !stops.length) return;
    const at = stops.indexOf(document.activeElement as HTMLElement);
    if (e.shiftKey && at === 0) {
      e.preventDefault();
      a.focus();
    } else if (!e.shiftKey && at === stops.length - 1) {
      const next = nextTabStop(a, box.current);
      if (!next) return;
      e.preventDefault();
      next.focus();
    }
  };

  useLayoutEffect(() => {
    const place = () => {
      setBottom(composerGap());
      setLeft(paneLeft());
    };
    place();
    window.addEventListener('resize', place);
    window.visualViewport?.addEventListener?.('resize', place);
    return () => {
      window.removeEventListener('resize', place);
      window.visualViewport?.removeEventListener?.('resize', place);
    };
  }, []);

  // 8 s, paused while the pointer is over it or focus is inside (WCAG 2.2.1); it starts again on leave.
  useEffect(() => {
    if (undo === 'busy' || held || onSwitch.focus || onSwitch.hover) return undefined;
    const h = setTimeout(onClose, undo === 'failed' ? 6000 : timeout);
    return () => clearTimeout(h);
  }, [undo, held, onSwitch.focus, onSwitch.hover, timeout, onClose]);

  const fix =
    t.action === 'connect' && onConnect
      ? { label: 'Connect GitHub', run: onConnect, key: 'connect' }
      : t.action === 'type'
        ? { label: 'Type a repo', run: onTypeRepo, key: 'type' }
        : { label: 'Open Dev desk', run: onOpenDesk, key: 'open' };

  const doUndo = () => {
    if (undo === 'busy') return;
    setUndo('busy');
    onUndo().then(onClose, () => setUndo('failed'));
  };

  const mono = (p: { text: string; mono?: boolean }, i: number) =>
    p.mono ? (
      <code key={i} className="font-mono text-[12px] font-semibold text-zinc-50 break-all">
        {p.text}
      </code>
    ) : (
      <React.Fragment key={i}>{p.text}</React.Fragment>
    );

  const toast = (
    <div
      className={`kw-dv-ontoast-wrap fixed z-40 pointer-events-none ${phone ? 'inset-x-[6px]' : ''}`}
      style={{ bottom: bottom ?? (phone ? 'calc(var(--kw-tabbar-h, 64px) + 8px)' : '16px'), ...(phone ? {} : { left: `${left}px` }) }}
      data-dev-on-toast={state.status ?? 'checking'}
      data-phone={phone || undefined}
    >
      <div
        ref={box}
        role="status"
        aria-live="polite"
        onKeyDown={onToastKey}
        onPointerEnter={() => setHeld(true)}
        onPointerLeave={() => setHeld(false)}
        onFocus={() => setHeld(true)}
        onBlur={(e) => {
          if (!e.currentTarget.contains(e.relatedTarget as Node | null)) setHeld(false);
        }}
        className={`kw-dv-ontoast pointer-events-auto relative ${phone ? 'w-full' : 'w-[360px] max-w-[calc(100vw-32px)]'} pl-3.5 pr-10 pt-2.5 pb-1.5`}
      >
        <p className="text-meta leading-snug" data-dev-on-toast-title>
          <span className="kw-dv-ontoast-head font-mono font-semibold text-sky-300" aria-hidden="true">
            HEAD → dev
          </span>{' '}
          <strong className="font-semibold text-zinc-50">{t.title}</strong>
        </p>
        <p className="mt-0.5 text-meta text-zinc-300 leading-snug" data-dev-on-toast-text>
          {undo === 'failed' ? <span className="text-bad">Couldn’t turn it off. It’s still on.</span> : t.body.map(mono)}
        </p>
        <div className="mt-1 -ml-2 flex items-center gap-0.5">
          <button
            type="button"
            onClick={() => {
              fix.run();
              onClose();
            }}
            data-dev-on-toast-action={fix.key}
            className={`min-h-[44px] sm:min-h-[32px] px-2 rounded-lg text-meta font-semibold text-[#c9c0ff] hover:bg-violet-500/10 ${focusRing}`}
          >
            {fix.label}
          </button>
          <button
            type="button"
            onClick={doUndo}
            aria-disabled={undo === 'busy' || undefined}
            data-dev-on-toast-undo
            className={`min-h-[44px] sm:min-h-[32px] px-2 rounded-lg text-meta font-semibold text-zinc-300 hover:bg-white/5 aria-disabled:opacity-60 ${focusRing}`}
          >
            {undo === 'busy' ? 'Undoing…' : 'Undo'}
          </button>
        </div>
        <button
          type="button"
          onClick={() => {
            // Keyboard focus was in the toast: it goes back to the switch, not to the page top.
            const back = box.current?.contains(document.activeElement) ? anchorRef.current() : null;
            onClose();
            back?.focus();
          }}
          aria-label="Dismiss"
          className={`absolute top-1 right-1 grid place-items-center min-w-[44px] min-h-[44px] sm:min-w-[32px] sm:min-h-[32px] rounded-lg text-zinc-400 hover:text-zinc-100 hover:bg-white/5 ${focusRing}`}
        >
          <X size={14} aria-hidden="true" />
        </button>
      </div>
    </div>
  );
  // On the body, so a sliding sidebar drawer never carries it off screen.
  return typeof document === 'undefined' ? toast : createPortal(toast, document.body);
};
