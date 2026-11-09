import React, { useEffect, useRef, useState } from 'react';
import { AlertTriangle, Download, Loader2, RefreshCw, RotateCcw, Shield, ShieldCheck } from 'lucide-react';
import { AgentVmState, Chip, ChipTone, SETTING_NAME, announcement, chipsFor, preparingCopy, rowModel } from '../claudeVm';
import { ComputerModeApi, ComputerModeError } from './ComputerModeSwitch';

const TONE: Record<ChipTone, string> = {
  isolated: 'border-teal-300/30 bg-teal-300/[.08] text-ok',
  neutral: 'border-zinc-800 bg-zinc-950/60 text-zinc-400',
  preparing: 'border-sky-500/35 bg-sky-500/10 text-sky-300',
  stopped: 'border-amber-500/35 bg-amber-500/10 text-warn',
};

const ChipIcon: React.FC<{ chip: Chip }> = ({ chip }) => {
  if (chip.tone === 'isolated') return <ShieldCheck size={12} aria-hidden="true" />;
  if (chip.tone === 'stopped') return <AlertTriangle size={12} aria-hidden="true" />;
  if (chip.tone === 'preparing')
    return chip.indeterminate ? (
      <Loader2 size={12} className="animate-spin motion-reduce:animate-none" aria-hidden="true" />
    ) : (
      <Download size={12} aria-hidden="true" />
    );
  return null;
};

/** One "Right now" chip: where an AI runs at this moment (never the setting itself). */
export const AgentChip: React.FC<{ chip: Chip }> = ({ chip }) => (
  <li
    data-agent-chip={chip.engine}
    data-tone={chip.tone}
    title={chip.title ?? undefined}
    className={`inline-flex items-center gap-[5px] h-6 px-2 rounded-md border text-[11px] whitespace-nowrap transition-opacity duration-150 motion-reduce:transition-none ${TONE[chip.tone]}`}
  >
    <ChipIcon chip={chip} />
    <span className="sr-only">{chip.spoken}</span>
    <span aria-hidden="true" className="font-medium text-zinc-200">
      {chip.name}
    </span>
    <span aria-hidden="true" className="text-zinc-600">
      ·
    </span>
    <span aria-hidden="true">{chip.label}</span>
  </li>
);

const PreparingBlock: React.FC<{ progress: AgentVmState['progress'] }> = ({ progress }) => {
  const copy = preparingCopy(progress);
  return (
    <div data-claude-iso-status="preparing" className="mt-2.5 ml-[23px] text-[11px] leading-[1.45]">
      <div className="flex items-baseline gap-3">
        <p className="flex-1 text-zinc-300">{copy.line1}</p>
        {copy.figure && <span className="shrink-0 tabular-nums text-zinc-400">{copy.figure}</span>}
      </div>
      {progress ? (
        <div
          role="progressbar"
          aria-label="Setting up Claude’s sealed box"
          aria-valuemin={0}
          aria-valuemax={progress.total_bytes}
          aria-valuenow={progress.done_bytes}
          aria-valuetext={copy.figure ?? undefined}
          className="mt-1.5 h-1 rounded-full bg-zinc-800 overflow-hidden"
        >
          <div
            className="h-full w-full origin-left bg-sky-500 transition-transform duration-200 ease-linear motion-reduce:transition-none"
            style={{ transform: `scaleX(${progress.done_bytes / progress.total_bytes})` }}
          />
        </div>
      ) : null}
      <p className="mt-1.5 text-zinc-500">{copy.line2}</p>
    </div>
  );
};

/**
 * "Run Claude inside the VM" (design/ui.md): one switch under the VM card, the "Right now" chips
 * and, when something needs the person, one status line. VM mode only; nothing until loaded.
 */
