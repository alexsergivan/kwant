// The toast after the switch turns Dev mode on (0.7.1, flow F3, Mira's flow copy §3). The person stays
// on their page; this says what Dev mode found and offers the one next step, plus Undo, the only
// one-press way off. role="status", 8 s (held while the pointer or focus is inside), or until dismissed.
// Desktop: bottom-left over the main pane, 360 px. Phone: full width minus 12 px, docked above the chat
// box so it never covers Send (above the tab bar when there is no chat box). A "checking" toast updates
// in place when the status arrives, without restarting the 8 s.

import React, { useEffect, useLayoutEffect, useState } from 'react';
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
}

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

export const DevOnToast: React.FC<DevOnToastProps> = ({ state, onOpenDesk, onConnect, onTypeRepo, onUndo, onClose, phone = false, timeout = 8000 }) => {
  const t = onToast(state);
  const [undo, setUndo] = useState<'idle' | 'busy' | 'failed'>('idle');
  const [bottom, setBottom] = useState<string | null>(null);
  const [held, setHeld] = useState(false);
  const [left, setLeft] = useState(16);

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
    if (undo === 'busy' || held) return undefined;
    const h = setTimeout(onClose, undo === 'failed' ? 6000 : timeout);
    return () => clearTimeout(h);
  }, [undo, held, timeout, onClose]);

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
        role="status"
        aria-live="polite"
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
          onClick={onClose}
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
