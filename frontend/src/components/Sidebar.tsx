import { Hash, Plus, CheckCircle2 } from 'lucide-react';
import { ModernAvatar } from './ModernAvatar';

export interface Kwant {
  id: string;
  name: string;
  title: string;
  persona_prompt: string;
  avatar_sprite: string;
  driver_type: string;
  preferred_model: string;
}

export interface Channel {
  id: string;
  name: string;
  description?: string;
  lead_kwant_id: string;
  member_kwants: string[];
  routing_mode: string;
}

interface SidebarProps {
  kwants: Kwant[];
  channels: Channel[];
  selectedId: string;
  isChannel: boolean;
  onSelectKwant: (kwant: Kwant) => void;
  onSelectChannel: (channel: Channel) => void;
  onSummonKwant: () => void;
}

export const Sidebar: React.FC<SidebarProps> = ({
  kwants,
  channels,
  selectedId,
  isChannel,
  onSelectKwant,
  onSelectChannel,
  onSummonKwant,
}) => {
  return (
    <aside className="w-64 border-r border-zinc-800/80 bg-zinc-900/40 flex flex-col h-full select-none flex-shrink-0">
      {/* Top Header & New Agent Action */}
      <div className="p-3 border-b border-zinc-800/60 flex items-center justify-between">
        <span className="text-xs font-semibold uppercase tracking-wider text-zinc-400">Workspace</span>
        <button
          onClick={onSummonKwant}
          className="flex items-center gap-1 px-2 py-1 text-xs font-medium text-sky-400 hover:text-sky-300 hover:bg-sky-500/10 rounded-md border border-sky-500/20 transition-all"
        >
          <Plus size={13} />
          <span>New Agent</span>
        </button>
      </div>

      <div className="flex-1 overflow-y-auto p-2 space-y-4">
        {/* Channels Section */}
        <div>
          <div className="px-2 py-1 text-[11px] font-semibold text-zinc-500 uppercase tracking-wider">
            Channels
          </div>
          <div className="mt-1 space-y-0.5">
            {channels.map((c) => {
              const active = isChannel && selectedId === c.id;
              return (
                <button
                  key={c.id}
                  onClick={() => onSelectChannel(c)}
                  className={`w-full flex items-center gap-2.5 px-2.5 py-1.5 rounded-lg text-xs font-medium transition-all text-left ${
                    active
                      ? 'bg-zinc-800 text-zinc-100 shadow-sm border border-zinc-700/50'
                      : 'text-zinc-400 hover:text-zinc-200 hover:bg-zinc-800/50'
                  }`}
                >
                  <Hash size={15} className={active ? 'text-sky-400' : 'text-zinc-500'} />
                  <span className="truncate">{c.name}</span>
                </button>
              );
            })}
          </div>
        </div>

        {/* AI Agents Fleet Section */}
        <div>
          <div className="px-2 py-1 flex items-center justify-between text-[11px] font-semibold text-zinc-500 uppercase tracking-wider">
            <span>Agent Fleet ({kwants.length})</span>
          </div>
          <div className="mt-1 space-y-1">
            {kwants.map((k) => {
              const active = !isChannel && selectedId === k.id;
              return (
                <button
                  key={k.id}
                  onClick={() => onSelectKwant(k)}
                  className={`w-full flex items-center gap-3 p-2 rounded-xl text-left transition-all ${
                    active
                      ? 'bg-zinc-800/90 text-zinc-100 shadow-sm border border-zinc-700/60'
                      : 'text-zinc-400 hover:text-zinc-200 hover:bg-zinc-800/40'
                  }`}
                >
                  <ModernAvatar
                    role={k.avatar_sprite}
                    name={k.name}
                    size={32}
                    isOnline={true}
                  />
                  <div className="flex-1 min-w-0">
                    <div className="flex items-center justify-between gap-1">
                      <span className="text-xs font-semibold text-zinc-200 truncate">{k.name}</span>
                      <span className="text-[10px] text-zinc-500 font-mono uppercase bg-zinc-900 px-1 py-0.2 rounded border border-zinc-800">
                        {k.preferred_model === 'auto' ? 'Laya' : k.preferred_model.split('-')[0]}
                      </span>
                    </div>
                    <p className="text-[11px] text-zinc-400 truncate mt-0.5">{k.title}</p>
                  </div>
                </button>
              );
            })}
          </div>
        </div>
      </div>

      {/* Footer Host Bridge status */}
      <div className="p-3 border-t border-zinc-800/60 bg-zinc-950/40 text-[11px] text-zinc-500 flex items-center justify-between">
        <span className="flex items-center gap-1.5">
          <CheckCircle2 size={13} className="text-emerald-400" />
          <span>Local CLI Fleet Active</span>
        </span>
      </div>
    </aside>
  );
};
