// Which sites a bot's own browser may open (browser-borrowed/contract.md). The server normalizes
// entries (lowercase, a pasted URL keeps its host, punycode) and names a bad one in a 400; the UI
// shows the stored lists it gets back.
import type { TagLimits } from './tagList';

export type BrowserSites = {
  browser_allowed_sites: string[];
  browser_blocked_sites: string[];
  browser_block_private_network: boolean;
};

export const SITE_LIMITS: TagLimits = { max: 200, maxChars: 0, split: /[\s,]+/, noun: 'site' };

/** The bot's rules as the server sent them, or null when this server doesn't have them. */
export function sitesOf(bot: Partial<BrowserSites> | null | undefined): BrowserSites | null {
  if (!bot || !Array.isArray(bot.browser_allowed_sites) || !Array.isArray(bot.browser_blocked_sites)) return null;
  return {
    browser_allowed_sites: bot.browser_allowed_sites,
    browser_blocked_sites: bot.browser_blocked_sites,
    browser_block_private_network: !!bot.browser_block_private_network,
  };
}

const sameList = (a: string[], b: string[]) => a.length === b.length && a.every((x, i) => x === b[i]);

/**
 * Only the fields that changed, for PUT /api/kwants/:id: one left out stays as it is. Profile
 * edits then never depend on the site rules, which a bad entry or a bot's token would refuse.
 */
export function changedSites(before: BrowserSites, after: BrowserSites): Partial<BrowserSites> {
  const out: Partial<BrowserSites> = {};
  if (!sameList(before.browser_allowed_sites, after.browser_allowed_sites)) out.browser_allowed_sites = after.browser_allowed_sites;
  if (!sameList(before.browser_blocked_sites, after.browser_blocked_sites)) out.browser_blocked_sites = after.browser_blocked_sites;
  if (before.browser_block_private_network !== after.browser_block_private_network)
    out.browser_block_private_network = after.browser_block_private_network;
  return out;
}

/** One line for the closed Browser section: "No limits", "Only wikipedia.org · never 2 sites · …". */
export function sitesSummary(s: BrowserSites): string {
  const sites = (n: number) => (n === 1 ? '1 site' : `${n} sites`);
  const parts: string[] = [];
  const a = s.browser_allowed_sites;
  if (a.length) parts.push(a.length === 1 ? `Only ${a[0]}` : `Only ${sites(a.length)}`);
  const b = s.browser_blocked_sites;
  if (b.length) parts.push(b.length === 1 ? `never ${b[0]}` : `never ${sites(b.length)}`);
  if (s.browser_block_private_network) parts.push('no local network');
  if (!parts.length) return 'Any site';
  const line = parts.join(' · ');
  return line[0].toUpperCase() + line.slice(1);
}

export type FieldErrorKey = 'covers' | 'avoids' | 'browser_allowed_sites' | 'browser_blocked_sites';

/**
 * Splits a 400 that names a field ("browser_allowed_sites: \"a*b.com\" isn't a site: …", "covers:
 * at most 20 entries") into the field and what to show next to it.
 */
export function fieldError(message: string): { field: FieldErrorKey; text: string } | null {
  // The app capitalizes server messages, so "Browser_allowed_sites: …" counts too.
  const m = /^(covers|avoids|browser_allowed_sites|browser_blocked_sites):\s*(.+)$/is.exec(message.trim());
  if (!m) return null;
  const text = m[2].trim();
  return { field: m[1].toLowerCase() as FieldErrorKey, text: text[0].toUpperCase() + text.slice(1) };
}
