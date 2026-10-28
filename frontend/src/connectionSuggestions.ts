// A bot's offer to connect an app it can't reach (message meta kind "connection_suggestion", from
// src/server/connection_suggest.rs). Connect starts the app's sign-in on Composio through the
// person-only POST /api/connections/composio/apps/:slug/connect, opens it outside kwant, and watches
// GET /api/connections/composio/apps/:slug/status until the app is connected. Nothing connects until
// the person signs in on the app's own page.

import { parseMetaObject } from './questions';
import { ConnectionsError, connectionsApi } from './connections';
import { inMacApp, pollUntil } from './externalLinks';

/** Whether the space's Composio connector could start a sign-in when the bot posted the card. */
export type ComposioReady = 'ready' | 'off' | 'needs_key' | 'not_added';

export interface ConnectionSuggestionMeta {
  kind: 'connection_suggestion';
  app: string;
  name: string;
  reason: string;
  composio: ComposioReady;
  kwant_id: string | null;
}

const READY: ComposioReady[] = ['ready', 'off', 'needs_key', 'not_added'];

export function parseConnectionMeta(raw: unknown): ConnectionSuggestionMeta | null {
  const m = parseMetaObject(raw);
  if (!m || m.kind !== 'connection_suggestion') return null;
  const app = typeof m.app === 'string' ? m.app.trim() : '';
  if (!/^[a-z0-9_-]{1,64}$/.test(app)) return null;
  const name = typeof m.name === 'string' && m.name.trim() ? m.name.trim() : app;
  return {
    kind: 'connection_suggestion',
    app,
    name,
    reason: typeof m.reason === 'string' ? m.reason.trim() : '',
    composio: READY.includes(m.composio as ComposioReady) ? (m.composio as ComposioReady) : 'ready',
    kwant_id: typeof m.kwant_id === 'string' ? m.kwant_id : null,
  };
}

/** Why Connect can't start yet, in the card's words; null when it can. */
export function composioMissingText(r: ComposioReady): string | null {
  switch (r) {
    case 'not_added':
      return 'Connecting apps goes through Composio, which isn’t added yet. Add it in Connections first (free account at composio.dev).';
    case 'needs_key':
      return 'Composio is added but has no API key yet. Add your Composio API key in Connections first.';
    case 'off':
      return 'The Composio connector is switched off. Switch it on in Connections first.';
    default:
      return null;
  }
}

/**
 * Where the card is:
 * - `ready`: Connect.
 * - `missing`: Composio can't start a sign-in; the reason and a way to Connections.
 * - `opening`: asking the server for the sign-in address.
 * - `waiting`: the sign-in is open outside kwant; the card watches for it to finish.
 * - `connected`: the server reports the app connected.
 * - `timed_out`: no finished sign-in seen in 10 minutes.
 * - `unchecked`: this server can't report whether the app is connected, so the card never claims it.
 * - `error`: the sign-in couldn't start.
 */
export type CardPhase = 'ready' | 'missing' | 'opening' | 'waiting' | 'connected' | 'timed_out' | 'unchecked' | 'error';

export interface CardState {
  phase: CardPhase;
  /** `missing` and `error`: what to tell the person. */
  message?: string;
  /** `waiting`: the sign-in address when no tab could be opened, so the person can open it themselves. */
  link?: string;
}

export const initialState = (meta: Pick<ConnectionSuggestionMeta, 'composio'>): CardState => {
  const missing = composioMissingText(meta.composio);
  return missing ? { phase: 'missing', message: missing } : { phase: 'ready' };
};

export interface ConnectApi {
  connect: (slug: string) => Promise<{ redirect_url: string }>;
  status: (slug: string) => Promise<{ connected: boolean }>;
}

export const defaultConnectApi: ConnectApi = {
  connect: connectionsApi.connectComposioApp,
  status: connectionsApi.composioAppStatus,
};

/** A 409 from either call: Composio isn't added, has no key or no user, so Connect can't work. */
const composioRefused = (e: unknown) => e instanceof ConnectionsError && e.status === 409;
/** A 404 from the status call: an older server without it. */
const noStatusRoute = (e: unknown) => e instanceof ConnectionsError && e.status === 404;

/** Whether the app is already connected; null when unknown (older server, Composio unusable, offline). */
export async function connectedNow(api: ConnectApi, slug: string): Promise<boolean | null> {
  try {
    return (await api.status(slug)).connected === true;
  } catch {
    return null;
  }
}

type OpenWin = Pick<Window, 'open'> & { location: Pick<Location, 'assign'> };

/**
 * One press of Connect (or "Open the sign-in again"): starts the sign-in, opens it outside kwant and
 * watches the server until the app is connected. Must be called inside the click, so the browser lets
 * the new tab open. Reports every change through `onState` and returns a function that stops watching.
 */
export function startConnect(
  slug: string,
  onState: (s: CardState) => void,
  {
    api = defaultConnectApi,
    win = window as unknown as OpenWin,
    mac = inMacApp(),
    everyMs,
    forMs,
  }: { api?: ConnectApi; win?: OpenWin; mac?: boolean; everyMs?: number; forMs?: number } = {},
): () => void {
  let stop: (() => void) | null = null;
  let cancelled = false;
  // Open the tab now, while the click still counts as the person's; the address comes after the fetch.
  // The Mac app opens no second window: its outside links go to the default browser.
  const tab = mac ? null : win.open('', '_blank');
  if (tab) tab.opener = null;
  onState({ phase: 'opening' });
  (async () => {
    let url: string;
    try {
      url = (await api.connect(slug)).redirect_url;
    } catch (e) {
      tab?.close();
      if (cancelled) return;
      onState(
        composioRefused(e)
          ? { phase: 'missing', message: e instanceof Error ? e.message : 'Composio can’t start this sign-in. Check it in Connections.' }
          : { phase: 'error', message: e instanceof Error ? e.message : 'The sign-in didn’t start. Try again.' },
      );
      return;
    }
    if (cancelled) {
      tab?.close();
      return;
    }
    let link: string | undefined;
    if (tab) tab.location.href = url;
    else if (mac) win.location.assign(url);
    else link = url; // blocked: the card shows the address as a link, which a click always opens
    onState({ phase: 'waiting', link });
    let outcome: CardState | null = null;
    stop = pollUntil(
      async () => {
        try {
          return (await api.status(slug)).connected === true;
        } catch (e) {
          if (noStatusRoute(e)) outcome = { phase: 'unchecked', link };
          else if (composioRefused(e)) outcome = { phase: 'missing', message: (e as Error).message };
          else return false;
          return true;
        }
      },
      () => onState(outcome ?? { phase: 'connected' }),
      { everyMs, forMs, onTimeout: () => onState({ phase: 'timed_out' }) },
    );
    if (cancelled) stop();
  })();
  return () => {
    cancelled = true;
    stop?.();
  };
}
