// The space controls under the space header (explainer spec §1–§5, kwant 0.7.2): one block with two
// identical rows. Row 1 is Dev mode: a label, a "?" that opens the explainer, the switch, and one line
// under it that says what Dev mode is doing (devSubLine). Row 2 is Approvals: the space's setting as a
// chip that opens Settings › This space, and what still asks. The header subtitle never changes, and the
// </> badge lives on the rail only.

import React, { useCallback, useEffect, useLayoutEffect, useRef, useState } from 'react';
import { createPortal } from 'react-dom';
import { ArrowUpRight, ChevronRight, CircleHelp, CodeXml, Lock, Shield, ShieldCheck, ShieldOff, X } from 'lucide-react';
import { DEV_FEATURES, DEV_GUIDE_URL, EXPLAINER, SUB_ACTION_TEXT, type DevModeState, type DevSub, type DevSubAction, devSubLine } from '../devMode';
import { approvalsRow, type SpaceApprovals } from '../approvalMode';
import { DevModeSwitch, useDevMode } from './DevModeSwitch';
import { backGestureEntry } from './BrowserControlOverlay';

export interface SpaceControlsProps {
  space: { id: string; name: string; approvals?: SpaceApprovals };
  phone?: boolean;
  /** The space's bots, for what still asks when approvals differ per bot. */
  bots: Array<{ approvals_effective?: string | null }>;
  onTurnedOn?: (s: DevModeState) => void;
  onTurnedOff?: () => void;
  /** The sub-line actions and the explainer's "Open Dev desk". Absent: the action isn't drawn. */
  onOpenDevDesk?: () => void;
  onSeeRepos?: () => void;
  onLinkRepo?: () => void;
  onConnect?: () => void;
  /** The Approvals value: Settings › This space (turning approvals off stays there, behind its confirm). */
  onOpenSettings: () => void;
}

export const SpaceControls: React.FC<SpaceControlsProps> = ({ space, phone = false, bots, onTurnedOn, onTurnedOff, onOpenDevDesk, onSeeRepos, onLinkRepo, onConnect, onOpenSettings }) => {
  const dev = useDevMode(space.id);
  const [look, setLook] = useState({ armed: false, disabled: false, on: false });
  const onLook = useCallback((l: { armed: boolean; disabled: boolean; on: boolean }) => setLook(l), []);
  const [open, setOpen] = useState(false);
  const help = useRef<HTMLButtonElement>(null);
  const switchBtn = useRef<HTMLButtonElement | null>(null);
  useEffect(() => setOpen(false), [space.id]);

  const sub = devSubLine(dev, { armed: look.armed, disabled: look.disabled });
  const lbl = `dv-lbl-${space.id}`;
  const subId = `dv-sub-${space.id}`;
  const dlg = `dvx-${space.id}`;
  const on = !!dev?.on;
  const actions: Record<DevSubAction, (() => void) | undefined> = { repos: onSeeRepos, link: onLinkRepo, connect: onConnect };
  const act = sub.action ? actions[sub.action] : undefined;
  const appr = approvalsRow(space.approvals, bots, on);

  const close = useCallback((refocus: boolean) => {
    setOpen(false);
    if (refocus) setTimeout(() => help.current?.focus(), 0);
  }, []);

  return (
    <div role="group" aria-label="Space controls" className={`kw-sc${phone ? ' kw-sc-phone' : ''}`} data-space-controls>
      <div className="kw-sc-row" data-sc-row="dev">
        <CodeXml size={16} strokeWidth={2} aria-hidden="true" className={`kw-sc-icon ${on || look.armed ? 'text-sky-300' : 'text-zinc-500'}`} />
        <span className="kw-sc-label">
          <span id={lbl}>Dev mode</span>
          <button
            ref={help}
            type="button"
            className="kw-sc-help"
            aria-label="What is Dev mode?"
            aria-haspopup="dialog"
            aria-expanded={open}
            aria-controls={dlg}
            data-dv-help
            data-open={open || undefined}
            onClick={() => setOpen((v) => !v)}
          >
            <CircleHelp size={14} aria-hidden="true" />
          </button>
        </span>
        <DevModeSwitch
          key={space.id}
          space={space}
          phone={phone}
          labelledBy={lbl}
          describedBy={subId}
          onLook={onLook}
          buttonRef={switchBtn}
          onTurnedOn={onTurnedOn}
          onTurnedOff={onTurnedOff}
        />
        <DevSubLine id={subId} sub={sub} phone={phone} onAction={act} />
      </div>
      {/* Answer 3: while the space's approvals are still loading, a skeleton the row's height, no text. */}
      {!appr && space.approvals === undefined && (
        <div className="kw-sc-row kw-sc-row-skeleton" data-sc-row="approvals" data-sc-skeleton aria-hidden="true">
          <span className="kw-sc-skel kw-sc-skel-icon" />
          <span className="kw-sc-skel kw-sc-skel-line" />
          <span className="kw-sc-skel kw-sc-skel-chip" />
        </div>
      )}
      {appr && (
        <div className="kw-sc-row" data-sc-row="approvals">
          {appr.value === 'ask' ? (
            <ShieldCheck size={16} strokeWidth={2} aria-hidden="true" className="kw-sc-icon text-ok" />
          ) : appr.value === 'off' ? (
            <ShieldOff size={16} strokeWidth={2} aria-hidden="true" className="kw-sc-icon text-warn" />
          ) : (
            <Shield size={16} strokeWidth={2} aria-hidden="true" className="kw-sc-icon text-zinc-500" />
          )}
          <span className="kw-sc-label">Approvals</span>
          <button type="button" className="kw-sc-value" aria-label={appr.label} title={phone ? undefined : appr.title ?? undefined} onClick={onOpenSettings} data-approvals-value={appr.value}>
            <span className="kw-sc-chip" data-tone={appr.tone} aria-hidden="true">
              {appr.chip}
            </span>
            <ChevronRight size={14} aria-hidden="true" className="text-zinc-500" />
          </button>
          {appr.sub && (
            <p className="kw-sc-sub" data-tone={appr.value !== 'off' ? 'warn' : undefined} data-approvals-sub>
              <span className="truncate">{appr.sub}</span>
            </p>
          )}
        </div>
      )}
      {open && (
        <DevExplainer
          id={dlg}
          space={space.name}
          on={on}
          disabled={look.disabled}
          phone={phone}
          anchor={help}
          onClose={close}
          onTurnOn={() => {
            close(false);
            const b = switchBtn.current;
            b?.focus();
            b?.click();
          }}
          onOpenDesk={
            onOpenDevDesk &&
            (() => {
              close(false);
              onOpenDevDesk();
            })
          }
        />
      )}
    </div>
  );
};

