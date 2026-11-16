/**
 * The composer's "Ask:" row in a team chat: the team's lead first, then everyone else in the order they
 * came (roster order), so the row reads like the team home does (15e). No lead, or a lead who isn't in
 * the list, leaves the order as it is. Never drops or repeats anyone.
 */
export function askOrder<T extends { id: string }>(members: readonly T[], leadId: string | null | undefined): T[] {
  const i = leadId ? members.findIndex((m) => m.id === leadId) : -1;
  if (i <= 0) return members.slice();
  return [members[i], ...members.slice(0, i), ...members.slice(i + 1)];
}
