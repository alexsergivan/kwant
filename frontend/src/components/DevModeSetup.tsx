// First switch-on: which repo is this space about? (design §3). The Repository field checks the
// repo 400 ms after typing stops; Turn on Dev mode stays disabled until the check says Found.
// The design's fourth row, "Ready to ship", is out of 0.7 (plan D-cut), so three rows show.

import React, { useEffect, useMemo, useRef, useState } from 'react';
import { Eye, GitMerge, Loader2, Lock, ShieldAlert } from 'lucide-react';
import { CheckerUpdate, DevModeState, FieldState, createRepoChecker, deployLabel, devApi, fieldView, normaliseRepo, RepoCheck } from '../devMode';

export interface DevModeSetupProps {
  space: { id: string; name: string };
  initial?: DevModeState | null;
  onCancel: () => void;
  onDone: (s: DevModeState) => void;
  /** Opens Connections (Connect GitHub, Mark production keys). Hidden when not given. */
  onOpenConnections?: () => void;
  /** Test hooks. */
  check?: (repo: string) => Promise<RepoCheck>;
  save?: (body: { on: boolean; repo: string; branch: string | null }) => Promise<DevModeState>;
  productionCount?: number;
}

const ROWS = [
  { icon: GitMerge, title: 'What landed', line: 'Each merge to main becomes a short card in plain words. Checked every 5 minutes.' },
  { icon: Eye, title: 'Second pair of eyes', line: 'Each open PR is reviewed by a bot that didn’t write it, on a different engine.' },
  { icon: ShieldAlert, title: 'Production floor', line: 'Deploys, pushes to main and destructive commands always ask you, even with approvals off.' },
];

const chipTone: Record<string, string> = { ok: 'kw-chip-ok', warn: 'kw-chip-warn', bad: 'kw-chip-bad', info: 'kw-chip-info' };

