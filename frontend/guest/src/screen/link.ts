// The big screen's link (pa5b): /p/<code>#screen=<token>. Part of the phone entry (tiny, no screen code), because it
// must run before anything else: the token is read and wiped from the address bar before the first request, and no
// history entry keeps it. A session flag keeps a reload of this tab a screen; it then reads with the cookie that the
// sign-in set. Returns the token, '' for such a reload, or undefined for a phone.
export interface LinkWindow {
  location: { hash: string; pathname: string; search: string };
  history: { replaceState: (data: unknown, unused: string, url?: string) => void };
  sessionStorage?: { getItem: (k: string) => string | null; setItem: (k: string, v: string) => void } | null;
}

export function screenToken(w: LinkWindow, code: string): string | undefined {
  const m = /(?:^#|&)screen=([^&]*)/.exec(w.location.hash);
  let flag: LinkWindow['sessionStorage'] = null;
  try {
    flag = w.sessionStorage;
  } catch {
    flag = null;
  }
  const key = `kw-screen-${code}`;
  if (m) {
    w.history.replaceState(null, '', w.location.pathname + w.location.search);
    try {
      flag?.setItem(key, '1');
    } catch {
      // Private mode: a reload falls back to the phone page, which shows nothing private.
    }
    let token = m[1];
    try {
      token = decodeURIComponent(token);
    } catch {
      // keep it as written
    }
    return token;
  }
  try {
    return flag?.getItem(key) ? '' : undefined;
  } catch {
    return undefined;
  }
}
