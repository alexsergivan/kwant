import React from 'react';
import { CheckCircle2, AlertCircle, Ban, ShieldCheck, ChevronRight, Wrench } from 'lucide-react';
import { RecordedToolCall, formatArgs, humanizeToolName, parseToolCalls } from '../mcp';

const STATUS: Record<string, { icon: React.ReactNode; label: string; cls: string }> = {
  ok: { icon: <CheckCircle2 size={12} />, label: 'Done', cls: 'text-emerald-300' },
  allowed: { icon: <ShieldCheck size={12} />, label: 'Allowed', cls: 'text-emerald-300' },
  error: { icon: <AlertCircle size={12} />, label: 'Failed', cls: 'text-rose-300' },
  denied: { icon: <Ban size={12} />, label: 'Denied', cls: 'text-zinc-400' },
};

const ToolCallRow: React.FC<{ call: RecordedToolCall }> = ({ call }) => {
  const status = STATUS[call.status ?? ''] ?? { icon: <Wrench size={12} />, label: call.status ?? '', cls: 'text-zinc-400' };
  const args = formatArgs(call.arguments);
  const hasDetail = !!args || !!call.result;
  const head = (
    <>
      <span className={`flex-shrink-0 ${status.cls}`} title={status.label}>
        {status.icon}
      </span>
      <span className="text-zinc-200 font-medium truncate">{humanizeToolName(call.tool ?? call.name)}</span>
      {call.server && <span className="text-zinc-500 truncate">· {call.server}</span>}
      <span className="ml-auto flex items-center gap-2 flex-shrink-0 text-[10px] text-zinc-500">
        <span className={status.cls}>{status.label}</span>
        {typeof call.duration_ms === 'number' && <span className="font-mono">{call.duration_ms} ms</span>}
        {hasDetail && <ChevronRight size={12} className="transition-transform group-open:rotate-90" />}
      </span>
    </>
  );

  if (!hasDetail) return <li className="flex items-center gap-2 px-3 py-1.5 text-[11px]">{head}</li>;
  return (
    <li>
      <details className="group">
        <summary className="flex items-center gap-2 px-3 py-1.5 text-[11px] cursor-pointer list-none hover:bg-zinc-900/60 [&::-webkit-details-marker]:hidden">
          {head}
        </summary>
        <div className="px-3 pb-2.5 space-y-1.5">
          {args && (
            <div>
              <p className="text-[10px] uppercase tracking-wide text-zinc-500 mb-0.5">Input</p>
              <pre className="font-mono text-[11px] text-zinc-300 bg-zinc-950/80 border border-zinc-800 rounded-md p-2 whitespace-pre-wrap break-all max-h-40 overflow-y-auto">{args}</pre>
            </div>
          )}
          {call.result && (
            <div>
              <p className="text-[10px] uppercase tracking-wide text-zinc-500 mb-0.5">Result</p>
              <pre className="font-mono text-[11px] text-zinc-300 bg-zinc-950/80 border border-zinc-800 rounded-md p-2 whitespace-pre-wrap break-all max-h-40 overflow-y-auto">{call.result}</pre>
            </div>
          )}
        </div>
      </details>
    </li>
  );
};

/** The MCP tools a reply used, recorded in the message's `tool_calls`. */
export const ToolCallsList: React.FC<{ raw: unknown }> = ({ raw }) => {
  const calls = parseToolCalls(raw);
  if (calls.length === 0) return null;
  return (
    <div className="mt-2 w-full max-w-2xl rounded-xl border border-zinc-800 bg-zinc-950/50 overflow-hidden">
      <p className="flex items-center gap-1.5 px-3 pt-2 pb-1 text-[10px] font-medium uppercase tracking-wide text-zinc-500">
        <Wrench size={11} className="text-sky-400/80" />
        Used {calls.length} {calls.length === 1 ? 'tool' : 'tools'}
      </p>
      <ul className="divide-y divide-zinc-800/70">
        {calls.map((call, i) => (
          <ToolCallRow key={call.id ?? i} call={call} />
        ))}
      </ul>
    </div>
  );
};
