import React, { useEffect, useState } from 'react';
import { Plug, Terminal, Globe, Info, Loader2 } from 'lucide-react';
import { McpServer, EngineKeys, mcpSupportFor } from '../mcp';

interface KwantToolsPickerProps {
  /** Engine the kwant runs on (preferred_model), to say honestly when tools won't work. */
  model: string;
  selected: string[];
  onChange: (ids: string[]) => void;
  onManageServers?: () => void;
}

/**
 * "Tools this kwant can use": one switch per registered MCP server. Every server is on for every
 * kwant unless switched off here. A server paused in Tools shows greyed and keeps this kwant's
 * choice, so turning it back on there restores it.
 */
export const KwantToolsPicker: React.FC<KwantToolsPickerProps> = ({ model, selected, onChange, onManageServers }) => {
  const [servers, setServers] = useState<McpServer[] | null>(null);
  const [keys, setKeys] = useState<EngineKeys>({ anthropic: false, openai: false, xai: false });

  useEffect(() => {
    let cancelled = false;
    fetch('/api/mcp/servers')
      .then((r) => (r.ok ? r.json() : []))
      .then((list: McpServer[]) => !cancelled && setServers(list))
      .catch(() => !cancelled && setServers([]));
    fetch('/api/settings')
      .then((r) => r.json())
      .then(
        (s) =>
          !cancelled &&
          setKeys({ anthropic: !!s.anthropic_api_key, openai: !!s.openai_api_key, xai: !!s.xai_api_key }),
      )
      .catch(() => undefined);
    return () => {
      cancelled = true;
    };
  }, []);

  const support = mcpSupportFor(model, keys);
  const toggle = (id: string) =>
    onChange(selected.includes(id) ? selected.filter((s) => s !== id) : [...selected, id]);
  const offCount = servers ? servers.filter((s) => s.enabled && !selected.includes(s.id)).length : 0;

  return (
    <div>
      <div className="flex items-center justify-between mb-1">
        <label className="text-xs font-medium text-zinc-400">
          Tools this kwant can use <span className="text-zinc-600 font-normal">· asks you before each use</span>
        </label>
        {onManageServers && servers && servers.length > 0 && (
          <button type="button" onClick={onManageServers} className="text-[11px] text-sky-300 hover:text-sky-200">
            Manage servers
          </button>
        )}
      </div>

      {servers && servers.length > 0 && (
        <p className="mb-1.5 text-[11px] text-zinc-500 leading-snug">
          Every tool server is on by default, including ones you add later. Switch off any this kwant shouldn’t have
          {offCount > 0 && <span className="text-zinc-400"> · {offCount} off</span>}.
        </p>
      )}

      {support.note && (
        <p
          className={`mb-1.5 flex items-start gap-1.5 px-2.5 py-2 rounded-lg border text-[11px] leading-snug ${
            support.supported
              ? 'bg-zinc-950/60 border-zinc-800 text-zinc-400'
              : 'bg-amber-500/5 border-amber-500/20 text-amber-200/90'
          }`}
        >
          <Info size={13} className={`flex-shrink-0 mt-px ${support.supported ? 'text-zinc-500' : 'text-amber-400'}`} />
          <span>{support.note}</span>
        </p>
      )}

      {servers === null ? (
        <div className="py-3 flex justify-center text-zinc-500">
          <Loader2 size={16} className="animate-spin" aria-label="Loading tool servers" />
        </div>
      ) : servers.length === 0 ? (
        <div className="flex items-center justify-between gap-2 px-3 py-2.5 rounded-lg border border-dashed border-zinc-800 text-[11px] text-zinc-500">
          <span className="flex items-center gap-1.5">
            <Plug size={13} />
            No tool servers yet.
          </span>
          {onManageServers && (
            <button type="button" onClick={onManageServers} className="font-medium text-sky-300 hover:text-sky-200">
              Add one
            </button>
          )}
        </div>
      ) : (
        <ul className={`rounded-lg border border-zinc-800 divide-y divide-zinc-800/80 bg-zinc-950/60 ${support.supported ? '' : 'opacity-60'}`}>
          {servers.map((s) => {
            const paused = !s.enabled;
            const on = !paused && selected.includes(s.id);
            return (
              <li key={s.id}>
                <button
                  type="button"
                  role="switch"
                  aria-checked={on}
                  aria-disabled={paused || undefined}
                  disabled={paused}
                  aria-label={paused ? `${s.name}, paused in Tools` : undefined}
                  onClick={() => toggle(s.id)}
                  className={`w-full flex items-center gap-2.5 px-3 py-2 text-left transition-colors ${
                    paused ? 'cursor-not-allowed opacity-60' : 'hover:bg-zinc-900/60'
                  }`}
                >
                  <span className="text-zinc-500">{s.transport === 'stdio' ? <Terminal size={13} /> : <Globe size={13} />}</span>
                  <span className="flex-1 min-w-0">
                    <span className="block text-xs text-zinc-200 truncate">{s.name}</span>
                    {paused && (
                      <span className="block text-[10px] text-zinc-500">
                        Paused in Tools{selected.includes(s.id) ? '. This kwant gets it once it’s on.' : ''}
                      </span>
                    )}
                  </span>
                  <span className={`relative flex-shrink-0 w-8 h-[18px] rounded-full transition-colors ${on ? 'bg-sky-600' : 'bg-zinc-700'}`}>
                    <span className={`absolute top-0.5 w-3.5 h-3.5 rounded-full bg-white shadow transition-all ${on ? 'left-[16px]' : 'left-0.5'}`} />
                  </span>
                </button>
              </li>
            );
          })}
        </ul>
      )}
    </div>
  );
};
