import React, { useId, useRef, useState } from 'react';
import { ShieldAlert, Check, CheckCheck, X, MessageSquare, FileText, Clock, CreditCard, GraduationCap, ChevronDown, Lock } from 'lucide-react';
import { playSfx } from '../audio/chiptune';
import { formatArgs, parseMcpToolName } from '../mcp';
import { tierReason } from '../flightSchool';
import { toolAction, homeAction, homePhrase, type HomeAction } from '../toolLabel';
import { localClock } from '../clock';
import { isPhoneLayout } from '../layout';
import { parseProdFloor, type BackupResult, type FloorBackup, type FloorResolveOpts } from '../dev/types';
import { ProdFloorCard } from './dev/ProdFloorCard';
import { DENY_REASON_MAX } from '../away';

export interface PendingApproval {
  id: string;
  tool_name: string;
  details: Record<string, unknown>;
  /**
   * The bot's turn stopped waiting for this card (status `waiting`), but it's still open: Allow
   * starts the bot again to make that call, Deny closes it.
   */
  waiting?: boolean;
  /** When the bot asked (ISO time), for the card's header; left out when unknown. */
  asked_at?: string;
}

interface ModernApprovalCardProps {
  approval: PendingApproval;
  /** `opts` only from the production-floor card: Allow without a backup (22f). */
  onResolve: (id: string, decision: 'allow_once' | 'always_allow' | 'deny', opts?: FloorResolveOpts) => void;
  /** While an answer is being sent. */
  disabled?: boolean;
  /** Forces the phone or desktop look (tests); by default it follows the window when the card appears. */
  phone?: boolean;
  /** Production floor (Dev mode): takes a backup; defaults to the space's backup route. */
  onBackup?: (spaceId: string) => Promise<BackupResult>;
  /** Floor card only (22f): saves the backup command / re-reads the backup state; tests and the rig fake them. */
  onSetCommand?: (spaceId: string, command: string) => Promise<void>;
  onLoadBackup?: (spaceId: string) => Promise<FloorBackup | null>;
  /** 29q: the Say why field stacks (field, then a full-width button) at every width, for a narrow column like the Inbox list. */
  stackWhy?: boolean;
}

/** Server/tool/arguments of an MCP approval, or null for other tools. */
const mcpView = (approval: PendingApproval) => {
  const d = approval.details ?? {};
  const fromName = parseMcpToolName(approval.tool_name);
  if (d.source !== 'mcp' && !fromName) return null;
  return {
    server: typeof d.server === 'string' ? d.server : fromName?.server ?? '',
    tool: typeof d.tool === 'string' ? d.tool : fromName?.tool ?? approval.tool_name,
    kwant: typeof d.kwant_name === 'string' ? d.kwant_name : null,
    args: formatArgs(d.arguments),
    raw: d.arguments,
  };
};

/** Arguments worth a line of their own on the card, in this order (0.6.13 spec C1). */
const KEY_FACTS = ['title', 'name', 'path', 'files', 'url', 'repo', 'command'];

/**
 * Up to two short "key: value" lines from a tool's arguments, so the card says what the call is
 * about without opening the details. Text, numbers, and an array of strings (joined); everything
 * else stays in the details.
 */
export function keyFacts(args: unknown, max = 2): Array<[string, string]> {
  if (!args || typeof args !== 'object' || Array.isArray(args)) return [];
  const a = args as Record<string, unknown>;
  const out: Array<[string, string]> = [];
  for (const k of KEY_FACTS) {
    const v = a[k];
    const text = typeof v === 'string' ? v.trim()
      : typeof v === 'number' ? String(v)
      : Array.isArray(v) ? v.filter((x) => typeof x === 'string').join(', ')
      : '';
    if (text) out.push([k, text]);
    if (out.length >= max) break;
  }
  return out;
}

