import { useCallback, useEffect, useRef, useState } from 'react';

import { browserStreamUrl } from './browserViewport';
import { PageDialog, SelectRequest, Size, parseDialog, parseSelect, streamErrorText } from './browserInput';
import { onRecordingFrame } from './showOnce';
import { POINT_TIMEOUT_MS, PointError, pointErrorCode } from './point';

export interface BrowserFrame {
  id: number;
  image: string;
  /** The page's viewport in CSS px (the image itself may be dpr times larger). */
  width: number;
  height: number;
}

/** Said when taking control fails because the bot is in the middle of something. */
export const BOT_BUSY_TEXT = 'The bot is finishing a step, try again in a moment.';

/** Said when taking control fails because the person already holds this browser in another window (code held_by_person). */
export const HELD_BY_PERSON_TEXT = "You're controlling this browser in another window. Press Hand back there to take over here.";

/**
 * A refused take_control: `held_by_person` can never win by retrying, so it gets its own sentence.
 * `bot_busy`, `failed` and an older server's error with no code read as the busy bot, as before;
 * the text match covers a server that sends the held sentence without a code.
 */
export function takeoverRefusedText(message: { code?: unknown; error?: unknown }): string {
  if (message.code === 'held_by_person') return HELD_BY_PERSON_TEXT;
  if (message.code === undefined && /another window/i.test(String(message.error ?? ''))) return HELD_BY_PERSON_TEXT;
  return BOT_BUSY_TEXT;
}

