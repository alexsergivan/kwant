import React, { useEffect, useLayoutEffect, useRef, useState } from 'react';
import { createPortal } from 'react-dom';
import { useEscapeLayer } from '../escape';
import { usePhoneLayout } from '../usePhoneLayout';
import { devApi, devStore } from '../devMode';
import { forgeChoiceName, forgeConnectedText, forgeHost, type LooseSource } from '../looseEnds';
import type { OtherForge } from '../devMode';
import { GitHubConnectForm } from './LooseEndsView';

/**
 * One GitHub connect form, opened in place (0.7.3 B1). Every Connect GitHub (the Dev desk's status
 * line and empty state, the Dev mode toast, Dev mode setup, Team settings' repo line) calls
 * `openGitHubConnect`; the host mounted once in App shows the form as a popover next to the button on
 * the desktop and as a sheet on the phone. Nothing navigates to Loose ends any more.
 */
export const GITHUB_CONNECT_EVENT = 'kwnt:github-connect';
/** Sent after a GitHub login is added from the in-place form; `detail.space` is the space it was opened for. */
export const GITHUB_CONNECTED_EVENT = 'kwnt:github-connected';

export interface GitHubConnectRequest {
  /** The space whose Dev mode reads the login again once it's added. */
  space?: string;
  /** The button it opens next to (and focus goes back to). Defaults to the focused element. */
  anchor?: HTMLElement | null;
  /** 0.7.4 (G4): a repo row's "Connect GitLab" / "Connect Gitea": the forge form, prefilled with `https://<host>`. */
  forge?: { forge: OtherForge; url: string } | null;
}

export function openGitHubConnect(req: GitHubConnectRequest = {}): void {
  if (typeof window === 'undefined') return;
  const anchor = req.anchor ?? (document.activeElement instanceof HTMLElement && document.activeElement !== document.body ? document.activeElement : null);
  window.dispatchEvent(new window.CustomEvent<GitHubConnectRequest>(GITHUB_CONNECT_EVENT, { detail: { space: req.space, anchor, forge: req.forge ?? null } }));
}

/** The same form in place for a GitLab or Gitea host (0.7.4): popover on the desktop, sheet on the phone. */
export const openForgeConnect = (req: GitHubConnectRequest & { forge: { forge: OtherForge; url: string } }): void => openGitHubConnect(req);

/** The layer's name: "Connect GitHub", "Connect GitLab" or "Connect Gitea". */
export const connectTitle = (req: Pick<GitHubConnectRequest, 'forge'>): string => (req.forge ? `Connect ${forgeChoiceName(req.forge.forge) ?? 'server'}` : 'Connect GitHub');
/** The toast after the in-place form saved. */
export const connectedToast = (source: LooseSource, test?: unknown): { title: string; body: string } =>
  source.kind === 'forge'
    ? { title: `${forgeChoiceName(source.config?.forge) ?? 'Server'} connected`, body: `${forgeConnectedText(test, forgeHost(String(source.config?.url ?? '')))} Dev mode reads its reviews and CI now.` }
    : { title: 'GitHub connected', body: 'Loose ends and Dev mode both use it.' };

const WIDTH = 380;
const GAP = 8;
const EDGE = 12;

/** Where the popover goes: under the anchor (or above it when there's no room), kept inside the window. */
export function popoverPlace(anchor: { top: number; bottom: number; left: number; right: number } | null, vw: number, vh: number, height: number, want = WIDTH) {
  const width = Math.min(want, vw - EDGE * 2);
  if (!anchor) return { left: Math.round((vw - width) / 2), top: Math.max(EDGE, Math.round((vh - height) / 3)), width };
  const left = Math.round(Math.min(Math.max(EDGE, anchor.left), vw - width - EDGE));
  const below = anchor.bottom + GAP;
  const above = anchor.top - GAP - height;
  const top = below + height <= vh - EDGE ? below : above >= EDGE ? above : Math.max(EDGE, vh - EDGE - height);
  return { left, top: Math.round(top), width };
}