/** Command and folder of a kwant shell command on this computer, or null for other tools. */
export const localShellView = (approval: PendingApproval) => {
  const d = approval.details ?? {};
  if (d.source !== 'local_shell') return null;
  const args = (d.arguments ?? {}) as Record<string, unknown>;
  return {
    command: typeof args.command === 'string' ? args.command : '',
    cwd: typeof args.cwd === 'string' && args.cwd ? args.cwd : null,
  };
};

const BUILTIN_SHELL = ['Bash'];
const BUILTIN_FILE = ['Write', 'Edit', 'MultiEdit', 'NotebookEdit'];
const s = (v: unknown) => (typeof v === 'string' && v ? v : null);

/**
 * An approval known only by its tool and a one-line summary (a reload against a server that
 * doesn't send the details yet), or null.
 */
export const summaryView = (approval: PendingApproval) => {
  const d = approval.details ?? {};
  if (d.source !== 'summary') return null;
  const tool = s(d.tool) ?? approval.tool_name;
  return { tool, kwant: s(d.kwant_name), summary: s(d.summary), builtin: BUILTIN_SHELL.includes(tool) || BUILTIN_FILE.includes(tool) };
};

/**
 * A Claude CLI built-in tool (Bash, Write, Edit, ...) the bot asks to use: the command or file in
 * plain words, or null for other tools. Reads `details.arguments` (or `details.input`, the CLI's
 * own name for it) and works without `source` when the tool name is a known built-in.
 */
export const builtinView = (approval: PendingApproval) => {
  const d = approval.details ?? {};
  const tool = s(d.tool) ?? approval.tool_name;
  const known = BUILTIN_SHELL.includes(tool) || BUILTIN_FILE.includes(tool);
  if (d.source === 'summary' || (d.source !== 'claude_builtin' && !known)) return null;
  const raw = d.arguments ?? d.input;
  const args = (raw && typeof raw === 'object' && !Array.isArray(raw) ? raw : {}) as Record<string, unknown>;
  const base = { tool, kwant: s(d.kwant_name), cwd: s(d.cwd), outsideFolder: d.outside_folder === true };
  if (BUILTIN_SHELL.includes(tool)) {
    return { ...base, kind: 'shell' as const, command: s(args.command) ?? '', description: s(args.description), path: null, change: null };
  }
  if (BUILTIN_FILE.includes(tool)) {
    const path = s(args.file_path) ?? s(args.notebook_path) ?? s(args.path);
    const edits = Array.isArray(args.edits) ? args.edits.length : 0;
    const change =
      tool === 'Write'
        ? `Write ${typeof args.content === 'string' ? `${args.content.split('\n').length} lines` : 'the file'}`
        : tool === 'MultiEdit'
          ? `${edits} ${edits === 1 ? 'edit' : 'edits'}`
          : tool === 'NotebookEdit'
            ? 'Change a notebook cell'
            : 'Replace text';
    return { ...base, kind: 'file' as const, command: null, description: null, path, change };
  }
  return { ...base, kind: 'other' as const, command: null, description: null, path: null, change: null, args: formatArgs(args) };
};

/**
 * Why the server's payment floor (src/mcp/payment_floor.rs) asked, or null for a card that isn't
 * a payment. A payment always asks, and allowing it covers that one call.
 */
export const paymentReason = (approval: PendingApproval) => {
  const why = approval.details?.payment_reason;
  if (typeof why !== 'string') return null;
  return why.trim().replace(/[.\s]+$/, '') || 'it looks like paying';
};

/**
 * 26h: whose credits a paid generation spends ("Black Forest Labs", "Picsart"), from the server's
 * `details.credits`, or null for any other card. The card never guesses a price.
 */
export const paymentCredits = (approval: PendingApproval) => {
  const who = approval.details?.credits;
  return typeof who === 'string' && who.trim() ? who.trim() : null;
};

/**
 * Who started a turn, as the card names it after "Started by": the server's raw label
 * (`details.started_by`) mapped to a name people know (Mira's table, #569). Internal names
 * nobody sees on screen become plain nouns; a label not listed shows its part before the first ":".
 */
