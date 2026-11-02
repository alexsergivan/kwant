// Settings covers the space's sidebar and chat but not the space rail. While it's open, everything
// under it is inert, so Tab, clicks and typing can't reach the hidden composer; when it closes,
// focus goes back to what opened it, or to the composer when that's gone.
import { RefObject, useLayoutEffect, useRef } from 'react';

/** Outside the covered area, but still a way into Settings that focus can go back to. */
export const SETTINGS_OPENER = 'data-settings-opener';

/** Where focus lands after Settings closes when what opened it is gone (a link, a space switch). */
export function focusAfterSettings(opener: HTMLElement | null, content: HTMLElement | null): HTMLElement | null {
  if (opener?.isConnected && !opener.closest('[inert]')) return opener;
  return content?.querySelector<HTMLElement>('[data-composer]') ?? content?.querySelector<HTMLElement>('[data-space-settings-open]') ?? null;
}

/**
 * Makes `content` inert while Settings is open, and puts focus back when it closes. Focus is only
 * moved if it was lost with Settings (Close, Back to chats, Escape); focus elsewhere, such as on
 * the space rail, stays where it is.
 */
export function useSettingsCover(open: boolean, content: RefObject<HTMLElement>) {
  const opener = useRef<HTMLElement | null>(null);
  const wasOpen = useRef(false);
  // A layout effect: it runs before Settings moves focus in, so what opened it is still focused.
  useLayoutEffect(() => {
    const el = content.current;
    if (open && !wasOpen.current) {
      const a = document.activeElement as HTMLElement | null;
      opener.current = a && a !== document.body && (el?.contains(a) || a.hasAttribute(SETTINGS_OPENER)) ? a : null;
    }
    el?.toggleAttribute('inert', open);
    if (!open && wasOpen.current) {
      const lost = !document.activeElement || document.activeElement === document.body;
      if (lost) focusAfterSettings(opener.current, el)?.focus();
      opener.current = null;
    }
    wasOpen.current = open;
  }, [open]);
}
