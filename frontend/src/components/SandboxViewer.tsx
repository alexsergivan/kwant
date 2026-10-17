import React, { useCallback, useEffect, useRef, useState } from 'react';
import { Globe, Loader2, RefreshCw, Share2, MousePointerClick } from 'lucide-react';
import { ComputerModeChip, ComputerModeError, LocalBrowserNote, useComputerMode } from './ComputerModeSwitch';

// Must match the sandbox Chromium --window-size (docker/entrypoint.sh)
const VIEWPORT_W = 1280;
const VIEWPORT_H = 800;
const POLL_MS = 2000;
const SPECIAL_KEYS = ['Enter', 'Backspace', 'Tab', 'Escape', 'ArrowUp', 'ArrowDown', 'ArrowLeft', 'ArrowRight', 'Delete', 'Home', 'End', 'PageUp', 'PageDown'];

type InteractPayload = {
  action: 'click' | 'type' | 'key' | 'scroll';
  x?: number;
  y?: number;
  text?: string;
  key?: string;
  code?: string;
  delta_x?: number;
  delta_y?: number;
};

/** Full-page live view of a sandbox's Chromium. Everything happens in the same
 *  browser the agents use, so they see the resulting page on their next turn. */
export const SandboxViewer: React.FC<{ teamId: string }> = ({ teamId }) => {
  const [screenshot, setScreenshot] = useState<string | null>(null);
  const [url, setUrl] = useState('');
  const [title, setTitle] = useState('');
  const [inputUrl, setInputUrl] = useState('');
  const [isBusy, setIsBusy] = useState(false);
  const [isFocused, setIsFocused] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const imageRef = useRef<HTMLImageElement>(null);
  const viewportRef = useRef<HTMLDivElement>(null);
  const busyRef = useRef(false);
  const urlInputFocusedRef = useRef(false);
  const lastWheelRef = useRef(0);
  const api = `/api/sandbox/${encodeURIComponent(teamId)}`;
  const computer = useComputerMode(true);
  const isLocal = computer.state.settings?.mode === 'local';

  const applyUrl = useCallback((next?: string) => {
    if (!next || next === 'about:blank') return;
    setUrl(next);
    if (!urlInputFocusedRef.current) setInputUrl(next);
  }, []);

  const refresh = useCallback(async () => {
    if (busyRef.current) return;
    try {
      const res = await fetch(`${api}/screenshot`);
      if (!res.ok) throw new Error(`HTTP ${res.status}`);
      const data = await res.json();
      if (data.image_base64) setScreenshot(data.image_base64);
      applyUrl(data.url);
      if (data.title) setTitle(data.title);
      setError(null);
    } catch (err) {
      setError('Sandbox browser is not reachable. Is the sandbox running?');
    }
  }, [api, applyUrl]);

  const run = useCallback(
    async (fn: () => Promise<void>) => {
      busyRef.current = true;
      setIsBusy(true);
      try {
        await fn();
      } catch (err) {
        console.error('Sandbox viewer action failed:', err);
      } finally {
        busyRef.current = false;
        setIsBusy(false);
      }
    },
    []
  );

  const interact = useCallback(
    (payload: InteractPayload) =>
      run(async () => {
        const res = await fetch(`${api}/interact`, {
          method: 'POST',
          headers: { 'Content-Type': 'application/json' },
          body: JSON.stringify(payload),
        });
        if (!res.ok) return;
        const data = await res.json();
        if (data.screenshot) setScreenshot(data.screenshot);
        applyUrl(data.url);
      }),
    [api, run, applyUrl]
  );

  const navigate = useCallback(
    (target: string) =>
      run(async () => {
        let next = target.trim();
        if (!next) return;
        if (!/^[a-z]+:\/\//i.test(next)) next = `https://${next}`;
        const res = await fetch(`${api}/navigate`, {
          method: 'POST',
          headers: { 'Content-Type': 'application/json' },
          body: JSON.stringify({ url: next }),
        });
        if (res.ok) {
          const data = await res.json();
          applyUrl(data.url);
          if (data.title) setTitle(data.title);
        }
      }).then(refresh),
    [api, run, applyUrl, refresh]
  );

  // Initial load + polling so page changes (loads, animations, bot actions) show up
  useEffect(() => {
    if (isLocal) return;
    refresh();
    const timer = window.setInterval(() => {
      if (document.visibilityState === 'visible') refresh();
    }, POLL_MS);
    return () => window.clearInterval(timer);
  }, [refresh, isLocal]);

  // Live updates when the bot (or the in-app pane) drives the same browser
  useEffect(() => {
    const sse = new EventSource('/api/events');
    sse.onmessage = (e) => {
      try {
        const event = JSON.parse(e.data);
        if (event.data?.team_id !== teamId) return;
        if (event.type === 'browser_screenshot' || event.type === 'BrowserScreenshot') {
          if (event.data.image_base64) setScreenshot(event.data.image_base64);
          applyUrl(event.data.url);
        } else if (event.type === 'browser_navigated' || event.type === 'BrowserNavigated') {
          applyUrl(event.data.url);
          if (event.data.title) setTitle(event.data.title);
        }
      } catch {
        // ignore malformed events
      }
    };
    return () => sse.close();
  }, [teamId, applyUrl]);

  useEffect(() => {
    document.title = `${title || url || 'Sandbox browser'} · Kwant sandbox`;
  }, [title, url]);

  const toViewport = (clientX: number, clientY: number) => {
    const img = imageRef.current;
    const rect = img?.getBoundingClientRect();
    if (!img || !rect || rect.width === 0) return null;
    // Account for object-contain letterboxing
    const ratio = (img.naturalWidth || VIEWPORT_W) / (img.naturalHeight || VIEWPORT_H);
    let w = rect.width;
    let h = rect.width / ratio;
    if (h > rect.height) {
      h = rect.height;
      w = rect.height * ratio;
    }
    const x = clientX - rect.left - (rect.width - w) / 2;
    const y = clientY - rect.top - (rect.height - h) / 2;
    if (x < 0 || y < 0 || x > w || y > h) return null;
    return {
      x: Math.round((x / w) * VIEWPORT_W),
      y: Math.round((y / h) * VIEWPORT_H),
    };
  };

  const handleClick = (e: React.MouseEvent) => {
    viewportRef.current?.focus();
    const p = toViewport(e.clientX, e.clientY);
    if (p) interact({ action: 'click', ...p });
  };

  const handleWheel = (e: React.WheelEvent) => {
    const now = Date.now();
    if (now - lastWheelRef.current < 120) return;
    lastWheelRef.current = now;
    const p = toViewport(e.clientX, e.clientY) || { x: VIEWPORT_W / 2, y: VIEWPORT_H / 2 };
    interact({ action: 'scroll', ...p, delta_x: e.deltaX, delta_y: e.deltaY });
  };

  const handleKeyDown = (e: React.KeyboardEvent) => {
    if (e.metaKey || e.ctrlKey || e.altKey) return; // let browser shortcuts (incl. paste) through
    if (SPECIAL_KEYS.includes(e.key)) {
      e.preventDefault();
      interact({ action: 'key', key: e.key, code: e.code });
    } else if (e.key.length === 1) {
      e.preventDefault();
      interact({ action: 'type', text: e.key });
    }
  };

  const handlePaste = (e: React.ClipboardEvent) => {
    const text = e.clipboardData.getData('text');
    if (text) {
      e.preventDefault();
      interact({ action: 'type', text });
    }
  };

  return (
    <div className="h-screen w-screen flex flex-col bg-zinc-950 text-zinc-200">
      <header className="flex items-center gap-3 px-4 py-2.5 border-b border-zinc-800 bg-zinc-900/60">
        <div className="flex items-center gap-2 flex-shrink-0">
          <span className={`w-2.5 h-2.5 rounded-full ${error ? 'bg-zinc-500' : 'bg-emerald-400 animate-pulse'}`} />
          <span className="text-sm font-semibold">Sandbox browser</span>
        </div>

        <form
          className="flex-1 flex items-center gap-2 px-3 py-1.5 rounded-lg bg-zinc-950 border border-zinc-800 focus-within:border-sky-500/50 min-w-0"
          onSubmit={(e) => {
            e.preventDefault();
            navigate(inputUrl);
          }}
        >
          <Globe size={14} className="text-zinc-500 flex-shrink-0" />
          <input
            value={inputUrl}
            onChange={(e) => setInputUrl(e.target.value)}
            onFocus={() => (urlInputFocusedRef.current = true)}
            onBlur={() => {
              urlInputFocusedRef.current = false;
              setInputUrl(url);
            }}
            placeholder="Enter a URL to open in the sandbox"
            className="flex-1 bg-transparent outline-none text-sm font-mono min-w-0"
          />
          {isBusy && <Loader2 size={14} className="animate-spin text-sky-400 flex-shrink-0" />}
        </form>

        <button
          type="button"
          onClick={() => (url ? navigate(url) : refresh())}
          title="Reload page"
          className="p-2 rounded-md text-zinc-400 hover:text-zinc-100 hover:bg-zinc-800 cursor-pointer"
        >
          <RefreshCw size={14} />
        </button>

        <ComputerModeChip api={computer} />

        <div
          className="flex items-center gap-1.5 px-2.5 py-1 rounded-full text-xs font-medium text-emerald-300 bg-emerald-500/10 border border-emerald-500/30 flex-shrink-0"
          title="This is the same browser your agents use. They will see the current page on their next reply."
        >
          <Share2 size={12} />
          <span>Shared with your agents</span>
        </div>
      </header>

      {!isLocal && (
        <div className="px-4 py-1.5 text-[11px] text-zinc-500 border-b border-zinc-900 flex items-center gap-2">
          <MousePointerClick size={12} className={isFocused ? 'text-sky-400' : ''} />
          <span>
            {isFocused
              ? 'Controlling the sandbox: clicks, scrolling, typing and paste go to the page.'
              : 'Click the page to take control. Your agents will see wherever you leave this browser.'}
          </span>
          {title && <span className="ml-auto truncate text-zinc-400">{title}</span>}
        </div>
      )}

      {computer.state.error && (
        <div className="px-4 pt-2">
          <ComputerModeError error={computer.state.error} onDismiss={computer.dismissError} />
        </div>
      )}

      <main className="flex-1 min-h-0 flex items-center justify-center p-4 overflow-hidden">
        {isLocal ? (
          <div className="w-full max-w-md flex">
            <LocalBrowserNote />
          </div>
        ) : error && !screenshot ? (
          <div className="text-sm text-zinc-400">{error}</div>
        ) : (
          <div
            ref={viewportRef}
            tabIndex={0}
            onClick={handleClick}
            onWheel={handleWheel}
            onKeyDown={handleKeyDown}
            onPaste={handlePaste}
            onFocus={() => setIsFocused(true)}
            onBlur={() => setIsFocused(false)}
            className={`relative max-w-full max-h-full outline-none rounded-lg overflow-hidden border shadow-2xl cursor-pointer ${
              isFocused ? 'border-sky-500/60 shadow-sky-500/10' : 'border-zinc-800'
            }`}
            style={{ aspectRatio: `${VIEWPORT_W} / ${VIEWPORT_H}`, width: `min(100%, calc((100vh - 7rem) * ${VIEWPORT_W / VIEWPORT_H}))` }}
          >
            {screenshot ? (
              <img
                ref={imageRef}
                src={`data:image/jpeg;base64,${screenshot}`}
                alt={title || 'Sandbox browser'}
                draggable={false}
                className="w-full h-full object-contain select-none bg-white"
              />
            ) : (
              <div className="w-full h-full flex items-center justify-center text-zinc-500 text-sm gap-2">
                <Loader2 size={16} className="animate-spin" /> Connecting to sandbox…
              </div>
            )}
          </div>
        )}
      </main>
    </div>
  );
};