const STARTED_BY_NAMES: Record<string, string> = {
  'Loose ends: prepare drafts and next steps': 'Loose ends',
  Watch: 'Watch',
  Verify: 'a task check',
  Audit: 'an audit',
  'Goal draft': 'a goal draft',
  'Skill Foundry': 'a skill draft',
  'Skill Foundry replay': 'a skill draft’s test run',
  'Idea jar': 'Idea jar',
  'Show it once': 'Show it once',
  'Dev mode card': 'Dev mode',
};

export const startedByName = (label: string) => {
  const raw = label.trim();
  return STARTED_BY_NAMES[raw] ?? (raw.split(':')[0].trim() || raw);
};

/**
 * Why a card asks although nobody typed a request: kwant started the turn on its own (the
 * server's `details.started_by`). A strict turn (`details.strict_ask`, a Skill Foundry draft)
 * asks even on Don’t ask, so it says so instead. Null on an ordinary turn's card.
 */
export const startedByNote = (approval: PendingApproval) => {
  const label = approval.details?.started_by;
  if (typeof label !== 'string' || !label.trim()) return null;
  const who = startedByName(label);
  return approval.details?.strict_ask === true
    ? `Started by ${who}, not by you. Skill drafts always ask, even on Don’t ask.`
    : `Started by ${who}, not by you, so each change asks first. Bots on Don’t ask (Settings › This space) run these without a card.`;
};

/** The line under a still-open card's heading. */
export const waitingNote = (bot: string | null) =>
  `${bot ?? 'The bot'} stopped waiting for an answer and nothing ran. Allow starts it again to make this call; Deny closes the card.`;

/**
 * What the bot asks to do, in words, after "<Bot> wants to": "open a pull request on GitHub".
 * The card's sentence and the Inbox title both use it, so they always say the same thing.
 */
export function approvalAction(approval: PendingApproval): string {
  if (localShellView(approval)) return 'run a command on your computer';
  const mcp = mcpView(approval);
  const builtin = mcp ? null : builtinView(approval);
  if (builtin?.kind === 'shell') return 'run a command';
  if (builtin?.kind === 'file') return `${builtin.tool === 'Write' ? 'write a file' : 'change a file'}${builtin.outsideFolder ? ' outside its own folder' : ''}`;
  if (builtin) return toolAction(builtin.tool);
  const summary = summaryView(approval);
  if (summary) return toolAction(summary.tool);
  const home = homeView(approval);
  if (home) return homePhrase(home, 'base');
  return mcp ? toolAction(mcp.tool, mcp.server || null) : toolAction(approval.tool_name);
}

/** A Home Assistant call in house words (26e), or null for any other tool. */
export function homeView(approval: PendingApproval): HomeAction | null {
  const mcp = mcpView(approval);
  if (!mcp) return null;
  const args = mcp.raw && typeof mcp.raw === 'object' && !Array.isArray(mcp.raw) ? (mcp.raw as Record<string, unknown>) : null;
  return homeAction(mcp.tool, args, approval.details?.home_always_ask === true);
}

/** The facts under a Home Assistant card: Area (when it isn't the target), light settings, and where. */
function homeFacts(raw: unknown): Array<[string, string]> {
  const a = raw && typeof raw === 'object' && !Array.isArray(raw) ? (raw as Record<string, unknown>) : {};
  const out: Array<[string, string]> = [];
  const area = typeof a.area === 'string' && a.area.trim() ? a.area.trim() : null;
  // The area is a fact unless it became the target itself (no name in the call).
  const named = typeof a.name === 'string' && a.name.trim() !== '';
  if (area && named) out.push(['Area', area]);
  if (typeof a.brightness === 'number' || (typeof a.brightness === 'string' && a.brightness)) out.push(['Brightness', `${a.brightness} %`]);
  if (typeof a.color === 'string' && a.color) out.push(['Colour', a.color]);
  out.push(['In', 'Home Assistant']);
  return out;
}

