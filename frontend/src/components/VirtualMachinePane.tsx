import React, { useState } from 'react';
import { X, Globe, Terminal, RefreshCw, ExternalLink, Cpu } from 'lucide-react';
import { playSfx } from '../audio/chiptune';

export interface TerminalLogEntry {
  command: string;
  output: string;
  timestamp?: string;
}

export interface VirtualMachinePaneProps {
  isOpen: boolean;
  onClose: () => void;
  teamName?: string;
  activeUrl?: string;
  pageTitle?: string;
  screenshotBase64?: string | null;
  isRunning?: boolean;
  terminalLogs?: TerminalLogEntry[];
  onRefresh?: () => void;
  onRestartVm?: () => void;
}

export const VirtualMachinePane: React.FC<VirtualMachinePaneProps> = ({
  isOpen,
  onClose,
  teamName = 'Team',
  activeUrl = '',
  pageTitle = '',
  screenshotBase64,
  isRunning = true,
  terminalLogs = [],
  onRefresh,
  onRestartVm,
}) => {
  const [activeTab, setActiveTab] = useState<'browser' | 'terminal'>('browser');

  if (!isOpen) return null;

  return (
    <div className="w-96 border-l border-zinc-800 bg-zinc-950 flex flex-col h-full shadow-2xl z-30 select-none animate-in slide-in-from-right duration-200">
      {/* Top Header */}
      <div className="h-14 border-b border-zinc-800 px-4 flex items-center justify-between bg-zinc-900/60">
        <div className="flex items-center gap-2">
          <div className={`w-2.5 h-2.5 rounded-full ${isRunning ? 'bg-emerald-400 animate-pulse' : 'bg-zinc-500'}`} />
          <div>
            <div className="text-xs font-semibold text-zinc-100 flex items-center gap-1.5">
              <span>Sandbox VM</span>
              <span className="text-[10px] font-mono text-zinc-400">(@{teamName})</span>
            </div>
            <p className="text-[10px] text-zinc-400">Debian 12 • Chromium CDP</p>
          </div>
        </div>

        <div className="flex items-center gap-1">
          {onRefresh && (
            <button
              type="button"
              onClick={() => {
                playSfx('click');
                onRefresh();
              }}
              title="Refresh Screenshot"
              className="p-1 rounded-md text-zinc-400 hover:text-zinc-100 hover:bg-zinc-800 transition-colors cursor-pointer"
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
            className="p-1 rounded-md text-zinc-400 hover:text-zinc-100 hover:bg-zinc-800 transition-colors cursor-pointer"
          >
            <X size={15} />
          </button>
        </div>
      </div>

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
          <span>Terminal Logs</span>
        </button>
      </div>

      {/* Content Area */}
      <div className="flex-1 overflow-hidden flex flex-col">
        {activeTab === 'browser' ? (
          <div className="flex-1 flex flex-col p-3 gap-2 overflow-hidden">
            {/* Browser Address Bar */}
            <div className="flex items-center gap-2 p-1.5 bg-zinc-900 border border-zinc-800 rounded-lg text-xs">
              <Globe size={13} className="text-zinc-400 flex-shrink-0" />
              <input
                type="text"
                readOnly
                value={activeUrl || 'about:blank'}
                className="flex-1 bg-transparent text-zinc-200 font-mono text-[11px] truncate focus:outline-none"
              />
              {activeUrl && activeUrl !== 'about:blank' && (
                <a
                  href={activeUrl}
                  target="_blank"
                  rel="noopener noreferrer"
                  className="text-zinc-400 hover:text-sky-400 p-0.5"
                  title="Open in new window"
                >
                  <ExternalLink size={12} />
                </a>
              )}
            </div>

            {pageTitle && (
              <div className="text-[11px] text-zinc-400 truncate px-1 font-medium">
                {pageTitle}
              </div>
            )}

            {/* Viewport Frame */}
            <div className="flex-1 border border-zinc-800 rounded-xl bg-zinc-950 overflow-hidden flex items-center justify-center relative shadow-inner">
              {screenshotBase64 ? (
                <img
                  src={
                    screenshotBase64.startsWith('data:')
                      ? screenshotBase64
                      : `data:image/jpeg;base64,${screenshotBase64}`
                  }
                  alt="Browser Viewport"
                  className="w-full h-full object-contain"
                />
              ) : (
                <div className="text-center p-6 text-zinc-600 text-xs">
                  <Globe size={32} className="mx-auto mb-2 opacity-40 text-zinc-500" />
                  <p>Browser idle</p>
                  <p className="text-[10px] text-zinc-500 mt-1">Bot will navigate here when needed</p>
                </div>
              )}
            </div>
          </div>
        ) : (
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
        )}
      </div>

      {/* Footer Status */}
      <div className="p-2.5 border-t border-zinc-800 bg-zinc-900/60 flex items-center justify-between text-[11px] text-zinc-400">
        <div className="flex items-center gap-1.5">
          <Cpu size={12} className={isRunning ? 'text-emerald-400' : 'text-zinc-500'} />
          <span>{isRunning ? 'Container Active' : 'Container Inactive'}</span>
        </div>
        {onRestartVm && (
          <button
            type="button"
            onClick={() => {
              playSfx('click');
              onRestartVm();
            }}
            className="hover:text-zinc-200 text-zinc-400 flex items-center gap-1 transition-colors cursor-pointer"
          >
            <RefreshCw size={11} />
            <span>Restart VM</span>
          </button>
        )}
      </div>
    </div>
  );
};
