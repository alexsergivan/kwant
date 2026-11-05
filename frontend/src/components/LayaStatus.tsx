// Laya's status, shown in App settings (it used to be the header's ROUTER pill).
import React from 'react';
import { Cpu } from 'lucide-react';

export const isMac = typeof navigator !== 'undefined' && /Mac|iPhone|iPad/.test(navigator.platform || navigator.userAgent);

type LayaState = 'unknown' | 'external' | 'installing' | 'starting' | 'online' | 'offline' | 'failed';

interface VegaStatus {
  state: 'off' | 'waiting' | 'installing' | 'starting' | 'active' | 'fallback';
  detail: string;
  /** Size of the downloaded vega model, once it ran. */
  model_mb?: number | null;
  /** Where that model is cached. */
  model_cache?: string | null;
  crashes?: number;
}

interface LayaStatus {
  state: LayaState;
  detail: string;
  url: string;
  managed: boolean;
  /** The engine answering routing questions now (absent on older servers). */
  engine?: 'standard' | 'vega';
  vega?: VegaStatus;
}

export const useLayaStatus = (): LayaStatus | null => {
  const [status, setStatus] = React.useState<LayaStatus | null>(null);
  React.useEffect(() => {
    let cancelled = false;
    let timer: ReturnType<typeof setTimeout>;
    const poll = async () => {
      let next: LayaStatus | null = null;
      try {
        const res = await fetch('/api/laya/status');
        if (res.ok) next = await res.json();
      } catch {
        // server restarting; keep the last known status
      }
      if (cancelled) return;
      if (next) setStatus(next);
      const up = next?.state === 'online' || next?.state === 'external';
      timer = setTimeout(poll, up ? 20000 : 4000);
    };
    poll();
    return () => {
      cancelled = true;
      clearTimeout(timer);
    };
  }, []);
  return status;
};

export const layaPill = (status: LayaStatus | null) => {
  const ready = 'Laya is online · picks the best bot for each message';
  switch (status?.state) {
    case 'online':
    case 'external':
      return { dot: 'bg-emerald-400', label: ready };
    case 'installing':
      return { dot: 'bg-amber-400 animate-pulse', label: 'Laya is installing · picking by keywords meanwhile' };
    case 'starting':
      return { dot: 'bg-amber-400 animate-pulse', label: 'Laya is loading its model · picking by keywords meanwhile' };
    case 'failed':
    case 'offline':
      return { dot: 'bg-rose-500', label: 'Laya is offline · picking by keywords' };
    default:
      return { dot: 'bg-zinc-500 animate-pulse', label: 'Checking Laya…' };
  }
};


/** Laya's state for App settings. */
export const LayaStatusRow: React.FC = () => {
  const laya = useLayaStatus();
  const pill = layaPill(laya);
  return (
    <div className="flex items-start gap-3 bg-zinc-950/60 p-3 rounded-xl border border-zinc-800">
      <Cpu size={15} className="mt-0.5 text-sky-400 shrink-0" aria-hidden="true" />
      <div className="flex-1 min-w-0">
        <p className="text-xs font-medium text-zinc-200">Laya, the router</p>
        <p className="mt-0.5 flex items-center gap-1.5 text-[11px] text-zinc-400" role="status">
          <span className={`w-2 h-2 rounded-full shrink-0 ${pill.dot}`} aria-hidden="true" />
          <span>{pill.label}</span>
        </p>
        {laya && (
          <p className="mt-0.5 text-[11px] text-zinc-500 break-all">
            {laya.url}
            {laya.managed ? ' (started by the app)' : ''}
            {laya.detail ? ` · ${laya.detail}` : ''}
          </p>
        )}
        {laya?.vega?.model_mb != null && (
          <p className="mt-0.5 text-[11px] text-zinc-500 break-all">
            vega model: {(laya.vega.model_mb / 1024).toFixed(1)} GB
            {laya.vega.model_cache ? ` in ${laya.vega.model_cache}` : ''}
          </p>
        )}
      </div>
    </div>
  );
};
