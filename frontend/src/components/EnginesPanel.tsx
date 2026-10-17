import React, { useState } from 'react';
import { AlertTriangle, ChevronDown, ChevronRight, Cpu, Key, Loader2, RefreshCw, Server } from 'lucide-react';
import { EngineInfo, Tone, checkedAgo, isAbsent, onlyCliReady, sortEngines, statusView } from '../engines';
import { playSfx } from '../audio/chiptune';
import type { EnginesApi } from '../useEngines';
import { claudeNeedsSignIn } from '../claudeLogin';
import { ClaudeSignIn, ClaudeSignOut } from './ClaudeSignIn';

const DOT: Record<Tone, string> = {
  ok: 'bg-emerald-400 shadow-sm shadow-emerald-500/50',
  warn: 'bg-amber-400',
  error: 'bg-rose-400',
  muted: 'bg-zinc-600',
  busy: 'bg-sky-400 animate-pulse motion-reduce:animate-none',
};
const TEXT: Record<Tone, string> = {
  ok: 'text-emerald-300',
  warn: 'text-amber-300',
  error: 'text-rose-300',
  muted: 'text-zinc-500',
  busy: 'text-sky-300',
};
const KIND_ICON = { cli: Cpu, api: Key, local: Server } as const;

/** Inline `code` in a reason like "Not signed in: run `claude login`". */
const withCode = (text: string) =>
  text.split(/(`[^`]+`)/).map((part, i) =>
    part.startsWith('`') && part.endsWith('`') ? (
      <code key={i} className="font-mono text-[10.5px] px-1 py-px rounded bg-zinc-800 text-zinc-200 select-text whitespace-nowrap">
        {part.slice(1, -1)}
      </code>
    ) : (
      <React.Fragment key={i}>{part}</React.Fragment>
    ),
  );

export const EngineRow: React.FC<{ engine: EngineInfo; onToggle?: (on: boolean) => void; children?: React.ReactNode }> = ({ engine, onToggle, children }) => {
  const view = statusView(engine);
  const Icon = KIND_ICON[engine.kind] ?? Cpu;
  const off = engine.enabled === false;
  const labelId = `engine-${engine.id}-name`;
  // A sign-in button replaces "run `claude auth login`".
  const showDetail = !off && engine.detail && engine.status !== 'ready' && !(children && engine.status === 'signed_out');
  return (
    <li
      data-engine={engine.id}
      data-status={off ? 'off' : engine.status}
      className="bg-zinc-900/90 px-3 py-2 rounded-lg border border-zinc-800 text-xs"
    >
      <div className="flex items-start gap-2.5">
        <span className={`mt-1 w-2 h-2 shrink-0 rounded-full ${DOT[view.tone]}`} aria-hidden="true" />
        <div className="flex-1 min-w-0">
          <div className="flex items-center gap-1.5 flex-wrap">
            <Icon size={12} className="text-zinc-500 shrink-0" aria-hidden="true" />
            <span id={labelId} className={`font-medium ${off ? 'text-zinc-500' : 'text-zinc-200'}`}>
              {engine.name}
            </span>
            {engine.version && <span className="text-[10px] font-mono text-zinc-500">v{engine.version}</span>}
          </div>
          {showDetail && <p className="mt-0.5 text-[11px] text-zinc-400 leading-snug break-words">{withCode(engine.detail!)}</p>}
          {!off && engine.status === 'ready' && engine.models.length > 0 && (
            <p className="mt-0.5 text-[11px] text-zinc-500 truncate" title={engine.models.map((m) => m.label).join(', ')}>
              {engine.models.length === 1 ? '1 model' : `${engine.models.length} models`}
              {' · '}
              {engine.models.slice(0, 3).map((m) => m.label).join(', ')}
              {engine.models.length > 3 ? '…' : ''}
            </p>
          )}
        </div>
        <span className={`shrink-0 text-[11px] font-mono ${TEXT[view.tone]}`}>{view.label}</span>
        {onToggle && typeof engine.enabled === 'boolean' && (
          <button
            type="button"
            role="switch"
            aria-checked={!off}
            aria-labelledby={labelId}
            title={off ? 'Off: kwnt never uses it' : 'On: kwnt may use it'}
            onClick={() => {
              playSfx('click');
              onToggle(off);
            }}
            className={`relative shrink-0 mt-px w-8 h-[18px] rounded-full transition-colors focus-visible:outline focus-visible:outline-2 focus-visible:outline-sky-500 ${
              off ? 'bg-zinc-700' : 'bg-sky-600'
            }`}
          >
            <span
              className={`absolute top-[2px] left-[2px] w-[14px] h-[14px] rounded-full bg-white transition-transform motion-reduce:transition-none ${off ? '' : 'translate-x-[14px]'}`}
            />
          </button>
        )}
      </div>
      {/* Full width under the name, lined up with it */}
      {!off && children && <div className="pl-[18px]">{children}</div>}
    </li>
  );
};

/** Sign in or out of Claude Code from its row; nothing for other engines. */
const accountAction = (e: EngineInfo, api: EnginesApi, focusClaudeSignIn?: number) => {
  if (e.id !== 'claude' || e.enabled === false) return null;
  if (e.status === 'signed_out') return <ClaudeSignIn onSignedIn={api.reload} focusRequest={focusClaudeSignIn} />;
  if (e.status === 'ready') return <ClaudeSignOut onSignedOut={api.apply} />;
  return null;
};

/** The Settings "Engines" list: honest status per engine, ready ones first, with Re-check.
 *  `focusClaudeSignIn` changes to bring "Sign in with Claude" into view and focus it. */
export const EnginesPanel: React.FC<{ api: EnginesApi; now?: number; focusClaudeSignIn?: number }> = ({ api, now = Date.now(), focusClaudeSignIn }) => {
  const { engines, error, rechecking, recheck, setEnabled } = api;
  const [showAbsent, setShowAbsent] = useState(false);
  const sorted = engines ? sortEngines(engines) : [];
  const present = sorted.filter((e) => !isAbsent(e));
  const absent = sorted.filter(isAbsent);
  const readyCount = sorted.filter((e) => e.status === 'ready' && e.enabled !== false).length;
  const lastChecked = sorted.map((e) => e.checked_at).filter(Boolean).sort().pop() ?? null;
  const ago = checkedAgo(lastChecked, now);

  return (
    <section aria-labelledby="engines-title" className="bg-zinc-950/60 p-4 rounded-xl border border-zinc-800 space-y-3">
      <div className="flex items-center gap-2">
        <Cpu size={15} className="text-sky-400" aria-hidden="true" />
        <h3 id="engines-title" className="text-xs font-medium text-zinc-300">Engines</h3>
        <button
          type="button"
          onClick={() => {
            playSfx('click');
            recheck();
          }}
          disabled={rechecking || engines === null}
          className="ml-auto flex items-center gap-1.5 px-2 py-1 rounded-md text-[11px] font-medium text-zinc-300 border border-zinc-800 hover:text-sky-200 hover:border-sky-500/40 disabled:opacity-50 transition-colors"
        >
          {rechecking ? <Loader2 size={12} className="animate-spin motion-reduce:animate-none" /> : <RefreshCw size={12} />}
          {rechecking ? 'Checking…' : 'Re-check'}
        </button>
      </div>

      <p className="text-[11px] text-zinc-400 leading-relaxed" aria-live="polite">
        {engines === null
          ? 'Checking which engines can run…'
          : engines.length === 0
            ? 'This server doesn’t report engine status yet.'
            : readyCount === 0
              ? claudeNeedsSignIn(engines)
                ? 'No engine can run right now. Sign in with Claude below, or add an API key.'
                : 'No engine can run right now. Sign in to a CLI below or add an API key.'
              : `${readyCount} ready${onlyCliReady(engines) ? ', all through your signed-in CLIs, so no API keys are needed' : ''}. Kwants only use engines that are ready.`}
        {ago && <span className="text-zinc-500"> Checked {ago}.</span>}
      </p>

      {error && (
        <p role="alert" className="flex items-center gap-1.5 text-[11px] text-rose-300">
          <AlertTriangle size={12} /> {error}
        </p>
      )}

      {present.length > 0 && (
        <ul className="grid grid-cols-1 gap-2" aria-label="Engines on this computer">
          {present.map((e) => (
            <EngineRow key={e.id} engine={e} onToggle={(on) => setEnabled(e.id, on)}>
              {accountAction(e, api, focusClaudeSignIn)}
            </EngineRow>
          ))}
        </ul>
      )}

      {absent.length > 0 && (
        <div>
          <button
            type="button"
            aria-expanded={showAbsent}
            aria-controls="engines-absent"
            onClick={() => setShowAbsent((v) => !v)}
            className="flex items-center gap-1 text-[11px] text-zinc-500 hover:text-zinc-300 transition-colors"
          >
            {showAbsent ? <ChevronDown size={12} /> : <ChevronRight size={12} />}
            Not available on this computer ({absent.length})
          </button>
          {showAbsent && (
            <ul id="engines-absent" className="mt-2 grid grid-cols-1 gap-2">
              {absent.map((e) => (
                <EngineRow key={e.id} engine={e} onToggle={(on) => setEnabled(e.id, on)} />
              ))}
            </ul>
          )}
        </div>
      )}
    </section>
  );
};
