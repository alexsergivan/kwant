// Notifications on this device: the OS notification the open page shows, and Web Push, which
// reaches the installed app (or the browser) when kwant is closed. The server sends a push for
// each kind ticked under "Tell me when" to every subscribed device (src/server/push.rs); the
// service worker (sw.js) shows it. Page and push use the same tag, so one replaces the other.
import { BROWSER_NOTIFY_KEY, NotifyEvent, notificationPermission } from './continuity';
import { isIos, isStandalone } from './pwa';
import { spacePath } from './spaces';

/** Same as the server's push tag: `kwnt-<space|none>-<kind>-<channel|bot>`. */
export function notificationTag(n: Pick<NotifyEvent, 'kind' | 'kwant_id' | 'channel_id'> & { space_id?: string | null }): string {
  return `kwnt-${n.space_id || 'none'}-${n.kind}-${n.channel_id ?? n.kwant_id}`;
}

/** Where a tap on the notification goes: the chat's own address, or the app's start. */
export function notificationPath(n: { space_id?: string | null; channel_id?: string | null; kwant_id?: string | null }): string {
  if (!n.space_id) return '/';
  return spacePath(n.space_id, { channelId: n.channel_id, botId: n.channel_id ? null : n.kwant_id });
}

/** What a tap on a notification asks the open app to show (sw.js posts it; App and Shell act on it). */
export interface NotificationOpen {
  type: 'kwnt-notification-open';
  kind: string;
  space_id: string | null;
  channel_id: string | null;
  kwant_id: string | null;
}

export function isNotificationOpen(m: unknown): m is NotificationOpen {
  return !!m && typeof m === 'object' && (m as { type?: unknown }).type === 'kwnt-notification-open';
}

/** `?space=…&channel=…` / `&kwant=…`: the push payload's url, for a link opened outside the worker. */
export function targetFromQuery(search: string): { spaceId: string | null; channelId: string | null; botId: string | null } | null {
  const q = new URLSearchParams(search);
  const spaceId = q.get('space');
  const channelId = q.get('channel');
  const botId = channelId ? null : q.get('kwant');
  if (!spaceId && !channelId && !botId) return null;
  return { spaceId, channelId, botId };
}

// ---------------------------------------------------------------------------
// Can this device get notifications, and how?

export type DeviceNotifySupport =
  /** Push and page notifications: works with kwant closed. */
  | 'push'
  /** Only the open page's notifications (no service worker or push here). */
  | 'page'
  /** Plain http on another computer: browsers allow neither. */
  | 'insecure'
  /** iPhone or iPad in Safari: push needs the Home Screen app. */
  | 'ios-install'
  | 'unsupported';

export interface NotifyEnv {
  secure: boolean;
  ios: boolean;
  standalone: boolean;
  notification: boolean;
  serviceWorker: boolean;
  pushManager: boolean;
}

export function deviceNotifySupport(e: NotifyEnv): DeviceNotifySupport {
  if (!e.secure) return 'insecure';
  if (e.ios && !e.standalone) return 'ios-install';
  if (e.serviceWorker && e.pushManager && e.notification) return 'push';
  return e.notification ? 'page' : 'unsupported';
}

export function notifyEnv(win: Window = window): NotifyEnv {
  return {
    secure: win.isSecureContext,
    ios: isIos(win.navigator),
    standalone: isStandalone(win),
    notification: 'Notification' in win,
    serviceWorker: 'serviceWorker' in win.navigator,
    pushManager: 'PushManager' in win,
  };
}

/** The line under "Notify me on this device": what it does, or why it can't be on. */
export function deviceNotifyHint(s: {
  support: DeviceNotifySupport;
  permission: NotificationPermission | 'unsupported';
  on: boolean;
  android: boolean;
  standalone: boolean;
}): { text: string; warn: boolean } {
  switch (s.support) {
    case 'insecure':
      return { text: 'Notifications need https://. Open this site through its https address to turn this on.', warn: true };
    case 'ios-install':
      return { text: 'On iPhone and iPad, add kwant bots to your Home Screen first: tap Share, then Add to Home Screen. Then turn this on in that app.', warn: true };
    case 'unsupported':
      return { text: "This browser can't show notifications.", warn: true };
  }
  if (s.permission === 'denied') {
    const how = s.android && s.standalone
      ? 'Long-press the kwant bots icon, tap App info, then Notifications, and allow them.'
      : "Allow notifications for this site in the browser's site settings (the icon left of the address), then come back.";
    return { text: `Notifications are blocked for this site. ${how}`, warn: true };
  }
  if (s.support === 'page') return { text: 'Only while this tab is open in the background. Each notification names its space.', warn: false };
  return { text: s.on ? 'Comes through even when the app is closed. Tap one to open its chat.' : 'Get a notification when a bot needs you, even with the app closed.', warn: false };
}

export const isAndroid = (nav: Pick<Navigator, 'userAgent'>): boolean => /Android/.test(nav.userAgent);

// ---------------------------------------------------------------------------
// The subscription

