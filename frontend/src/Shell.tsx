// The app shell: the spaces, the rail and space dialogs. Everything inside a space
// lives in <SpaceApp key={space.id}>, so switching remounts it: fresh state, a fresh event stream
// for the new space, and nothing from the old space can flash.
import React, { useCallback, useEffect, useRef, useState } from 'react';
import { AlertTriangle, Loader2 } from 'lucide-react';
import { SpaceApp, SpaceTarget } from './App';
import { SpaceView, sameView } from './nav';
import { ArchivedSpacesModal, NewSpaceModal, SpaceRail } from './components/Spaces';
import { Toasts, Toast } from './components/Toasts';
import { InstallHint } from './components/InstallHint';
import { AppUpdateBanner } from './components/AppUpdateBanner';
import { playSfx } from './audio/chiptune';
import {
  Space,
  eventsUrl,
  lastChat,
  lastSpace,
  parseSpacePath,
  pickSpace,
  rememberChat,
  rememberSpace,
  setCurrentSpace,
  spacePath,
  spacesApi,
} from './spaces';
import { NotifyEvent, TOAST_KINDS, browserNotifyEnabled, notificationPermission, shouldShowOsNotification } from './continuity';
import { isNotificationOpen, resyncPushOnLoad, showDeviceNotification, targetFromQuery } from './push';

const POLL_MS = 15000;

