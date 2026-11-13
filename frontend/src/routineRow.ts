// 0.9.7 routine row (Mira's UX audit #5, autonomy/routine-row/brief.md): a routine's run posts its
// instructions into the chat as a user message whose sender id is `Routine: <name>` (scheduler.rs
// `execute`). The chat drew it as the person's own "You" bubble; it now draws a routine row instead.
// The check is on sender_id, never on the text, so a person who types "Routine: ..." stays "You".

import { ROUTINE_PREFIX } from './doneAway';

export interface RoutineSource {
  sender_role: string;
  sender_id?: string | null;
}

/** The routine's name when this message is a routine's post, else null. An unnamed routine gives ''. */
export function routineOf(msg: RoutineSource): string | null {
  if (msg.sender_role !== 'user') return null;
  const id = msg.sender_id ?? '';
  if (!ROUTINE_PREFIX.test(id)) return null;
  return id.replace(ROUTINE_PREFIX, '').trim();
}

/** The row's label: "Routine · <name>", or "Routine" when the name is blank. */
export const routineLabel = (name: string): string => (name ? `Routine · ${name}` : 'Routine');

/** The instructions on one line for the collapsed row: whitespace folded; CSS adds the ellipsis. */
export const routineLine = (text: string | null | undefined): string => (text ?? '').replace(/\s+/g, ' ').trim();
