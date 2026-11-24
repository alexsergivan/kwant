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
  'home-assistant': 'Home Assistant',
  homeassistant: 'Home Assistant',
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
  browser_upload: ['uploading a file to a website', 'upload a file to a website'],
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

/**
 * Claude's own built-in names and the key they read as (tool_words.rs `BUILT_IN_ALIASES`, 33u). Only for a
 * built-in (no server): an MCP server's own `read` tool isn't Claude's Read.
 */
const BUILT_IN_ALIASES: Record<string, string> = { read: 'read_file', write: 'write_file', edit: 'edit_file', multiedit: 'edit_file' };

/**
 * Claude Code's bundled browser tools, `agent_browser_<x>` (seen as `mcp__browser__agent_browser_snapshot`), 33u:
 * `<x>` reads as `browser_<x>` when that key exists, else as the key here, else as `browser_act` ("using the
 * browser"). Never the raw name. tool_words.rs `AGENT_BROWSER`.
 */
const AGENT_BROWSER: Record<string, string> = {
  back: 'browser_open',
  forward: 'browser_open',
  reload: 'browser_open',
  read: 'browser_snapshot',
  get_text: 'browser_snapshot',
  get_title: 'browser_snapshot',
  get_url: 'browser_snapshot',
};

const has = (o: object, k: string) => Object.prototype.hasOwnProperty.call(o, k);
const own = (o: Record<string, Verb>, k: string): Verb | undefined => (has(o, k) ? o[k] : undefined);

/** The words for a tool, as tool_words.rs `verb` finds them. The same list is pinned on both sides (tests/fixtures/tool_words.json). */
function known(tool: string, builtIn: boolean): Verb | undefined {
  const lower = tool.trim().toLowerCase();
  if (builtIn && has(BUILT_IN_ALIASES, lower)) return own(VERBS, BUILT_IN_ALIASES[lower]);
  const direct = own(VERBS, lower);
  if (direct) return direct;
  if (!lower.startsWith('agent_browser_')) return undefined;
  const x = lower.slice('agent_browser_'.length);
  return own(VERBS, `browser_${x}`) ?? (has(AGENT_BROWSER, x) ? own(VERBS, AGENT_BROWSER[x]) : undefined) ?? VERBS.browser_act;
}

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
  const v = known(p.tool, !p.server);
  if (v) {
    const words = v[form];
    return v[2] && p.server ? `${words} on ${serverName(p.server)}` : words;
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
 * 28h: how an approval step reads once the card was answered. A card that was never answered, or a reply that
 * stopped, doesn't claim a choice: its chip or icon already says what happened.
 */
const SETTLED_APPROVAL: Record<string, string> = {
  ok: 'You said OK',
  declined: 'You said no',
  unanswered: 'Asked for your OK',
  stopped: 'Asked for your OK',
};

/**
 * 28j: how a human step (sign in, solve a CAPTCHA, enter a code in the bot's browser) reads once it's settled.
 * The server titles it "Waiting for you: <reason>" (human_steps.rs:587); the reason is the bot's own sentence
 * ("Ben needs you to sign in to tiktok.com…"), so it's kept exactly as written. Open steps read as today.
 */
const SETTLED_HUMAN_STEP: Record<string, string> = {
  done: 'You did it',
  resolved_auto: 'Cleared by itself',
  declined: 'You said no',
  timed_out: 'Asked you',
  stopped: 'Asked you',
};

/**
 * A step title from the server, with a raw tool name made readable. The server writes an unknown
 * tool as "Using create_pull_request (github)", and an approval as "Waiting for your approval: <that>";
 * every other title (already in words) comes back unchanged. Pass the step's status so an answered
 * approval says what the person chose ("You said OK: running a command"), not that it still waits, and a
 * settled human step says what happened ("You did it: <reason>").
 */
export function readableStepTitle(title: string, status?: string): string {
  const human = /^Waiting for you: (.+)$/s.exec(title);
  const humanSettled = human && status ? SETTLED_HUMAN_STEP[status] : undefined;
  if (human && humanSettled) return `${humanSettled}: ${human[1]}`;
  const using = /^Using `?([A-Za-z0-9_.:-]+)`?(?: \(([^)]+)\))?$/;
  const approval = /^Waiting for your approval: (.+)$/.exec(title);
  if (approval) {
    const rest = approval[1].trim();
    const inner = using.exec(rest);
    const ha = inner ? homeAction(inner[1]) : null;
    const settled = status ? SETTLED_APPROVAL[status] : undefined;
    if (settled) {
      // Always the -ing phrase, so every settled row reads "<what you chose>: <what it was>".
      const what = ha ? homePhrase(ha, 'ing') : inner ? toolLabel(inner[1], inner[2]) : `${rest.charAt(0).toLowerCase()}${rest.slice(1)}`;
      return `${settled}: ${what}`;
    }
    if (ha) return `Waiting for your OK to ${homePhrase(ha, 'base')}`;
    return inner ? `Waiting for your OK to ${toolAction(inner[1], inner[2])}` : `Waiting for your OK: ${rest.charAt(0).toLowerCase()}${rest.slice(1)}`;
  }
  const m = using.exec(title.trim());
  if (m) {
    const ha = homeAction(m[1]);
    // The step carries no args, so the target is "something" (design §3.4). Done steps use the past form.
    if (ha) return ha.ing === 'Looking at your home' ? 'Looked at your home' : `${cap(homePhrase(ha, 'past'))} · Home Assistant`;
  }
  return m ? cap(toolLabel(m[1], m[2])) : title;
}

