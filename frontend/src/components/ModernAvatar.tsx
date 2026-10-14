import React from 'react';
import { Layers, Code2, Terminal, Shield, Zap, User, Bot } from 'lucide-react';

interface ModernAvatarProps {
  role?: string;
  name?: string;
  size?: number; // pixel diameter
  isOnline?: boolean;
}

export const ModernAvatar: React.FC<ModernAvatarProps> = ({
  role = 'default',
  name = '',
  size = 36,
  isOnline,
}) => {
  const norm = (role + ' ' + name).toLowerCase();

  let gradient = 'from-zinc-700 to-zinc-900 text-zinc-300';
  let Icon = Bot;

  if (norm.includes('user') || norm.includes('player')) {
    gradient = 'from-blue-600 to-indigo-700 text-white';
    Icon = User;
  } else if (norm.includes('architect') || norm.includes('cyber_mage') || norm.includes('mage')) {
    gradient = 'from-violet-600 to-purple-800 text-violet-100';
    Icon = Layers;
  } else if (norm.includes('tech') || norm.includes('ninja') || norm.includes('pixel') || norm.includes('lead')) {
    gradient = 'from-sky-500 to-blue-700 text-sky-100';
    Icon = Code2;
  } else if (norm.includes('devops') || norm.includes('terminal') || norm.includes('knight') || norm.includes('shell')) {
    gradient = 'from-emerald-500 to-teal-700 text-emerald-100';
    Icon = Terminal;
  } else if (norm.includes('security') || norm.includes('shield')) {
    gradient = 'from-amber-500 to-orange-700 text-amber-100';
    Icon = Shield;
  } else if (norm.includes('fast') || norm.includes('grok')) {
    gradient = 'from-rose-500 to-pink-700 text-rose-100';
    Icon = Zap;
  }

  const iconSize = Math.max(14, Math.round(size * 0.48));

  return (
    <div className="relative inline-flex flex-shrink-0" style={{ width: size, height: size }}>
      <div
        className={`w-full h-full rounded-xl bg-gradient-to-br ${gradient} flex items-center justify-center shadow-sm border border-white/10`}
      >
        <Icon size={iconSize} strokeWidth={2.2} />
      </div>
      {isOnline !== undefined && (
        <span
          className={`absolute -bottom-0.5 -right-0.5 w-2.5 h-2.5 rounded-full ring-2 ring-zinc-950 ${
            isOnline ? 'bg-emerald-400' : 'bg-zinc-600'
          }`}
        />
      )}
    </div>
  );
};
