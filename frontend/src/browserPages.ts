// 0.9.3: every job (a bot in a chat) has its own page in its team's browser, and a sign-in or
// CAPTCHA step holds only its own page. The browser pane shows one page at a time and offers the
// team's other pages in a small strip (impl.md "For Pixel: a tab strip").

/** Separates the parts of a job page key: `team#bot#chat`. The team page's key is the team id. */
export const PAGE_SEP = '#';

/** One page of a team's browser, as `GET /api/sandbox/:team/pages` lists it. */
export interface BrowserPage {
  page: string;
  team_id: string;
  /** The bot of a job page; null for the team page. */
  kwant_id: string | null;
  chat_id: string | null;
  /** False for the team page (the person's own and the bots' with no chat). */
  job: boolean;
  url: string | null;
  title: string | null;
  /** An open step (sign-in, code, CAPTCHA) is waiting on this page. */
  held: boolean;
  idle_seconds: number | null;
}

export interface BrowserPageList {
  team_id: string;
  /** The page the live view shows when no `?page=` is given. */
  live: string;
  max_job_pages: number;
  /** Newest first; the team page is always there. */
  pages: BrowserPage[];
}

/** The team a page belongs to: the key itself for the team page. */
export const pageTeam = (key: string) => key.split(PAGE_SEP)[0];

/** A step's page: its own `page_key`, else (a step from before job pages) the team page. */
export function stepPage(step: { page_key?: string | null; sandbox_target: string | null } | null | undefined): string | null {
  if (!step) return null;
  return step.page_key || step.sandbox_target || null;
}

/**
 * Which page the pane shows, in order:
 *  1. the page the person picked (a tab, or a card's Sign in / Solve), while it still exists;
 *  2. else the page of the newest open step for this team;
 *  3. else nothing: the server's default (`live`).
 * `pages` is the latest list, or null before the first one loads (then a pick or step page is
 * trusted as is). A new step never moves the view off a picked page: the pick comes first.
 */
export function choosePage(input: { picked: string | null; stepPage: string | null; pages: readonly string[] | null }): string | null {
  const { picked, stepPage: step, pages } = input;
  const exists = (key: string | null): key is string => !!key && (!pages || pages.includes(key));
  if (exists(picked)) return picked;
  if (exists(step)) return step;
  return null;
}

/** The page the pane is showing: the chosen one, else the server's default, else the team page. */
export function shownPage(chosen: string | null, list: BrowserPageList | null, team: string | undefined): string | null {
  return chosen ?? list?.live ?? team ?? null;
}

/** Only the host of an address ("tiktok.com"), as the step card shows it. */
export function pageHost(url: string | null | undefined): string | null {
  if (!url) return null;
  try {
    const u = new URL(url);
    if (u.protocol !== 'http:' && u.protocol !== 'https:') return null;
    return u.hostname.replace(/^www\./, '') || null;
  } catch {
    return null;
  }
}

/** The tab's name: the bot's name for a job page, "Shared" for the team page. */
export function pageOwner(p: Pick<BrowserPage, 'job' | 'kwant_id'>, botName: (id: string) => string | undefined): string {
  if (!p.job || !p.kwant_id) return PAGE_COPY.shared;
  return botName(p.kwant_id) || PAGE_COPY.someBot;
}

/** Where the tab is: the site's host, else the title (not an "about:" one), else "nothing open". */
export function pageWhere(p: Pick<BrowserPage, 'url' | 'title'>): string {
  const title = p.title?.trim();
  return pageHost(p.url) || (title && !title.startsWith('about:') ? title : null) || PAGE_COPY.blank;
}

/** The tab's tooltip and accessible name: "Architect's page: tiktok.com (needs you)". */
export function pageLabel(p: BrowserPage, botName: (id: string) => string | undefined): string {
  const owner = pageOwner(p, botName);
  const whose = p.job ? `${owner}'s page` : PAGE_COPY.sharedLong;
  return `${whose}: ${pageWhere(p)}${p.held ? ` (${PAGE_COPY.waiting})` : ''}`;
}

export const PAGE_COPY = {
  stripName: 'Pages in this browser',
  shared: 'Shared',
  sharedLong: 'Shared page',
  someBot: 'A bot',
  blank: 'nothing open',
  waiting: 'needs you',
  handBackFirst: 'Hand back to switch pages',
};

// Live list refreshes: App hears BrowserNavigated, SandboxStatusChanged and human_step_updated
// for a team and pokes the pane's strip here.
type Listener = (team: string) => void;
const listeners = new Set<Listener>();

export function notifyBrowserPages(teamOrPage: string | null | undefined): void {
  if (!teamOrPage) return;
  const team = pageTeam(teamOrPage);
  for (const l of listeners) l(team);
}

export function onBrowserPagesChanged(l: Listener): () => void {
  listeners.add(l);
  return () => {
    listeners.delete(l);
  };
}

/** The team's pages, or null when the server has none to list (an older server, or no browser yet). */
export async function fetchBrowserPages(team: string, signal?: AbortSignal): Promise<BrowserPageList | null> {
  const res = await fetch(`/api/sandbox/${encodeURIComponent(team)}/pages`, { signal });
  if (!res.ok) return null;
  return parsePageList(await res.json().catch(() => null));
}

/** Keeps a parsed list only when it is the shape we expect (an older server has no such route). */
export function parsePageList(raw: unknown): BrowserPageList | null {
  if (!raw || typeof raw !== 'object') return null;
  const r = raw as Record<string, unknown>;
  if (!Array.isArray(r.pages) || typeof r.team_id !== 'string') return null;
  const pages: BrowserPage[] = [];
  for (const p of r.pages) {
    if (!p || typeof p !== 'object' || typeof (p as BrowserPage).page !== 'string') continue;
    const q = p as Record<string, unknown>;
    pages.push({
      page: q.page as string,
      team_id: typeof q.team_id === 'string' ? q.team_id : (r.team_id as string),
      kwant_id: typeof q.kwant_id === 'string' && q.kwant_id ? q.kwant_id : null,
      chat_id: typeof q.chat_id === 'string' && q.chat_id ? q.chat_id : null,
      job: q.job === true,
      url: typeof q.url === 'string' ? q.url : null,
      title: typeof q.title === 'string' ? q.title : null,
      held: q.held === true,
      idle_seconds: typeof q.idle_seconds === 'number' ? q.idle_seconds : null,
    });
  }
  return {
    team_id: r.team_id as string,
    live: typeof r.live === 'string' && r.live ? r.live : (r.team_id as string),
    max_job_pages: typeof r.max_job_pages === 'number' ? r.max_job_pages : 0,
    pages,
  };
}
