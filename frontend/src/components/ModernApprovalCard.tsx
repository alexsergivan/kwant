import React from 'react';
import { ShieldAlert, Check, CheckCheck, X, Plug, FileText, Terminal } from 'lucide-react';
import { playSfx } from '../audio/chiptune';
import { formatArgs, humanizeToolName, parseMcpToolName } from '../mcp';

export interface PendingApproval {
  id: string;
  tool_name: string;
  details: Record<string, unknown>;
}

interface ModernApprovalCardProps {
  approval: PendingApproval;
  onResolve: (id: string, decision: 'allow_once' | 'always_allow' | 'deny') => void;
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
  };
};

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
 * A Claude CLI built-in tool (Bash, Write, Edit, ...) the bot asks to use: the command or file in
 * plain words, or null for other tools. Reads `details.arguments` (or `details.input`, the CLI's
 * own name for it) and works without `source` when the tool name is a known built-in.
 */
export const builtinView = (approval: PendingApproval) => {
  const d = approval.details ?? {};
  const tool = s(d.tool) ?? approval.tool_name;
  const known = BUILTIN_SHELL.includes(tool) || BUILTIN_FILE.includes(tool);
  if (d.source !== 'claude_builtin' && !known) return null;
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

export const ModernApprovalCard: React.FC<ModernApprovalCardProps> = ({
  approval,
  onResolve,
}) => {
  const handleDecision = (decision: 'allow_once' | 'always_allow' | 'deny') => {
    playSfx('click');
    onResolve(approval.id, decision);
  };
  const mcp = mcpView(approval);
  const shell = localShellView(approval);
  const builtin = shell || mcp ? null : builtinView(approval);
  const who = builtin?.kwant ? <span className="font-medium text-zinc-100">{builtin.kwant}</span> : 'A bot';

  return (
    <div className="my-3 border border-amber-500/30 bg-amber-950/20 rounded-xl p-4 text-zinc-100 max-w-2xl shadow-lg backdrop-blur-sm">
      <div className="flex items-center gap-2 mb-2 text-amber-400 font-semibold text-xs tracking-wide">
        <ShieldAlert size={16} />
        <span>Action Approval Required</span>
      </div>

      {shell ? (
        <div className="bg-zinc-900/90 rounded-lg p-3 border border-zinc-800 text-xs space-y-2 mb-3">
          <p className="text-zinc-300">A bot wants to run a command on your computer.</p>
          <pre className="font-mono text-[11px] text-zinc-100 bg-zinc-950/70 border border-zinc-800 rounded-md p-2 whitespace-pre-wrap break-all max-h-64 overflow-y-auto">
            {shell.command || '(empty command)'}
          </pre>
          {shell.cwd && (
            <p className="text-[11px] text-zinc-400 break-all">
              <span className="text-zinc-500">In folder:</span> <span className="font-mono text-zinc-300">{shell.cwd}</span>
            </p>
          )}
        </div>
      ) : builtin?.kind === 'shell' ? (
        <div className="bg-zinc-900/90 rounded-lg p-3 border border-zinc-800 text-xs space-y-2 mb-3">
          <p className="flex items-center gap-1.5 text-zinc-300">
            <Terminal size={13} className="shrink-0 text-sky-400" aria-hidden="true" />
            <span>{who} wants to run a command.</span>
          </p>
          {builtin.description && <p className="text-zinc-400 break-words">{builtin.description}</p>}
          <pre className="font-mono text-[11px] text-zinc-100 bg-zinc-950/70 border border-zinc-800 rounded-md p-2 whitespace-pre-wrap break-all max-h-64 overflow-y-auto">
            {builtin.command || '(empty command)'}
          </pre>
          {builtin.cwd && (
            <p className="text-[11px] text-zinc-400 break-all">
              <span className="text-zinc-500">In folder:</span> <span className="font-mono text-zinc-300">{builtin.cwd}</span>
            </p>
          )}
        </div>
      ) : builtin?.kind === 'file' ? (
        <div className="bg-zinc-900/90 rounded-lg p-3 border border-zinc-800 text-xs space-y-2 mb-3">
          <p className="flex items-center gap-1.5 text-zinc-300">
            <FileText size={13} className="shrink-0 text-sky-400" aria-hidden="true" />
            <span>
              {who} wants to {builtin.tool === 'Write' ? 'write a file' : 'change a file'}
              {builtin.outsideFolder ? ' outside its own folder' : ''}.
            </span>
          </p>
          <p className="font-mono text-[11px] text-zinc-100 bg-zinc-950/70 border border-zinc-800 rounded-md p-2 break-all">
            {builtin.path || '(no file given)'}
          </p>
          <p className="text-[11px] text-zinc-400 break-words">
            {builtin.change}
            {builtin.cwd && (
              <>
                {' · '}
                <span className="text-zinc-500">Its folder:</span> <span className="font-mono text-zinc-300 break-all">{builtin.cwd}</span>
              </>
            )}
          </p>
        </div>
      ) : builtin ? (
        <div className="bg-zinc-900/90 rounded-lg p-3 border border-zinc-800 text-xs space-y-2 mb-3">
          <p className="text-zinc-300">
            {who} wants to use <span className="font-medium text-sky-200">{builtin.tool}</span>.
          </p>
          {'args' in builtin && builtin.args ? (
            <pre className="font-mono text-[11px] text-zinc-300 bg-zinc-950/70 border border-zinc-800 rounded-md p-2 whitespace-pre-wrap break-all max-h-48 overflow-y-auto">
              {builtin.args}
            </pre>
          ) : (
            <p className="text-[11px] text-zinc-500">No arguments.</p>
          )}
        </div>
      ) : mcp ? (
        <div className="bg-zinc-900/90 rounded-lg p-3 border border-zinc-800 text-xs space-y-2 mb-3">
          <p className="text-zinc-300">
            {mcp.kwant ? <span className="font-medium text-zinc-100">{mcp.kwant}</span> : 'A bot'} wants to use{' '}
            <span className="font-medium text-sky-200">{humanizeToolName(mcp.tool)}</span>
            {mcp.server && (
              <>
                {' '}from <span className="inline-flex items-center gap-1 font-medium text-zinc-100"><Plug size={11} className="text-sky-400" />{mcp.server}</span>
              </>
            )}
            .
          </p>
          <p className="font-mono text-[11px] text-zinc-500">
            {mcp.server} · {mcp.tool}
          </p>
          {mcp.args ? (
            <pre className="font-mono text-[11px] text-zinc-300 bg-zinc-950/70 border border-zinc-800 rounded-md p-2 whitespace-pre-wrap break-all max-h-48 overflow-y-auto">
              {mcp.args}
            </pre>
          ) : (
            <p className="text-[11px] text-zinc-500">No arguments.</p>
          )}
        </div>
      ) : (
        <div className="bg-zinc-900/90 rounded-lg p-3 border border-zinc-800 text-xs font-mono space-y-1 mb-3">
          <div className="text-zinc-400">
            <span className="text-sky-400 font-semibold">Tool:</span> {approval.tool_name}
          </div>
          <div className="text-zinc-300 break-all overflow-x-auto max-h-36 py-1">
            {JSON.stringify(approval.details, null, 2)}
          </div>
        </div>
      )}

      <div className="flex flex-wrap items-center justify-end gap-2">
        <button
          type="button"
          onClick={() => handleDecision('deny')}
          className="flex items-center gap-1.5 px-3 py-1.5 min-h-[40px] sm:min-h-0 rounded-lg focus-visible:outline focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-sky-400 text-xs font-medium text-zinc-300 bg-zinc-800 hover:bg-zinc-700 transition-colors"
        >
          <X size={14} />
          <span>Deny</span>
        </button>

        <button
          type="button"
          onClick={() => handleDecision('allow_once')}
          className="flex items-center gap-1.5 px-3 py-1.5 min-h-[40px] sm:min-h-0 rounded-lg focus-visible:outline focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-sky-400 text-xs font-medium text-white bg-emerald-600 hover:bg-emerald-500 transition-colors shadow-sm"
        >
          <Check size={14} />
          <span>Allow Once</span>
        </button>

        {/* Local shell approvals are never remembered, so only offer a one-off answer. */}
        {!shell && (
          <button
            type="button"
            onClick={() => handleDecision('always_allow')}
            title={mcp || builtin ? 'Allow this tool for this bot until the app restarts' : undefined}
            className="flex items-center gap-1.5 px-3 py-1.5 min-h-[40px] sm:min-h-0 rounded-lg focus-visible:outline focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-sky-400 text-xs font-medium text-white bg-sky-600 hover:bg-sky-500 transition-colors shadow-sm"
          >
            <CheckCheck size={14} />
            <span>{builtin ? 'Always this session' : 'Always Allow'}</span>
          </button>
        )}
      </div>
    </div>
  );
};
