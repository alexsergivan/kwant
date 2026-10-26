import { useCallback, useEffect, useRef, useState } from 'react';

import { browserStreamUrl } from './browserViewport';

export function useBrowserStream(id: string | undefined, visible: boolean) {
  const socket = useRef<WebSocket | null>(null);
  const sequence = useRef(0);
  const pending = useRef(new Set<number>());
  const [connected, setConnected] = useState(false);
  const [controlling, setControlling] = useState(false);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [frame, setFrame] = useState<{ id: number; image: string } | null>(null);
  const [location, setLocation] = useState<{ url: string; title: string } | null>(null);
  // After each tap: whether it focused a text field on the page (the phone keyboard follows it)
  const [tapFocus, setTapFocus] = useState<{ seq: number; editable: boolean } | null>(null);

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
        const message = JSON.parse(event.data);
        switch (message.type) {
          case 'ping': ws.send(JSON.stringify({ action: 'pong' })); break;
          case 'frame': attempts = 0; setConnected(true); setFrame({ id: message.id, image: message.image }); break;
          case 'location': setLocation({ url: message.url || '', title: message.title || '' }); break;
          case 'control': setControlling(message.active); break;
          case 'error': setError(message.error); pending.current.delete(message.id); setBusy(pending.current.size > 0); break;
          case 'ack':
            pending.current.delete(message.id);
            setBusy(pending.current.size > 0);
            if (typeof message.editable === 'boolean') setTapFocus({ seq: message.id, editable: message.editable });
            break;
        }
      };
      ws.onclose = () => {
        if (disposed || socket.current !== ws) return;
        socket.current = null;
        pending.current.clear();
        setConnected(false);
        setControlling(false);
        setBusy(false);
        setFrame(null);
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
      setConnected(false);
      setControlling(false);
      setBusy(false);
      setFrame(null);
      setLocation(null);
      setTapFocus(null);
    };
  }, [id, visible]);

  /** `input` waits for the page's ack (shows busy); pointer moves don't, they're fire-and-forget. */
  const send = useCallback((payload: Record<string, unknown>, input = false) => {
    const ws = socket.current;
    if (!ws || ws.readyState !== WebSocket.OPEN) { setError('Browser is reconnecting. Try again when connected.'); return; }
    if (ws.bufferedAmount > 64 * 1024 || pending.current.size >= 128) {
      setError('Browser input is busy. Wait for pending input to finish.'); return;
    }
    const id = ++sequence.current;
    if (input) { pending.current.add(id); setBusy(true); }
    setError(null);
    ws.send(JSON.stringify({ ...payload, id }));
  }, []);

  const acknowledgeFrame = useCallback((id: number) => {
    const ws = socket.current;
    if (ws?.readyState === WebSocket.OPEN) ws.send(JSON.stringify({ action: 'frame_ack', id }));
  }, []);

  return { connected, controlling, busy, error, frame, location, tapFocus, send, acknowledgeFrame };
}
