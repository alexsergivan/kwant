// 33a (Mira's design check, fix 1): a kept approval line sits where its card was, in time order with
// the messages, instead of under the newest answer.

const time = (v: string | null | undefined): number => {
  const t = v ? Date.parse(v) : NaN;
  return Number.isFinite(t) ? t : NaN;
};

/**
 * Where each kept line goes among the chat's items (oldest first): `after.get(i)` are the lines drawn
 * right after item `i` (-1: before the first item), `end` the ones drawn where they always were, after
 * every message. A line goes after the last item sent at or before it was asked; a line asked after
 * every item, or with no time (or an unreadable one), stays at the end. Items without a time are
 * skipped. The lines keep their given order within a place.
 */
export function placeSettled<T extends { asked_at?: string | null }>(
  itemTimes: Array<string | null | undefined>,
  lines: T[],
): { after: Map<number, T[]>; end: T[] } {
  const times = itemTimes.map(time);
  const after = new Map<number, T[]>();
  const end: T[] = [];
  for (const line of lines) {
    const asked = time(line.asked_at);
    if (!Number.isFinite(asked)) { end.push(line); continue; }
    let at = -1;
    let later = false;
    times.forEach((t, i) => {
      if (!Number.isFinite(t)) return;
      if (t <= asked) at = i;
      else later = true;
    });
    // Nothing timed comes after it: today's place, after every message.
    if (!later || at === times.length - 1) { end.push(line); continue; }
    const list = after.get(at);
    if (list) list.push(line); else after.set(at, [line]);
  }
  return { after, end };
}
