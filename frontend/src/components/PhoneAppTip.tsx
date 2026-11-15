import React, { useState } from 'react';
import { Smartphone, X } from 'lucide-react';
import { isAndroid, isIos } from '../pwa';
import { dismissPhoneTip, phoneTipDismissed } from '../nav';

// The desktop sidebar's way to "Use kwant bots on your phone" (PhoneAppCard, Settings › All spaces).
// Apart from the card so the sidebar doesn't load the card's sign-in pieces.

export const PHONE_APP_ANCHOR = 'phone-app';
export const PHONE_APP_TITLE = 'phone-app-title';

/** Scrolls Settings › All spaces to the card and focuses its title. Settings may still be mounting,
 *  so it tries again for a moment. */
export function focusPhoneApp(doc: Document = document, tries = 40): void {
  const el = doc.getElementById(PHONE_APP_ANCHOR);
  if (!el) {
    if (tries > 0) setTimeout(() => focusPhoneApp(doc, tries - 1), 50);
    return;
  }
  const reduce = doc.defaultView?.matchMedia?.('(prefers-reduced-motion: reduce)').matches;
  el.scrollIntoView?.({ block: 'start', behavior: reduce ? 'auto' : 'smooth' });
  doc.getElementById(PHONE_APP_TITLE)?.focus({ preventScroll: true });
}

/** The desktop sidebar's one-time tip. Clicking it opens the card and counts as seen. */
export const PhoneAppTip: React.FC<{ className: string; onOpen: () => void }> = ({ className, onOpen }) => {
  const [dismissed, setDismissed] = useState(() => phoneTipDismissed());
  if (dismissed) return null;
  if (typeof navigator !== 'undefined' && (isIos(navigator) || isAndroid(navigator))) return null;
  const hide = () => {
    dismissPhoneTip();
    setDismissed(true);
  };
  return (
    <div className="hidden md:block rounded-lg border border-zinc-800 bg-zinc-900/60" data-phone-tip>
      <div className="flex items-center">
        <button
          type="button"
          onClick={() => {
            hide();
            onOpen();
          }}
          className={className}
        >
          <Smartphone size={15} className="text-sky-400" aria-hidden="true" />
          <span className="truncate flex-1">Tip: use kwant bots on your phone</span>
        </button>
        <button
          type="button"
          onClick={hide}
          aria-label="Dismiss the phone tip"
          title="Dismiss. Find it any time in Settings › All spaces."
          className="shrink-0 mr-1 p-1 rounded text-zinc-500 hover:text-zinc-200 hover:bg-zinc-800 focus-visible:outline focus-visible:outline-2 focus-visible:outline-sky-500"
        >
          <X size={12} aria-hidden="true" />
        </button>
      </div>
    </div>
  );
};
