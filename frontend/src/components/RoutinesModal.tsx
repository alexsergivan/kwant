import React, { useEffect, useRef } from 'react';
import { CalendarClock, X } from 'lucide-react';
import { RoutinesPanel } from './RoutinesPanel';
import type { Channel, Kwant } from './Sidebar';
import { playSfx } from '../audio/chiptune';

/** Every kwant's routines in one list. */
export const RoutinesModal: React.FC<{ isOpen: boolean; onClose: () => void; kwants: Kwant[]; channels: Channel[]; onOpenSettings?: () => void }> = ({
  isOpen,
  onClose,
  kwants,
  channels,
  onOpenSettings,
}) => {
  const closeRef = useRef(onClose);
  closeRef.current = onClose;
  useEffect(() => {
    if (!isOpen) return;
    const onKey = (e: KeyboardEvent) => e.key === 'Escape' && closeRef.current();
    window.addEventListener('keydown', onKey);
    return () => window.removeEventListener('keydown', onKey);
  }, [isOpen]);

  if (!isOpen) return null;
  return (
    <div className="kw-dialog fixed inset-0 z-50 bg-black/70 backdrop-blur-sm flex items-center justify-center p-2 sm:p-4" onMouseDown={(e) => e.target === e.currentTarget && onClose()}>
      <div
        role="dialog"
        aria-modal="true"
        aria-labelledby="routines-title"
        className="kw-dialog-panel bg-zinc-900 border border-zinc-800 rounded-2xl max-w-2xl w-full max-h-[94vh] overflow-y-auto p-4 sm:p-6 text-zinc-100 shadow-2xl space-y-5"
      >
        <div className="flex items-center justify-between border-b border-zinc-800 pb-4">
          <div className="flex items-center gap-2.5">
            <div className="w-8 h-8 rounded-lg bg-sky-500/10 border border-sky-500/20 flex items-center justify-center text-sky-400">
              <CalendarClock size={16} />
            </div>
            <div>
              <h2 id="routines-title" className="text-sm font-semibold text-zinc-100">Routines</h2>
              <p className="text-xs text-zinc-400">Scheduled work for the bots in this space</p>
            </div>
          </div>
          <button
            type="button"
            onClick={() => {
              playSfx('click');
              onClose();
            }}
            aria-label="Close"
            className="p-1.5 rounded-lg text-zinc-400 hover:text-zinc-100 hover:bg-zinc-800 transition-colors"
          >
            <X size={18} />
          </button>
        </div>
        <RoutinesPanel kwants={kwants} channels={channels} onOpenSettings={onOpenSettings} />
      </div>
    </div>
  );
};
