// A team's chat is the channel `team-<team name>`. A team renamed by an older version kept its chat
// under the old name, so names also match ignoring (ASCII) case, as the server does; some events
// carry the team's id where the chat's would be, so an id match counts too.

type Named = { id: string; name: string };

const fold = (s: string) => s.replace(/[A-Z]/g, (c) => c.toLowerCase());

/** Whether channel `c` is team `t`'s chat. */
export const isTeamChannel = (c: Named, t: Named) => c.id === t.id || fold(c.name) === fold(`team-${t.name}`);

/** Team `t`'s chat among `channels`: an exact match first, else one that differs only in case. */
export const teamChannelOf = <C extends Named>(channels: C[], t: Named): C | undefined =>
  channels.find((c) => c.id === t.id || c.name === `team-${t.name}`) ?? channels.find((c) => isTeamChannel(c, t));

/** The team whose chat channel `c` is, if any. */
export const teamOfChannel = <T extends Named>(teams: T[], c: Named): T | undefined =>
  teams.find((t) => t.id === c.id || c.name === `team-${t.name}`) ?? teams.find((t) => isTeamChannel(c, t));