/** The Dev mode row's second line (spec §2): an LED, the text, and at most one action on the right. */
const DevSubLine: React.FC<{ id: string; sub: DevSub; phone: boolean; onAction?: () => void }> = ({ id, sub, phone, onAction }) => (
  <div className="kw-sc-sub" data-dev-sub={sub.state} data-tone={sub.tone ?? undefined}>
    {sub.led && <span className="dv-led kw-sc-led" data-s={sub.led} aria-hidden="true" />}
    <span id={id} className="kw-sc-sub-text" data-dev-sub-text>
      {sub.parts.map((p, i) =>
        p.repo ? (
          <span key={i} className="kw-sc-repo" title={p.text}>
            {p.text}
          </span>
        ) : p.branch ? (
          <span key={i} className="kw-sc-branch">
            {p.text}
          </span>
        ) : (
          <span key={i} className="shrink-0 whitespace-pre">
            {p.text}
          </span>
        ),
      )}
    </span>
    {sub.state === 'armed' && !phone && (
      <kbd className="dv-kbd ml-auto" aria-hidden="true">
        Esc
      </kbd>
    )}
    {sub.action && onAction && (
      <button type="button" className="kw-sc-act" aria-label={SUB_ACTION_TEXT[sub.action].label} onClick={onAction} data-dev-sub-action={sub.action}>
        {SUB_ACTION_TEXT[sub.action].text}
      </button>
    )}
  </div>
);

interface ExplainerProps {
  id: string;
  space: string;
  on: boolean;
  disabled: boolean;
  phone: boolean;
  anchor: React.RefObject<HTMLButtonElement>;
  onClose: (refocus: boolean) => void;
  onTurnOn: () => void;
  onOpenDesk?: () => void;
}

const focusables = (root: HTMLElement | null): HTMLElement[] =>
  root ? [...root.querySelectorAll<HTMLElement>('a[href], button:not([disabled]), [tabindex]:not([tabindex="-1"])')] : [];