// ---- Home Assistant (26e, design autonomy/connect-home-assistant §3) ----

/** [base, ing, past]. `target: false` means the verb never takes a target (GetLiveContext). */
type HaVerb = { base: string; ing: string; past: string; target?: false };
const v = (base: string, ing: string, past: string, target?: false): HaVerb => ({ base, ing, past, target });

const HA_VERBS: Record<string, HaVerb> = {
  getlivecontext: v('look at your home', 'Looking at your home', 'Looked at your home', false),
  hassturnon: v('turn on', 'Turning on', 'Turned on'),
  hassturnoff: v('turn off', 'Turning off', 'Turned off'),
  hasslightset: v('adjust', 'Adjusting', 'Adjusted'),
  hassclimatesettemperature: v('set the temperature', 'Setting the temperature', 'Set the temperature'),
  hasssetposition: v('move', 'Moving', 'Moved'),
  hassmediapause: v('pause', 'Pausing', 'Paused'),
  hassmediaunpause: v('play', 'Playing', 'Played'),
  hassmedianext: v('skip to the next track', 'Skipping to the next track', 'Skipped to the next track'),
  hassgetstate: v('check', 'Checking', 'Checked'),
  schedule_command: v('set something for later in Home Assistant', 'Setting something for later', 'Set something for later'),
};

const LOCK_ON = v('lock', 'Locking', 'Locked');
const LOCK_OFF = v('unlock', 'Unlocking', 'Unlocked');
const OPEN = v('open', 'Opening', 'Opened');
const CLOSE = v('close', 'Closing', 'Closed');
const SAFE_ON = v('lock or close', 'Locking or closing', 'Locked or closed');
const SAFE_OFF = v('unlock or open', 'Unlocking or opening', 'Unlocked or opened');

const DOMAIN_ALL: Record<string, string> = {
  light: 'all lights',
  switch: 'all switches',
  fan: 'all fans',
  cover: 'all blinds and covers',
  media_player: 'all speakers and TVs',
  climate: 'the heating',
  lock: 'all locks',
};

export interface HomeAction {
  base: string;
  ing: string;
  past: string;
  /** Who or what it acts on, in words ("Living room lamp"); null when the call names nothing. */
  target: string | null;
  /** Words after the target ("to 21 °C"), or after the verb when there's no target. */
  rest?: string;
  /** Whether this verb takes a target at all (false for "look at your home"). */
  takesTarget: boolean;
}

