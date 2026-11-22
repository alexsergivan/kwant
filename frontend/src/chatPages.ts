// 33k long-chat-loads-earlier: a chat's messages arrive in pages of PAGE (the server's default), newest first.
// The latest page is reloaded after every reply; earlier pages come from `before=<first id>` as the person scrolls
// up. These keep the pages already loaded when the latest page comes back, and say when a chat is at its start.

/** The server's page size (routes.rs messages_page default). Fewer back means nothing older is left. */
export const PAGE = 100;

interface HasId {
  id: string;
}

/**
 * The cached list after the latest page `latest` comes back. A short page is the whole chat, so it wins as is.
 * Otherwise the cached messages older than the page's first one stay in front of it (each id once). When the page
 * doesn't reach back to anything cached (more than a page arrived meanwhile), the older pages would leave a gap, so
 * the page replaces them and earlier ones load again from it.
 */
export function mergeLatest<M extends HasId>(cached: readonly M[] | undefined, latest: M[]): M[] {
  if (!cached?.length || latest.length < PAGE) return latest;
  const at = cached.findIndex((m) => m.id === latest[0].id);
  if (at <= 0) return latest;
  const ids = new Set(latest.map((m) => m.id));
  return [...cached.slice(0, at).filter((m) => !ids.has(m.id)), ...latest];
}

/** The cached list with an earlier page put in front (each id once). */
export function prependOlder<M extends HasId>(cached: readonly M[] | undefined, older: M[]): M[] {
  const list = cached ?? [];
  const ids = new Set(list.map((m) => m.id));
  return [...older.filter((m) => !ids.has(m.id)), ...list];
}

/** Whether a chat is at its start after a page came back: a short page has nothing before it. */
export const reachedStart = (page: readonly unknown[]) => page.length < PAGE;

/**
 * The start line's words: a thread is "this conversation" (its chat title is the bot's name), a channel or team
 * names itself ("Start of #general"), a direct chat names the bot.
 */
export function startLabel(where: { thread: boolean; channel: boolean; title: string }): string {
  if (where.thread) return 'Start of this conversation';
  return where.channel ? `Start of ${where.title}` : `Start of your chat with ${where.title}`;
}
