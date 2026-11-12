// 0.9.5: the plain line a chat shows when one of a bot's connections fails (meta kind
// "tool_failure"): "Gmail stopped working for Ben: it needs to be connected again. Reconnect".
// Quiet like a tool note; the fix button opens Connections. Once the connection works again the
// line stays, says so, and loses its button.
import React from 'react';
import { Unplug } from 'lucide-react';
import { ToolFailureMeta } from '../toolFailures';

export const ToolFailureLine: React.FC<{ meta: ToolFailureMeta; messageId?: string; flash?: boolean; onFix?: () => void }> = ({
  meta,
  messageId,
  flash = false,
  onFix,
}) => {
  const open = !meta.resolved_at;
  const urgent = open && meta.needs_you;
  return (
    <div
      data-message-id={messageId}
      data-tool-failure-line={meta.cause}
      className={`flex items-start gap-2 pl-11 pr-4 py-1 rounded-lg text-[12.5px] leading-snug text-zinc-400 ${flash ? 'bg-sky-500/10' : ''}`}
    >
      <Unplug size={13} className={`mt-[3px] shrink-0 ${urgent ? 'text-amber-400' : 'text-zinc-500'}`} aria-hidden="true" />
      <p className="min-w-0 break-words">
        {meta.line}
        {!open && ' It works again.'}
        {open && meta.fix && onFix && (
          <>
            {' '}
            <button
              type="button"
              onClick={onFix}
              className="font-medium text-zinc-200 underline-offset-2 hover:underline rounded focus-visible:outline focus-visible:outline-2 focus-visible:outline-sky-500"
            >
              {meta.fix.label}
            </button>
          </>
        )}
      </p>
    </div>
  );
};
