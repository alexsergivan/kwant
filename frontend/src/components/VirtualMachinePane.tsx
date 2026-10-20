import React, { useState, useRef, useEffect, useCallback, useMemo } from 'react';
import {
  X,
  Globe,
  Terminal,
  RefreshCw,
  ExternalLink,
  Cpu,
  Key,
  MousePointerClick,
  Keyboard,
  Loader2,
  Laptop,
} from 'lucide-react';
import { playSfx } from '../audio/chiptune';
import { mapBrowserPoint } from '../browserViewport';
import { useBrowserStream } from '../useBrowserStream';
import { ComputerModeChip, ComputerModeError, LocalBrowserNote, OpenWindowButton, useComputerMode } from './ComputerModeSwitch';
import { browserFooterLabel, isBlankPage, showsLocalWindow } from '../computerMode';
import { loginMatchesPage } from '../savedPasswords';
import { PendingLoginPrompt, SavedPasswordsPanel, useSavedPasswords } from './SavedPasswordsPanel';

export interface TerminalLogEntry {
  command: string;
  output: string;
  timestamp?: string;
}

export interface VirtualMachinePaneProps {
  isOpen: boolean;
  onClose: () => void;
  teamId?: string;
  teamName?: string;
  activeUrl?: string;
  pageTitle?: string;
  screenshotBase64?: string | null;
  isRunning?: boolean;
  browserError?: string | null;
  terminalLogs?: TerminalLogEntry[];
  onRefresh?: () => void;
  onRestartVm?: () => void;
  onNavigate?: (url: string) => void;
}