/** `page` (0.9.3): which of the team's pages to show; null or undefined is the server's default. A change reconnects. */
export function useBrowserStream(id: string | undefined, visible: boolean, page?: string | null) {
  const socket = useRef<WebSocket | null>(null);
  const sequence = useRef(0);
  const pending = useRef(new Set<number>());
  const takingControl = useRef(false);
  const lastImage = useRef<string | null>(null);
  const [connected, setConnected] = useState(false);
  // The page the open stream shows (null: the server's default), set with its first frame
  const [livePage, setLivePage] = useState<string | null>(null);
  const [controlling, setControlling] = useState(false);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [frame, setFrame] = useState<BrowserFrame | null>(null);
  const [location, setLocation] = useState<{ url: string; title: string } | null>(null);
  // After each tap: whether it focused a text field on the page (the phone keyboard follows it)
  const [tapFocus, setTapFocus] = useState<{ seq: number; editable: boolean } | null>(null);
  const [viewport, setViewport] = useState<Size | null>(null);
  const [select, setSelect] = useState<SelectRequest | null>(null);
  const [dialog, setDialog] = useState<PageDialog | null>(null);
  const [cursor, setCursor] = useState<string>('default');
  // The last resize was answered `ignored`: the page follows a real window and keeps its size
  const [resizeIgnored, setResizeIgnored] = useState(false);
  // Actions the server answered "Invalid browser input" for (an older server): hide their buttons
  const [unsupported, setUnsupported] = useState<ReadonlySet<string>>(new Set());
  const actionsById = useRef(new Map<number, string>());
  // G3 point requests waiting for their answer, by id: they never touch the panel's banner or busy state
  const waiters = useRef(new Map<number, { resolve: (m: Record<string, any>) => void; reject: (e: PointError) => void; timer: ReturnType<typeof setTimeout> }>());
  const failWaiters = () => {
    for (const w of waiters.current.values()) {
      clearTimeout(w.timer);
      w.reject(new PointError('failed'));
    }
    waiters.current.clear();
  };

  useEffect(() => {
    if (!id || id === 'default' || !visible) return;
    let disposed = false;
    let reconnect: ReturnType<typeof setTimeout>;
    let attempts = 0;
    const connect = () => {
      const ws = new WebSocket(browserStreamUrl(window.location, id, page));
      socket.current = ws;
      ws.onopen = () => {
        if (disposed) { ws.close(); return; }
        setConnected(false);
        setError(null);
      };
      ws.onmessage = (event) => {
        if (disposed || socket.current !== ws) return;
        let message: Record<string, any>;
        try { message = JSON.parse(event.data); } catch { return; }
        const waiter = (message.type === 'ack' || message.type === 'error') && typeof message.id === 'number' ? waiters.current.get(message.id) : undefined;
        if (waiter) {
          waiters.current.delete(message.id);
          clearTimeout(waiter.timer);
          if (message.type === 'ack') waiter.resolve(message);
          else {
            const code = pointErrorCode(message);
            if (code === 'unsupported') setUnsupported((s) => new Set(s).add('point'));
            waiter.reject(new PointError(code, typeof message.error === 'string' ? message.error : undefined));
          }
          return;
        }
        switch (message.type) {
          case 'ping': ws.send(JSON.stringify({ action: 'pong' })); break;
          case 'frame': {
            attempts = 0;
            setConnected(true);
            setLivePage(page ?? null);
            const width = Number(message.width) > 0 ? Number(message.width) : 1280;
            const height = Number(message.height) > 0 ? Number(message.height) : 800;
            // An identical picture leaves <img src> unchanged, so no load event comes to acknowledge it
            if (message.image === lastImage.current) ws.send(JSON.stringify({ action: 'frame_ack', id: message.id }));
            lastImage.current = message.image;
            setFrame({ id: message.id, image: message.image, width, height });
            break;
          }
          case 'location': setLocation({ url: message.url || '', title: message.title || '' }); break;
          case 'control':
            takingControl.current = false;
            setControlling(!!message.active);
            if (message.viewport && message.viewport.width > 0) setViewport({ width: message.viewport.width, height: message.viewport.height });
            if (!message.active) { setSelect(null); setDialog(null); setCursor('default'); }
            break;
          case 'select': setSelect(parseSelect(message)); break;
          case 'dialog': setDialog(parseDialog(message)); break;
          case 'dialog_closed': setDialog(null); break;
          case 'cursor': if (typeof message.cursor === 'string') setCursor(message.cursor); break;
          case 'clipboard':
            if (typeof message.text === 'string' && message.text) navigator.clipboard?.writeText(message.text).catch(() => {});
            break;
          case 'error': {
            const action = typeof message.id === 'number' ? actionsById.current.get(message.id) : undefined;
            if (action && /invalid browser input/i.test(String(message.error))) {
              setUnsupported((s) => new Set(s).add(action));
            } else if (action === 'take_control' || (takingControl.current && message.id === undefined)) {
              takingControl.current = false;
              setError(takeoverRefusedText(message));
            } else {
              const text = streamErrorText(action, message.error);
              if (text) setError(text);
            }
            if (typeof message.id === 'number') { pending.current.delete(message.id); actionsById.current.delete(message.id); }
            setBusy(pending.current.size > 0);
            break;
          }
          case 'ack':
            if (actionsById.current.get(message.id) === 'resize') setResizeIgnored(message.ignored === true);
            pending.current.delete(message.id);
            actionsById.current.delete(message.id);
            setBusy(pending.current.size > 0);
            if (typeof message.editable === 'boolean') setTapFocus({ seq: message.id, editable: message.editable });
            break;
          default:
            // Show it once: step, near, stopped and state frames (api.md §5)
            if (typeof message.type === 'string' && message.type.startsWith('skill.recording.')) onRecordingFrame(message);
        }
      };
      ws.onclose = () => {
        if (disposed || socket.current !== ws) return;
        socket.current = null;
        pending.current.clear();
        actionsById.current.clear();
        failWaiters();
        takingControl.current = false;
        setConnected(false);
        setControlling(false);
        setBusy(false);
        setFrame(null);
        lastImage.current = null;
        setSelect(null);
        setDialog(null);
        setResizeIgnored(false);
        setError((current) => current || 'Browser disconnected. Reconnecting…');
        reconnect = setTimeout(connect, Math.min(1000 * 2 ** attempts++, 10000));
      };
    };
    connect();
    return () => {
      disposed = true;
      clearTimeout(reconnect);
      socket.current?.close();
      socket.current = null;
      pending.current.clear();
      actionsById.current.clear();
      failWaiters();
      setConnected(false);
      setLivePage(null);
      setControlling(false);
      setBusy(false);
      setFrame(null);
      setLocation(null);
      setTapFocus(null);
      setSelect(null);
      setDialog(null);
      setViewport(null);
      setResizeIgnored(false);
    };
  }, [id, visible, page]);

  /**
   * `input` marks a discrete action that waits for the page's ack (and shows "Relaying…").
   * Moves, wheel steps and keys don't: they stream, and a stuck one must never block the rest.
   */
  const send = useCallback((payload: Record<string, unknown>, input = false) => {
    const ws = socket.current;
    if (!ws || ws.readyState !== WebSocket.OPEN) { setError('Browser is reconnecting. Try again when connected.'); return; }
    if (input && (ws.bufferedAmount > 64 * 1024 || pending.current.size >= 128)) {
      setError('Browser input is busy. Wait for pending input to finish.'); return;
    }
    // A stream of moves on a slow link: drop moves, never presses, releases or keys
    if (!input && payload.action === 'mouse' && payload.type === 'move' && ws.bufferedAmount > 256 * 1024) return;
    const id = ++sequence.current;
    if (input) { pending.current.add(id); setBusy(true); }
    if (typeof payload.action === 'string') {
      actionsById.current.set(id, payload.action);
      if (actionsById.current.size > 512) actionsById.current.delete(actionsById.current.keys().next().value as number);
      if (payload.action === 'take_control') takingControl.current = true;
    }
    if (input || payload.action === 'take_control') setError(null);
    ws.send(JSON.stringify({ ...payload, id }));
  }, []);

  const acknowledgeFrame = useCallback((id: number) => {
    const ws = socket.current;
    if (ws?.readyState === WebSocket.OPEN) ws.send(JSON.stringify({ action: 'frame_ack', id }));
  }, []);

  /**
   * Gives the page its own size back while keeping control (the phone overlay closing or going to
   * the background). Quiet: a closed socket already restored it on the server.
   */
  const restoreViewport = useCallback(() => {
    const ws = socket.current;
    if (ws?.readyState === WebSocket.OPEN) ws.send(JSON.stringify({ action: 'restore_viewport', id: ++sequence.current }));
  }, []);

  /**
   * A G3 point action that waits for its own answer: resolves with the ack, rejects with a PointError
   * (its `code`, or `failed` after 5 s, on a close or with no socket). Never shows the panel's banner.
   */
  const request = useCallback((payload: Record<string, unknown>, timeoutMs = POINT_TIMEOUT_MS): Promise<Record<string, any>> => {
    const ws = socket.current;
    if (!ws || ws.readyState !== WebSocket.OPEN) return Promise.reject(new PointError('failed'));
    const id = ++sequence.current;
    return new Promise((resolve, reject) => {
      const timer = setTimeout(() => {
        waiters.current.delete(id);
        reject(new PointError('failed'));
      }, timeoutMs);
      waiters.current.set(id, { resolve, reject, timer });
      ws.send(JSON.stringify({ ...payload, id }));
    });
  }, []);

  const clearSelect = useCallback(() => setSelect(null), []);
  const clearDialog = useCallback(() => setDialog(null), []);
  const clearError = useCallback(() => setError(null), []);

  return {
    /** The page the connected stream shows, null for the server's default (meaningful while `connected`). */
    page: livePage,
    connected, controlling, busy, error, frame, location, tapFocus, viewport, select, dialog, cursor, unsupported, resizeIgnored,
    send, request, acknowledgeFrame, restoreViewport, clearSelect, clearDialog, clearError,
  };
}

export type BrowserStream = ReturnType<typeof useBrowserStream>;
