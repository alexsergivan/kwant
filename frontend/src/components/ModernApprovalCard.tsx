import React, { useId, useState } from 'react';
import { ShieldAlert, Check, CheckCheck, X, FileText, Clock, CreditCard, GraduationCap, ChevronDown } from 'lucide-react';
import { playSfx } from '../audio/chiptune';
import { formatArgs, parseMcpToolName } from '../mcp';
import { tierReason } from '../flightSchool';
import { toolAction } from '../toolLabel';
import { localClock } from '../clock';
import { isPhoneLayout } from '../layout';

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
  onResolve: (id: string, decision: 'allow_once' | 'always_allow' | 'deny') => void;
  /** While an answer is being sent. */
  disabled?: boolean;
  /** Forces the phone or desktop look (tests); by default it follows the window when the card appears. */
  phone?: boolean;
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
const KEY_FACTS = ['title', 'name', 'path', 'url', 'repo', 'command'];

/**
 * Up to two short "key: value" lines from a tool's arguments, so the card says what the call is
 * about without opening the details. Only text and numbers; everything else stays in the details.
 */
export function keyFacts(args: unknown, max = 2): Array<[string, string]> {
  if (!args || typeof args !== 'object' || Array.isArray(args)) return [];
  const a = args as Record<string, unknown>;
  const out: Array<[string, string]> = [];
  for (const k of KEY_FACTS) {
    const v = a[k];
    const text = typeof v === 'string' ? v.trim() : typeof v === 'number' ? String(v) : '';
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

/** The line under a still-open card's heading. */
export const waitingNote = (bot: string | null) =>
  `${bot ?? 'The bot'} stopped waiting for an answer and nothing ran. Allow starts it again to make this call; Deny closes the card.`;

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

const PRE = 'font-mono text-xs leading-[18px] whitespace-pre-wrap break-words [overflow-wrap:anywhere] max-h-[240px] overflow-y-auto rounded-md border border-zinc-800 bg-zinc-950/70 p-2';

export const ModernApprovalCard: React.FC<ModernApprovalCardProps> = ({
  approval,
  onResolve,
  disabled = false,
  phone,
}) => {
  const handleDecision = (decision: 'allow_once' | 'always_allow' | 'deny') => {
    if (disabled) return;
    playSfx('click');
    onResolve(approval.id, decision);
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
  const tier = payment ? null : tierReason(approval.details);
  const time = localClock(approval.asked_at);
  const offerAlways = !shell && !waiting && !payment;

  const bot = botName ? <strong className="font-semibold text-zinc-50">{botName}</strong> : 'A bot';
  // "<Bot> wants to open a pull request on <Server>", the server in bold.
  let what: React.ReactNode;
  if (shell) what = 'run a command on your computer';
  else if (builtin?.kind === 'shell') what = 'run a command';
  else if (builtin?.kind === 'file') what = `${builtin.tool === 'Write' ? 'write a file' : 'change a file'}${builtin.outsideFolder ? ' outside its own folder' : ''}`;
  else if (builtin) what = toolAction(builtin.tool);
  else if (summary) what = toolAction(summary.tool);
  else if (mcp) {
    const action = toolAction(mcp.tool, mcp.server || null);
    const on = / on (.+)$/.exec(action);
    what = on ? <>{action.slice(0, on.index)} on <strong className="font-semibold text-zinc-50">{on[1]}</strong></> : action;
  } else what = toolAction(approval.tool_name);

  const facts: Array<[string, string]> = mcp
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

  const secondary =
    'inline-flex items-center justify-center gap-1.5 min-h-[44px] sm:min-h-[36px] px-3 rounded-lg border border-[#3f3868] bg-[#231d4b] text-sm font-medium text-[#ecebfa] hover:bg-[#2c2552] transition-colors focus-visible:outline focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-[#b8adff] disabled:opacity-50 disabled:cursor-not-allowed';

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

      {payment && (
        <div role="note" className="flex items-start gap-2 mt-2.5 rounded-lg border border-rose-500/30 bg-rose-950/30 px-3 py-2 text-sm leading-5">
          <CreditCard size={14} className="mt-0.5 shrink-0 text-rose-300" aria-hidden="true" />
          <div className="min-w-0 space-y-0.5">
            <p className="text-rose-100 break-words">
              <span className="font-semibold">This looks like a payment:</span> {payment}.
            </p>
            <p className="text-xs leading-[18px] text-zinc-400">Payments always ask you, even with approvals off. Allowing covers this one payment.</p>
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
          remembered, a late answer only lets that one call run, and a payment always asks again,
          so none of them gets an "always". */}
      <div data-approval-actions className="mt-3 flex flex-col gap-2 sm:flex-row-reverse sm:items-center sm:justify-start">
        <button
          type="button"
          disabled={disabled}
          onClick={() => handleDecision('allow_once')}
          data-approval-allow=""
          className="w-full sm:w-auto inline-flex items-center justify-center gap-1.5 min-h-[48px] sm:min-h-[36px] px-4 rounded-lg bg-[#059669] hover:bg-emerald-500 text-[15px] sm:text-sm font-semibold text-white shadow-sm transition-colors focus-visible:outline focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-[#b8adff] disabled:opacity-50 disabled:cursor-not-allowed"
        >
          <Check size={16} aria-hidden="true" />
          <span>{waiting ? 'Allow and continue' : 'Allow once'}</span>
        </button>
        <div className={`grid gap-2 sm:flex ${offerAlways ? 'grid-cols-2' : 'grid-cols-1'}`}>
          <button type="button" disabled={disabled} onClick={() => handleDecision('deny')} className={secondary}>
            <X size={14} aria-hidden="true" />
            <span>Deny</span>
          </button>
          {offerAlways && (
            <button
              type="button"
              disabled={disabled}
              onClick={() => handleDecision('always_allow')}
              title={perSession ? 'Allow this tool for this bot in this conversation' : mcp ? 'Allow this tool for this bot until the app restarts' : undefined}
              className={secondary}
            >
              <CheckCheck size={14} aria-hidden="true" />
              <span>{perSession ? 'Always this session' : 'Always allow'}</span>
            </button>
          )}
        </div>
      </div>
    </div>
  );
};
