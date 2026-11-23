// 0.9.5: the line kwnt posts in a chat when one of a bot's connections fails (meta kind
// "tool_failure", see src/server/tool_failures.rs): "Gmail stopped working for Ben: it needs to be
// connected again. Connect again". One per connection per turn; nothing when a retry fixed it.
import { parseMetaObject } from './questions';

export type ToolFailureCause = 'sign_in' | 'reconnect' | 'not_started' | 'down';

export interface ToolFailureMeta {
  kind: 'tool_failure';
  kwant_id: string | null;
  /** The connection's name, e.g. "Gmail". */
  connection: string;
  cause: ToolFailureCause;
  /** The line's words, without the fix link. */
  line: string;
  /** Only the person can fix it (a sign-in or a reconnect). */
  needs_you: boolean;
  /** Where to fix it; null for kwant's own tools. */
  fix: { label: string; connection_id: string | null } | null;
  /** Set once a later call through the connection worked. */
  resolved_at: string | null;
}

/**
 * 34h: one name for the fix everywhere (chat line, Inbox): "Sign in again" for an expired sign-in,
 * "Connect again" for an app to connect again (Composio), and the server's own word otherwise.
 * By cause, so lines posted before 0.9.47 (label "Reconnect") read the same.
 */
export const fixLabel = (cause: string | null | undefined, label?: string | null): string =>
  cause === 'sign_in' ? 'Sign in again' : cause === 'reconnect' ? 'Connect again' : label || 'Check it';

const CAUSES: readonly ToolFailureCause[] = ['sign_in', 'reconnect', 'not_started', 'down'];
const str = (v: unknown): string | null => (typeof v === 'string' && v ? v : null);

export function parseToolFailureMeta(raw: unknown): ToolFailureMeta | null {
  const m = parseMetaObject(raw);
  if (!m || m.kind !== 'tool_failure') return null;
  const line = str(m.line);
  const connection = str(m.connection);
  if (!line || !connection) return null;
  const fix = m.fix && typeof m.fix === 'object' ? (m.fix as Record<string, unknown>) : null;
  const label = fix ? str(fix.label) : null;
  return {
    kind: 'tool_failure',
    kwant_id: str(m.kwant_id),
    connection,
    cause: CAUSES.includes(m.cause as ToolFailureCause) ? (m.cause as ToolFailureCause) : 'down',
    line,
    needs_you: m.needs_you === true,
    fix: label ? { label, connection_id: str(fix?.connection_id) } : null,
    resolved_at: str(m.resolved_at),
  };
}
