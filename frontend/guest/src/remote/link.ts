// The host remote's link (pa4b): /p/<code>#remote=<token>, minted on the card ("Open the remote"). Part of the phone
// entry (tiny, no remote code), because it must run before anything else: the token is read and wiped from the
// address bar before the first request, and no history entry keeps it. A session flag keeps a reload of this tab the
// remote; it then reads with the kph_<code> cookie the sign-in set. Returns the token, '' for such a reload, or
// undefined for a guest's phone. Same shape as the big screen's screenToken (screen/link.ts).
import type { LinkWindow } from '../screen/link';

export function remoteToken(w: LinkWindow, code: string): string | undefined {
  const m = /(?:^#|&)remote=([^&]*)/.exec(w.location.hash);
  let flag: LinkWindow['sessionStorage'] = null;
  try {
    flag = w.sessionStorage;
  } catch {
    flag = null;
  }
  const key = `kw-remote-${code}`;
  if (m) {
    w.history.replaceState(null, '', w.location.pathname + w.location.search);
    try {
      flag?.setItem(key, '1');
    } catch {
      // Private mode: a reload falls back to the guest page, which shows nothing private.
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