export const GitHubConnectHost: React.FC<{ onConnected?: (space: string | undefined, source: LooseSource, test?: unknown) => void }> = ({ onConnected }) => {
  const [req, setReq] = useState<(GitHubConnectRequest & { n: number }) | null>(null);
  useEffect(() => {
    const on = (e: Event) => {
      const detail = (e as CustomEvent<GitHubConnectRequest>).detail ?? {};
      setReq((r) => ({ ...detail, n: (r?.n ?? 0) + 1 }));
    };
    window.addEventListener(GITHUB_CONNECT_EVENT, on);
    return () => window.removeEventListener(GITHUB_CONNECT_EVENT, on);
  }, []);
  if (!req) return null;
  return (
    <GitHubConnectLayer
      key={req.n}
      req={req}
      onClose={() => setReq(null)}
      onAdded={(source, test) => {
        setReq(null);
        if (req.space) devApi.get(req.space).then((st) => devStore.set(req.space!, st), () => undefined);
        window.dispatchEvent(new window.CustomEvent(GITHUB_CONNECTED_EVENT, { detail: { space: req.space, kind: source.kind } }));
        onConnected?.(req.space, source, test);
      }}
    />
  );
};

const GitHubConnectLayer: React.FC<{ req: GitHubConnectRequest; onClose: () => void; onAdded: (s: LooseSource, test?: unknown) => void }> = ({ req, onClose, onAdded }) => {
  const phone = usePhoneLayout();
  const ref = useRef<HTMLDivElement>(null);
  const [place, setPlace] = useState<{ left: number; top: number; width: number } | null>(null);
  const close = () => {
    onClose();
    // Back to the button it opened from, when it's still on screen.
    const a = req.anchor;
    if (a?.isConnected) window.setTimeout(() => a.focus({ preventScroll: true }), 0);
  };
  useEscapeLayer(ref, true, close, { modal: true, fallback: () => (req.anchor?.isConnected ? req.anchor : null) });

  // The desktop popover follows its button and its own height (the card, then the token field).
  useLayoutEffect(() => {
    if (phone) return undefined;
    const measure = () => {
      const r = req.anchor?.isConnected ? req.anchor.getBoundingClientRect() : null;
      setPlace(popoverPlace(r, window.innerWidth, window.innerHeight, ref.current?.offsetHeight ?? 320));
    };
    measure();
    const ro = typeof ResizeObserver !== 'undefined' ? new ResizeObserver(measure) : null;
    if (ref.current) ro?.observe(ref.current);
    window.addEventListener('resize', measure);
    window.addEventListener('scroll', measure, true);
    return () => {
      ro?.disconnect();
      window.removeEventListener('resize', measure);
      window.removeEventListener('scroll', measure, true);
    };
  }, [phone, req.anchor]);

  // A click outside closes the popover (the phone sheet has its own backdrop).
  useEffect(() => {
    if (phone) return undefined;
    const down = (e: MouseEvent) => {
      const t = e.target as Node | null;
      if (!t || ref.current?.contains(t) || req.anchor?.contains(t)) return;
      // Bitwarden's login form and other layers opened from inside sit in the popover; anything else is outside.
      close();
    };
    document.addEventListener('mousedown', down);
    return () => document.removeEventListener('mousedown', down);
  });

  const title = connectTitle(req);
  const form = req.forge ? (
    <GitHubConnectForm onAdded={onAdded} onCancel={close} land scroll={false} heading={title} forge={{ url: req.forge.url, choice: req.forge.forge }} />
  ) : (
    <GitHubConnectForm onAdded={onAdded} onCancel={close} land scroll={false} />
  );

  if (phone)
    return createPortal(
      <div className="fixed inset-0 z-[70] flex flex-col justify-end" data-github-connect-sheet>
        <button type="button" aria-label="Close" tabIndex={-1} onClick={close} className="absolute inset-0 bg-black/50 cursor-default" />
        <div ref={ref} tabIndex={-1} role="dialog" aria-modal="true" aria-label={title} className="kw-sheet outline-none relative max-h-[88vh] overflow-y-auto rounded-t-2xl bg-zinc-900 border-t border-zinc-800 p-2 pb-[calc(0.5rem+env(safe-area-inset-bottom))] shadow-2xl">
          <div className="flex justify-center pt-1 pb-2" aria-hidden="true">
            <span className="w-9 h-1 rounded-sm bg-zinc-700" />
          </div>
          {form}
        </div>
      </div>,
      document.body,
    );

  return createPortal(
    <div
      ref={ref}
      tabIndex={-1}
      role="dialog"
      aria-modal="false"
      aria-label={title}
      data-github-connect-popover
      style={place ? { left: place.left, top: place.top, width: place.width } : { visibility: 'hidden', left: 0, top: 0, width: WIDTH }}
      className="fixed z-[70] outline-none max-h-[calc(100vh-24px)] overflow-y-auto rounded-xl bg-zinc-900 shadow-2xl shadow-black/60 ring-1 ring-zinc-700"
    >
      {form}
    </div>,
    document.body,
  );
};
