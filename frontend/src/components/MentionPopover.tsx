import React, { useEffect, useRef } from 'react';
import { Users } from 'lucide-react';
import { ModernAvatar } from './ModernAvatar';

export interface MentionItem {
  type: 'kwant' | 'team';
  id: string;
  name: string;
  title?: string;
  avatar_sprite?: string;
  memberCount?: number;
}

export interface MentionPopoverProps {
  items: MentionItem[];
  selectedIndex: number;
  onSelect: (item: MentionItem) => void;
  position?: { top: number; left: number };
}

export const MentionPopover: React.FC<MentionPopoverProps> = ({
  items,
  selectedIndex,
  onSelect,
  position,
}) => {
  const itemRefs = useRef<(HTMLButtonElement | null)[]>([]);

  useEffect(() => {
    if (itemRefs.current[selectedIndex]) {
      itemRefs.current[selectedIndex]?.scrollIntoView({
        block: 'nearest',
      });
    }
  }, [selectedIndex]);

  if (items.length === 0) {
    return null;
  }

  return (
    <div
      className="absolute bottom-full left-0 mb-2 bg-zinc-900 border border-zinc-800 rounded-xl shadow-2xl p-1.5 min-w-[240px] max-w-sm max-h-64 overflow-y-auto z-50 backdrop-blur-md"
      style={
        position
          ? { top: `${position.top}px`, left: `${position.left}px`, bottom: 'auto' }
          : undefined
      }
    >
      <div className="space-y-0.5">
        {items.map((item, index) => {
          const isSelected = index === selectedIndex;
          const showSectionHeader =
            index === 0 || items[index - 1].type !== item.type;

          return (
            <React.Fragment key={`${item.type}-${item.id}`}>
              {showSectionHeader && (
                <div className="text-[10px] font-semibold text-zinc-500 uppercase tracking-wider px-2.5 py-1 select-none">
                  {item.type === 'kwant' ? 'Kwants' : 'Teams'}
                </div>
              )}
              <button
                type="button"
                ref={(el) => (itemRefs.current[index] = el)}
                onClick={() => onSelect(item)}
                onMouseDown={(e) => {
                  // Prevent input blur before click event fires
                  e.preventDefault();
                }}
                className={`w-full flex items-center justify-between gap-3 px-2.5 py-1.5 rounded-lg text-left transition-colors text-xs ${
                  isSelected
                    ? 'bg-zinc-800 text-zinc-100'
                    : 'text-zinc-300 hover:bg-zinc-800/60 hover:text-zinc-100'
                }`}
              >
                <div className="flex items-center gap-2.5 min-w-0">
                  {item.type === 'kwant' ? (
                    <ModernAvatar
                      role={item.title || item.name}
                      name={item.name}
                      size={24}
                    />
                  ) : (
                    <div className="w-6 h-6 rounded-lg bg-sky-500/10 border border-sky-500/20 text-sky-400 flex items-center justify-center flex-shrink-0">
                      <Users size={13} />
                    </div>
                  )}

                  <div className="min-w-0 truncate">
                    <div className="font-medium truncate text-zinc-100">
                      @{item.name}
                    </div>
                    {item.title && (
                      <div className="text-[10px] text-zinc-400 truncate">
                        {item.title}
                      </div>
                    )}
                  </div>
                </div>

                {item.type === 'team' && item.memberCount !== undefined && (
                  <span className="flex-shrink-0 text-[10px] font-mono px-1.5 py-0.5 rounded-full bg-zinc-800/80 text-zinc-400 border border-zinc-700/50">
                    {item.memberCount} {item.memberCount === 1 ? 'member' : 'members'}
                  </span>
                )}
              </button>
            </React.Fragment>
          );
        })}
      </div>
    </div>
  );
};
