// Where a space opens around its first bot (0.9.54 first-bot-opens-general, Q7/F8 and simplify-review Minor d).
// The checklist promises "#general opens with it", so the first bot opens #general; later bots open their own
// chat, and a bot made inside a team opens that team. On a phone, #general opens first while its Get started
// card shows, instead of the Chats list.

export type AfterNewBot = 'team' | 'general' | 'bot';

/** What opens once the New bot form has made a bot. `botsBefore`: the space's bots before this one. */
export function afterNewBot(o: { botsBefore: number; inTeam: boolean }): AfterNewBot {
  if (o.inTeam) return 'team';
  return o.botsBefore === 0 ? 'general' : 'bot';
}

/** The space's #general: by the space's id for it, else by name (the server makes it with the first bot). */
export function generalOf<C extends { id: string; name: string }>(channels: C[], generalId: string | null | undefined): C | null {
  return channels.find((c) => c.id === generalId) ?? channels.find((c) => c.name === 'general') ?? null;
}

export interface PhoneFirstViewInput {
  /** A link, a notification or a page asked for something: leave it. */
  linked: boolean;
  /** Something can answer here (null: not known yet). */
  canRun: boolean | null;
  /** Get started was dismissed in this space. */
  dismissed: boolean;
  /** No bots: the phone already shows the space's checklist. */
  hasBots: boolean;
  /** #general is the chat open now (where a space opens without a link). */
  inGeneral: boolean;
  /** #general's messages have loaded, so `cardShows` is known. */
  generalLoaded: boolean;
  /** #general shows the Get started card (gettingStarted.showCard). */
  cardShows: boolean;
}

/** A phone's first view on opening a space: #general, the Chats list, or wait to know. */
export function phoneFirstView(o: PhoneFirstViewInput): 'general' | 'list' | 'wait' {
  if (o.canRun === null) return 'wait';
  if (o.linked || o.dismissed) return 'list';
  // C1: nothing can run yet, so #general's card is the next step whatever #general holds.
  if (o.canRun === false) return 'general';
  if (!o.hasBots || !o.inGeneral) return 'list';
  if (!o.generalLoaded) return 'wait';
  return o.cardShows ? 'general' : 'list';
}
