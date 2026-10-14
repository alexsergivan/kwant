import React from 'react';
import { SpriteAvatar } from './SpriteAvatar';

export interface Kwant {
  id: string;
  name: string;
  title: string;
  persona_prompt: string;
  avatar_sprite: string;
  preferred_model: string;
  driver_type: string;
}

export interface Channel {
  id: string;
  name: string;
  description: string | null;
  lead_kwant_id: string;
}

interface KwantSidebarProps {
  kwants: Kwant[];
  channels: Channel[];
  selectedId: string;
  isChannel: boolean;
  onSelectKwant: (kwant: Kwant) => void;
  onSelectChannel: (channel: Channel) => void;
  onSummonKwant: () => void;
}

export const KwantSidebar: React.FC<KwantSidebarProps> = ({
  kwants,
  channels,
  selectedId,
  isChannel,
  onSelectKwant,
  onSelectChannel,
  onSummonKwant,
}) => {
  return (
    <aside className="w-64 bg-arcade-dark border-r-4 border-black flex flex-col h-full select-none">
      {/* Channels / Guild Section */}
      <div className="p-3 border-b-2 border-arcade-border">
        <div className="flex items-center justify-between text-arcade-amber font-pixel text-[9px] mb-2 px-1">
          <span># GUILD ROOMS</span>
        </div>
        <div className="space-y-1">
          {channels.map((chan) => {
            const active = isChannel && selectedId === chan.id;
            return (
              <button
                key={chan.id}
                onClick={() => onSelectChannel(chan)}
                className={`w-full text-left px-2 py-1.5 font-retro text-xs border border-transparent flex items-center gap-2 transition-all ${
                  active
                    ? 'bg-arcade-panel border-arcade-cyan text-arcade-cyan font-bold shadow-pixel-sm'
                    : 'text-zinc-400 hover:text-white hover:bg-zinc-900'
                }`}
              >
                <span className="text-arcade-amber">#</span>
                <span className="truncate">{chan.name}</span>
              </button>
            );
          })}
        </div>
      </div>

      {/* Kwants Party Roster */}
      <div className="flex-1 overflow-y-auto p-3">
        <div className="flex items-center justify-between text-arcade-cyan font-pixel text-[9px] mb-3 px-1">
          <span>⚔️ PARTY ROSTER</span>
          <span className="text-zinc-500">[{kwants.length}]</span>
        </div>

        <div className="space-y-2">
          {kwants.map((k) => {
            const active = !isChannel && selectedId === k.id;
            return (
              <div
                key={k.id}
                onClick={() => onSelectKwant(k)}
                className={`p-2 border-2 cursor-pointer transition-all flex items-center gap-3 ${
                  active
                    ? 'bg-arcade-panel border-arcade-amber shadow-pixel-amber'
                    : 'bg-zinc-900/60 border-zinc-800 hover:border-zinc-700'
                }`}
              >
                <SpriteAvatar sprite={k.avatar_sprite} size={36} />
                <div className="flex-1 min-w-0">
                  <div className="flex items-center justify-between">
                    <h3 className="font-pixel text-[10px] text-white truncate">{k.name}</h3>
                    <span className="font-pixel text-[8px] text-arcade-green">[READY]</span>
                  </div>
                  <p className="font-retro text-[10px] text-zinc-400 truncate mt-0.5">{k.title}</p>
                  <div className="flex items-center gap-1 mt-1">
                    <span className="font-pixel text-[7px] bg-black text-arcade-amber px-1 border border-zinc-700">
                      {k.preferred_model.toUpperCase()}
                    </span>
                  </div>
                </div>
              </div>
            );
          })}
        </div>
      </div>

      {/* Footer / Summon Button */}
      <div className="p-3 border-t-2 border-arcade-border bg-arcade-panel">
        <button
          onClick={onSummonKwant}
          className="w-full pixel-btn pixel-btn-cyan text-[9px]"
        >
          + SUMMON KWANT
        </button>
      </div>
    </aside>
  );
};