const str = (x: unknown): string | null => (typeof x === 'string' && x.trim() ? x.trim() : null);
const firstStr = (x: unknown): string | null => (Array.isArray(x) ? str(x[0]) : str(x));
const humanise = (n: string) => (/^[a-z0-9]+(_[a-z0-9]+)+$/.test(n) ? cap(n.replace(/_/g, ' ')) : n);

function haTarget(args: Record<string, unknown>, domain: string | null): string | null {
  const name = str(args.name);
  if (name) return humanise(name);
  for (const k of ['area', 'floor'] as const) {
    const place = str(args[k]);
    if (place) return domain === 'light' ? `${place} lights` : `everything in ${place}`;
  }
  if (domain) return DOMAIN_ALL[domain] ?? `all ${domain.replace(/_/g, ' ')}s`;
  return null;
}

/**
 * A Home Assistant tool call in house words, or null when the tool isn't HA's. `alwaysAsk` is the
 * server's 26f `home_always_ask` mark (not the started-by `always_ask`) for locks, doors, gates and alarms: with no domain in the args, an on/off call
 * then reads "unlock or open", never "turn off Front door".
 */
export function homeAction(tool: string, args?: Record<string, unknown> | null, alwaysAsk?: boolean): HomeAction | null {
  const bare = split(tool).tool.replace(/^(intent__|homeassistant__)/i, '');
  const known = HA_VERBS[bare.toLowerCase()];
  if (!known) return null;
  const a = args && typeof args === 'object' ? args : {};
  const domain = firstStr(a.domain);
  const deviceClass = firstStr(a.device_class);
  let verb = known;
  const on = bare.toLowerCase() === 'hassturnon';
  const off = bare.toLowerCase() === 'hassturnoff';
  if (on || off) {
    if (domain === 'lock') verb = on ? LOCK_ON : LOCK_OFF;
    else if (domain === 'cover' || domain === 'valve' || (deviceClass && /^(garage|gate|door)$/.test(deviceClass))) verb = on ? OPEN : CLOSE;
    else if (!domain && alwaysAsk) verb = on ? SAFE_ON : SAFE_OFF;
  }
  if (known.target === false) return { ...verb, target: null, takesTarget: false };
  if (bare.toLowerCase() === 'schedule_command') {
    const cmd = str(a.command);
    const cut = cmd && cmd.length > 60 ? `${cmd.slice(0, 59)}…` : cmd;
    return cut
      ? { base: 'do later in Home Assistant:', ing: verb.ing, past: verb.past, target: cut, takesTarget: true }
      : { ...verb, target: null, takesTarget: false };
  }
  const target = haTarget(a, domain);
  let rest: string | undefined;
  if (bare.toLowerCase() === 'hassclimatesettemperature' && (typeof a.temperature === 'number' || str(a.temperature))) {
    // "set the temperature" + target reads as "set <T> to 21 °C".
    const t = `${a.temperature} °C`;
    return target
      ? { base: 'set', ing: 'Setting', past: 'Set', target, rest: `to ${t}`, takesTarget: true }
      : { ...verb, target: null, rest: `to ${t}`, takesTarget: true };
  }
  if (bare.toLowerCase() === 'hasssetposition' && (typeof a.position === 'number' || str(a.position))) rest = `to ${a.position} %`;
  if (bare.toLowerCase() === 'hassmedianext' && target) return { ...verb, target, rest: undefined, takesTarget: true, base: 'skip to the next track on' };
  return { ...verb, target, rest, takesTarget: true };
}

/** One HA action as plain text in a form: "turn off Living room lamp", "turn something off". */
export function homePhrase(h: HomeAction, form: 'base' | 'ing' | 'past'): string {
  const words = form === 'base' ? h.base : form === 'ing' ? h.ing.toLowerCase() : h.past.toLowerCase();
  if (!h.takesTarget) return [words, h.rest].filter(Boolean).join(' ');
  if (h.target) return [words, h.target, h.rest].filter(Boolean).join(' ');
  // "turn off" → "turn something off"; other verbs → "<verb> something".
  // "set the temperature" already names what it acts on.
  const sp = /^(.*) (on|off)$/.exec(words);
  const head = sp ? `${sp[1]} something ${sp[2]}` : /\bthe\b/.test(words) ? words : `${words} something`;
  return [head, h.rest].filter(Boolean).join(' ');
}

