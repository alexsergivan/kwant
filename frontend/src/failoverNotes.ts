// Which reply carries a failover notice. Pure, so the chat and the node tests share it.

/** One "answered on another account" line, for the bot that was switched. */
export interface SwitchNote {
  /** The bot's name, as the chat shows it. */
  name: string;
  text: string;
  /** When the switch happened (ms). */
  at: number;
}

interface Msg {
  id: string;
  sender_role: string;
  sender_id: string;
  created_at?: string | null;
}

/** The notes come first, then the replies: a note goes under the first reply by its bot written at or
 * after it. Returns message id → text; notes with no reply yet are left for the live bubble. */
export function assignSwitchNotes(
  notes: SwitchNote[],
  messages: Msg[],
  nameOf: (m: Msg) => string,
  skewMs = 5000,
): Map<string, string> {
  const out = new Map<string, string>();
  for (const note of [...notes].sort((a, b) => a.at - b.at)) {
    const hit = messages.find((m) => {
      if (m.sender_role === 'user' || out.has(m.id)) return false;
      const t = Date.parse(m.created_at ?? '');
      return nameOf(m) === note.name && !Number.isNaN(t) && t >= note.at - skewMs;
    });
    if (hit) out.set(hit.id, note.text);
  }
  return out;
}

/** The note for the reply that is streaming now: the newest for that bot that no saved reply carries. */
export function liveSwitchNote(notes: SwitchNote[], assigned: Map<string, string>, speaker: string, now = Date.now(), maxAgeMs = 15 * 60_000): string | null {
  const taken = new Set(assigned.values());
  const open = notes
    .filter((n) => n.name === speaker && !taken.has(n.text) && now - n.at < maxAgeMs)
    .sort((a, b) => b.at - a.at);
  return open[0]?.text ?? null;
}
