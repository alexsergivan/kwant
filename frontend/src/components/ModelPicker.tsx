import React, { useId } from 'react';
import { AlertTriangle, ChevronRight, Loader2 } from 'lucide-react';
import { claudeNeedsSignIn } from '../claudeLogin';
import { Profile, readyCustomAccounts } from '../profiles';
import {
  CustomSource,
  EngineInfo,
  EngineModel,
  FAMILIES,
  Family,
  FamilyOption,
  encodePick,
  familyLabel,
  familyOptions,
  groupByTier,
  parsePick,
  pickHealth,
} from '../engines';

interface ModelPickerProps {
  /** A kwant's preferred_model: "auto", "claude", "claude/sonnet"… */
  value: string;
  onChange: (value: string) => void;
  /** From useEngines; null while loading, [] from a server without engine checks. */
  engines: EngineInfo[] | null;
  /** Opens Settings → Engines. */
  onOpenSettings?: () => void;
  /** Opens Settings at "Sign in with Claude". */
  onSignInClaude?: () => void;
  /** The space's accounts: Custom API models come from each account's own server. */
  profiles?: Profile[];
  /** The bot's pinned account, and how to change it, so a Custom API model picks its account too. */
  profileId?: string | null;
  onPickAccount?: (profileId: string | null) => void;
}

/** A ready Custom API account's models, with its own model first when its server doesn't list it. */
const customModels = (p: Profile): EngineModel[] => {
  const listed: EngineModel[] = (p.custom?.models ?? []).map((m) => ({ id: m.id, label: m.label, tier: m.tier, default: m.default }));
  const own = p.custom?.model;
  return own && !listed.some((m) => m.id === own) ? [{ id: own, label: own, default: true }, ...listed] : listed;
};

/** A Custom API option's value: the account and the model, so the same model on two servers stays apart. */
const SEP = '\n';

const SELECT =
  'w-full bg-zinc-950 border border-zinc-800 rounded-lg px-3 py-2 text-xs text-zinc-100 focus:border-sky-500 focus:outline-none transition-colors disabled:opacity-50';

/** One Engine → Model choice used wherever a kwant's model is set. Only engines that can run are offered. */
export const ModelPicker: React.FC<ModelPickerProps> = ({ value, onChange, engines, onOpenSettings, onSignInClaude, profiles, profileId, onPickAccount }) => {
  const id = useId();
  const pick = parsePick(value);
  const customAccounts = readyCustomAccounts(profiles);
  const custom: CustomSource[] = customAccounts.map((p) => ({ name: p.name, models: customModels(p) }));
  // A server without engine checks: offer every family, as before.
  const options: FamilyOption[] =
    engines && engines.length === 0
      ? // An older server has no Custom API.
        FAMILIES.filter((f) => f.id !== 'custom').map((f) => ({ id: f.id, label: f.label, via: [], models: [] }))
      : familyOptions(engines ?? [], custom);
  const health = engines && engines.length === 0 ? ({ kind: 'ok' } as const) : pickHealth(value, engines, custom);
  const family = pick.family;
  const current = family && family !== 'auto' ? options.find((o) => o.id === family) : undefined;
  const staleFamily = family && family !== 'auto' && !current ? family : null;
  const models = current?.models ?? [];
  const staleModel = pick.model && !models.some((m) => m.id === pick.model) ? pick.model : null;

  const pickFamily = (f: string) => onChange(f === 'auto' ? 'auto' : encodePick(f as Family, null));
  const pickModel = (m: string) => family && family !== 'auto' && onChange(encodePick(family, m || null));

  // Custom API: one group per account, and picking a model pins its account as well.
  const byAccount = family === 'custom' && customAccounts.length > 0;
  const owner = byAccount && pick.model
    ? customAccounts.find((p) => p.id === profileId && customModels(p).some((m) => m.id === pick.model)) ??
      customAccounts.find((p) => customModels(p).some((m) => m.id === pick.model))
    : undefined;
  const pickCustom = (v: string) => {
    if (!v) return onChange('custom');
    const [pid, ...rest] = v.split(SEP);
    onChange(encodePick('custom', rest.join(SEP)));
    onPickAccount?.(pid);
  };

  const hintId = `${id}-hint`;
  const describedBy = health.kind !== 'ok' ? hintId : undefined;

  return (
    <div className="space-y-1.5">
      <div className="grid grid-cols-1 sm:grid-cols-2 gap-2">
        <div>
          <label htmlFor={`${id}-engine`} className="block text-xs font-medium text-zinc-400 mb-1">
            Engine
          </label>
          <select
            id={`${id}-engine`}
            value={family === null ? value : family}
            onChange={(e) => pickFamily(e.target.value)}
            aria-describedby={describedBy}
            aria-invalid={health.kind === 'engine_unavailable' || undefined}
            className={`${SELECT} ${health.kind === 'engine_unavailable' ? 'border-amber-500/60' : ''}`}
          >
            <option value="auto">Auto (Laya picks)</option>
            {options.map((o) => (
              <option key={o.id} value={o.id}>
                {o.via.length ? `${o.label} · ${o.via.join(', ')}` : o.label}
              </option>
            ))}
            {staleFamily && <option value={staleFamily}>{`${familyLabel(staleFamily)} (can't run now)`}</option>}
            {family === null && <option value={value}>{value}</option>}
          </select>
        </div>

        <div>
          <label htmlFor={`${id}-model`} className="block text-xs font-medium text-zinc-400 mb-1">
            Model
          </label>
          {byAccount ? (
            <select
              id={`${id}-model`}
              value={owner && pick.model ? `${owner.id}${SEP}${pick.model}` : pick.model ?? ''}
              onChange={(e) => pickCustom(e.target.value)}
              aria-describedby={describedBy}
              className={`${SELECT} ${health.kind === 'model_missing' ? 'border-amber-500/60' : ''}`}
            >
              <option value="">Each account’s own model</option>
              {customAccounts.map((p) => (
                <optgroup key={p.id} label={p.name}>
                  {customModels(p).map((m) => (
                    <option key={m.id} value={`${p.id}${SEP}${m.id}`}>
                      {`${m.label} · ${p.name}`}
                    </option>
                  ))}
                </optgroup>
              ))}
              {pick.model && !owner && <option value={pick.model}>{`${pick.model} (not offered now)`}</option>}
            </select>
          ) : (
          <select
            id={`${id}-model`}
            value={pick.model ?? ''}
            onChange={(e) => pickModel(e.target.value)}
            disabled={family === 'auto' || family === null}
            aria-describedby={describedBy}
            className={`${SELECT} ${health.kind === 'model_missing' ? 'border-amber-500/60' : ''}`}
          >
            <option value="">{family === 'auto' ? 'Laya picks per task' : 'Auto (Laya picks)'}</option>
            {groupByTier(models).map((g) => (
              <optgroup key={g.label} label={g.label}>
                {g.models.map((m) => (
                  <option key={m.id} value={m.id}>
                    {m.default ? `${m.label} (default)` : m.label}
                  </option>
                ))}
              </optgroup>
            ))}
            {staleModel && <option value={staleModel}>{`${staleModel} (not offered now)`}</option>}
          </select>
          )}
        </div>
      </div>

      <PickHint id={hintId} health={health} onOpenSettings={onOpenSettings} onSignInClaude={claudeNeedsSignIn(engines) ? onSignInClaude : undefined} />
    </div>
  );
};

