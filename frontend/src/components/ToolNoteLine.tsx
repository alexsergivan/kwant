// 0.9.0 P5: the grey line a bot's chat shows when it files or repeats a tool note (meta kind
// "tool_note"). Quiet like a tool step: no card, no badge. "See the note" opens Connections on it.
import React from 'react';
import { StickyNote } from 'lucide-react';
import { ToolNoteMeta, chatLineParts } from '../toolNotes';

export const ToolNoteLine: React.FC<{ meta: ToolNoteMeta; botName: string; messageId?: string; flash?: boolean; onOpen?: (noteId: string) => void }> = ({ meta, botName, messageId, flash = false, onOpen }) => {
  const [before, after] = chatLineParts(botName, meta);
  return (
    <div data-message-id={messageId} className={`flex items-start gap-2 pl-11 pr-4 py-1 rounded-lg text-[12.5px] leading-snug text-zinc-400 ${flash ? 'bg-sky-500/10' : ''}`} data-tool-note-line={meta.note_id}>
      <StickyNote size={13} className="mt-[3px] shrink-0 text-zinc-500" aria-hidden="true" />
      <p className="min-w-0 break-words">
        {before}
        <code className="font-mono text-zinc-300">{meta.tool}</code>
        {after}
        {onOpen && (
          <>
            {' '}
            <button
              type="button"
              onClick={() => onOpen(meta.note_id)}
              className="font-medium text-zinc-200 underline-offset-2 hover:underline rounded focus-visible:outline focus-visible:outline-2 focus-visible:outline-sky-500"
            >
              See the note
            </button>
          </>
        )}
      </p>
    </div>
  );
};
