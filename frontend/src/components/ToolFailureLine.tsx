// 0.9.5: the plain line a chat shows when one of a bot's connections fails (meta kind
// "tool_failure"): "Gmail stopped working for Ben: it needs you to sign in again. Sign in again".
// Quiet like a tool note; the fix button opens Connections on that connection's row (34h). Once the connection works again the
// line stays, says so in green with a plugged icon, and loses its button. Mira's design check
// (release-095/tool-failure-line/design-check.md F3–F6): a needs-you line is brighter with an
// amber button and tells a screen reader; lines of one turn sit 4 px apart; the button has a
// 44 px hit area on a phone. That area is the invisible ::before, so the button opts out of the
// phones' 44 px min-height (data-compact), which would make its line of text twice as tall.
import React from 'react';
import { Plug, Unplug } from 'lucide-react';
import { ToolFailureMeta, fixLabel } from '../toolFailures';

export const ToolFailureLine: React.FC<{
  meta: ToolFailureMeta;
  messageId?: string;
  flash?: boolean;
  /** The message before is also a kwant line (a tool failure or tool note): sit 4 px under it (F5). */
  tight?: boolean;
  /** Gets the line's connection id ("mcp:<id>") so Connections opens on its row. */
  onFix?: (connectionId?: string) => void;
}> = ({ meta, messageId, flash = false, tight = false, onFix }) => {
  const open = !meta.resolved_at;
  const urgent = open && meta.needs_you;
  const Icon = open ? Unplug : Plug;
  return (
    <div
      data-message-id={messageId}
      data-tool-failure-line={meta.cause}
      className={`flex items-start gap-2 pl-11 pr-4 py-1 rounded-lg text-[12.5px] leading-snug ${urgent ? 'text-zinc-300' : 'text-zinc-400'} ${tight ? '!mt-1' : ''} ${flash ? 'bg-sky-500/10' : ''}`}
    >
      <Icon size={13} className={`mt-[3px] shrink-0 ${urgent ? 'text-amber-400' : 'text-zinc-500'}`} aria-hidden="true" />
      <p className="min-w-0 break-words">
        {urgent && <span className="sr-only">Needs you: </span>}
        {meta.line}
        {!open && <span className="text-ok"> It works again.</span>}
        {open && meta.fix && onFix && (
          <>
            {' '}
            <button
              type="button"
              data-compact
              onClick={() => onFix(meta.fix?.connection_id ?? undefined)}
              className={`relative before:absolute before:content-[''] before:-inset-x-2 before:-inset-y-[14px] font-medium ${urgent ? 'text-amber-200' : 'text-zinc-200'} underline-offset-2 hover:underline rounded focus-visible:outline focus-visible:outline-2 focus-visible:outline-sky-500`}
            >
              {fixLabel(meta.cause, meta.fix.label)}
            </button>
          </>
        )}
      </p>
    </div>
  );
};
