// Readable verbs for tool names (0.6.13 spec C2). The working strip, the step rows and the approval
// card say what a bot does in words ("opening a pull request on GitHub"), never the raw
// `create_pull_request (github)`. A tool we don't know falls back to its name with spaces.

/** How a few well-known servers are written. Others are shown as they come. */
const SERVERS: Record<string, string> = {
  github: 'GitHub',
  gitlab: 'GitLab',
  linear: 'Linear',
  slack: 'Slack',
  notion: 'Notion',
  jira: 'Jira',
  stripe: 'Stripe',
  gmail: 'Gmail',
  googlecalendar: 'Google Calendar',
  google_calendar: 'Google Calendar',
  googledrive: 'Google Drive',
  google_drive: 'Google Drive',
};

/** [the "-ing" form after "<Bot> is", the plain form after "<Bot> wants to", whether "on <server>" follows]. */
type Verb = [ing: string, base: string, onServer?: boolean];

const VERBS: Record<string, Verb> = {
  create_pull_request: ['opening a pull request', 'open a pull request', true],
  merge_pull_request: ['merging a pull request', 'merge a pull request', true],
  create_issue: ['opening an issue', 'open an issue', true],
  add_issue_comment: ['commenting on an issue', 'comment on an issue', true],
  create_branch: ['creating a branch', 'create a branch', true],
  push_files: ['pushing files', 'push files', true],
  create_or_update_file: ['changing a file', 'change a file', true],
  send_message: ['sending a message', 'send a message', true],
  send_email: ['sending an email', 'send an email', true],
  browser_navigate: ['opening a page', 'open a page'],
  browser_open: ['opening a page', 'open a page'],
  open_url: ['opening a page', 'open a page'],
  web_fetch: ['opening a page', 'open a page'],
  webfetch: ['opening a page', 'open a page'],
  browser_click: ['using the browser', 'use the browser'],
  browser_type: ['using the browser', 'use the browser'],
  browser_act: ['using the browser', 'use the browser'],
  browser_snapshot: ['reading the browser page', 'read the browser page'],
  browser_screenshot: ['looking at the browser page', 'look at the browser page'],
  ask_user: ['asking you a question', 'ask you a question'],
  request_human_step: ['asking you to do a step in the browser', 'ask you to do a step in the browser'],
  web_search: ['searching the web', 'search the web'],
  websearch: ['searching the web', 'search the web'],
  bash: ['running a command', 'run a command'],
  execute_shell: ['running a command on your computer', 'run a command on your computer'],
  read_file: ['reading a file', 'read a file'],
  write_file: ['writing a file', 'write a file'],
  edit_file: ['changing a file', 'change a file'],
};

/** `mcp__github__create_pull_request` → the tool and its server; other names as given. */
function split(name: string, server?: string | null): { tool: string; server: string | null } {
  const m = /^mcp__(.+?)__(.+)$/.exec(name ?? '');
  if (m) return { tool: m[2], server: server || m[1] };
  return { tool: name ?? '', server: server || null };
}

const serverName = (s: string) => SERVERS[s.toLowerCase()] ?? s;
const spaced = (t: string) => t.replace(/[_-]+/g, ' ').replace(/\s+/g, ' ').trim();

function verb(name: string, server: string | null | undefined, form: 0 | 1): string {
  const p = split(name, server);
  const known = VERBS[p.tool.toLowerCase()];
  if (known) {
    const words = known[form];
    return known[2] && p.server ? `${words} on ${serverName(p.server)}` : words;
  }
  const head = form === 0 ? 'using' : 'use';
  const tool = spaced(p.tool) || 'a tool';
  return p.server ? `${head} ${tool} (${p.server})` : `${head} ${tool}`;
}

/** What a bot is doing with a tool, after "<Bot> is": "opening a pull request on GitHub". */
export const toolLabel = (name: string, server?: string | null): string => verb(name, server, 0);

/** What a bot asks to do with a tool, after "<Bot> wants to": "open a pull request on GitHub". */
export const toolAction = (name: string, server?: string | null): string => verb(name, server, 1);

const cap = (t: string) => (t ? t.charAt(0).toUpperCase() + t.slice(1) : t);

/**
 * A step title from the server, with a raw tool name made readable. The server writes an unknown
 * tool as "Using create_pull_request (github)", and an approval as "Waiting for your approval: <that>";
 * every other title (already in words) comes back unchanged.
 */
export function readableStepTitle(title: string): string {
  const using = /^Using `?([A-Za-z0-9_.:-]+)`?(?: \(([^)]+)\))?$/;
  const approval = /^Waiting for your approval: (.+)$/.exec(title);
  if (approval) {
    const inner = using.exec(approval[1].trim());
    const rest = approval[1].trim();
    return inner ? `Waiting for your OK to ${toolAction(inner[1], inner[2])}` : `Waiting for your OK: ${rest.charAt(0).toLowerCase()}${rest.slice(1)}`;
  }
  const m = using.exec(title.trim());
  return m ? cap(toolLabel(m[1], m[2])) : title;
}