/**
 * What Dev mode is (spec §4): a non-modal popover beside the sidebar on desktop, a modal bottom sheet on
 * phones. Esc, the X or a click outside closes it and focus goes back to the "?".
 */
/** The phone sheet's slide-down on close (answer 4): mirrors the 200 ms open, none under reduced motion. */
export const SHEET_CLOSE_MS = 150;
const reducedMotion = () => typeof window !== 'undefined' && !!window.matchMedia?.('(prefers-reduced-motion: reduce)').matches;

export const DevExplainer: React.FC<ExplainerProps> = ({ id, space, on, disabled, phone, anchor, onClose: close, onTurnOn, onOpenDesk }) => {
  const card = useRef<HTMLDivElement>(null);
  // Phone: closing slides the sheet down first, then it unmounts. Desktop closes at once.
  const [closing, setClosing] = useState(false);
  const closingRef = useRef(false);
  const closeRef = useRef(close);
  closeRef.current = close;
  const onClose = useCallback(
    (refocus: boolean) => {
      if (closingRef.current) return;
      if (!phone || reducedMotion()) {
        closingRef.current = true;
        closeRef.current(refocus);
        return;
      }
      closingRef.current = true;
      setClosing(true);
      window.setTimeout(() => closeRef.current(refocus), SHEET_CLOSE_MS);
    },
    [phone],
  );
  // Phone: the back gesture closes the sheet. Opening adds a history entry; closing any other way takes it off.
  useEffect(() => (phone ? backGestureEntry(() => onClose(true)) : undefined), [phone, onClose]);
  const primary = useRef<HTMLButtonElement>(null);
  const [pos, setPos] = useState<{ left: number; top: number; arrow: number; width: number } | null>(null);

  // Desktop: 10 px right of the sidebar, the arrow at the "?", clamped to the viewport with 12 px margins.
  useLayoutEffect(() => {
    if (phone) return undefined;
    const place = () => {
      const q = anchor.current?.getBoundingClientRect();
      if (!q) return;
      const side = anchor.current?.closest('[data-kw-sidebar]')?.getBoundingClientRect();
      const left = Math.round((side?.right ?? q.right) + 10);
      const width = Math.max(240, Math.min(368, window.innerWidth - left - 12));
      const h = card.current?.offsetHeight ?? 0;
      const mid = q.top + q.height / 2;
      const top = Math.round(Math.max(12, Math.min(mid - 24, window.innerHeight - h - 12)));
      setPos({ left, top, arrow: Math.round(h > 28 ? Math.max(14, Math.min(mid - top, h - 14)) : mid - top), width });
    };
    place();
    window.addEventListener('resize', place);
    return () => window.removeEventListener('resize', place);
  }, [phone, anchor]);

  // Focus the primary button when it opens (or the first control when there is none). The desktop popover
  // is hidden until it is placed, and a hidden button can't take focus, so wait for its position.
  const focused = useRef(false);
  const ready = phone || pos !== null;
  useEffect(() => {
    if (!ready || focused.current) return;
    focused.current = true;
    (primary.current ?? focusables(card.current)[0])?.focus();
  }, [ready]);

  // Outside click closes the popover; focus returns to "?" only if it was inside the card.
  useEffect(() => {
    if (phone) return undefined;
    const down = (e: PointerEvent) => {
      const t = e.target as Node | null;
      if (!t || card.current?.contains(t) || anchor.current?.contains(t)) return;
      onClose(!!card.current?.contains(document.activeElement));
    };
    document.addEventListener('pointerdown', down);
    return () => document.removeEventListener('pointerdown', down);
  }, [phone, anchor, onClose]);

  // Phone: the sheet is modal, so everything behind it is inert while it is open.
  useEffect(() => {
    if (!phone) return undefined;
    const host = card.current?.closest('[data-dvx-portal]');
    const others = [...document.body.children].filter((el) => el !== host && !el.hasAttribute('inert'));
    others.forEach((el) => el.setAttribute('inert', ''));
    return () => others.forEach((el) => el.removeAttribute('inert'));
  }, [phone]);

  const onKeyDown = (e: React.KeyboardEvent) => {
    if (e.key === 'Escape') {
      e.stopPropagation();
      onClose(true);
      return;
    }
    if (e.key !== 'Tab') return;
    const items = focusables(card.current);
    if (!items.length) return;
    const first = items[0];
    const last = items[items.length - 1];
    if (e.shiftKey && document.activeElement === first) {
      e.preventDefault();
      if (phone) last.focus();
      else onClose(true); // not a trap on desktop: Shift+Tab out of the first item goes back to "?"
    } else if (!e.shiftKey && document.activeElement === last) {
      e.preventDefault();
      first.focus();
    }
  };

  const swipe = useRef<number | null>(null);
  const titleId = 'dvx-title';
  const body = (
    <>
      <p className="kw-dvx-kicker">{on ? EXPLAINER.kickerOn(space) : EXPLAINER.kickerOff}</p>
      <h2 id={titleId} className="kw-dvx-title">
        {EXPLAINER.title}
      </h2>
      <p className="kw-dvx-lead">{on ? EXPLAINER.leadOn : EXPLAINER.leadOff}</p>
      <ol className="kw-dvx-list">
        {DEV_FEATURES.map((f) => (
          <li key={f.key} data-dev-feature={f.key}>
            <span className="kw-dvx-n" aria-hidden="true">
              {f.n}
            </span>
            <span>
              <strong>{f.title}</strong> {f.line}
            </span>
          </li>
        ))}
      </ol>
      <p className="kw-dvx-trust">
        <Lock size={14} aria-hidden="true" className="shrink-0 text-ok mt-0.5" />
        <span>
          {EXPLAINER.trustLead}
          <strong>{EXPLAINER.trustPost}</strong>.
        </span>
      </p>
      <div className="kw-dvx-actions">
        {on ? (
          onOpenDesk && (
            <button ref={primary} type="button" className="kw-dvx-primary" onClick={onOpenDesk} data-dvx-primary="desk">
              {EXPLAINER.openDesk}
            </button>
          )
        ) : disabled ? (
          <p className="kw-dvx-owner" data-dvx-not-owner>
            {EXPLAINER.notOwner}
          </p>
        ) : (
          <button ref={primary} type="button" className="kw-dvx-primary" onClick={onTurnOn} data-dvx-primary="on">
            {EXPLAINER.turnOn}
          </button>
        )}
        <a className="kw-dvx-guide" href={DEV_GUIDE_URL} target="_blank" rel="noopener noreferrer" data-dvx-guide>
          {EXPLAINER.guide}
          <ArrowUpRight size={12} aria-hidden="true" />
          <span className="sr-only"> (opens in a new tab)</span>
        </a>
      </div>
      {on && <p className="kw-dvx-note">{EXPLAINER.noteOn}</p>}
      <button type="button" className="kw-dvx-close" aria-label="Close" onClick={() => onClose(true)} data-dvx-close>
        <X size={14} aria-hidden="true" />
      </button>
    </>
  );

  const content = phone ? (
    <div className="kw-dvx-layer" data-dvx-portal>
      <div className="kw-dvx-scrim" aria-hidden="true" onClick={() => onClose(true)} data-dvx-scrim data-closing={closing || undefined} />
      <div
        ref={card}
        id={id}
        role="dialog"
        aria-modal="true"
        aria-labelledby={titleId}
        className="kw-dvx kw-dvx-sheet"
        data-closing={closing || undefined}
        onKeyDown={onKeyDown}
        onTouchStart={(e) => (swipe.current = e.touches[0]?.clientY ?? null)}
        onTouchEnd={(e) => {
          const y0 = swipe.current;
          swipe.current = null;
          const y1 = e.changedTouches[0]?.clientY;
          if (y0 !== null && y1 !== undefined && y1 - y0 > 64 && (card.current?.scrollTop ?? 0) <= 0) onClose(true);
        }}
        data-dev-explainer="sheet"
      >
        <span className="kw-dvx-grab" aria-hidden="true" />
        {body}
      </div>
    </div>
  ) : (
    <div data-dvx-portal>
      <div
        ref={card}
        id={id}
        role="dialog"
        aria-labelledby={titleId}
        className="kw-dvx kw-dvx-pop"
        style={pos ? { left: pos.left, top: pos.top, width: pos.width } : { visibility: 'hidden' }}
        onKeyDown={onKeyDown}
        data-dev-explainer="popover"
      >
        <span className="kw-dvx-arrow" aria-hidden="true" style={pos ? { top: pos.arrow - 5.5 } : undefined} />
        {body}
      </div>
    </div>
  );
  return createPortal(content, document.body);
};
