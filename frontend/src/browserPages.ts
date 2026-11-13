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

// ---------- Browser panel (0.9.8, design browser-panel/design.md A, B) ----------

/** The bot browser's own open/navigate tools. WebFetch and fetch never touch it, so they don't count. */
export const BROWSER_PAGE_TOOLS: readonly string[] = ['browser_open', 'browser_navigate', 'navigate', 'open_url'];

/** A tool's own name: `mcp__kwnt__browser_open` → `browser_open`. */
export function ownToolName(tool: string | null | undefined): string {
  if (!tool) return '';
  const i = tool.lastIndexOf('__');
  return (i >= 0 ? tool.slice(i + 2) : tool).toLowerCase();
}

/** The fields of an activity step the trail reads (a saved reply's or a running turn's). */
export interface PageStep {
  kind: string;
  status?: string;
  tool?: string;
  detail?: string;
  /** The title the browser reported once the page loaded (server 0.9.8+, optional). */
  page_title?: string;
  started_at: string;
}

/** One row of the trail: a page the bot opened in its browser. */
export interface ReplyPage {
  /** When the bot opened it (the newest visit), ISO time from the step. */
  at: string;
  url: string;
  host: string;
  title: string;
}

/** An http(s) address without its #hash, or null when it isn't one. */
function pageAddress(raw: string | null | undefined): URL | null {
  if (!raw) return null;
  try {
    const u = new URL(raw.trim());
    if (u.protocol !== 'http:' && u.protocol !== 'https:') return null;
    u.hash = '';
    return u;
  } catch {
    return null;
  }
}

// Titles the app saw live (`browser_navigated` carries one), per address, for a session.
const TITLE_MAX = 200;
const titles = new Map<string, string>();

/** Remembers a page's title (the newest 200 addresses). */
export function noteTitle(url: string | null | undefined, title: string | null | undefined): void {
  const u = pageAddress(url);
  const t = title?.trim();
  if (!u || !t || t.startsWith('about:')) return;
  const key = u.href;
  titles.delete(key);
  titles.set(key, t);
  while (titles.size > TITLE_MAX) titles.delete(titles.keys().next().value as string);
}

/** A title the app saw for this address, if any. */
export function titleFor(url: string | null | undefined): string | null {
  const u = pageAddress(url);
  return (u && titles.get(u.href)) || null;
}

/** For tests: forget every title. */
export function clearTitles(): void {
  titles.clear();
}

/** The trail's line 1: the step's page title, else one seen live, else the path ("/" reads as the host). */
export function pageTitleOf(url: string, stepTitle?: string | null): string {
  const t = stepTitle?.trim();
  if (t) return t;
  const seen = titleFor(url);
  if (seen) return seen;
  const u = pageAddress(url);
  if (!u) return url;
  const path = decodeURI(u.pathname).replace(/\/+$/, '');
  return path ? path + u.search : pageHost(u.href) ?? u.host;
}

/** Whether a step opened a page in the bot's browser: an own open/navigate tool with an address that didn't fail. */
export function isPageStep(s: PageStep): boolean {
  return s.kind === 'tool' && s.status !== 'failed' && BROWSER_PAGE_TOOLS.includes(ownToolName(s.tool)) && !!pageAddress(s.detail);
}

/** The pages a reply opened in the bot's browser: distinct addresses (hash stripped), newest first. */
export function pagesOfReply(steps: readonly PageStep[] | null | undefined): ReplyPage[] {
  const out = new Map<string, { at: string; url: string; host: string; stepTitle: string | null }>();
  for (const s of steps ?? []) {
    if (!isPageStep(s)) continue;
    const u = pageAddress(s.detail)!;
    const key = u.href;
    // A later visit moves the page up; the newest reported title wins
    const prev = out.get(key);
    out.delete(key);
    out.set(key, { at: s.started_at, url: key, host: pageHost(key) ?? u.host, stepTitle: s.page_title?.trim() || prev?.stepTitle || null });
  }
  return [...out.values()].reverse().map(p => ({ at: p.at, url: p.url, host: p.host, title: pageTitleOf(p.url, p.stepTitle) }));
}

/** The chip's words: "Ben opened 2 pages", or while browsing the host and the count from 2 up. */
export function browsedChipText(botName: string, pages: readonly ReplyPage[], live: boolean): { lead: string; host: string | null; count: string | null; label: string } {
  const n = pages.length;
  if (live) {
    const host = pages[0]?.host ?? null;
    return { lead: `${botName} is on`, host, count: n >= 2 ? `${n} pages` : null, label: `Watch ${botName}'s browser` };
  }
  return { lead: `${botName} opened ${n} ${n === 1 ? 'page' : 'pages'}`, host: null, count: null, label: `Show the ${n === 1 ? 'page' : `${n} pages`} ${botName} opened` };
}

/** "10:45" in the person's locale, without AM/PM (the panel header has the full time). */
export function trailTime(iso: string, locale?: string): string {
  const d = new Date(iso);
  if (Number.isNaN(d.getTime())) return '';
  return d
    .toLocaleTimeString(locale ?? [], { hour: 'numeric', minute: '2-digit' })
    .replace(/\s?[AaPp]\.?\s?[Mm]\.?$/, '')
    .trim();
}