/** The 26f note: these never get "Always allow". */
export const LOCK_NOTE = 'Locks, doors, garage doors, gates and alarms always ask. Always allow isn’t offered for them.';

/** The pre/JSON style inside a card: wraps between words, and only splits a word too long for the line. */
/** "Show details": closed on a phone, so Allow is on screen with the card's top; open on desktop. */
const ApprovalDetails: React.FC<{ phone?: boolean; children: React.ReactNode }> = ({ phone, children }) => {
  const [open, setOpen] = useState(() => !(phone ?? isPhoneLayout()));
  const id = useId();
  return (
    <div className="mt-1.5">
      <button
        type="button"
        aria-expanded={open}
        aria-controls={id}
        onClick={() => setOpen((o) => !o)}
        data-approval-details-toggle=""
        className="-ml-2 inline-flex items-center gap-1 min-h-[44px] sm:min-h-[32px] px-2 rounded-lg text-sm text-[#a39dcc] hover:text-zinc-100 focus-visible:outline focus-visible:outline-2 focus-visible:outline-[#b8adff]"
      >
        {open ? 'Hide details' : 'Show details'}
        <ChevronDown size={14} aria-hidden="true" className={`transition-transform motion-reduce:transition-none ${open ? 'rotate-180' : ''}`} />
      </button>
      {open && (
        <div id={id} className="mt-1 space-y-1.5" data-approval-details="">
          {children}
        </div>
      )}
    </div>
  );
};

/** The card's secondary button (Deny, Always allow, Deny and tell). */
const SECONDARY =
  'inline-flex items-center justify-center gap-1.5 min-h-[44px] sm:min-h-[36px] px-3 rounded-lg border border-[#3f3868] bg-[#231d4b] text-sm font-medium text-[#ecebfa] hover:bg-[#2c2552] transition-colors focus-visible:outline focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-[#b8adff] disabled:opacity-50 disabled:cursor-not-allowed';

/** 29r: Deny and Always in their row: equal halves on a phone (basis 0, never narrower than the label); from sm their own width, filling the line only if the row wraps (the narrow Inbox list). */
const ROW_FIT = 'flex-1 basis-0 sm:flex-auto';

/**
 * 29p "Say why": a quiet button in the action row that opens a one-line field under it, with "Deny and tell
 * <bot>". Its own component so the card itself keeps no state. Fragment children of the action row: the form
 * takes a whole line of its own (basis-full) on a wide screen and comes last on a phone.
 */
