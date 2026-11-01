/**
 * Sign-ins that leave kwant. In a browser they open in a popup or, when it is blocked, in this
 * tab, and kwnt's callback page brings the tab back. The desktop app's window only ever shows kwant
 * (apps/mac/src/links.rs): every outside page opens in the default browser, with no opener and
 * no way to come back by itself, so the app watches the server for the finished sign-in instead.
 */

/** sessionStorage mark for kwnt's callback page: this tab left kwant to sign in, bring it back.
 *  Same key as RETURN_KEY in src/server/mcp_oauth.rs. */
export const OAUTH_RETURN_KEY = 'kwnt-mcp-oauth-return';

type AppWindow = { __KWANT_APP__?: unknown };

/** The marks the desktop app sets on its window (apps/mac/src/main.rs APP_MARK), one per OS. */
export const DESKTOP_APPS: readonly string[] = ['mac', 'linux'];

/** True inside the kwant desktop app, which marks its window with `__KWANT_APP__ = 'mac'` or `'linux'`. */
export const inDesktopApp = (w: unknown = typeof window !== 'undefined' ? window : undefined): boolean => {
  const mark = w ? (w as AppWindow).__KWANT_APP__ : undefined;
  return typeof mark === 'string' && DESKTOP_APPS.includes(mark);
};

/** The OS the desktop app names itself after in Settings → About: 'Linux' for the Linux app, else 'Mac'. */
export const desktopAppOs = (w: unknown = typeof window !== 'undefined' ? window : undefined): 'Mac' | 'Linux' =>
  (w ? (w as AppWindow).__KWANT_APP__ : undefined) === 'linux' ? 'Linux' : 'Mac';

/**
 * Goes to a sign-in page from this tab. In a browser the tab leaves kwant and the callback page
 * returns it; in the desktop app the navigation is turned into the default browser and the window
 * stays where it is.
 */
export function leaveForSignIn(url: string, w: Window = window): void {
  if (!inDesktopApp(w)) {
    try {
      w.sessionStorage.setItem(OAUTH_RETURN_KEY, '1');
    } catch {
      // storage off: the callback page then says to go back instead of going back itself
    }
  }
  w.location.assign(url);
}

/** What a server's sign-in looked like before Connect, to tell a new sign-in from an old one. */
export type SignInMark = { connected: boolean; expires_at: string | null } | undefined;

/** A sign-in finished since `before`: newly connected, or connected again with a new expiry. */
export const signInChanged = (before: SignInMark, now: SignInMark): boolean =>
  !!now?.connected && (!before?.connected || before.expires_at !== now.expires_at);

/**
 * Calls `check` every `everyMs` until it says done (then `onDone`) or `forMs` passes (then
 * `onTimeout`). A failed check just waits for the next one. Returns a stop function.
 */
export function pollUntil(
  check: () => Promise<boolean>,
  onDone: () => void,
  { everyMs = 2000, forMs = 10 * 60_000, onTimeout }: { everyMs?: number; forMs?: number; onTimeout?: () => void } = {},
): () => void {
  let stopped = false;
  let timer: ReturnType<typeof setTimeout> | null = null;
  const until = Date.now() + forMs;
  const tick = async () => {
    if (stopped) return;
    let done = false;
    try {
      done = await check();
    } catch {
      done = false;
    }
    if (stopped) return;
    if (done) {
      stopped = true;
      onDone();
    } else if (Date.now() >= until) {
      stopped = true;
      onTimeout?.();
    } else {
      timer = setTimeout(tick, everyMs);
    }
  };
  timer = setTimeout(tick, everyMs);
  return () => {
    stopped = true;
    if (timer !== null) clearTimeout(timer);
  };
}

/** The sign-in state of one MCP server, from GET /api/mcp/servers. */
export async function mcpSignIn(serverId: string, fetcher: typeof fetch = fetch): Promise<SignInMark> {
  const res = await fetcher('/api/mcp/servers');
  if (!res.ok) throw new Error(`HTTP ${res.status}`);
  const list = (await res.json()) as Array<{ id: string; oauth?: { connected: boolean; expires_at: string | null } }>;
  return list.find((s) => s.id === serverId)?.oauth;
}
