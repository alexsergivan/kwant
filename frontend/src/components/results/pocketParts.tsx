// Pocket cards' small shared parts (pa4, moved here unchanged for pa10a so the Secret Santa card reuses them):
// rows, bars, callouts, button classes, the "comes later" button, the join link field and the QR tile.

import React, { useEffect, useState } from 'react';
import { Check, Copy } from 'lucide-react';
import { QrImage } from '../DeviceSignIn';
import type { PocketJoinMode } from '../../smartResults';

export const ROW = 'flex items-start gap-2 text-[15px] leading-[22px] text-zinc-200 sm:text-[13px] sm:leading-5';
export const ICON = 'mt-[3px] shrink-0 sm:mt-0.5';
export const Row: React.FC<{ icon: React.ReactNode; children: React.ReactNode; aside?: React.ReactNode; muted?: boolean; tone?: 'warn' }> = ({ icon, children, aside, muted, tone }) => (
  <li className={`${ROW} ${muted ? 'text-zinc-400' : ''}`}>
    <span className={`${ICON} ${tone === 'warn' ? 'text-warn' : muted ? 'text-zinc-500' : 'text-sky-300'}`}>{icon}</span>
    <span className="min-w-0 flex-1">{children}</span>
    {aside && <span className="shrink-0">{aside}</span>}
  </li>
);

/** `warn` is a text colour only in tailwind.config, so the fill reads the token directly. */
export const WARN_FILL = 'bg-[color:var(--kw-warn)]';
export const Bar: React.FC<{ value: number; label: string; tone?: 'accent' | 'warn' }> = ({ value, label, tone = 'accent' }) => (
  <div className="h-1.5 w-full overflow-hidden rounded-full bg-white/[0.06]" role="progressbar" aria-label={label} aria-valuemin={0} aria-valuemax={100} aria-valuenow={Math.round(value * 100)}>
    <div
      className={`h-full w-full origin-left rounded-full ${tone === 'warn' ? WARN_FILL : 'bg-sky-400'}`}
      data-tone={tone}
      style={{ transform: `scaleX(${Math.max(0, Math.min(1, value))})`, transition: 'transform 200ms var(--kw-ease)' }}
    />
  </div>
);

export const Callout: React.FC<{ tone?: 'warn'; children: React.ReactNode }> = ({ tone, children }) => (
  <div
    className={`rounded-xl border px-3.5 py-3 text-[15px] leading-[22px] sm:text-[13px] sm:leading-5 ${
      tone === 'warn' ? 'border-amber-300/35 bg-amber-300/[0.07] text-zinc-200' : 'border-hairline bg-surface-rail/60 text-zinc-300'
    }`}
  >
    {children}
  </div>
);

// Buttons: 44 px on phone, 36 px on desktop, 15 px type on phone (design.md §1 "Phone 390").
export const BTN = 'min-h-[44px] sm:min-h-[36px] whitespace-nowrap text-[15px] sm:text-meta';
// Half-width secondary buttons on phone (two columns): 14 px and tighter padding so labels stay on one line.
export const HALF = 'min-h-[44px] sm:min-h-[36px] whitespace-nowrap text-[14px] px-2.5 sm:px-3 sm:text-meta';
export const PRIMARY = `kw-btn-primary kw-hover-deepen ${BTN}`;
export const GHOST = `kw-btn-ghost ${BTN}`;
export const STOP = `inline-flex items-center justify-center gap-1.5 rounded-xl px-3 font-medium text-bad hover:bg-rose-300/10 kw-focus ${BTN}`;
export const LATER_CLS = 'aria-disabled:cursor-not-allowed aria-disabled:opacity-50 aria-disabled:hover:bg-white/[0.03] aria-disabled:hover:text-zinc-200';

/** A button whose route comes later: focusable, says why on hover and to a screen reader, does nothing. */
export const LaterButton: React.FC<{ icon: React.ReactNode; why: string; half?: boolean; className?: string; children: React.ReactNode }> = ({ icon, why, half, className = '', children }) => (
  <button type="button" aria-disabled="true" title={why} aria-description={why} data-later={why} onClick={(e) => e.preventDefault()} className={`${half ? `kw-btn-ghost ${HALF}` : GHOST} ${LATER_CLS} ${className}`}>
    {icon}
    {children}
  </button>
);

export const CopyLink: React.FC<{ url: string }> = ({ url }) => {
  const [done, setDone] = useState<'yes' | 'failed' | null>(null);
  useEffect(() => {
    if (!done) return;
    const t = setTimeout(() => setDone(null), 2000);
    return () => clearTimeout(t);
  }, [done]);
  const copy = async () => {
    try {
      await navigator.clipboard.writeText(url);
      setDone('yes');
    } catch {
      setDone('failed');
    }
  };
  return (
    <button type="button" onClick={copy} className={`${GHOST} shrink-0`} data-copy-link>
      {done === 'yes' ? <Check size={14} aria-hidden="true" /> : <Copy size={14} aria-hidden="true" />}
      <span aria-live="polite">{done === 'yes' ? 'Copied' : done === 'failed' ? "Couldn't copy" : 'Copy link'}</span>
    </button>
  );
};

export const LinkField: React.FC<{ url: string }> = ({ url }) => (
  <div className="flex min-w-0 flex-col gap-2 sm:flex-row sm:items-center">
    <input
      readOnly
      value={url}
      aria-label="Join link"
      onFocus={(e) => e.currentTarget.select()}
      className="min-h-[44px] min-w-0 flex-1 truncate rounded-lg border border-hairline-strong bg-surface-rail px-2.5 font-mono text-meta text-zinc-200 kw-focus sm:min-h-[36px]"
    />
    <CopyLink url={url} />
  </div>
);

export const NoLinkYet: React.FC<{ mode: PocketJoinMode }> = ({ mode }) => (
  <p className="text-[13px] leading-5 text-zinc-400 sm:text-meta" data-no-link>
    {mode === 'anywhere'
      ? "The from-anywhere link isn't set up on this box yet. Same Wi-Fi works now."
      : "Your box hasn't said its Wi-Fi address yet. It shows here as soon as it does."}
  </p>
);

export const Qr: React.FC<{ url: string; size: 'full' | 'compact'; name: string }> = ({ url, size, name }) => (
  <div
    className={`shrink-0 rounded-[10px] bg-white p-2 ${size === 'full' ? 'mx-auto w-[200px] sm:mx-0 sm:w-[168px]' : 'w-24 sm:w-28'}`}
    data-qr
  >
    <QrImage text={url} label={`QR code to join ${name}`} />
  </div>
);