export const App: React.FC = () => {
  const [spaces, setSpaces] = useState<Space[] | null>(null);
  const [loadError, setLoadError] = useState<string | null>(null);
  const [currentId, setCurrentId] = useState<string | null>(null);
  // Where to land in the space being opened (a link, a notification, the last chat there).
  const [target, setTarget] = useState<SpaceTarget | null>(null);
  const [creating, setCreating] = useState<Space[] | null>(null);
  // Where the next space opens instead of a chat: its address (/inbox, /settings/accounts…), or
  // Accounts right after "Add accounts next".
  const [startView, setStartView] = useState<SpaceView | null>(null);
  // A view for the open space to show: the rail's gear, Back and Forward.
  const [viewRequest, setViewRequest] = useState<{ view: SpaceView | null; seq: number } | null>(null);
  const requestView = useCallback((view: SpaceView | null) => setViewRequest((r) => ({ view, seq: (r?.seq ?? 0) + 1 })), []);
  // The view the open space shows (null: a chat), as last reported.
  const viewRef = useRef<SpaceView | null>(null);
  // The open chat, the address to return to when the view closes.
  const chatRef = useRef<SpaceTarget | null>(null);
  const [showArchived, setShowArchived] = useState(false);
  const [toasts, setToasts] = useState<Toast[]>([]);
  const toastIdRef = useRef(0);
  const dismissToast = useCallback((id: number) => setToasts((prev) => prev.filter((t) => t.id !== id)), []);
  const currentRef = useRef(currentId);
  currentRef.current = currentId;

  const reload = useCallback(async () => {
    try {
      const list = await spacesApi.list();
      setSpaces(list);
      setLoadError(null);
      return list;
    } catch (e) {
      setLoadError(e instanceof Error ? e.message : 'Couldn’t load spaces.');
      return null;
    }
  }, []);

  /** Opens a space: the request scope changes before the new space renders. */
  const open = useCallback((id: string, to?: SpaceTarget | null, replace = false, view: SpaceView | null = null) => {
    const t = to ?? lastChat(id) ?? null;
    setStartView(view);
    setViewRequest(null);
    viewRef.current = view;
    chatRef.current = t;
    setCurrentSpace(id);
    rememberSpace(id);
    setTarget(t);
    setCurrentId(id);
    const path = spacePath(id, t ?? undefined, view ?? undefined);
    if (window.location.pathname !== path) {
      if (replace) window.history.replaceState(null, '', path);
      else window.history.pushState(null, '', path);
    }
  }, []);

  // First load: the space in the address, then the last one used, then Default.
  useEffect(() => {
    reload().then((list) => {
      if (!list) return;
      // A notification's `/?space=…&channel=…` link counts as the chat's own address.
      const q = targetFromQuery(window.location.search);
      const route = q ? { spaceId: q.spaceId, channelId: q.channelId ?? undefined, botId: q.botId ?? undefined } : parseSpacePath(window.location.pathname);
      const s = pickSpace(list, route.spaceId, lastSpace());
      if (!s) return;
      const here = route.spaceId === s.id;
      const fromUrl = here && (route.channelId || route.botId) ? { channelId: route.channelId, botId: route.botId } : null;
      // An old address (/away, /loose-ends, /connections…) opens its new place, rewritten in place.
      open(s.id, fromUrl, true, here && 'view' in route ? route.view ?? null : null);
    });
  }, [reload, open]);

  // Back and forward between spaces.
  useEffect(() => {
    const onPop = () => {
      const route = parseSpacePath(window.location.pathname);
      if (!route.spaceId || !spaces?.some((s) => s.id === route.spaceId)) return;
      const view = route.view ?? null;
      if (route.spaceId !== currentRef.current) {
        setCurrentSpace(route.spaceId);
        setTarget({ channelId: route.channelId, botId: route.botId });
        setStartView(view);
        setViewRequest(null);
        viewRef.current = view;
        setCurrentId(route.spaceId);
        return;
      }
      // Same space: Back from the Inbox or Settings goes back to the chat, and Forward again.
      if (!sameView(view, viewRef.current)) {
        viewRef.current = view;
        requestView(view);
      }
    };
    window.addEventListener('popstate', onPop);
    return () => window.removeEventListener('popstate', onPop);
  }, [spaces, requestView]);

  // Unread counts for the other spaces.
  useEffect(() => {
    const t = setInterval(() => !document.hidden && reload(), POLL_MS);
    const onFocus = () => reload();
    window.addEventListener('focus', onFocus);
    return () => {
      clearInterval(t);
      window.removeEventListener('focus', onFocus);
    };
  }, [reload]);

  const switchTo = useCallback(
    (id: string, to?: SpaceTarget | null) => {
      if (id === currentRef.current && !to) return;
      playSfx('click');
      open(id, to);
      reload();
    },
    [open, reload],
  );

  // Ctrl/⌘ 1–9 switches spaces.
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (!(e.metaKey || e.ctrlKey) || e.altKey || e.shiftKey) return;
      const n = Number(e.key);
      if (!Number.isInteger(n) || n < 1 || n > 9) return;
      const s = spaces?.[n - 1];
      if (!s) return;
      e.preventDefault();
      switchTo(s.id);
    };
    window.addEventListener('keydown', onKey);
    return () => window.removeEventListener('keydown', onKey);
  }, [spaces, switchTo]);

  // Notifications from the other spaces: the open space's own stream only carries its events.
  const activeCount = spaces?.length ?? 0;
  useEffect(() => {
    if (activeCount < 2) return;
    const sse = new EventSource(eventsUrl(null));
    sse.onmessage = (e) => {
      if (typeof e.data !== 'string' || !e.data.includes('otify')) return;
      try {
        const event = JSON.parse(e.data);
        if (event.type !== 'notify' && event.type !== 'Notify') return;
        const n = event.data as NotifyEvent;
        const sid = n.space_id ?? event.space_id;
        if (!sid || sid === currentRef.current) return;
        const name = n.space_name ?? 'another space';
        const go = () => switchTo(sid, n.channel_id ? { channelId: n.channel_id } : n.kwant_id ? { botId: n.kwant_id } : null);
        reload();
        if (!document.hidden && (TOAST_KINDS as string[]).includes(n.kind)) {
          const id = ++toastIdRef.current;
          setToasts((prev) => [...prev.slice(-2), { id, title: n.title, detail: n.body, action: { label: `Open in ${name}`, onClick: go } }]);
          return;
        }
        if (!shouldShowOsNotification({ enabled: browserNotifyEnabled(), permission: notificationPermission(), hidden: document.hidden, kind: n.kind })) return;
        showDeviceNotification({ ...n, space_id: sid }, go).catch(() => undefined);
      } catch {
        // not an event we know
      }
    };
    return () => sse.close();
  }, [activeCount, switchTo, reload]);

  // A tap on a notification from another space (sw.js posts it): switch there. The open space's
  // own are SpaceApp's, which can also open a question or browser step.
  useEffect(() => {
    const sw = 'serviceWorker' in navigator ? navigator.serviceWorker : null;
    if (!sw) return;
    const onMessage = (e: MessageEvent) => {
      const m = e.data;
      if (!isNotificationOpen(m) || !m.space_id || m.space_id === currentRef.current) return;
      if (!spaces?.some((s) => s.id === m.space_id)) return;
      switchTo(m.space_id, m.channel_id ? { channelId: m.channel_id } : m.kwant_id ? { botId: m.kwant_id } : null);
    };
    sw.addEventListener('message', onMessage);
    return () => sw.removeEventListener('message', onMessage);
  }, [spaces, switchTo]);

  // Re-tell the server about this device's push subscription (it drops one the push service
  // rejected, or on sign-out), once per load.
  useEffect(() => {
    resyncPushOnLoad().catch(() => undefined);
  }, []);

  const openCreate = useCallback(() => {
    playSfx('click');
    // Archived spaces keep their names and colours, so they count as taken.
    spacesApi.archived().then(
      (archived) => setCreating([...(spaces ?? []), ...archived]),
      () => setCreating(spaces ?? []),
    );
  }, [spaces]);

  const current = spaces?.find((s) => s.id === currentId) ?? null;

  if (!spaces || !current) {
    return (
      <div className="h-[var(--kwnt-app-height,100dvh)] w-screen flex items-center justify-center bg-zinc-950 text-zinc-400 text-xs">
        {loadError ? (
          <div role="alert" className="flex flex-col items-center gap-3">
            <p className="flex items-center gap-2 text-rose-300">
              <AlertTriangle size={14} /> {loadError}
            </p>
            <button type="button" onClick={() => window.location.reload()} className="px-3 py-1.5 rounded-lg bg-zinc-800 text-zinc-200 hover:bg-zinc-700">
              Try again
            </button>
          </div>
        ) : spaces && spaces.length === 0 ? (
          <p>No spaces yet.</p>
        ) : (
          <p className="flex items-center gap-2">
            <Loader2 size={14} className="animate-spin motion-reduce:animate-none" /> Loading…
          </p>
        )}
      </div>
    );
  }

  return (
    <div className="h-[var(--kwnt-app-height,100dvh)] w-screen flex flex-col bg-zinc-950 text-zinc-100 overflow-hidden font-sans pt-[env(safe-area-inset-top)] pl-[env(safe-area-inset-left)] pr-[env(safe-area-inset-right)]">
      <AppUpdateBanner />
      <div className="flex-1 min-h-0 flex">
        <SpaceRail spaces={spaces} currentId={current.id} onSwitch={switchTo} onCreate={openCreate} onOpenAppSettings={() => requestView({ kind: 'settings', scope: 'app' })} />
        <SpaceApp
          key={current.id}
          space={current}
          spaces={spaces}
          initialTarget={target}
          initialView={startView}
          viewRequest={viewRequest}
          onViewChange={(v) => {
            const kindChanged = (v?.kind ?? null) !== (viewRef.current?.kind ?? null);
            viewRef.current = v;
            const path = spacePath(current.id, chatRef.current ?? lastChat(current.id) ?? undefined, v ?? undefined);
            if (window.location.pathname === path) return;
            // Opening or leaving the Inbox or Settings is a step Back undoes; moving between their tabs isn't.
            if (kindChanged) window.history.pushState(null, '', path);
            else window.history.replaceState(null, '', path);
          }}
          onSwitchSpace={switchTo}
          onCreateSpace={openCreate}
          onOpenArchived={() => setShowArchived(true)}
          onSpacesChanged={reload}
          onSpaceArchived={(s) => {
            reload().then((list) => {
              const next = list?.find((x) => x.is_default) ?? list?.[0];
              if (next) open(next.id);
            });
            const id = ++toastIdRef.current;
            setToasts((prev) => [...prev, { id, title: `${s.name} is archived`, detail: 'Its routines are paused. Restore it from Archived spaces.' }]);
          }}
          onChatChange={(t) => {
            rememberChat(current.id, t);
            chatRef.current = t;
            // The Inbox or Settings keeps its own address over the chat underneath.
            if (viewRef.current) return;
            const path = spacePath(current.id, t);
            if (window.location.pathname !== path) window.history.replaceState(null, '', path);
          }}
        />
      </div>

      {creating && (
        <NewSpaceModal
          spaces={creating}
          onClose={() => setCreating(null)}
          onCreated={(s, addAccounts) => {
            setCreating(null);
            setSpaces((prev) => (prev ? [...prev, s] : [s]));
            open(s.id, null, false, addAccounts ? { kind: 'settings', scope: 'space', tab: 'accounts' } : null);
            reload();
          }}
        />
      )}
      {showArchived && (
        <ArchivedSpacesModal
          onClose={() => setShowArchived(false)}
          onRestored={(s) => {
            setShowArchived(false);
            setSpaces((prev) => (prev ? [...prev.filter((x) => x.id !== s.id), s] : [s]));
            open(s.id, null);
            reload();
          }}
        />
      )}
      <Toasts toasts={toasts} onDismiss={dismissToast} position="top" />
      <InstallHint />
    </div>
  );
};