export const VirtualMachinePane: React.FC<VirtualMachinePaneProps> = ({
  isOpen,
  onClose,
  teamId,
  teamName = 'Team',
  activeUrl = '',
  pageTitle = '',
  screenshotBase64,
  isRunning = true,
  browserError,
  terminalLogs = [],
  onRefresh,
  onRestartVm,
  onNavigate,
}) => {
  const [activeTab, setActiveTab] = useState<'browser' | 'terminal' | 'vault'>('browser');
  const [inputUrl, setInputUrl] = useState(activeUrl);
  // No event fires when the browser catches a login, so a new screenshot or page loads the list again.
  const passwords = useSavedPasswords(teamId, `${activeUrl}|${screenshotBase64?.length ?? 0}|${screenshotBase64?.slice(-32) ?? ''}`);
  const savedCount = passwords.logins.length;

  // Local Chrome streams here like the VM, unless the person opened it as a window: then there's
  // nothing to show. Unknown counts as vm.
  const computer = useComputerMode(isOpen);
  const isLocal = computer.state.settings?.mode === 'local';
  const isWindow = showsLocalWindow(computer.state.settings);

  // Interactive Remote Control State
  const stream = useBrowserStream(teamId, isOpen && activeTab === 'browser' && !isWindow && !computer.windowPending);
  const isControlMode = stream.controlling;
  const isInteracting = stream.busy;
  const [quickTypeText, setQuickTypeText] = useState('');
  const [clickRipples, setClickRipples] = useState<Array<{ id: number; x: number; y: number }>>([]);
  const lastWheelTimeRef = useRef<number>(0);
  const imageRef = useRef<HTMLImageElement>(null);
  const viewportRef = useRef<HTMLDivElement>(null);

  // Once the stream reports where the tab is, trust it; before that, the last URL the app heard of.
  const pageUrl = stream.location ? stream.location.url : activeUrl;
  const currentScreenshot = isBlankPage(pageUrl) ? null : stream.frame?.image || screenshotBase64;
  // A hidden frame never loads, so acknowledge it here or the stream stalls waiting for the ack.
  const blankFrameId = isBlankPage(pageUrl) ? stream.frame?.id : undefined;
  const { acknowledgeFrame } = stream;
  useEffect(() => {
    if (blankFrameId !== undefined) acknowledgeFrame(blankFrameId);
  }, [blankFrameId, acknowledgeFrame]);
  const controlError = stream.error || (!stream.connected ? browserError : null);
  // A live stream is ready; otherwise trust isRunning only while no error says the service is down.
  const browserReady = stream.connected || (isRunning && !controlError);
  useEffect(() => {
    if (stream.location?.url) setInputUrl(stream.location.url);
  }, [stream.location]);

  useEffect(() => {
    setInputUrl(activeUrl);
  }, [activeUrl]);

  const loginsOnThisPage = useMemo(() => passwords.logins.filter((l) => loginMatchesPage(l, pageUrl)).length, [passwords.logins, pageUrl]);

  // Dispatch interactive action to backend
  const sendInteraction = useCallback(
    (payload: {
      action: 'click' | 'type' | 'key' | 'scroll';
      x?: number;
      y?: number;
      text?: string;
      key?: string;
      code?: string;
      delta_x?: number;
      delta_y?: number;
    }) => {
      stream.send({ ...payload, delta_x: payload.delta_x || 0, delta_y: payload.delta_y || 0 }, true);
    },
    [stream.send]
  );

  // Viewport Click Handler
  const handleViewportClick = (e: React.MouseEvent<HTMLDivElement>) => {
    if (!isControlMode || !imageRef.current) return;
    const img = imageRef.current;
    const rect = img.getBoundingClientRect();
    const point = mapBrowserPoint(e.clientX, e.clientY, rect, img.naturalWidth, img.naturalHeight);
    if (!point) return;
    viewportRef.current?.focus({ preventScroll: true });
    const frame = e.currentTarget.getBoundingClientRect();

    playSfx('click');
    const rippleId = Date.now();
    setClickRipples((prev) => [...prev, { id: rippleId, x: e.clientX - frame.left, y: e.clientY - frame.top }]);
    setTimeout(() => {
      setClickRipples((prev) => prev.filter((r) => r.id !== rippleId));
    }, 600);

    sendInteraction({ action: 'click', ...point });
  };

  // Viewport Wheel / Scroll Handler
  const handleViewportWheel = (e: React.WheelEvent<HTMLDivElement>) => {
    if (!isControlMode || !imageRef.current) return;
    const now = Date.now();
    if (now - lastWheelTimeRef.current < 120) return;
    lastWheelTimeRef.current = now;

    const img = imageRef.current;
    const rect = img.getBoundingClientRect();
    const point = mapBrowserPoint(e.clientX, e.clientY, rect, img.naturalWidth, img.naturalHeight);
    if (!point) return;

    sendInteraction({ action: 'scroll', ...point, delta_x: e.deltaX, delta_y: e.deltaY });
  };

  // Viewport Keydown Handler
  const handleViewportKeyDown = (e: React.KeyboardEvent<HTMLDivElement>) => {
    if (!isControlMode) return;
    const targetTag = (e.target as HTMLElement)?.tagName?.toLowerCase();
    if (targetTag === 'input' || targetTag === 'textarea') return;

    if (['Enter', 'Backspace', 'Tab', 'Escape', 'ArrowUp', 'ArrowDown', 'ArrowLeft', 'ArrowRight'].includes(e.key)) {
      e.preventDefault();
      playSfx('blip');
      sendInteraction({ action: 'key', key: e.key, code: e.code });
    } else if (e.key.length === 1 && !e.metaKey && !e.ctrlKey) {
      e.preventDefault();
      playSfx('blip');
      sendInteraction({ action: 'type', text: e.key });
    }
  };

  // Quick Type Submit
  const handleQuickTypeSubmit = (e: React.FormEvent) => {
    e.preventDefault();
    if (!quickTypeText.trim()) return;
    playSfx('coin');
    sendInteraction({ action: 'type', text: quickTypeText });
    setQuickTypeText('');
  };

  if (!isOpen) return null;

  return (
    <div className="w-96 border-l border-zinc-800 bg-zinc-950 flex flex-col h-full shadow-2xl z-30 select-none animate-in slide-in-from-right duration-200">
      {/* Top Header */}
      <div className="h-14 border-b border-zinc-800 px-4 flex items-center justify-between gap-2 bg-zinc-900/60">
        <div className="flex items-center gap-2 min-w-0">
          <div className={`shrink-0 w-2.5 h-2.5 rounded-full ${browserReady ? 'bg-emerald-400 animate-pulse' : 'bg-zinc-500'}`} />
          <div className="min-w-0">
            <div className="text-xs font-semibold text-zinc-100 truncate" title={`@${teamName}`}>@{teamName}</div>
            <p className="text-[10px] text-zinc-400 truncate">{isLocal ? 'Your Chrome' : 'Live Chromium'}</p>
          </div>
        </div>

        <div className="flex items-center gap-1 shrink-0">
          <ComputerModeChip api={computer} />
          <OpenWindowButton api={computer} />

          {/* Interactive Control Toggle */}
          {!isWindow && (
            <button
              type="button"
              onClick={() => {
                playSfx('click');
                stream.send({ action: isControlMode ? 'release_control' : 'take_control' });
                if (!isControlMode) setActiveTab('browser');
              }}
              disabled={!stream.connected}
              title={isControlMode ? 'Exit Control Mode' : 'Take Control of Bot Browser'}
              className={`p-1.5 rounded-md transition-colors cursor-pointer flex items-center gap-1 ${
                isControlMode
                  ? 'bg-cyan-500/20 text-cyan-300 border border-cyan-500/40 shadow-sm shadow-cyan-500/20 animate-pulse'
                  : 'text-zinc-400 hover:text-zinc-100 hover:bg-zinc-800'
              }`}
            >
              <MousePointerClick size={13} className={isControlMode ? 'text-cyan-400' : ''} />
              <span className="text-[10px] font-semibold whitespace-nowrap">{isControlMode ? 'Controlling' : 'Take Control'}</span>
            </button>
          )}

          <button
            type="button"
            onClick={() => {
              playSfx('click');
              setActiveTab((curr) => (curr === 'vault' ? 'browser' : 'vault'));
            }}
            title={`Saved logins (${savedCount})`}
            aria-label={`Saved logins (${savedCount})`}
            className={`p-1.5 rounded-md transition-colors cursor-pointer relative ${
              activeTab === 'vault'
                ? 'text-sky-400 bg-sky-500/10'
                : 'text-zinc-400 hover:text-zinc-100 hover:bg-zinc-800'
            }`}
          >
            <Key size={13} />
            {savedCount > 0 && (
              <span className="absolute -top-1 -right-1 min-w-3.5 h-3.5 px-0.5 bg-sky-500 text-[9px] font-bold text-zinc-950 rounded-full flex items-center justify-center">
                {savedCount}
              </span>
            )}
          </button>

          {onRefresh && (
            <button
              type="button"
              onClick={() => {
                playSfx('click');
                onRefresh();
                passwords.reload();
              }}
              title="Refresh Screenshot & Passwords"
              className="p-1.5 rounded-md text-zinc-400 hover:text-zinc-100 hover:bg-zinc-800 transition-colors cursor-pointer"
            >
              <RefreshCw size={13} />
            </button>
          )}

          <button
            type="button"
            onClick={() => {
              playSfx('click');
              onClose();
            }}
            title="Close VM Pane"
            className="p-1.5 rounded-md text-zinc-400 hover:text-zinc-100 hover:bg-zinc-800 transition-colors cursor-pointer"
          >
            <X size={15} />
          </button>
        </div>
      </div>

      {computer.state.error && (
        <div className="px-3 pt-2">
          <ComputerModeError error={computer.state.error} onDismiss={computer.dismissError} />
        </div>
      )}

      {/* Tabs */}
      <div className="flex border-b border-zinc-800 bg-zinc-900/30 text-xs font-medium">
        <button
          type="button"
          onClick={() => {
            playSfx('click');
            setActiveTab('browser');
          }}
          className={`flex-1 py-2 flex items-center justify-center gap-1.5 transition-colors border-b-2 cursor-pointer ${
            activeTab === 'browser'
              ? 'border-sky-500 text-sky-400 bg-sky-500/10'
              : 'border-transparent text-zinc-400 hover:text-zinc-200'
          }`}
        >
          <Globe size={13} />
          <span>Browser</span>
        </button>
        <button
          type="button"
          onClick={() => {
            playSfx('click');
            setActiveTab('terminal');
          }}
          className={`flex-1 py-2 flex items-center justify-center gap-1.5 transition-colors border-b-2 cursor-pointer ${
            activeTab === 'terminal'
              ? 'border-sky-500 text-sky-400 bg-sky-500/10'
              : 'border-transparent text-zinc-400 hover:text-zinc-200'
          }`}
        >
          <Terminal size={13} />
          <span>Terminal</span>
        </button>
        <button
          type="button"
          onClick={() => {
            playSfx('click');
            setActiveTab('vault');
          }}
          className={`flex-1 py-2 flex items-center justify-center gap-1.5 transition-colors border-b-2 cursor-pointer relative ${
            activeTab === 'vault'
              ? 'border-sky-500 text-sky-400 bg-sky-500/10'
              : 'border-transparent text-zinc-400 hover:text-zinc-200'
          }`}
        >
          <Key size={13} />
          <span>Logins</span>
          {savedCount > 0 && (
            <span className="text-[10px] px-1 py-0.2 rounded-full bg-zinc-800 text-zinc-300 font-mono">
              {savedCount}
            </span>
          )}
        </button>
      </div>

      {teamId && passwords.pending && <PendingLoginPrompt sandboxId={teamId} pending={passwords.pending} onDone={passwords.reload} />}

      {/* Content Area */}
      <div className="flex-1 overflow-hidden flex flex-col">
        {activeTab === 'browser' && isWindow ? (
          <div className="flex-1 flex flex-col p-3 overflow-hidden">
            <LocalBrowserNote api={computer} />
          </div>
        ) : activeTab === 'browser' ? (
          <div className="flex-1 flex flex-col p-3 gap-2 overflow-hidden">
            {/* Browser Address Bar */}
            <div className="flex items-center gap-1.5 p-1.5 bg-zinc-900 border border-zinc-800 rounded-lg text-xs">
              <Globe size={13} className="text-zinc-400 flex-shrink-0" />
              <input
                type="text"
                readOnly={!onNavigate}
                value={inputUrl}
                onChange={(e) => setInputUrl(e.target.value)}
                onKeyDown={(e) => {
                  if (e.key === 'Enter' && onNavigate && inputUrl.trim()) {
                    let target = inputUrl.trim();
                    if (!target.startsWith('http://') && !target.startsWith('https://')) {
                      target = `https://${target}`;
                    }
                    if (isControlMode) stream.send({ action: 'navigate', url: target }, true);
                    else onNavigate(target);
                  }
                }}
                placeholder="https://..."
                className="flex-1 bg-transparent text-zinc-200 font-mono text-[11px] truncate focus:outline-none"
              />

              {loginsOnThisPage > 0 && (
                <button
                  type="button"
                  onClick={() => {
                    playSfx('click');
                    setActiveTab('vault');
                  }}
                  className="flex items-center gap-1 px-1.5 py-0.5 rounded bg-emerald-500/15 text-emerald-400 border border-emerald-500/25 text-[10px] hover:bg-emerald-500/25 transition-colors cursor-pointer flex-shrink-0"
                  title={`${loginsOnThisPage} saved login${loginsOnThisPage === 1 ? '' : 's'} for this address`}
                >
                  <Key size={10} />
                  <span className="font-mono">{loginsOnThisPage} saved</span>
                </button>
              )}

              {teamId && teamId !== 'default' && (
                <a
                  href={`/sandbox/${encodeURIComponent(teamId)}`}
                  target="_blank"
                  rel="noopener"
                  className="text-zinc-400 hover:text-sky-400 p-0.5 flex-shrink-0"
                  title="Open the live sandbox browser in a new tab (shared with your bots)"
                >
                  <ExternalLink size={12} />
                </a>
              )}
            </div>

            {(stream.location?.title || pageTitle) && (
              <div className="text-[11px] text-zinc-400 truncate px-1 font-medium">
                {stream.location?.title || pageTitle}
              </div>
            )}

            {/* Interactive Control Mode Banner */}
            {isControlMode && (
              <div className="flex items-center justify-between px-2.5 py-1 rounded-lg bg-cyan-950/60 border border-cyan-500/30 text-[11px] text-cyan-300 animate-in fade-in duration-150">
                <div className="flex items-center gap-1.5">
                  <span className="w-1.5 h-1.5 rounded-full bg-cyan-400 animate-ping" />
                  <span className="font-medium">Direct Control Active</span>
                </div>
                <span className="text-[10px] text-cyan-400/80">Click & type on screen</span>
              </div>
            )}

            {/* Quick Type Input Bar (When in Control Mode) */}
            {isControlMode && (
              <form
                onSubmit={handleQuickTypeSubmit}
                className="flex items-center gap-1.5 p-1 bg-zinc-900 border border-cyan-500/40 rounded-lg text-xs"
              >
                <Keyboard size={12} className="text-cyan-400 flex-shrink-0 ml-1" />
                <input
                  type="text"
                  value={quickTypeText}
                  onChange={(e) => setQuickTypeText(e.target.value)}
                  placeholder="Type into focused field (e.g. search, input)..."
                  className="flex-1 bg-transparent text-zinc-100 text-[11px] font-mono focus:outline-none"
                />
                <button
                  type="submit"
                  disabled={!quickTypeText.trim()}
                  className="px-2 py-0.5 rounded bg-cyan-500 hover:bg-cyan-400 text-zinc-950 font-semibold text-[10px] cursor-pointer disabled:opacity-40"
                >
                  Send
                </button>
              </form>
            )}

            {/* Viewport Frame */}
            {controlError && (
              <div role="alert" className="px-2.5 py-2 rounded-lg border border-red-500/30 bg-red-950/40 text-xs text-red-300">
                {controlError}
              </div>
            )}
            <div
              ref={viewportRef}
              tabIndex={0}
              onKeyDown={handleViewportKeyDown}
              onClick={handleViewportClick}
              onWheel={handleViewportWheel}
              className={`flex-1 border rounded-xl bg-zinc-950 overflow-hidden flex items-center justify-center relative shadow-inner focus:outline-none ${
                isControlMode
                  ? 'border-cyan-500/60 ring-2 ring-cyan-500/20 cursor-crosshair'
                  : 'border-zinc-800'
              }`}
            >
              {/* Relaying feedback pill */}
              {isInteracting && (
                <div className="absolute top-2 right-2 px-2 py-0.5 rounded-full bg-zinc-950/90 border border-cyan-500/40 text-cyan-300 text-[10px] font-mono flex items-center gap-1.5 z-20 shadow-md pointer-events-none">
                  <Loader2 size={10} className="animate-spin text-cyan-400" />
                  <span>Relaying...</span>
                </div>
              )}

              {/* Click Ripple Animations */}
              {clickRipples.map((r) => (
                <span
                  key={r.id}
                  style={{ left: r.x, top: r.y }}
                  className="absolute w-5 h-5 -ml-2.5 -mt-2.5 rounded-full border-2 border-cyan-400 bg-cyan-400/30 animate-ping pointer-events-none z-30"
                />
              ))}

              {currentScreenshot ? (
                <img
                  key={stream.frame?.id}
                  ref={imageRef}
                  src={
                    currentScreenshot.startsWith('data:')
                      ? currentScreenshot
                      : `data:image/jpeg;base64,${currentScreenshot}`
                  }
                  alt="Browser Viewport"
                  onLoad={() => { if (stream.frame) stream.acknowledgeFrame(stream.frame.id); }}
                  onError={() => { if (stream.frame) stream.acknowledgeFrame(stream.frame.id); }}
                  draggable={false}
                  className="w-full h-full object-contain pointer-events-auto"
                />
              ) : (
                <div className="text-center p-6 text-zinc-600 text-xs">
                  <Globe size={32} className="mx-auto mb-2 opacity-40 text-zinc-500" />
                  <p>{browserReady ? 'Browser idle' : 'Browser unavailable'}</p>
                  <p className="text-[10px] text-zinc-500 mt-1">{browserReady ? 'Enter a URL above to open a page' : isLocal ? 'Check that Chrome is installed, then use Restart Browser below' : 'Start Docker, then use Restart Browser below'}</p>
                </div>
              )}
            </div>
          </div>
        ) : activeTab === 'terminal' ? (
          /* Terminal Tab */
          <div className="flex-1 p-3 overflow-y-auto font-mono text-xs text-zinc-300 space-y-2 bg-zinc-950">
            {terminalLogs.length === 0 ? (
              <div className="text-center text-zinc-600 text-xs py-8">
                <Terminal size={24} className="mx-auto mb-1 opacity-40" />
                <span>No commands executed in container yet</span>
              </div>
            ) : (
              terminalLogs.map((log, index) => (
                <div key={index} className="p-2 bg-zinc-900/60 border border-zinc-800/80 rounded-lg">
                  <div className="flex items-center justify-between text-sky-400 font-semibold text-[11px]">
                    <span>$ {log.command}</span>
                    {log.timestamp && (
                      <span className="text-[9px] text-zinc-500 font-normal">
                        {log.timestamp.includes('T') ? log.timestamp.split('T')[1].split('.')[0] : log.timestamp}
                      </span>
                    )}
                  </div>
                  <pre className="text-[10px] text-zinc-300 mt-1 whitespace-pre-wrap font-mono leading-relaxed">{log.output}</pre>
                </div>
              ))
            )}
          </div>
        ) : (
          /* Saved logins */
          teamId && teamId !== 'default' ? (
            <SavedPasswordsPanel
              sandboxId={teamId}
              state={passwords}
              pageUrl={pageUrl}
              onOpen={
                onNavigate
                  ? (origin) => {
                      playSfx('click');
                      onNavigate(origin);
                      setActiveTab('browser');
                    }
                  : undefined
              }
            />
          ) : (
            <p className="flex-1 p-6 text-center text-xs text-zinc-500">Open a bot, team or channel to see its saved logins.</p>
          )
        )}
      </div>

      {/* Footer Status */}
      <div className="p-2.5 border-t border-zinc-800 bg-zinc-900/60 flex items-center justify-between text-[11px] text-zinc-400">
        <div className="flex items-center gap-1.5" data-browser-footer>
          {isLocal ? (
            <Laptop size={12} className={browserReady || isWindow ? 'text-amber-300/80' : 'text-zinc-500'} />
          ) : (
            <Cpu size={12} className={browserReady ? 'text-emerald-400' : 'text-zinc-500'} />
          )}
          <span>{browserFooterLabel(computer.state.settings, browserReady)}</span>
        </div>
        {onRestartVm && !isWindow && (
          <button
            type="button"
            onClick={() => {
              playSfx('click');
              onRestartVm();
            }}
            className="hover:text-zinc-200 text-zinc-400 flex items-center gap-1 transition-colors cursor-pointer"
          >
            <RefreshCw size={11} />
            <span>Restart Browser</span>
          </button>
        )}
      </div>
    </div>
  );
};
