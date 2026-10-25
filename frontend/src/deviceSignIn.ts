// Signing a new browser in from a signed-in device: the new browser shows a short code and waits;
// a signed-in device sees "New device wants access" and approves or denies it. Also the list of
// signed-in browsers in Settings, with Sign out for each.

/** What `POST /api/auth/device` started: the secret stays in this browser, the user code is shown. */
export interface DeviceRequest {
  device_code: string;
  user_code: string;
  expires_in: number;
  interval: number;
}

export type DeviceStart = { kind: 'started'; request: DeviceRequest } | { kind: 'too_many'; retryAfter: number };

export type DevicePoll = 'pending' | 'approved' | 'denied' | 'expired';

/** A device waiting to be let in, as a signed-in device sees it. */
export interface PendingDevice {
  user_code: string;
  browser: string;
  ip: string;
  created_at: string;
  expires_at: string;
}

/** A signed-in browser. */
export interface SignedInSession {
  id: string;
  browser: string;
  ip: string;
  via: 'token' | 'link' | 'device';
  created_at: string;
  last_seen: string;
  current: boolean;
}

const json = { 'Content-Type': 'application/json' };

/** Asks to be let in. */
export async function startDeviceRequest(send: typeof fetch = (...a) => fetch(...a)): Promise<DeviceStart> {
  const res = await send('/api/auth/device', { method: 'POST', headers: json, body: '{}' });
  if (res.status === 429) {
    const body = await res.json().catch(() => null);
    const retryAfter = Number(body?.retry_after ?? res.headers.get('Retry-After')) || 60;
    return { kind: 'too_many', retryAfter };
  }
  if (!res.ok) throw new Error(`The server answered ${res.status}.`);
  return { kind: 'started', request: await res.json() };
}

/** Whether the request was decided. "approved" comes with the session cookie, once. */
export async function pollDeviceRequest(deviceCode: string, send: typeof fetch = (...a) => fetch(...a)): Promise<DevicePoll> {
  const res = await send('/api/auth/device/poll', { method: 'POST', headers: json, body: JSON.stringify({ device_code: deviceCode }) });
  if (res.status === 403) return 'denied';
  if (res.status === 410) return 'expired';
  if (!res.ok) throw new Error(`The server answered ${res.status}.`);
  const body = await res.json().catch(() => null);
  return body?.status === 'approved' ? 'approved' : 'pending';
}

/** Devices waiting to be let in, or null when this browser may not see them (or the server is older). */
export async function listDeviceRequests(send: typeof fetch = (...a) => fetch(...a)): Promise<PendingDevice[] | null> {
  // Quiet: a background check never opens the sign-in screen by itself
  const res = await send('/api/signin/requests', { cache: 'no-store', kwntQuiet: true } as RequestInit);
  if (!res.ok) return null;
  const body = await res.json().catch(() => null);
  return Array.isArray(body) ? body : null;
}

/** Approves or denies a waiting device. "gone" when it expired or was decided elsewhere. */
export async function decideDeviceRequest(userCode: string, approve: boolean, send: typeof fetch = (...a) => fetch(...a)): Promise<'ok' | 'gone'> {
  const res = await send(`/api/signin/requests/${approve ? 'approve' : 'deny'}`, {
    method: 'POST',
    headers: json,
    body: JSON.stringify({ user_code: userCode }),
  });
  if (res.status === 404) return 'gone';
  if (!res.ok) throw new Error(`The server answered ${res.status}.`);
  return 'ok';
}

export async function listSessions(send: typeof fetch = (...a) => fetch(...a)): Promise<SignedInSession[]> {
  const res = await send('/api/signin/sessions', { cache: 'no-store' });
  if (!res.ok) throw new Error(`The server answered ${res.status}.`);
  return res.json();
}

/** Signs that browser out. A session that is already gone counts as done. */
export async function revokeSession(id: string, send: typeof fetch = (...a) => fetch(...a)): Promise<void> {
  const res = await send(`/api/signin/sessions/${encodeURIComponent(id)}`, { method: 'DELETE' });
  if (!res.ok && res.status !== 404) throw new Error(`The server answered ${res.status}.`);
}

/** How a session was made, in words. */
export function viaLabel(via: SignedInSession['via']): string {
  if (via === 'link') return 'Signed in with a link';
  if (via === 'device') return 'Approved from another device';
  return 'Signed in with the token';
}

/** "just now", "5 min ago", "3 h ago", "2 days ago". */
export function ago(iso: string, now: number = Date.now()): string {
  const t = Date.parse(iso);
  if (Number.isNaN(t)) return '';
  const s = Math.max(0, Math.round((now - t) / 1000));
  if (s < 60) return 'just now';
  const min = Math.floor(s / 60);
  if (min < 60) return `${min} min ago`;
  const h = Math.floor(min / 60);
  if (h < 24) return `${h} h ago`;
  const d = Math.floor(h / 24);
  return d === 1 ? '1 day ago' : `${d} days ago`;
}

/** "9:41" left until `expiresAt` (ms), never negative. */
export function countdown(expiresAt: number, now: number = Date.now()): string {
  const s = Math.max(0, Math.ceil((expiresAt - now) / 1000));
  return `${Math.floor(s / 60)}:${String(s % 60).padStart(2, '0')}`;
}