export const ClaudeIsolationRow: React.FC<{ api: ComputerModeApi }> = ({ api }) => {
  const settings = api.state.settings;
  const vm = settings?.claudeVm ?? null;
  const claude = vm?.agents.claude;

  // The live region speaks at the start, the end and on a failure; never per progress tick.
  const prev = useRef<AgentVmState | undefined>(claude);
  const [said, setSaid] = useState('');
  useEffect(() => {
    const text = announcement(prev.current, claude);
    prev.current = claude;
    if (text) setSaid(text);
  }, [claude?.state, claude?.reason]); // eslint-disable-line react-hooks/exhaustive-deps

  const [checking, setChecking] = useState(false);
  if (!settings || !vm || settings.mode !== 'vm') return null;

  const pending = api.isoPending ?? null;
  const model = rowModel(vm, pending);
  const busy = pending !== null;
  const chips = chipsFor(vm);
  const describedBy = ['claude-iso-hint', model.status ? 'claude-iso-status' : null].filter(Boolean).join(' ');
  const toggle = () => {
    if (model.disabled || busy) return;
    api.setIsolateClaude?.(!vm.on);
  };
  const check = async () => {
    if (checking) return;
    setChecking(true);
    try {
      await api.refresh?.();
    } finally {
      setChecking(false);
    }
  };
  const status = model.status;

  return (
    <div data-claude-iso className="p-3 rounded-xl border border-zinc-800 bg-zinc-950/60">
      <div className="flex items-start gap-3">
        <div className="flex-1 min-w-0">
          <p id="claude-iso-label" className={`flex items-center gap-2 text-xs font-medium ${model.disabled ? 'text-zinc-500' : 'text-zinc-200'}`}>
            <Shield size={15} className={model.disabled ? 'text-zinc-600' : 'text-sky-400'} aria-hidden="true" />
            {SETTING_NAME}
          </p>
          <p id="claude-iso-hint" className="mt-1 pl-[23px] text-[11px] leading-[1.45] text-zinc-500">
            {model.hint}
          </p>
        </div>
        <button
          type="button"
          role="switch"
          data-switch="claude-isolation"
          aria-checked={model.checked}
          aria-labelledby="claude-iso-label"
          aria-describedby={describedBy}
          aria-disabled={model.disabled || undefined}
          aria-busy={busy || undefined}
          onClick={toggle}
          className={`group shrink-0 -m-3 p-3 rounded-full focus-visible:outline focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-sky-500 ${
            model.disabled ? 'cursor-not-allowed' : busy ? 'cursor-progress' : ''
          }`}
        >
          <span
            className={`relative block w-9 h-5 rounded-full transition-colors motion-reduce:transition-none ${
              model.disabled ? (model.checked ? 'bg-sky-600/35' : 'bg-zinc-800') : model.checked ? 'bg-sky-600' : 'bg-zinc-700'
            }`}
          >
            <span
              className={`absolute top-0.5 left-0.5 w-4 h-4 rounded-full transition-transform duration-150 motion-reduce:transition-none ${
                model.disabled ? 'bg-zinc-600' : 'bg-white'
              } ${model.checked ? 'translate-x-4' : ''}`}
            />
          </span>
          {busy && <span className="sr-only">(saving)</span>}
        </button>
      </div>

      <div className={`mt-2.5 pl-[23px] flex flex-wrap items-center gap-1.5 ${busy ? 'opacity-60' : ''}`}>
        <span className="basis-full sm:basis-auto text-[11px] text-zinc-500" aria-hidden="true">
          Right now
        </span>
        <ul aria-label="Where each AI runs right now" className="contents">
          {chips.map((c) => (
            <AgentChip key={c.engine} chip={c} />
          ))}
        </ul>
      </div>
      <span className="sr-only" aria-live="polite" data-claude-iso-live>
        {said}
      </span>

      {status?.kind === 'preparing' && (
        <div id="claude-iso-status" role="status">
          <PreparingBlock progress={status.progress} />
        </div>
      )}
      {status?.kind === 'quiet' && (
        <p id="claude-iso-status" data-claude-iso-status="quiet" className="mt-2.5 ml-[23px] text-[11px] text-zinc-500">
          {status.text}
        </p>
      )}
      {status?.kind === 'stopped' && (
        <div
          data-claude-iso-status="stopped"
          className="mt-2.5 ml-[23px] flex flex-col sm:flex-row sm:items-center gap-2 px-2.5 py-2 rounded-lg border border-amber-500/30 bg-amber-950/20"
        >
          <p id="claude-iso-status" className="flex items-start gap-1.5 flex-1 min-w-0 text-[11px] leading-[1.45] text-amber-200">
            <AlertTriangle size={13} className="mt-px shrink-0 text-amber-400" aria-hidden="true" />
            <span className="min-w-0 [overflow-wrap:anywhere]">{status.text}</span>
          </p>
          {status.action && (
            <button
              type="button"
              data-claude-iso-action={status.action}
              onClick={status.action === 'check' ? check : () => api.setIsolateClaude?.(true)}
              disabled={checking || busy}
              aria-busy={checking || busy || undefined}
              className="shrink-0 ml-[19px] sm:ml-0 inline-flex items-center justify-center gap-1.5 min-h-[44px] sm:min-h-[32px] px-3 rounded-lg text-[11px] font-medium border text-amber-100 bg-amber-500/15 border-amber-500/40 hover:bg-amber-500/25 disabled:opacity-70 focus-visible:outline focus-visible:outline-2 focus-visible:outline-amber-400"
            >
              {checking || (busy && status.action === 'retry') ? (
                <Loader2 size={12} className="animate-spin motion-reduce:animate-none" aria-hidden="true" />
              ) : status.action === 'check' ? (
                <RefreshCw size={12} aria-hidden="true" />
              ) : (
                <RotateCcw size={12} aria-hidden="true" />
              )}
              {status.action === 'check' ? 'Check again' : 'Try again'}
            </button>
          )}
        </div>
      )}
      {api.isoError && api.isoError !== model.hint && !(status?.kind === 'quiet' && status.text === api.isoError) && (
        <div className="mt-2.5 ml-[23px]">
          <ComputerModeError error={api.isoError} onDismiss={api.dismissIsoError} />
        </div>
      )}
    </div>
  );
};