const SayWhy: React.FC<{ botName: string | null; disabled: boolean; stack?: boolean; onDeny: (reason: string) => void }> = ({ botName, disabled, stack = false, onDeny }) => {
  // null while closed, else the reason typed so far.
  const [why, setWhy] = useState<string | null>(null);
  const id = useId();
  const ref = useRef<HTMLButtonElement>(null);
  const tellWho = botName ?? 'the bot';
  return (
    <>
      <button
        ref={ref}
        type="button"
        disabled={disabled}
        aria-expanded={why !== null}
        aria-controls={id}
        onClick={() => setWhy((w) => (w === null ? '' : null))}
        data-approval-say-why=""
        className={`self-start sm:self-auto sm:mr-auto -ml-2 sm:ml-0 inline-flex items-center gap-1.5 min-h-[44px] sm:min-h-[32px] px-2 rounded-lg text-sm ${why === null ? 'text-[#a39dcc]' : 'text-zinc-100'} hover:text-zinc-100 focus-visible:outline focus-visible:outline-2 focus-visible:outline-[#b8adff] disabled:opacity-50 disabled:cursor-not-allowed`}
      >
        <MessageSquare size={14} aria-hidden="true" />
        <span>Say why</span>
      </button>
      {why !== null && (
        <form
          id={id}
          data-approval-why=""
          className={`sm:basis-full flex flex-col gap-2 ${stack ? '' : 'sm:flex-row sm:items-center'}`}
          onSubmit={(e) => {
            e.preventDefault();
            onDeny(why);
          }}
        >
          <input
            type="text"
            autoFocus
            value={why}
            maxLength={DENY_REASON_MAX}
            disabled={disabled}
            onChange={(e) => setWhy(e.target.value.slice(0, DENY_REASON_MAX))}
            onKeyDown={(e) => {
              if (e.key !== 'Escape') return;
              e.preventDefault();
              e.stopPropagation();
              setWhy(null);
              ref.current?.focus();
            }}
            aria-label={`Tell ${tellWho} why you’re denying this`}
            placeholder="What should it do instead?"
            enterKeyHint="send"
            autoComplete="off"
            className={`min-w-0 w-full ${stack ? '' : 'sm:flex-1'} min-h-[44px] sm:min-h-[36px] px-3 rounded-lg border border-[#3f3868] bg-zinc-950/70 text-base sm:text-sm text-[#ecebfa] placeholder:text-[#a39dcc] focus-visible:outline focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-[#b8adff] disabled:opacity-50`}
          />
          <button type="submit" disabled={disabled} data-approval-deny-why="" className={`${SECONDARY} w-full ${stack ? '' : 'sm:w-auto sm:max-w-[50%]'} min-w-0`}>
            <X size={14} aria-hidden="true" className="shrink-0" />
            <span className="min-w-0 break-words [overflow-wrap:anywhere] text-left">Deny and tell {tellWho}</span>
          </button>
        </form>
      )}
    </>
  );
};

const PRE = 'font-mono text-xs leading-[18px] whitespace-pre-wrap break-words [overflow-wrap:anywhere] max-h-[240px] overflow-y-auto rounded-md border border-zinc-800 bg-zinc-950/70 p-2';