export const DevModeSetup: React.FC<DevModeSetupProps> = ({ space, initial, onCancel, onDone, onOpenConnections, check, save, productionCount = 0 }) => {
  const [raw, setRaw] = useState(initial?.repo ?? '');
  const [field, setField] = useState<CheckerUpdate>({ state: initial?.repo ? 'checking' : 'empty', check: null, error: null });
  const [saving, setSaving] = useState(false);
  const [saveError, setSaveError] = useState<string | null>(null);
  const primary = useRef<HTMLButtonElement>(null);
  const dialog = useRef<HTMLDivElement>(null);

  const checker = useMemo(() => createRepoChecker(check ?? ((r) => devApi.check(space.id, r)), setField), [check, space.id]);
  useEffect(() => {
    if (initial?.repo) checker.input(initial.repo);
    return () => checker.cancel();
    // Only the pre-filled repo is checked on open; typing calls checker.input.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [checker]);

  // Focus starts on the primary button (design §3), even while it's disabled for the check: then the field.
  useEffect(() => {
    const b = primary.current;
    if (b && !b.disabled) b.focus();
    else dialog.current?.querySelector<HTMLInputElement>('input')?.focus();
  }, []);
  // The pre-filled repo's check comes back Found: focus moves to the primary button, unless the person typed.
  const typed = useRef(false);
  useEffect(() => {
    if (field.state === 'found' && !typed.current && dialog.current?.contains(document.activeElement)) primary.current?.focus();
  }, [field.state]);

  const repo = normaliseRepo(raw);
  const view = fieldView(field.state as FieldState, repo, field.error);
  const found = field.state === 'found' ? field.check : null;
  const deploy = deployLabel(found?.deploy_hint ?? null);
  const canTurnOn = view.canTurnOn && !saving;

  const turnOn = () => {
    if (!canTurnOn || !repo) return;
    setSaving(true);
    setSaveError(null);
    (save ?? ((b) => devApi.put(space.id, b)))({ on: true, repo: found?.repo ?? repo, branch: found?.branch ?? null }).then(onDone, (e: unknown) => {
      setSaving(false);
      setSaveError(e instanceof Error ? e.message : 'Dev mode couldn’t be turned on.');
    });
  };

  const onKey = (e: React.KeyboardEvent) => {
    if (e.key === 'Escape') {
      e.stopPropagation();
      onCancel();
      return;
    }
    if (e.key !== 'Tab' || !dialog.current) return;
    const items = Array.from(dialog.current.querySelectorAll<HTMLElement>('button:not([disabled]),input'));
    if (!items.length) return;
    const first = items[0];
    const last = items[items.length - 1];
    if (e.shiftKey && document.activeElement === first) {
      e.preventDefault();
      last.focus();
    } else if (!e.shiftKey && document.activeElement === last) {
      e.preventDefault();
      first.focus();
    }
  };

  const help = view.helpRepo && view.help ? view.help.split('{repo}') : null;

  return (
    <div className="fixed inset-0 z-50 flex items-end sm:items-center justify-center bg-black/70 backdrop-blur-[4px] p-0 sm:p-4" data-dev-setup onMouseDown={(e) => e.target === e.currentTarget && onCancel()}>
      <div
        ref={dialog}
        role="dialog"
        aria-modal="true"
        aria-labelledby="dv-setup-title"
        onKeyDown={onKey}
        className="kw-dv-sheet w-full sm:w-[540px] max-h-[92dvh] overflow-y-auto rounded-t-3xl sm:rounded-3xl border border-white/14 bg-surface-raised p-6 flex flex-col gap-[18px] text-left"
      >
        <header>
          <h2 id="dv-setup-title" className="text-lg font-semibold text-zinc-50">
            Which repo is this space about?
          </h2>
          <p className="mt-1 text-body text-zinc-400">
            Dev mode for <strong className="font-semibold text-zinc-100">{space.name}</strong> reads this repo to show what landed, review PRs and guard deploys.
          </p>
        </header>

        <div>
          <label htmlFor="dv-repo" className="text-meta font-medium text-zinc-300">
            Repository
          </label>
          <div className="mt-1.5 flex items-center gap-2 rounded-xl border border-hairline-strong bg-surface-main px-3 h-10 focus-within:border-sky-400/70">
            <span className="font-mono text-[13px] text-zinc-500 shrink-0" aria-hidden="true">
              github.com/
            </span>
            <input
              id="dv-repo"
              value={raw}
              onChange={(e) => {
                typed.current = true;
                setRaw(e.target.value);
                setSaveError(null);
                checker.input(e.target.value);
              }}
              placeholder="owner/repo"
              spellCheck={false}
              autoCapitalize="off"
              autoComplete="off"
              aria-describedby="dv-repo-help"
              aria-invalid={field.state === 'invalid' || field.state === 'not_found' || undefined}
              className="min-w-0 flex-1 bg-transparent font-mono text-[13px] text-zinc-100 placeholder:text-zinc-600 outline-none"
            />
            {view.chip && (
              <span className={`kw-chip ${chipTone[view.chip.tone]} shrink-0`} data-dv-chip={field.state}>
                {view.chip.spinner && <Loader2 size={11} className="animate-spin motion-reduce:animate-none" aria-hidden="true" />}
                {view.chip.text}
              </span>
            )}
          </div>
          <p id="dv-repo-help" className="mt-1.5 text-meta text-zinc-400 min-h-[18px]" aria-live="polite">
            {help ? (
              <>
                {help[0]}
                <strong className="font-semibold text-zinc-200">{view.helpRepo}</strong>
                {help[1]}
              </>
            ) : (
              view.help
            )}
            {view.connect && onOpenConnections && (
              <button type="button" onClick={onOpenConnections} className="ml-2 font-medium text-sky-300 hover:text-sky-200 underline-offset-2 hover:underline">
                Connect GitHub
              </button>
            )}
          </p>
        </div>

        {found && (
          <div className="grid grid-cols-1 sm:grid-cols-3 gap-2" data-dv-tiles>
            <Tile label="Branch to watch">
              <span className="font-mono text-sky-300">{found.branch ?? 'main'}</span>
            </Tile>
            <Tile label="Deploy target">
              {deploy ? (
                <>
                  {deploy.name} <span className="text-zinc-500">({deploy.detail})</span>
                </>
              ) : (
                <span className="text-zinc-500">None found</span>
              )}
            </Tile>
            <Tile label="Production keys">
              <span className={productionCount ? '' : 'text-zinc-500'}>{productionCount ? `${productionCount} marked` : 'None marked'}</span>
              {onOpenConnections && (
                <button type="button" onClick={onOpenConnections} className="ml-1.5 text-meta font-medium text-sky-300 hover:text-sky-200">
                  Mark
                </button>
              )}
            </Tile>
          </div>
        )}

        <section aria-labelledby="dv-turns-on">
          <h3 id="dv-turns-on" className="text-2xs font-semibold uppercase tracking-wider text-zinc-500">
            What turns on
          </h3>
          <ul className="mt-2 flex flex-col gap-3">
            {ROWS.map(({ icon: Icon, title, line }) => (
              <li key={title} className="flex items-start gap-3">
                <span className="w-7 h-7 shrink-0 rounded-lg bg-accent-soft text-sky-300 flex items-center justify-center" aria-hidden="true">
                  <Icon size={14} />
                </span>
                <span className="min-w-0">
                  <span className="block text-[13px] font-semibold text-zinc-100">{title}</span>
                  <span className="block text-meta text-zinc-400">{line}</span>
                </span>
              </li>
            ))}
          </ul>
        </section>

        {saveError && (
          <p role="alert" className="text-meta text-bad">
            {saveError}
          </p>
        )}

        <footer className="flex flex-col sm:flex-row sm:items-center gap-3 pt-1">
          <p className="flex-1 flex items-start gap-1.5 text-meta text-zinc-500">
            <Lock size={12} className="mt-0.5 shrink-0" aria-hidden="true" />
            Only you can turn this on or off. kwant never posts to GitHub without asking.
          </p>
          <div className="flex gap-2 justify-end">
            <button
              type="button"
              onClick={onCancel}
              className="rounded-xl border border-hairline-strong px-3.5 h-9 text-meta font-medium text-zinc-200 hover:bg-zinc-800 focus-visible:outline focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-sky-400"
            >
              Cancel
            </button>
            <button
              ref={primary}
              type="button"
              onClick={turnOn}
              disabled={!canTurnOn}
              data-dv-turn-on
              className="inline-flex items-center gap-1.5 rounded-xl bg-accent-gradient px-3.5 h-9 text-meta font-semibold text-white disabled:opacity-45 disabled:cursor-not-allowed focus-visible:outline focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-sky-400"
            >
              {saving && <Loader2 size={12} className="animate-spin motion-reduce:animate-none" aria-hidden="true" />}
              Turn on Dev mode
            </button>
          </div>
        </footer>
      </div>
    </div>
  );
};

const Tile: React.FC<{ label: string; children: React.ReactNode }> = ({ label, children }) => (
  <div className="rounded-xl border border-hairline bg-surface-main px-3 py-2">
    <p className="text-2xs font-semibold uppercase tracking-wider text-zinc-500">{label}</p>
    <p className="mt-0.5 text-meta text-zinc-200 truncate">{children}</p>
  </div>
);
