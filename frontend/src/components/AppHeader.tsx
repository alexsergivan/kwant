import React from 'react';
import { Settings, Volume2, VolumeX, Cpu } from 'lucide-react';
import { ModernAvatar } from './ModernAvatar';
import { isSfxMuted, toggleSfxMute, playSfx } from '../audio/chiptune';

interface AppHeaderProps {
  onOpenSettings: () => void;
  tokensUsed: number;
  activeModel?: string;
}

type LayaState = 'unknown' | 'external' | 'installing' | 'starting' | 'online' | 'offline' | 'failed';

interface LayaStatus {
  state: LayaState;
  detail: string;
  url: string;
  managed: boolean;
}

const useLayaStatus = (): LayaStatus | null => {
  const [status, setStatus] = React.useState<LayaStatus | null>(null);
  React.useEffect(() => {
    let cancelled = false;
    let timer: ReturnType<typeof setTimeout>;
    const poll = async () => {
      let next: LayaStatus | null = null;
      try {
        const res = await fetch('/api/laya/status');
        if (res.ok) next = await res.json();
      } catch {
        // server restarting; keep the last known status
      }
      if (cancelled) return;
      if (next) setStatus(next);
      const up = next?.state === 'online' || next?.state === 'external';
      timer = setTimeout(poll, up ? 20000 : 4000);
    };
    poll();
    return () => {
      cancelled = true;
      clearTimeout(timer);
    };
  }, []);
  return status;
};

const layaPill = (status: LayaStatus | null, activeModel: string) => {
  const ready = activeModel === 'auto' ? 'Laya • Dynamic Effort' : `Laya ➔ ${activeModel}`;
  switch (status?.state) {
    case 'online':
    case 'external':
      return { dot: 'bg-emerald-400', label: ready };
    case 'installing':
      return { dot: 'bg-amber-400 animate-pulse', label: 'Laya installing… • keyword routing' };
    case 'starting':
      return { dot: 'bg-amber-400 animate-pulse', label: 'Laya loading model… • keyword routing' };
    case 'failed':
    case 'offline':
      return { dot: 'bg-rose-500', label: 'Laya offline • keyword routing' };
    default:
      return { dot: 'bg-zinc-500 animate-pulse', label: 'Laya…' };
  }
};

export const AppHeader: React.FC<AppHeaderProps> = ({
  onOpenSettings,
  tokensUsed,
  activeModel = 'auto',
}) => {
  const [sfxMuted, setSfxMuted] = React.useState(isSfxMuted());
  const laya = useLayaStatus();
  const pill = layaPill(laya, activeModel);
  const pillTitle = laya ? `${laya.detail || laya.state}\n${laya.url}${laya.managed ? ' (managed by Kwant)' : ''}` : 'Checking Laya…';

  const handleToggleSfx = () => {
    const next = toggleSfxMute();
    setSfxMuted(next);
  };

  return (
    <header className="h-14 border-b border-zinc-800/80 bg-zinc-900/70 backdrop-blur-md px-4 flex items-center justify-between select-none z-30">
      {/* Brand & Fleet status */}
      <div className="flex items-center gap-3">
        <div className="flex items-center gap-2 font-semibold text-sm tracking-tight text-zinc-100">
          <ModernAvatar role="brand" name="Kwant" size={30} />
          <span className="text-[15px]">Kwant</span>
        </div>
        <span className="text-[11px] font-medium text-zinc-500 hidden sm:inline-block">
          your team of super-intelligent bots (kwants)
        </span>
      </div>

      {/* Center: Laya Decision Engine pill */}
      <div
        className="hidden md:flex items-center gap-2 bg-zinc-950/60 border border-zinc-800 px-3 py-1 rounded-full text-xs text-zinc-400 shadow-inner"
        title={pillTitle}
      >
        <span className={`w-2 h-2 rounded-full ${pill.dot}`} />
        <span className="text-zinc-500 font-medium">ROUTER:</span>
        <span className="text-zinc-200 font-semibold flex items-center gap-1">
          <Cpu size={13} className="text-sky-400" />
          <span>{pill.label}</span>
        </span>
      </div>

      {/* Right Controls */}
      <div className="flex items-center gap-2">
        {/* Token / Usage Counter */}
        <div className="hidden lg:flex items-center gap-1.5 px-2.5 py-1 rounded-md text-xs text-zinc-400 bg-zinc-800/40 border border-zinc-800">
          <span className="text-zinc-500 font-mono text-[11px]">Usage:</span>
          <span className="font-mono text-zinc-200 font-medium">{tokensUsed} tk</span>
        </div>

        {/* Audio Toggle */}
        <button
          onClick={handleToggleSfx}
          className="p-1.5 rounded-lg text-zinc-400 hover:text-zinc-100 hover:bg-zinc-800/80 transition-colors"
          title={sfxMuted ? 'Unmute UI sounds' : 'Mute UI sounds'}
        >
          {sfxMuted ? <VolumeX size={17} /> : <Volume2 size={17} />}
        </button>

        {/* Settings Button */}
        <button
          onClick={() => {
            playSfx('click');
            onOpenSettings();
          }}
          className="flex items-center gap-1.5 px-2.5 py-1.5 rounded-lg text-xs font-medium text-zinc-200 bg-zinc-800 hover:bg-zinc-700/80 border border-zinc-700/60 transition-all hover:border-zinc-600 shadow-sm"
        >
          <Settings size={14} className="text-zinc-400" />
          <span>Settings</span>
        </button>
      </div>
    </header>
  );
};