export const ModernApprovalCard: React.FC<ModernApprovalCardProps> = ({
  approval,
  onResolve,
  disabled = false,
  phone,
  onBackup,
  onSetCommand,
  onLoadBackup,
  stackWhy = false,
}) => {
  // Dev mode's production floor has its own card: blast radius, backup row, and never "always allow".
  const floor = parseProdFloor(approval.details);
  if (floor) {
    const shellCmd = localShellView(approval) ?? builtinView(approval);
    const kwant = approval.details?.kwant_name;
    return (
      <ProdFloorCard
        id={approval.id}
        floor={floor}
        bot={typeof kwant === 'string' && kwant ? kwant : null}
        command={shellCmd?.command ?? ''}
        cwd={shellCmd?.cwd ?? null}
        disabled={disabled}
        onBackup={onBackup}
        onSetCommand={onSetCommand}
        onLoadBackup={onLoadBackup}
        onResolve={(id, decision, opts) => {
          if (disabled) return;
          playSfx('click');
          onResolve(id, decision, opts);
        }}
      />
    );
  }
  const handleDecision = (decision: 'allow_once' | 'always_allow' | 'deny') => {
    if (disabled) return;
    playSfx('click');
    onResolve(approval.id, decision);
  };
  // 29p: Deny with the person's words, so the bot hears why at once. An empty field is a plain Deny.
  const denyWithReason = (text: string) => {
    if (disabled) return;
    playSfx('click');
    const reason = text.trim();
    onResolve(approval.id, 'deny', reason ? { reason } : undefined);
  };
  const mcp = mcpView(approval);
  const shell = localShellView(approval);
  const builtin = shell || mcp ? null : builtinView(approval);
  const summary = summaryView(approval);
  const name = builtin?.kwant ?? summary?.kwant;
  const perSession = !!builtin || !!summary?.builtin;
  const waiting = approval.waiting === true;
  const botName = name ?? mcp?.kwant ?? null;
  const payment = paymentReason(approval);
  const credits = payment ? paymentCredits(approval) : null;
  const tier = payment ? null : tierReason(approval.details);
  const startedBy = payment || tier ? null : startedByNote(approval);
  const time = localClock(approval.asked_at);
  // A session allow doesn't count in a turn kwant started on its own (the server asks again),
  // so those cards don't offer one.
  const alwaysAsk = approval.details?.always_ask === true;
  // 26f: a lock, door, garage door, gate or alarm call always asks; its own field, not `always_ask`.
  const homeAlwaysAsk = approval.details?.home_always_ask === true;
  const offerAlways = !shell && !waiting && !payment && !alwaysAsk && !homeAlwaysAsk;
  // 29p: a `waiting` card's turn has stopped, so nobody would hear the reason; plain Deny only there.
  const offerWhy = !waiting;

  const bot = botName ? <strong className="font-semibold text-zinc-50">{botName}</strong> : 'A bot';
  // "<Bot> wants to open a pull request on <Server>", the server in bold.
  const action = approvalAction(approval);
  const home = mcp && !summary ? homeView(approval) : null;
  // Home Assistant: bold the device, never the "on" of "turn on" (design §3.3).
  const on = mcp && !summary && !home ? / on (.+)$/.exec(action) : null;
  // 26f marks lock/door/alarm calls with `home_always_ask` (`always_ask` stays the started-by turn's
  // field). Only a Home Assistant card gets the lock note, as a second guard.
  const lockNote = !!home && homeAlwaysAsk;
  const what: React.ReactNode = home?.target ? (
    <>
      {home.base} <strong className="font-semibold text-zinc-50">{home.target}</strong>
      {home.rest ? ` ${home.rest}` : ''}
    </>
  ) : on ? <>{action.slice(0, on.index)} on <strong className="font-semibold text-zinc-50">{on[1]}</strong></> : action;

  const facts: Array<[string, string]> = home
    ? homeFacts(mcp?.raw)
    : mcp
    ? keyFacts(mcp.raw)
    : builtin?.kind === 'file' && builtin.path
      ? [['path', builtin.path]]
      : builtin && 'args' in builtin
        ? keyFacts((approval.details?.arguments ?? approval.details?.input) as unknown)
        : [];

  // What the details disclosure holds: the raw call, for the person who wants to check it.
  const details: React.ReactNode = shell ? (
    shell.cwd ? (
      <p className="text-xs leading-[18px] text-[#a39dcc] break-words [overflow-wrap:anywhere]">
        In folder: <span className="font-mono text-zinc-200">{shell.cwd}</span>
      </p>
    ) : null
  ) : summary ? (
    summary.summary ? <p className="text-xs leading-[18px] text-zinc-300 break-words">{summary.summary}</p> : null
  ) : builtin?.kind === 'shell' ? (
    builtin.cwd ? (
      <p className="text-xs leading-[18px] text-[#a39dcc] break-words [overflow-wrap:anywhere]">
        In folder: <span className="font-mono text-zinc-200">{builtin.cwd}</span>
      </p>
    ) : null
  ) : builtin?.kind === 'file' ? (
    <p className="text-xs leading-[18px] text-zinc-300 break-words">
      {builtin.change}
      {builtin.cwd && (
        <>
          {' · '}
          <span className="text-[#a39dcc]">Its folder:</span> <span className="font-mono text-zinc-200 [overflow-wrap:anywhere]">{builtin.cwd}</span>
        </>
      )}
    </p>
  ) : builtin ? (
    'args' in builtin && builtin.args ? <pre className={`${PRE} text-zinc-300`}>{builtin.args}</pre> : <p className="text-xs text-zinc-500">No arguments.</p>
  ) : mcp ? (
    <>
      <p className="font-mono text-xs leading-[18px] text-zinc-500 break-words [overflow-wrap:anywhere]">
        {mcp.server} · {mcp.tool}
      </p>
      {mcp.args ? <pre className={`${PRE} text-zinc-300`}>{mcp.args}</pre> : <p className="text-xs text-zinc-500">No arguments.</p>}
    </>
  ) : (
    <>
      <p className="font-mono text-xs leading-[18px] text-zinc-500 break-words [overflow-wrap:anywhere]">{approval.tool_name}</p>
      <pre className={`${PRE} text-zinc-300`}>{JSON.stringify(approval.details, null, 2)}</pre>
    </>
  );


  return (
    <div
      data-approval-card={approval.id}
      className={`my-3 border rounded-xl p-3 sm:p-4 text-zinc-100 max-w-2xl shadow-lg backdrop-blur-sm ${
        waiting ? 'border-sky-500/30 bg-sky-950/20' : 'border-amber-500/30 bg-amber-950/20'
      }`}
    >
      <div className="flex items-center gap-2.5" data-approval-header="">
        <span
          className={`shrink-0 inline-flex h-7 w-7 items-center justify-center rounded-full ${waiting ? 'bg-sky-500/15 text-sky-300' : 'bg-amber-500/15 text-amber-400'}`}
          aria-hidden="true"
        >
          {waiting ? <Clock size={16} /> : <ShieldAlert size={16} />}
        </span>
        <h3 className="min-w-0 flex-1 text-[15px] leading-5 font-semibold text-zinc-50">{waiting ? 'Still open: you can still allow' : 'Approval needed'}</h3>
        {time && (
          <time dateTime={approval.asked_at} className="shrink-0 text-xs leading-4 text-[#a39dcc] tabular-nums">
            {time}
          </time>
        )}
      </div>
      {waiting && <p className="mt-1.5 text-xs leading-[18px] text-zinc-400">{waitingNote(botName)}</p>}

      <p className="mt-2.5 text-base leading-6 text-zinc-200 break-words" data-approval-summary="">
        {bot} wants to {what}
      </p>
      {startedBy && (
        <p role="note" data-started-by className="mt-1 text-xs leading-[18px] text-zinc-400 break-words">
          {startedBy}
        </p>
      )}

      {(shell || builtin?.kind === 'shell') && (
        <>
          {builtin?.kind === 'shell' && builtin.description && <p className="mt-1 text-sm leading-5 text-zinc-400 break-words">{builtin.description}</p>}
          <pre className={`${PRE} mt-2 text-zinc-100`} aria-label="Command">
            {(shell ? shell.command : builtin?.command) || '(empty command)'}
          </pre>
        </>
      )}

      {facts.length > 0 && (
        <dl className="mt-2 space-y-1" data-approval-facts="">
          {facts.map(([k, v]) => (
            <div key={k} className="flex gap-2 text-sm leading-5 min-w-0">
              <dt className="shrink-0 text-[#a39dcc]">
                {builtin?.kind === 'file' && <FileText size={12} className="inline mr-1 -mt-0.5" aria-hidden="true" />}
                {k}
              </dt>
              <dd className="min-w-0 text-[#ecebfa] break-words [overflow-wrap:anywhere] line-clamp-2" title={v}>
                {v}
              </dd>
            </div>
          ))}
        </dl>
      )}

      {lockNote && (
        <p role="note" data-lock-note className="flex items-start gap-1.5 mt-2 text-xs leading-[18px] text-amber-200 break-words">
          <Lock size={14} className="mt-0.5 shrink-0" aria-hidden="true" />
          {LOCK_NOTE}
        </p>
      )}

      {payment && (
        <div role="note" className="flex items-start gap-2 mt-2.5 rounded-lg border border-rose-500/30 bg-rose-950/30 px-3 py-2 text-sm leading-5">
          <CreditCard size={14} className="mt-0.5 shrink-0 text-rose-300" aria-hidden="true" />
          <div className="min-w-0 space-y-0.5">
            {credits ? (
              <>
                <p className="text-rose-100 break-words" data-credits>
                  <span className="font-semibold">This uses your {credits} credits.</span>
                </p>
                <p className="text-xs leading-[18px] text-zinc-400">Anything that spends credits asks you first, even with approvals off. Allowing covers just this one.</p>
              </>
            ) : (
              <>
                <p className="text-rose-100 break-words">
                  <span className="font-semibold">This looks like a payment:</span> {payment}.
                </p>
                <p className="text-xs leading-[18px] text-zinc-400">Payments always ask you, even with approvals off. Allowing covers this one payment.</p>
              </>
            )}
          </div>
        </div>
      )}

      {tier && (
        <div role="note" data-tier-reason className="flex items-start gap-2 mt-2.5 rounded-lg border border-sky-500/30 bg-sky-950/30 px-3 py-2 text-sm leading-5">
          <GraduationCap size={14} className="mt-0.5 shrink-0 text-sky-300" aria-hidden="true" />
          <div className="min-w-0 space-y-0.5">
            <p className="text-sky-100 break-words">
              <span className="font-semibold">Flight School:</span> {tier}.
            </p>
            <p className="text-xs leading-[18px] text-zinc-400">This bot’s tier asks you here, even with approvals off.</p>
          </div>
        </div>
      )}

      {details && <ApprovalDetails phone={phone}>{details}</ApprovalDetails>}

      {/* Allow on a row of its own, first; Deny and Always allow share the row under it. On a wider
          screen they sit in one row, right-aligned, with Allow last. Local shell approvals are never
          remembered, a late answer only lets that one call run, and a payment or a turn kwant
          started on its own always asks again, so none of them gets an "always". */}
      <div data-approval-actions className="mt-3 flex flex-col gap-2 sm:flex-row-reverse sm:flex-wrap sm:items-center sm:justify-start">
        <button
          type="button"
          disabled={disabled}
          onClick={() => handleDecision('allow_once')}
          data-approval-allow=""
          className="w-full sm:w-auto inline-flex items-center justify-center gap-1.5 min-h-[48px] max-sm:!min-h-[48px] sm:min-h-[36px] px-4 rounded-lg bg-teal-300 hover:bg-teal-200 text-[15px] sm:text-sm font-semibold text-zinc-950 shadow-sm transition-colors focus-visible:outline focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-[#b8adff] disabled:opacity-50 disabled:cursor-not-allowed"
        >
          <Check size={16} aria-hidden="true" />
          <span>{waiting ? 'Allow and continue' : 'Allow once'}</span>
        </button>
        {/* 29r: labels never break. On a phone the two share the row in equal halves while both labels fit and
            the second drops to its own full-width line when they don't (320 px, Away and the Inbox, "Always this
            session"); no measuring, the wrap does it. From sm up both keep their own width, as before, and a wrapped one fills its line. */}
        <div data-approval-deny-row="" className="flex flex-wrap gap-2">
          <button type="button" disabled={disabled} onClick={() => handleDecision('deny')} className={`${SECONDARY} ${ROW_FIT}`}>
            <X size={14} aria-hidden="true" />
            <span className="whitespace-nowrap">Deny</span>
          </button>
          {offerAlways && (
            <button
              type="button"
              disabled={disabled}
              onClick={() => handleDecision('always_allow')}
              title={perSession ? 'Allow this tool for this bot in this conversation' : mcp ? 'Allow this tool for this bot in this conversation, also after kwant restarts' : undefined}
              className={`${SECONDARY} ${ROW_FIT}`}
            >
              <CheckCheck size={14} aria-hidden="true" />
              <span className="whitespace-nowrap">{perSession ? 'Always this session' : 'Always allow'}</span>
            </button>
          )}
        </div>
        {/* 29p: quiet, next to Deny (left of it on a wide screen, under the buttons on a phone). Deny itself stays one tap. */}
        {offerWhy && <SayWhy botName={botName} disabled={disabled} stack={stackWhy} onDeny={denyWithReason} />}
      </div>
    </div>
  );
};