/** base64url (the server's VAPID public key) to the bytes `pushManager.subscribe` wants. */
export function base64UrlToBytes(s: string): Uint8Array<ArrayBuffer> {
  const b64 = s.replace(/-/g, '+').replace(/_/g, '/').replace(/=+$/, '');
  const bin = atob(b64 + '='.repeat((4 - (b64.length % 4)) % 4));
  const out = new Uint8Array(new ArrayBuffer(bin.length));
  for (let i = 0; i < bin.length; i++) out[i] = bin.charCodeAt(i);
  return out;
}

/** Whether a subscription was made with this server key (a new key means subscribing again). */
export function sameKey(a: ArrayBuffer | null | undefined, b: Uint8Array): boolean {
  if (!a) return false;
  const x = new Uint8Array(a);
  return x.length === b.length && x.every((v, i) => v === b[i]);
}

type Fetch = (input: string, init?: RequestInit) => Promise<Response>;

async function errorOf(res: Response, what: string): Promise<Error> {
  const data = await res.json().catch(() => null);
  return new Error(data && typeof data.error === 'string' ? data.error : `${what} failed (HTTP ${res.status}).`);
}

/** The service worker's registration, or null where there isn't one (dev server, plain http). */
export async function swRegistration(nav: Navigator = navigator): Promise<ServiceWorkerRegistration | null> {
  if (!('serviceWorker' in nav)) return null;
  return (await nav.serviceWorker.getRegistration('/').catch(() => undefined)) ?? null;
}

/** Subscribes this device (or reuses its subscription) and tells the server. Idempotent: the
 * server keeps one entry per endpoint, so this also re-syncs after the server dropped it. */
export async function subscribePush(reg: ServiceWorkerRegistration, fetchFn: Fetch = fetch): Promise<PushSubscription> {
  const keyRes = await fetchFn('/api/push/key');
  if (!keyRes.ok) throw await errorOf(keyRes, 'Getting the server key');
  const { public_key } = (await keyRes.json()) as { public_key?: string };
  if (!public_key) throw new Error('The server has no push key.');
  const key = base64UrlToBytes(public_key);
  let sub = await reg.pushManager.getSubscription();
  if (sub && !sameKey(sub.options?.applicationServerKey, key)) {
    await sub.unsubscribe().catch(() => false);
    sub = null;
  }
  sub ??= await reg.pushManager.subscribe({ userVisibleOnly: true, applicationServerKey: key });
  const res = await fetchFn('/api/push/subscribe', {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify(sub.toJSON()),
  });
  if (!res.ok) throw await errorOf(res, 'Saving the subscription');
  return sub;
}

/** Stops pushes to this device, on the server and in the browser. */
export async function unsubscribePush(reg: ServiceWorkerRegistration, fetchFn: Fetch = fetch): Promise<void> {
  const sub = await reg.pushManager.getSubscription();
  if (!sub) return;
  await fetchFn('/api/push/unsubscribe', {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({ endpoint: sub.endpoint }),
  }).catch(() => undefined);
  await sub.unsubscribe().catch(() => false);
}

/** On app load: if this device has notifications on, make sure the server still has it. */
export async function resyncPush(opts: {
  enabled: boolean;
  permission: NotificationPermission | 'unsupported';
  support: DeviceNotifySupport;
  registration: () => Promise<ServiceWorkerRegistration | null>;
  fetchFn?: Fetch;
}): Promise<'synced' | 'skipped' | 'failed'> {
  if (!opts.enabled || opts.permission !== 'granted' || opts.support !== 'push') return 'skipped';
  const reg = await opts.registration();
  if (!reg) return 'skipped';
  try {
    await subscribePush(reg, opts.fetchFn);
    return 'synced';
  } catch {
    return 'failed';
  }
}

/** Runs resyncPush once with this page's state. */
export function resyncPushOnLoad(win: Window = window): Promise<string> {
  let enabled = false;
  try {
    enabled = win.localStorage.getItem(BROWSER_NOTIFY_KEY) === 'on';
  } catch {
    enabled = false;
  }
  return resyncPush({
    enabled,
    permission: notificationPermission(),
    support: deviceNotifySupport(notifyEnv(win)),
    registration: () => swRegistration(win.navigator),
  });
}

// ---------------------------------------------------------------------------
// The open page's own notification

const ICON = '/icons/icon-192.png';

/** Shows `n` as an OS notification. Through the service worker where there is one (Android
 * Chrome only allows that), so a tap goes through sw.js like a push; the constructor elsewhere. */
export async function showDeviceNotification(
  n: NotifyEvent & { space_id?: string | null },
  onClick: () => void,
  nav: Navigator = navigator,
): Promise<void> {
  const tag = notificationTag(n);
  const reg = await swRegistration(nav);
  if (reg) {
    await reg.showNotification(n.title, {
      body: n.body,
      tag,
      icon: ICON,
      badge: ICON,
      data: {
        path: notificationPath(n),
        kind: n.kind,
        space_id: n.space_id ?? null,
        channel_id: n.channel_id ?? null,
        kwant_id: n.kwant_id ?? null,
      },
    });
    return;
  }
  const note = new Notification(n.title, { body: n.body, tag, icon: ICON });
  note.onclick = () => {
    window.focus();
    onClick();
    note.close();
  };
}
