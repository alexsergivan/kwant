// Escape works the same in Settings and in the dialogs over it: it closes the top one, never the
// one under it, and never throws away typed text.
//
// An inline editor handles Escape first, with `editorProps`: it closes when nothing in it is
// unsaved and otherwise does nothing. Either way it calls preventDefault, which every layer
// around it respects. Editors also mark their root `data-editor`, plus `data-unsaved` while they
// hold changes, so Escape pressed elsewhere in the layer keeps them too. Text typed into any other
// field counts as unsaved while focus stays in that field.
import type React from 'react';
import { RefObject, useEffect, useRef } from 'react';
import { trapFocusTarget } from './imports';

const TEXT_TYPES = new Set(['text', 'search', 'url', 'email', 'password', 'tel', 'number']);
type TextField = HTMLInputElement | HTMLTextAreaElement;
export const isTextField = (el: EventTarget | null): el is TextField =>
  el instanceof Element && (el.tagName === 'TEXTAREA' || (el.tagName === 'INPUT' && TEXT_TYPES.has((el as HTMLInputElement).type)));

/** Props for an inline editor's root (usually its form). `close` is left out for one that stays on screen. */
export function editorProps(unsaved: boolean, close?: () => void) {
  return {
    'data-editor': '',
    'data-unsaved': unsaved ? '' : undefined,
    onKeyDown: (e: React.KeyboardEvent) => {
      if (e.key !== 'Escape' || e.defaultPrevented) return;
      // Clean and nothing of its own to close: the layer around it closes.
      if (!unsaved && !close) return;
      e.preventDefault();
      if (!unsaved) close?.();
    },
  };
}

/** The field focus is in and what it held when focus got there. */
type Typing = { el: TextField; value: string } | null;

/** Whether Escape would lose something in `root`: a marked editor, or text typed into the focused field. */
export function keepsText(root: Element, typing: Typing): boolean {
  if (root.querySelector('[data-unsaved]')) return true;
  if (!typing) return false;
  const { el, value } = typing;
  return el.isConnected && root.contains(el) && !el.closest('[data-editor]') && el.value !== value;
}

const FOCUSABLE = 'button:not([disabled]), textarea:not([disabled]), input:not([disabled]), select:not([disabled]), a[href], [tabindex]:not([tabindex="-1"])';
const FOLLOWING = 4; // Node.DOCUMENT_POSITION_FOLLOWING

/** A dialog that came after `root` in the page (opened from it, so above it), such as Tools over the bot editor. */
function dialogAbove(root: Element): boolean {
  return [...document.querySelectorAll('[role="dialog"], [role="alertdialog"]')].some(
    (d) => d !== root && !root.contains(d) && !d.contains(root) && !!(root.compareDocumentPosition(d) & FOLLOWING),
  );
}

/** Open layers, oldest first. Only the last one acts on Escape. */
const layers: object[] = [];

export interface LayerOptions {
  /** Changes held by the layer itself, such as a dialog's own form. */
  unsaved?: boolean;
  /** Something that isn't a layer is on top, checked when Escape is pressed. Dialogs default to one opened after them. */
  covered?: () => boolean;
  /** A dialog: focus moves in when it opens, Tab stays inside, and focus goes back to the opener when it closes. */
  modal?: boolean;
  /** Where focus goes when the opener is gone. */
  fallback?: () => HTMLElement | null | undefined;
}

/**
 * Escape closes this layer when it's on top and nothing in it is unsaved. The decision waits for
 * every keydown listener, so a dialog or editor inside that handles Escape (preventDefault) wins
 * whatever order the listeners were added in.
 */
export function useEscapeLayer(rootRef: RefObject<HTMLElement>, open: boolean, close: () => void, opts: LayerOptions = {}) {
  const latest = useRef({ close, opts });
  latest.current = { close, opts };
  // Read while rendering: an autoFocus inside would take focus before any effect runs.
  const opener = useRef<HTMLElement | null>(null);
  const wasOpen = useRef(false);
  if (open && !wasOpen.current && typeof document !== 'undefined') opener.current = document.activeElement as HTMLElement | null;
  wasOpen.current = open;

  useEffect(() => {
    if (!open) return;
    const me = {};
    layers.push(me);
    const root = rootRef.current;
    const modal = !!latest.current.opts.modal;
    let typing: Typing = null;
    const track = (el: Element | null) => {
      if (isTextField(el)) typing = { el, value: el.value };
    };
    const onFocus = (e: FocusEvent) => track(e.target as Element);
    root?.addEventListener('focusin', onFocus);
    if (root?.contains(document.activeElement)) track(document.activeElement);
    else if (modal) root?.focus();

    const timers = new Set<number>();
    const onKey = (e: KeyboardEvent) => {
      if (layers[layers.length - 1] !== me || !root) return;
      const { opts: o } = latest.current;
      if (e.key === 'Tab' && modal && root.contains(document.activeElement)) {
        const items = [...root.querySelectorAll<HTMLElement>(FOCUSABLE)].filter((el) => el.offsetParent !== null || el === document.activeElement);
        const next = trapFocusTarget(items.length, items.indexOf(document.activeElement as HTMLElement), e.shiftKey);
        if (next === null) return;
        e.preventDefault();
        items[next].focus();
        return;
      }
      if (e.key !== 'Escape' || (o.covered ? o.covered() : dialogAbove(root))) return;
      const t = window.setTimeout(() => {
        timers.delete(t);
        const { close: done, opts: now } = latest.current;
        if (e.defaultPrevented || !root.isConnected || now.unsaved || keepsText(root, typing)) return;
        done();
      });
      timers.add(t);
    };
    window.addEventListener('keydown', onKey);
    return () => {
      layers.splice(layers.indexOf(me), 1);
      root?.removeEventListener('focusin', onFocus);
      window.removeEventListener('keydown', onKey);
      timers.forEach((t) => window.clearTimeout(t));
      if (!modal) return;
      // Only when focus was lost with the dialog: a choice that moved it on (a link) keeps it.
      const a = document.activeElement;
      if (a && a !== document.body && a.isConnected) return;
      const o = opener.current;
      const back = o && o !== document.body && o.isConnected && !o.closest('[inert]') ? o : latest.current.opts.fallback?.();
      back?.focus();
    };
  }, [open]);
}
