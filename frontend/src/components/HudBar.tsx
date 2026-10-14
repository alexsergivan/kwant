import React from 'react';
import { isSfxMuted, toggleSfxMute } from '../audio/chiptune';

interface HudBarProps {
  crtEnabled: boolean;
  onToggleCrt: () => void;
  tokensUsed: number;
  totalTokensLimit?: number;
  activeModel?: string;
  isLayaOnline?: boolean;
}

export const HudBar: React.FC<HudBarProps> = ({
  crtEnabled,
  onToggleCrt,
  tokensUsed,
  totalTokensLimit = 4096,
  activeModel = 'grok',
  isLayaOnline = true,
}) => {
  const [sfxMuted, setSfxMuted] = React.useState(isSfxMuted());

  const handleToggleSfx = () => {
    const nextState = toggleSfxMute();
    setSfxMuted(nextState);
  };

  const manaPercentage = Math.min(100, Math.round((tokensUsed / totalTokensLimit) * 100));

  return (
    <header className="pixel-box bg-arcade-panel px-4 py-3 border-b-4 border-black flex flex-wrap items-center justify-between gap-4 z-40">
      {/* Brand & Level */}
      <div className="flex items-center gap-3">
        <div className="bg-arcade-amber text-black font-pixel text-xs px-2 py-1 border-2 border-black shadow-pixel-sm">
          KWNTS
        </div>
        <div className="font-pixel text-[10px] text-arcade-cyan tracking-wider hidden sm:block">
          LVL. 99 PARTY
        </div>
      </div>

      {/* RPG Gauges */}
      <div className="flex items-center gap-6 font-pixel text-[9px]">
        {/* HP Gauge */}
        <div className="flex items-center gap-2">
          <span className="text-red-400">HP:</span>
          <div className="w-24 h-3 bg-black border border-zinc-700 relative overflow-hidden">
            <div className="h-full bg-arcade-green w-full animate-pulse" />
          </div>
          <span className="text-zinc-400 hidden md:inline">100%</span>
        </div>

        {/* MP (Mana / Token) Gauge */}
        <div className="flex items-center gap-2">
          <span className="text-arcade-cyan">MP:</span>
          <div className="w-24 h-3 bg-black border border-zinc-700 relative overflow-hidden">
            <div
              className="h-full bg-arcade-cyan transition-all duration-300"
              style={{ width: `${Math.max(8, manaPercentage)}%` }}
            />
          </div>
          <span className="text-zinc-400 hidden md:inline">{tokensUsed} TKN</span>
        </div>

        {/* Active Router indicator */}
        <div className="hidden lg:flex items-center gap-1 bg-black/60 px-2 py-1 border border-zinc-700">
          <span className="w-2 h-2 rounded-none bg-arcade-amber inline-block animate-ping mr-1" />
          <span className="text-zinc-400">ROUTER:</span>
          <span className="text-arcade-amber uppercase">{isLayaOnline ? 'LAYA (33ms)' : activeModel}</span>
        </div>
      </div>

      {/* Control Switches */}
      <div className="flex items-center gap-2">
        <button
          onClick={handleToggleSfx}
          className={`pixel-btn ${sfxMuted ? 'pixel-btn-dark' : 'pixel-btn-amber'} text-[9px]`}
          title="Toggle 8-bit sound effects"
        >
          {sfxMuted ? '🔇 MUTE' : '🔊 SFX'}
        </button>

        <button
          onClick={onToggleCrt}
          className={`pixel-btn ${crtEnabled ? 'pixel-btn-cyan' : 'pixel-btn-dark'} text-[9px]`}
          title="Toggle retro CRT scanlines"
        >
          {crtEnabled ? '📺 CRT: ON' : '📺 CRT: OFF'}
        </button>
      </div>
    </header>
  );
};
