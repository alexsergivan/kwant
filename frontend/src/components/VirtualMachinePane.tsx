import React, { useState, useRef, useEffect, useCallback, useMemo } from 'react';
import {
  X,
  Globe,
  Terminal,
  RefreshCw,
  ExternalLink,
  Cpu,
  Key,
  Eye,
  EyeOff,
  Copy,
  Check,
  Trash2,
  Plus,
  ShieldCheck,
  Search,
  Lock,
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

export interface TerminalLogEntry {
  command: string;
  output: string;
  timestamp?: string;
}

export interface SavedCredential {
  id: string;
  domain: string;
  origin: string;
  username: string;
  password: string;
  last_used?: string;
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
  const [savedPasswords, setSavedPasswords] = useState<SavedCredential[]>([]);
  const [, setIsLoadingPasswords] = useState(false);
  const [revealedPasswords, setRevealedPasswords] = useState<Record<string, boolean>>({});
  const [copiedKey, setCopiedKey] = useState<string | null>(null);
  const [searchQuery, setSearchQuery] = useState('');
  const [isAdding, setIsAdding] = useState(false);
  const [newDomain, setNewDomain] = useState('');
  const [newUsername, setNewUsername] = useState('');
  const [newPassword, setNewPassword] = useState('');
  const [isSubmitting, setIsSubmitting] = useState(false);

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

  const loadPasswords = useCallback(async () => {
    if (!teamId || teamId === 'default') return;
    try {
      setIsLoadingPasswords(true);
      const res = await fetch(`/api/sandbox/${teamId}/passwords`);
      if (res.ok) {
        const data = await res.json();
        setSavedPasswords(data.passwords || []);
      }
    } catch (err) {
      console.error('Failed to load saved passwords:', err);
    } finally {
      setIsLoadingPasswords(false);
    }
  }, [teamId]);

  useEffect(() => {
    loadPasswords();
  }, [loadPasswords]);

  useEffect(() => {
    setInputUrl(activeUrl);
  }, [activeUrl]);

  // Extract hostname from active URL
  const activeDomain = useMemo(() => {
    if (!activeUrl || activeUrl === 'about:blank') return '';
    try {
      return new URL(activeUrl).hostname.toLowerCase();
    } catch {
      return '';
    }
  }, [activeUrl]);

  // Credentials matching active domain
  const matchingCreds = useMemo(() => {
    if (!activeDomain) return [];
    return savedPasswords.filter((p) => {
      const d = p.domain.toLowerCase();
      return activeDomain === d || activeDomain.endsWith(`.${d}`) || d.endsWith(`.${activeDomain}`);
    });
  }, [activeDomain, savedPasswords]);

  // Filtered credentials in vault
  const filteredPasswords = useMemo(() => {
    if (!searchQuery.trim()) return savedPasswords;
    const q = searchQuery.toLowerCase().trim();
    return savedPasswords.filter(
      (p) =>
        p.domain.toLowerCase().includes(q) ||
        p.username.toLowerCase().includes(q) ||
        p.origin.toLowerCase().includes(q)
    );
  }, [savedPasswords, searchQuery]);

  const handleCopy = (text: string, key: string) => {
    playSfx('click');
    navigator.clipboard.writeText(text);
    setCopiedKey(key);
    setTimeout(() => {
      setCopiedKey((curr) => (curr === key ? null : curr));
    }, 2000);
  };

  const toggleReveal = (id: string) => {
    playSfx('click');
    setRevealedPasswords((prev) => ({
      ...prev,
      [id]: !prev[id],
    }));
  };

  const handleAddCredential = async (e: React.FormEvent) => {
    e.preventDefault();
    if (!teamId || !newDomain.trim() || !newUsername.trim() || !newPassword.trim()) return;
    try {
      setIsSubmitting(true);
      let origin = newDomain.trim();
      if (!origin.startsWith('http://') && !origin.startsWith('https://')) {
        origin = `https://${origin}`;
      }
      let domain = origin.replace(/^https?:\/\//i, '').split('/')[0];
      if (domain.includes(':')) {
        domain = domain.split(':')[0];
      }

      const res = await fetch(`/api/sandbox/${teamId}/passwords`, {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({
          domain,
          origin,
          username: newUsername.trim(),
          password: newPassword.trim(),
        }),
      });

      if (res.ok) {
        playSfx('coin');
        setNewDomain('');
        setNewUsername('');
        setNewPassword('');
        setIsAdding(false);
        await loadPasswords();
      }
    } catch (err) {
      console.error('Failed to save password:', err);
    } finally {
      setIsSubmitting(false);
    }
  };

  const handleDeleteCredential = async (id: string) => {
    if (!teamId) return;
    try {
      playSfx('click');
      const res = await fetch(`/api/sandbox/${teamId}/passwords/${id}`, {
        method: 'DELETE',
      });
      if (res.ok) {
        setSavedPasswords((prev) => prev.filter((p) => p.id !== id));
      }
    } catch (err) {
      console.error('Failed to delete password:', err);
    }
  };

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
            title={`Saved Logins & Passwords (${savedPasswords.length})`}
            className={`p-1.5 rounded-md transition-colors cursor-pointer relative ${
              activeTab === 'vault'
                ? 'text-sky-400 bg-sky-500/10'
                : 'text-zinc-400 hover:text-zinc-100 hover:bg-zinc-800'
            }`}
          >
            <Key size={13} />
            {savedPasswords.length > 0 && (
              <span className="absolute -top-1 -right-1 min-w-3.5 h-3.5 px-0.5 bg-sky-500 text-[9px] font-bold text-zinc-950 rounded-full flex items-center justify-center">
                {savedPasswords.length}
              </span>
            )}
          </button>

          {onRefresh && (
            <button
              type="button"
              onClick={() => {
                playSfx('click');
                onRefresh();
                loadPasswords();
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
          {savedPasswords.length > 0 && (
            <span className="text-[10px] px-1 py-0.2 rounded-full bg-zinc-800 text-zinc-300 font-mono">
              {savedPasswords.length}
            </span>
          )}
        </button>
      </div>

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

              {matchingCreds.length > 0 && (
                <button
                  type="button"
                  onClick={() => {
                    playSfx('click');
                    setActiveTab('vault');
                    setSearchQuery(activeDomain);
                  }}
                  className="flex items-center gap-1 px-1.5 py-0.5 rounded bg-emerald-500/15 text-emerald-400 border border-emerald-500/25 text-[10px] hover:bg-emerald-500/25 transition-colors cursor-pointer flex-shrink-0"
                  title={`${matchingCreds.length} login saved for this site. Click to view credentials.`}
                >
                  <Key size={10} />
                  <span className="font-mono">{matchingCreds.length} saved</span>
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
          /* Saved Logins / Password Vault Tab */
          <div className="flex-1 flex flex-col p-3 gap-2 overflow-hidden bg-zinc-950">
            {/* Vault Controls */}
            <div className="flex items-center gap-1.5">
              <div className="flex-1 flex items-center gap-1.5 px-2 py-1 bg-zinc-900 border border-zinc-800 rounded-lg text-xs">
                <Search size={12} className="text-zinc-500 flex-shrink-0" />
                <input
                  type="text"
                  value={searchQuery}
                  onChange={(e) => setSearchQuery(e.target.value)}
                  placeholder="Search domains or users..."
                  className="w-full bg-transparent text-zinc-200 text-[11px] focus:outline-none placeholder:text-zinc-600"
                />
                {searchQuery && (
                  <button
                    type="button"
                    onClick={() => setSearchQuery('')}
                    className="text-zinc-500 hover:text-zinc-300 text-[10px]"
                  >
                    ×
                  </button>
                )}
              </div>
              <button
                type="button"
                onClick={() => {
                  playSfx('click');
                  setIsAdding((prev) => !prev);
                }}
                className={`flex items-center gap-1 px-2.5 py-1.5 rounded-lg text-[11px] font-medium transition-colors cursor-pointer ${
                  isAdding
                    ? 'bg-zinc-800 text-zinc-200'
                    : 'bg-sky-500/10 text-sky-400 border border-sky-500/20 hover:bg-sky-500/20'
                }`}
              >
                <Plus size={12} />
                <span>{isAdding ? 'Cancel' : 'Add'}</span>
              </button>
            </div>

            {/* Manual Entry Form */}
            {isAdding && (
              <form
                onSubmit={handleAddCredential}
                className="p-3 bg-zinc-900/90 border border-sky-500/30 rounded-xl space-y-2 text-xs animate-in fade-in duration-150"
              >
                <div className="text-[11px] font-semibold text-sky-400 flex items-center gap-1">
                  <Key size={12} />
                  <span>Save New Credential</span>
                </div>
                <div>
                  <label className="text-[10px] text-zinc-400 block mb-0.5">Domain or URL</label>
                  <input
                    type="text"
                    required
                    value={newDomain}
                    onChange={(e) => setNewDomain(e.target.value)}
                    placeholder="e.g. github.com or https://..."
                    className="w-full px-2 py-1 bg-zinc-950 border border-zinc-800 rounded text-[11px] text-zinc-200 focus:outline-none focus:border-sky-500 font-mono"
                  />
                </div>
                <div>
                  <label className="text-[10px] text-zinc-400 block mb-0.5">Username or Email</label>
                  <input
                    type="text"
                    required
                    value={newUsername}
                    onChange={(e) => setNewUsername(e.target.value)}
                    placeholder="user@example.com"
                    className="w-full px-2 py-1 bg-zinc-950 border border-zinc-800 rounded text-[11px] text-zinc-200 focus:outline-none focus:border-sky-500 font-mono"
                  />
                </div>
                <div>
                  <label className="text-[10px] text-zinc-400 block mb-0.5">Password</label>
                  <input
                    type="password"
                    required
                    value={newPassword}
                    onChange={(e) => setNewPassword(e.target.value)}
                    placeholder="••••••••••••"
                    className="w-full px-2 py-1 bg-zinc-950 border border-zinc-800 rounded text-[11px] text-zinc-200 focus:outline-none focus:border-sky-500 font-mono"
                  />
                </div>
                <div className="flex justify-end gap-1.5 pt-1">
                  <button
                    type="button"
                    onClick={() => setIsAdding(false)}
                    className="px-2.5 py-1 text-[11px] rounded bg-zinc-800 hover:bg-zinc-700 text-zinc-300 cursor-pointer"
                  >
                    Cancel
                  </button>
                  <button
                    type="submit"
                    disabled={isSubmitting}
                    className="px-2.5 py-1 text-[11px] rounded bg-sky-500 hover:bg-sky-400 text-zinc-950 font-semibold cursor-pointer disabled:opacity-50"
                  >
                    {isSubmitting ? 'Saving...' : 'Save Credential'}
                  </button>
                </div>
              </form>
            )}

            {/* Credential List */}
            <div className="flex-1 overflow-y-auto space-y-2 pr-0.5">
              {filteredPasswords.length === 0 ? (
                <div className="text-center py-10 px-4 text-zinc-500 text-xs">
                  <Lock size={28} className="mx-auto mb-2 opacity-30 text-zinc-400" />
                  <p className="font-medium text-zinc-400">No saved logins yet</p>
                  <p className="text-[10px] text-zinc-500 mt-1 leading-relaxed">
                    Credentials entered into website forms within the sandbox browser are automatically captured and saved here, or you can add them manually.
                  </p>
                </div>
              ) : (
                filteredPasswords.map((cred) => {
                  const isMatchingActive =
                    activeDomain &&
                    (activeDomain === cred.domain.toLowerCase() ||
                      activeDomain.endsWith(`.${cred.domain.toLowerCase()}`));
                  const isRevealed = !!revealedPasswords[cred.id];

                  return (
                    <div
                      key={cred.id}
                      className={`p-2.5 rounded-xl border transition-all text-xs bg-zinc-900/60 ${
                        isMatchingActive
                          ? 'border-emerald-500/40 shadow-sm shadow-emerald-500/5'
                          : 'border-zinc-800/80 hover:border-zinc-700'
                      }`}
                    >
                      {/* Card Header */}
                      <div className="flex items-center justify-between mb-1.5">
                        <div className="flex items-center gap-1.5 truncate">
                          <span className="font-semibold text-zinc-200 truncate">{cred.domain}</span>
                          {isMatchingActive && (
                            <span className="px-1.5 py-0.2 rounded bg-emerald-500/10 text-emerald-400 border border-emerald-500/20 text-[9px] font-mono flex-shrink-0">
                              Active Site
                            </span>
                          )}
                        </div>
                        <div className="flex items-center gap-1">
                          {onNavigate && (
                            <button
                              type="button"
                              onClick={() => {
                                playSfx('click');
                                onNavigate(cred.origin);
                                setActiveTab('browser');
                              }}
                              title="Open & Autofill in Browser"
                              className="text-zinc-400 hover:text-sky-400 p-1 rounded hover:bg-zinc-800/60 cursor-pointer"
                            >
                              <ExternalLink size={11} />
                            </button>
                          )}
                          <button
                            type="button"
                            onClick={() => handleDeleteCredential(cred.id)}
                            title="Delete credential"
                            className="text-zinc-500 hover:text-rose-400 p-1 rounded hover:bg-zinc-800/60 cursor-pointer"
                          >
                            <Trash2 size={11} />
                          </button>
                        </div>
                      </div>

                      {/* Username Row */}
                      <div className="flex items-center justify-between py-1 px-2 rounded bg-zinc-950/70 border border-zinc-800/50 mb-1">
                        <span className="text-[11px] font-mono text-zinc-300 truncate max-w-[200px]">
                          {cred.username}
                        </span>
                        <button
                          type="button"
                          onClick={() => handleCopy(cred.username, `user-${cred.id}`)}
                          title="Copy username"
                          className="text-zinc-500 hover:text-zinc-200 p-0.5 cursor-pointer ml-1"
                        >
                          {copiedKey === `user-${cred.id}` ? (
                            <Check size={11} className="text-emerald-400" />
                          ) : (
                            <Copy size={11} />
                          )}
                        </button>
                      </div>

                      {/* Password Row */}
                      <div className="flex items-center justify-between py-1 px-2 rounded bg-zinc-950/70 border border-zinc-800/50">
                        <span className="text-[11px] font-mono text-zinc-400 truncate max-w-[180px]">
                          {isRevealed ? cred.password : '••••••••••••'}
                        </span>
                        <div className="flex items-center gap-1 ml-1">
                          <button
                            type="button"
                            onClick={() => toggleReveal(cred.id)}
                            title={isRevealed ? 'Hide password' : 'Show password'}
                            className="text-zinc-500 hover:text-zinc-200 p-0.5 cursor-pointer"
                          >
                            {isRevealed ? <EyeOff size={11} /> : <Eye size={11} />}
                          </button>
                          <button
                            type="button"
                            onClick={() => handleCopy(cred.password, `pass-${cred.id}`)}
                            title="Copy password"
                            className="text-zinc-500 hover:text-zinc-200 p-0.5 cursor-pointer"
                          >
                            {copiedKey === `pass-${cred.id}` ? (
                              <Check size={11} className="text-emerald-400" />
                            ) : (
                              <Copy size={11} />
                            )}
                          </button>
                        </div>
                      </div>

                      {/* Card Footer */}
                      {cred.last_used && (
                        <div className="mt-1 text-[9px] text-zinc-500 text-right">
                          Last used {new Date(cred.last_used).toLocaleDateString()}
                        </div>
                      )}
                    </div>
                  );
                })
              )}
            </div>

            {/* Vault Security Note */}
            <div className="p-2 rounded-lg bg-zinc-900/40 border border-zinc-800/60 flex items-start gap-2 text-[10px] text-zinc-400">
              <ShieldCheck size={14} className="text-emerald-400 flex-shrink-0 mt-0.5" />
              <div className="leading-tight">
                <span className="text-zinc-300 font-medium">Automatic Autofill & Persistence</span>
                <p className="text-zinc-500 mt-0.5">
                  Saved logins and session cookies persist in the workspace vault across container restarts.
                </p>
              </div>
            </div>
          </div>
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
