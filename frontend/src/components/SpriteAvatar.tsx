import React from 'react';

interface SpriteProps {
  sprite: string;
  size?: number;
  className?: string;
}

export const SpriteAvatar: React.FC<SpriteProps> = ({ sprite, size = 48, className = '' }) => {
  // 8-bit pixel art SVG sprites designed on a 16x16 grid
  const renderSpriteGrid = () => {
    switch (sprite) {
      case 'cyber_mage':
        return (
          <svg viewBox="0 0 16 16" width={size} height={size} className={className} style={{ imageRendering: 'pixelated' }}>
            <rect x="0" y="0" width="16" height="16" fill="#181622" />
            {/* Mage Hat */}
            <rect x="6" y="1" width="4" height="2" fill="#7209b7" />
            <rect x="5" y="3" width="6" height="2" fill="#7209b7" />
            <rect x="3" y="5" width="10" height="1" fill="#f72585" />
            {/* Face */}
            <rect x="5" y="6" width="6" height="4" fill="#ffd166" />
            {/* Glowing cyan eyes */}
            <rect x="6" y="7" width="1" height="1" fill="#00f5d4" />
            <rect x="9" y="7" width="1" height="1" fill="#00f5d4" />
            {/* Robe */}
            <rect x="4" y="10" width="8" height="5" fill="#3a0ca3" />
            {/* Staff */}
            <rect x="13" y="4" width="1" height="11" fill="#b5179e" />
            <rect x="12" y="3" width="3" height="2" fill="#00f5d4" />
          </svg>
        );

      case 'pixel_ninja':
        return (
          <svg viewBox="0 0 16 16" width={size} height={size} className={className} style={{ imageRendering: 'pixelated' }}>
            <rect x="0" y="0" width="16" height="16" fill="#181622" />
            {/* Ninja Hood */}
            <rect x="4" y="2" width="8" height="5" fill="#2b2d42" />
            {/* Mask Slit */}
            <rect x="5" y="5" width="6" height="2" fill="#ffd166" />
            {/* Sharp Eyes */}
            <rect x="6" y="5" width="1" height="1" fill="#ef233c" />
            <rect x="9" y="5" width="1" height="1" fill="#ef233c" />
            {/* Body */}
            <rect x="4" y="8" width="8" height="7" fill="#1b1c24" />
            <rect x="6" y="8" width="4" height="3" fill="#ef233c" />
            {/* Katana */}
            <rect x="2" y="3" width="1" height="10" fill="#e0e1dd" />
            <rect x="1" y="11" width="3" height="1" fill="#fca311" />
          </svg>
        );

      case 'terminal_knight':
        return (
          <svg viewBox="0 0 16 16" width={size} height={size} className={className} style={{ imageRendering: 'pixelated' }}>
            <rect x="0" y="0" width="16" height="16" fill="#181622" />
            {/* CRT Helmet */}
            <rect x="3" y="2" width="10" height="7" fill="#4a4e69" />
            <rect x="4" y="3" width="8" height="5" fill="#03045e" />
            {/* Screen readout */}
            <rect x="6" y="5" width="4" height="1" fill="#38b000" />
            <rect x="5" y="6" width="3" height="1" fill="#38b000" />
            {/* Armor */}
            <rect x="4" y="9" width="8" height="6" fill="#9a8c98" />
            <rect x="6" y="10" width="4" height="4" fill="#fca311" />
            {/* Shield */}
            <rect x="1" y="7" width="2" height="7" fill="#c9ada7" />
          </svg>
        );

      case 'user':
        return (
          <svg viewBox="0 0 16 16" width={size} height={size} className={className} style={{ imageRendering: 'pixelated' }}>
            <rect x="0" y="0" width="16" height="16" fill="#181622" />
            {/* Hair */}
            <rect x="4" y="2" width="8" height="3" fill="#8d5b4c" />
            {/* Red Headband */}
            <rect x="4" y="5" width="8" height="1" fill="#f72585" />
            <rect x="12" y="6" width="2" height="2" fill="#f72585" />
            {/* Face */}
            <rect x="4" y="6" width="8" height="4" fill="#ffd166" />
            <rect x="6" y="7" width="1" height="1" fill="#000" />
            <rect x="9" y="7" width="1" height="1" fill="#000" />
            {/* Jacket */}
            <rect x="4" y="10" width="8" height="5" fill="#00f5d4" />
            <rect x="6" y="10" width="4" height="4" fill="#ffffff" />
          </svg>
        );

      default:
        // Glitch Rogue / Bot
        return (
          <svg viewBox="0 0 16 16" width={size} height={size} className={className} style={{ imageRendering: 'pixelated' }}>
            <rect x="0" y="0" width="16" height="16" fill="#181622" />
            <rect x="4" y="3" width="8" height="8" fill="#fca311" />
            <rect x="6" y="5" width="1" height="2" fill="#000" />
            <rect x="9" y="5" width="1" height="2" fill="#000" />
            <rect x="6" y="9" width="4" height="1" fill="#000" />
            <rect x="4" y="11" width="8" height="4" fill="#2c283e" />
          </svg>
        );
    }
  };

  return <div className="inline-block p-1 pixel-box bg-arcade-panel">{renderSpriteGrid()}</div>;
};
