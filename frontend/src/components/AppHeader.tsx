import React from 'react';
import { Settings, Volume2, VolumeX, Cpu } from 'lucide-react';
import { ModernAvatar } from './ModernAvatar';
import { isSfxMuted, toggleSfxMute, playSfx } from '../audio/chiptune';

interface AppHeaderProps {
  onOpenSettings: () => void;
  tokensUsed: number;
  activeModel?: string;
}

export const AppHeader: React.FC<AppHeaderProps> = ({
  onOpenSettings,
  tokensUsed,
  activeModel = 'auto',
}) => {
  const [sfxMuted, setSfxMuted] = React.useState(isSfxMuted());

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
        <span className="text-[11px] font-medium bg-zinc-800 text-zinc-300 px-2 py-0.5 rounded-full border border-zinc-700/60 hidden sm:inline-block">
          AI Fleet
        </span>
      </div>

      {/* Center: Laya Decision Engine pill */}
      <div className="hidden md:flex items-center gap-2 bg-zinc-950/60 border border-zinc-800 px-3 py-1 rounded-full text-xs text-zinc-400 shadow-inner">
        <span className="w-2 h-2 rounded-full bg-emerald-400 animate-pulse" />
        <span className="text-zinc-500 font-medium">ROUTER:</span>
        <span className="text-zinc-200 font-semibold flex items-center gap-1">
          <Cpu size={13} className="text-sky-400" />
          <span>{activeModel === 'auto' ? 'Laya (33ms) • Dynamic Effort' : `Laya ➔ ${activeModel}`}</span>
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
