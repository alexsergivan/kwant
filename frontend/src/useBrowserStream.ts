import { useCallback, useEffect, useRef, useState } from 'react';

import { browserStreamUrl } from './browserViewport';
import { PageDialog, SelectRequest, Size, parseDialog, parseSelect } from './browserInput';

export interface BrowserFrame {
  id: number;
  image: string;
  /** The page's viewport in CSS px (the image itself may be dpr times larger). */
  width: number;
  height: number;
}

/** Said when taking control fails because the bot is in the middle of something. */
export const BOT_BUSY_TEXT = 'The bot is finishing a step, try again in a moment.';

export function useBrowserStream(id: string | undefined, visible: boolean) {
  const socket = useRef<WebSocket | null>(null);
  const sequence = useRef(0);
  const pending = useRef(new Set<number>());
  const takingControl = useRef(false);
  const [connected, setConnected] = useState(false);
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
  // Actions the server answered "Invalid browser input" for (an older server): hide their buttons
  const [unsupported, setUnsupported] = useState<ReadonlySet<string>>(new Set());
  const actionsById = useRef(new Map<number, string>());

  useEffect(() => {
    if (!id || id === 'default' || !visible) return;
    let disposed = false;
    let reconnect: ReturnType<typeof setTimeout>;
    let attempts = 0;
    const connect = () => {
      const ws = new WebSocket(browserStreamUrl(window.location, id));
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
        switch (message.type) {
          case 'ping': ws.send(JSON.stringify({ action: 'pong' })); break;
          case 'frame': {
            attempts = 0;
            setConnected(true);
            const width = Number(message.width) > 0 ? Number(message.width) : 1280;
            const height = Number(message.height) > 0 ? Number(message.height) : 800;
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
              setError(BOT_BUSY_TEXT);
            } else {
              setError(message.error);
            }
            if (typeof message.id === 'number') { pending.current.delete(message.id); actionsById.current.delete(message.id); }
            setBusy(pending.current.size > 0);
            break;
          }
          case 'ack':
            pending.current.delete(message.id);
            actionsById.current.delete(message.id);
            setBusy(pending.current.size > 0);
            if (typeof message.editable === 'boolean') setTapFocus({ seq: message.id, editable: message.editable });
            break;
        }
      };
      ws.onclose = () => {
        if (disposed || socket.current !== ws) return;
        socket.current = null;
        pending.current.clear();
        actionsById.current.clear();
        takingControl.current = false;
        setConnected(false);
        setControlling(false);
        setBusy(false);
        setFrame(null);
        setSelect(null);
        setDialog(null);
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
      setConnected(false);
      setControlling(false);
      setBusy(false);
      setFrame(null);
      setLocation(null);
      setTapFocus(null);
      setSelect(null);
      setDialog(null);
      setViewport(null);
    };
  }, [id, visible]);

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

  const clearSelect = useCallback(() => setSelect(null), []);
  const clearDialog = useCallback(() => setDialog(null), []);
  const clearError = useCallback(() => setError(null), []);

  return {
    connected, controlling, busy, error, frame, location, tapFocus, viewport, select, dialog, cursor, unsupported,
    send, acknowledgeFrame, clearSelect, clearDialog, clearError,
  };
}

export type BrowserStream = ReturnType<typeof useBrowserStream>;