const SettingsLink: React.FC<{ onOpenSettings?: () => void }> = ({ onOpenSettings }) =>
  onOpenSettings ? (
    <button type="button" onClick={onOpenSettings} className="inline-flex items-center gap-0.5 font-medium text-sky-300 hover:text-sky-200 underline-offset-2 hover:underline">
      Open Settings <ChevronRight size={11} />
    </button>
  ) : (
    <span>Check Settings → Engines.</span>
  );

/** "Sign in with Claude", when that's what fixes it. */
const SignInClaudeLink: React.FC<{ onSignInClaude: () => void }> = ({ onSignInClaude }) => (
  <button type="button" onClick={onSignInClaude} className="inline-flex items-center gap-0.5 font-medium text-sky-300 hover:text-sky-200 underline-offset-2 hover:underline">
    Sign in with Claude <ChevronRight size={11} />
  </button>
);

const PickHint: React.FC<{ id: string; health: ReturnType<typeof pickHealth>; onOpenSettings?: () => void; onSignInClaude?: () => void }> = ({
  id,
  health,
  onOpenSettings,
  onSignInClaude,
}) => {
  switch (health.kind) {
    case 'ok':
      return null;
    case 'loading':
      return (
        <p id={id} className="flex items-center gap-1.5 text-[11px] text-zinc-500" aria-live="polite">
          <Loader2 size={11} className="animate-spin motion-reduce:animate-none" /> Checking which engines can run…
        </p>
      );
    case 'none_ready':
      return (
        <div id={id} role="status" className="rounded-lg border border-amber-500/30 bg-amber-500/5 px-3 py-2 text-[11px] text-amber-200 space-y-1">
          <p className="flex items-center gap-1.5 font-medium">
            <AlertTriangle size={12} /> No engine can run right now
          </p>
          <p className="text-amber-200/80">
            {onSignInClaude ? (
              <>
                Claude Code is installed but signed out. <SignInClaudeLink onSignInClaude={onSignInClaude} />
              </>
            ) : (
              <>
                Sign in to a CLI like Claude Code, or add an API key. <SettingsLink onOpenSettings={onOpenSettings} />
              </>
            )}
          </p>
        </div>
      );
    case 'engine_unavailable':
      return (
        <div id={id} role="status" className="rounded-lg border border-amber-500/30 bg-amber-500/5 px-3 py-2 text-[11px] text-amber-200 space-y-1">
          <p className="flex items-center gap-1.5 font-medium">
            <AlertTriangle size={12} /> {familyLabel(health.family)} can't run right now
          </p>
          <p className="text-amber-200/80">
            {health.reason ? `${health.reason.replace(/`/g, '').replace(/\.$/, '')}. ` : ''}
            This bot won't switch engine by itself: fix it, or pick another engine.{' '}
            {health.family === 'claude' && onSignInClaude ? <SignInClaudeLink onSignInClaude={onSignInClaude} /> : <SettingsLink onOpenSettings={onOpenSettings} />}
          </p>
        </div>
      );
    case 'model_missing':
      return (
        <p id={id} role="status" className="flex items-start gap-1.5 text-[11px] text-amber-300">
          <AlertTriangle size={12} className="mt-px shrink-0" />
          {familyLabel(health.family)} no longer offers “{health.model}”. Pick another model, or Auto.
        </p>
      );
    case 'unknown':
      return (
        <p id={id} role="status" className="flex items-start gap-1.5 text-[11px] text-amber-300">
          <AlertTriangle size={12} className="mt-px shrink-0" />
          “{health.value}” isn't a model the app knows. Pick an engine above.
        </p>
      );
  }
};