// ---- 36a one-status-steps: shell steps in plain words ----

/** The command a shell step ran, from its title (server `summarize`: "Running `…`", cut at 80 with "…"), or null. */
export function shellCommandOf(title: string): string | null {
  const m = /^Running `([\s\S]+)`$/.exec(title.trim());
  return m ? m[1] : null;
}

/** ssh options that take a value (OpenSSH), so the value isn't read as the host. */
const SSH_VALUE_OPTS = new Set('BbcDEeFIiJLlmOoPpQRSWw'.split(''));

/**
 * The host an `ssh` command connects to ("root@203.0.113.66" → "203.0.113.66"), or null when the command isn't
 * ssh or the host isn't plain (letters, digits, dots, dashes, colons for IPv6). Words only: nothing is resolved.
 */
export function sshHost(cmd: string): string | null {
  const words = cmd.trim().split(/\s+/);
  if (!words.length || !/(^|\/)(ssh|mosh)$/.test(words[0])) return null;
  for (let i = 1; i < words.length; i++) {
    const w = words[i];
    if (w === '--') {
      i++;
      if (i < words.length) return plainHost(words[i]);
      return null;
    }
    if (w.startsWith('-') && w.length > 1) {
      // "-p 22" takes the next word; "-p22" and "-oX=Y" carry their own; flags like "-tt" take none.
      const last = w[w.length - 1];
      if (w.length === 2 && SSH_VALUE_OPTS.has(last)) i++;
      continue;
    }
    return plainHost(w);
  }
  return null;
}

function plainHost(dest: string): string | null {
  let h = dest.replace(/^ssh:\/\//, '').replace(/^['"]|['"]$/g, '');
  h = h.slice(h.lastIndexOf('@') + 1).replace(/\/.*$/, '');
  if (/^\[[0-9a-fA-F:.]+\](:\d+)?$/.test(h)) h = h.slice(1, h.indexOf(']'));
  else if (/^[^:]+:\d+$/.test(h)) h = h.slice(0, h.indexOf(':'));
  return /^[A-Za-z0-9][A-Za-z0-9.\-_:]{0,252}$/.test(h) ? h : null;
}

/** Words that say a command couldn't reach its host (ssh, curl, git), so a retried step can say "didn't connect". */
const NO_CONNECT = /connection (timed out|refused|reset|closed)|could not resolve|no route to host|network is unreachable|operation timed out|connect to host|timed out/i;

/**
 * 36a: a shell step's title in words, with the host it ran on when it is ssh ("Running a command on 203.0.113.66").
 * A step the bot retried after says what happened: "A command on 203.0.113.66 didn't connect, so Quant tried again".
 * `host` is set so the row can set it in bold. Null for any other step (its title stays as readableStepTitle makes it).
 */
export function plainShellTitle(step: { title: string; detail?: string }, retried = false, who?: string): { text: string; host: string | null } | null {
  const cmd = shellCommandOf(step.title);
  if (!cmd) return null;
  const host = sshHost(cmd);
  if (retried) {
    const verb = NO_CONNECT.test(step.detail ?? '') ? 'didn’t connect' : 'didn’t work';
    const name = who?.trim() || 'the bot';
    return { text: `${host ? `A command on ${host}` : 'A command'} ${verb}, so ${name} tried again`, host };
  }
  return { text: host ? `Running a command on ${host}` : 'Running a command', host };
}

/**
 * 36a: the command behind "Show command": the step's detail when it is the whole command (a running or finished
 * step), else the title's (cut at 80, ending "…"; a failed step's detail is its error, not the command).
 */
export function shellCommandText(step: { title: string; detail?: string }): string | null {
  const cmd = shellCommandOf(step.title);
  if (!cmd) return null;
  const head = cmd.endsWith('…') ? cmd.slice(0, -1) : cmd;
  return step.detail && step.detail.startsWith(head) ? step.detail : cmd;
}
