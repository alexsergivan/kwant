import React from 'react';

interface ModernAvatarProps {
  role?: string;
  name?: string;
  size?: number; // pixel diameter
  isOnline?: boolean;
  className?: string;
}

export const ModernAvatar: React.FC<ModernAvatarProps> = ({
  role = 'default',
  name = '',
  size = 36,
  isOnline,
  className = '',
}) => {
  const norm = (role + ' ' + name).toLowerCase();

  // Determine avatar type and theme border/bg
  let type: 'user' | 'architect' | 'techlead' | 'devops' | 'security' | 'fast' | 'bot' = 'bot';
  let borderClass = 'border-zinc-700/60';

  if (norm.includes('user') || norm.includes('player') || norm.includes('hero')) {
    type = 'user';
    borderClass = 'border-indigo-500/40 shadow-indigo-500/10';
  } else if (norm.includes('architect') || norm.includes('cyber_mage') || norm.includes('mage')) {
    type = 'architect';
    borderClass = 'border-violet-500/40 shadow-violet-500/10';
  } else if (norm.includes('tech') || norm.includes('ninja') || norm.includes('pixel') || norm.includes('lead')) {
    type = 'techlead';
    borderClass = 'border-sky-500/40 shadow-sky-500/10';
  } else if (norm.includes('devops') || norm.includes('terminal') || norm.includes('knight') || norm.includes('shell') || norm.includes('infra')) {
    type = 'devops';
    borderClass = 'border-emerald-500/40 shadow-emerald-500/10';
  } else if (norm.includes('security') || norm.includes('shield') || norm.includes('guard')) {
    type = 'security';
    borderClass = 'border-amber-500/40 shadow-amber-500/10';
  } else if (norm.includes('fast') || norm.includes('grok') || norm.includes('zap') || norm.includes('speed')) {
    type = 'fast';
    borderClass = 'border-rose-500/40 shadow-rose-500/10';
  }

  // 16x16 Pixel Art SVG Sprite Renderer
  const renderPixelSprite = () => {
    switch (type) {
      case 'architect':
        return (
          <svg
            viewBox="0 0 16 16"
            className="w-full h-full"
            style={{ imageRendering: 'pixelated', shapeRendering: 'crispEdges' }}
          >
            {/* Background */}
            <rect width="16" height="16" fill="#1b102e" />
            {/* Sorcerer Hat & Crest */}
            <rect x="6" y="1" width="4" height="2" fill="#7b2cbf" />
            <rect x="5" y="3" width="6" height="2" fill="#9d4edd" />
            <rect x="3" y="5" width="10" height="1" fill="#f72585" />
            <rect x="7" y="2" width="2" height="1" fill="#e0aaff" />
            {/* Face & Glowing Visor */}
            <rect x="5" y="6" width="6" height="4" fill="#ffd166" />
            <rect x="6" y="7" width="2" height="1" fill="#00f5d4" />
            <rect x="9" y="7" width="2" height="1" fill="#00f5d4" />
            <rect x="7" y="8" width="2" height="1" fill="#c77dff" />
            {/* Robe */}
            <rect x="4" y="10" width="8" height="5" fill="#3c096c" />
            <rect x="6" y="10" width="4" height="5" fill="#5a189a" />
            {/* Staff with Energy Orb */}
            <rect x="13" y="3" width="1" height="12" fill="#b5179e" />
            <rect x="12" y="2" width="3" height="2" fill="#00f5d4" />
            <rect x="13" y="2" width="1" height="1" fill="#ffffff" />
          </svg>
        );

      case 'techlead':
        return (
          <svg
            viewBox="0 0 16 16"
            className="w-full h-full"
            style={{ imageRendering: 'pixelated', shapeRendering: 'crispEdges' }}
          >
            {/* Background */}
            <rect width="16" height="16" fill="#0e1726" />
            {/* Cyber Ninja Hood */}
            <rect x="4" y="2" width="8" height="5" fill="#1f2937" />
            <rect x="5" y="1" width="6" height="1" fill="#374151" />
            {/* Mask & Eyes */}
            <rect x="4" y="5" width="8" height="3" fill="#111827" />
            <rect x="5" y="5" width="6" height="2" fill="#ffd166" />
            <rect x="6" y="5" width="2" height="1" fill="#ef233c" />
            <rect x="9" y="5" width="2" height="1" fill="#ef233c" />
            {/* Torso & Tech Armor */}
            <rect x="4" y="8" width="8" height="7" fill="#1f2937" />
            <rect x="6" y="8" width="4" height="3" fill="#0284c7" />
            <rect x="7" y="11" width="2" height="4" fill="#38bdf8" />
            {/* Cyber Katana on back */}
            <rect x="2" y="2" width="1" height="12" fill="#e2e8f0" />
            <rect x="1" y="11" width="3" height="2" fill="#f59e0b" />
          </svg>
        );

      case 'devops':
        return (
          <svg
            viewBox="0 0 16 16"
            className="w-full h-full"
            style={{ imageRendering: 'pixelated', shapeRendering: 'crispEdges' }}
          >
            {/* Background */}
            <rect width="16" height="16" fill="#061e17" />
            {/* CRT Terminal Helmet */}
            <rect x="3" y="2" width="10" height="7" fill="#334155" />
            <rect x="4" y="3" width="8" height="5" fill="#022c22" />
            {/* Phosphor Green Console Screen (>_) */}
            <rect x="5" y="4" width="1" height="1" fill="#10b981" />
            <rect x="6" y="5" width="1" height="1" fill="#10b981" />
            <rect x="5" y="6" width="1" height="1" fill="#10b981" />
            <rect x="8" y="6" width="3" height="1" fill="#34d399" />
            {/* Armored Chassis */}
            <rect x="4" y="9" width="8" height="6" fill="#475569" />
            <rect x="6" y="10" width="4" height="4" fill="#f59e0b" />
            <rect x="7" y="11" width="2" height="2" fill="#fbbf24" />
            {/* Server Shield with Status LED */}
            <rect x="1" y="6" width="2" height="8" fill="#64748b" />
            <rect x="1" y="7" width="1" height="1" fill="#34d399" />
            <rect x="1" y="9" width="1" height="1" fill="#f59e0b" />
          </svg>
        );

      case 'security':
        return (
          <svg
            viewBox="0 0 16 16"
            className="w-full h-full"
            style={{ imageRendering: 'pixelated', shapeRendering: 'crispEdges' }}
          >
            {/* Background */}
            <rect width="16" height="16" fill="#201505" />
            {/* Sentinel Helm */}
            <rect x="4" y="2" width="8" height="6" fill="#d97706" />
            <rect x="6" y="1" width="4" height="2" fill="#fbbf24" />
            {/* Visor & Optic Sensor */}
            <rect x="5" y="4" width="6" height="2" fill="#451a03" />
            <rect x="7" y="4" width="2" height="2" fill="#38bdf8" />
            <rect x="8" y="4" width="1" height="1" fill="#ffffff" />
            {/* Golden Aegis Armor */}
            <rect x="4" y="8" width="8" height="7" fill="#b45309" />
            <rect x="6" y="9" width="4" height="5" fill="#f59e0b" />
            {/* Firewall Cross on Chest */}
            <rect x="7" y="10" width="2" height="3" fill="#ef4444" />
            <rect x="6" y="11" width="4" height="1" fill="#ef4444" />
          </svg>
        );

      case 'fast':
        return (
          <svg
            viewBox="0 0 16 16"
            className="w-full h-full"
            style={{ imageRendering: 'pixelated', shapeRendering: 'crispEdges' }}
          >
            {/* Background */}
            <rect width="16" height="16" fill="#240716" />
            {/* Spiked Neon Hair */}
            <rect x="4" y="1" width="2" height="3" fill="#f43f5e" />
            <rect x="7" y="1" width="2" height="4" fill="#fb7185" />
            <rect x="10" y="2" width="2" height="3" fill="#f43f5e" />
            {/* Headset & Visor */}
            <rect x="3" y="4" width="1" height="4" fill="#38bdf8" />
            <rect x="12" y="4" width="1" height="4" fill="#38bdf8" />
            <rect x="4" y="4" width="8" height="4" fill="#ffd166" />
            <rect x="5" y="5" width="6" height="2" fill="#06b6d4" />
            <rect x="6" y="5" width="4" height="1" fill="#ffffff" />
            {/* Cyber Jacket with Lightning Bolt */}
            <rect x="4" y="8" width="8" height="7" fill="#be123c" />
            <rect x="7" y="9" width="2" height="2" fill="#facc15" />
            <rect x="6" y="11" width="2" height="2" fill="#facc15" />
            <rect x="7" y="12" width="2" height="2" fill="#facc15" />
          </svg>
        );

      case 'user':
        return (
          <svg
            viewBox="0 0 16 16"
            className="w-full h-full"
            style={{ imageRendering: 'pixelated', shapeRendering: 'crispEdges' }}
          >
            {/* Background */}
            <rect width="16" height="16" fill="#111827" />
            {/* Hair */}
            <rect x="4" y="2" width="8" height="3" fill="#854d0e" />
            {/* Neon Headband */}
            <rect x="4" y="5" width="8" height="1" fill="#ec4899" />
            <rect x="12" y="6" width="2" height="2" fill="#ec4899" />
            {/* Face & Eyes */}
            <rect x="4" y="6" width="8" height="4" fill="#fde047" />
            <rect x="6" y="7" width="1" height="1" fill="#0f172a" />
            <rect x="9" y="7" width="1" height="1" fill="#0f172a" />
            {/* Cyber Bomber Jacket */}
            <rect x="4" y="10" width="8" height="5" fill="#3b82f6" />
            <rect x="6" y="10" width="4" height="5" fill="#ffffff" />
            <rect x="7" y="11" width="2" height="4" fill="#f59e0b" />
          </svg>
        );

      default:
        // Autonomous Droid / Bot
        return (
          <svg
            viewBox="0 0 16 16"
            className="w-full h-full"
            style={{ imageRendering: 'pixelated', shapeRendering: 'crispEdges' }}
          >
            {/* Background */}
            <rect width="16" height="16" fill="#18181b" />
            {/* Antenna */}
            <rect x="7" y="1" width="2" height="2" fill="#71717a" />
            <rect x="7" y="1" width="2" height="1" fill="#ef4444" />
            {/* Head Chassis */}
            <rect x="4" y="3" width="8" height="7" fill="#3f3f46" />
            {/* Square Glowing Eyes */}
            <rect x="5" y="5" width="2" height="2" fill="#38bdf8" />
            <rect x="9" y="5" width="2" height="2" fill="#38bdf8" />
            {/* Speaker Mouth */}
            <rect x="6" y="8" width="4" height="1" fill="#18181b" />
            {/* Body Chassis with Dual Status LEDs */}
            <rect x="4" y="10" width="8" height="5" fill="#27272a" />
            <rect x="6" y="11" width="1" height="1" fill="#10b981" />
            <rect x="9" y="11" width="1" height="1" fill="#f59e0b" />
            <rect x="6" y="13" width="4" height="1" fill="#3b82f6" />
          </svg>
        );
    }
  };

  return (
    <div
      className={`relative inline-flex flex-shrink-0 select-none ${className}`}
      style={{ width: size, height: size }}
    >
      <div
        className={`w-full h-full rounded-xl overflow-hidden shadow-sm border ${borderClass} flex items-center justify-center`}
      >
        {renderPixelSprite()}
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
