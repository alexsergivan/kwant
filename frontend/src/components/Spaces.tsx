import React, { useEffect, useRef, useState } from 'react';
import { Archive, Check, Loader2, Plus, RotateCcw, Settings, X, AlertTriangle, KeyRound } from 'lucide-react';
import { playSfx } from '../audio/chiptune';
import {
  SPACE_COLORS,
  SPACE_ICONS,
  Space,
  firstUnusedColor,
  spaceColor,
  spaceIcon,
  spaceNameError,
  spaceSummary,
  spaceSummaryTitle,
  spacesApi,
} from '../spaces';
import { useEscapeLayer } from '../escape';

const badge = (n: number) => (n > 99 ? '99+' : String(n));

/** A space's emoji on its colour. */
export const SpaceTile: React.FC<{ space: Pick<Space, 'id' | 'icon' | 'color'>; size?: number; className?: string }> = ({
  space,
  size = 40,
  className = '',
}) => (
  <span
    aria-hidden="true"
    className={`inline-flex items-center justify-center shrink-0 rounded-xl ${className}`}
    style={{ width: size, height: size, background: `${spaceColor(space)}33`, boxShadow: `inset 0 0 0 1px ${spaceColor(space)}66`, fontSize: size * 0.5 }}
  >
    {spaceIcon(space)}
  </span>
);

// ---------------------------------------------------------------------------
// Desktop rail

interface RailProps {
  spaces: Space[];
  currentId: string | null;
  onSwitch: (id: string) => void;
  onCreate: () => void;
  onOpenAppSettings: () => void;
}

export const SpaceRail: React.FC<RailProps> = ({ spaces, currentId, onSwitch, onCreate, onOpenAppSettings }) => (
  <nav aria-label="Spaces" className="hidden md:flex w-[68px] shrink-0 flex-col items-center gap-2 py-3 border-r border-zinc-800/80 bg-zinc-950 select-none">
    <ul className="flex flex-col items-center gap-2 flex-1 overflow-y-auto overflow-x-hidden w-full">
      {spaces.map((s, i) => {
        const active = s.id === currentId;
        const unread = active ? 0 : s.counts?.unread ?? 0;
        return (
          <li key={s.id} className="relative w-full flex justify-center">
            {active && <span className="absolute left-0 top-1.5 bottom-1.5 w-1 rounded-r bg-zinc-100" aria-hidden="true" />}
            <button
              type="button"
              onClick={() => onSwitch(s.id)}
              data-rail-space={s.id}
              aria-current={active ? 'true' : undefined}
              aria-label={`${s.name}${unread ? `, ${unread} unread` : ''}`}
              title={`${s.name}${i < 9 ? ` (Ctrl/⌘ ${i + 1})` : ''}`}
              className={`relative rounded-xl transition-all focus-visible:outline focus-visible:outline-2 focus-visible:outline-sky-400 ${
                active ? 'ring-2 ring-offset-2 ring-offset-zinc-950' : 'opacity-75 hover:opacity-100'
              }`}
              style={active ? ({ '--tw-ring-color': spaceColor(s) } as React.CSSProperties) : undefined}
            >
              <SpaceTile space={s} size={40} />
              {unread > 0 && (
                <span className="absolute -top-1.5 -right-1.5 min-w-[18px] px-1 py-px rounded-full bg-rose-500 text-white text-[10px] font-semibold text-center border-2 border-zinc-950">
                  {badge(unread)}
                </span>
              )}
            </button>
          </li>
        );
      })}
      <li>
        <button
          type="button"
          onClick={onCreate}
          aria-label="New space"
          title="New space"
          className="w-10 h-10 rounded-xl border border-dashed border-zinc-700 text-zinc-400 hover:text-zinc-100 hover:border-zinc-500 flex items-center justify-center transition-colors focus-visible:outline focus-visible:outline-2 focus-visible:outline-sky-400"
        >
          <Plus size={18} />
        </button>
      </li>
    </ul>
    <button
      type="button"
      onClick={onOpenAppSettings}
      data-settings-opener
      aria-label="Settings (all spaces)"
      title="Settings (all spaces)"
      className="w-10 h-10 rounded-xl text-zinc-400 hover:text-zinc-100 hover:bg-zinc-800 flex items-center justify-center transition-colors focus-visible:outline focus-visible:outline-2 focus-visible:outline-sky-400"
    >
      <Settings size={18} />
    </button>
  </nav>
);

// ---------------------------------------------------------------------------
// Switcher: a popover under the space header (desktop) or a bottom sheet (phone)

interface SwitcherProps {
  spaces: Space[];
  currentId: string | null;
  onSwitch: (id: string) => void;
  onCreate: () => void;
  onOpenSpaceSettings: () => void;
  onOpenArchived: () => void;
  onClose: () => void;
  /** Bottom sheet on phones. */
  sheet?: boolean;
}

export const SpaceSwitcher: React.FC<SwitcherProps> = ({ spaces, currentId, onSwitch, onCreate, onOpenSpaceSettings, onOpenArchived, onClose, sheet }) => {
  const ref = useRef<HTMLDivElement>(null);
  const current = spaces.find((s) => s.id === currentId);
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => e.key === 'Escape' && onClose();
    const onDown = (e: MouseEvent) =>
      !sheet &&
      ref.current &&
      !ref.current.contains(e.target as Node) &&
      !(e.target as Element).closest?.('[data-space-toggle]') &&
      onClose();
    window.addEventListener('keydown', onKey);
    window.addEventListener('mousedown', onDown);
    ref.current?.querySelector<HTMLElement>('[aria-current="true"]')?.focus();
    return () => {
      window.removeEventListener('keydown', onKey);
      window.removeEventListener('mousedown', onDown);
    };
  }, [onClose, sheet]);

  const item = 'w-full flex items-center gap-2.5 px-2.5 py-2 rounded-lg text-left text-xs transition-colors focus-visible:outline focus-visible:outline-2 focus-visible:outline-sky-400';
  const list = (
    <div ref={ref} role="menu" aria-label="Switch space" className="space-y-0.5">
      {spaces.map((s) => {
        const active = s.id === currentId;
        return (
          <button
            key={s.id}
            type="button"
            role="menuitem"
            aria-current={active ? 'true' : undefined}
            onClick={() => {
              onClose();
              if (!active) onSwitch(s.id);
            }}
            className={`${item} ${active ? 'bg-zinc-800 text-zinc-100' : 'text-zinc-300 hover:bg-zinc-800/60'}`}
          >
            <SpaceTile space={s} size={28} />
            <span className="flex-1 min-w-0">
              <span className="block font-medium truncate">{s.name}</span>
              <span className="block text-[11px] text-zinc-500 truncate" title={spaceSummaryTitle(s.counts)}>{spaceSummary(s.counts)}</span>
            </span>
            {!active && (s.counts?.unread ?? 0) > 0 && (
              <span className="min-w-[18px] px-1.5 py-px rounded-full bg-rose-500 text-white text-[10px] font-semibold text-center">{badge(s.counts.unread)}</span>
            )}
            {active && <Check size={14} className="text-sky-400" aria-hidden="true" />}
          </button>
        );
      })}
      <div className="my-1 border-t border-zinc-800" />
      <button type="button" role="menuitem" onClick={() => { onClose(); onCreate(); }} className={`${item} text-sky-300 hover:bg-sky-500/10`}>
        <Plus size={15} aria-hidden="true" /> New space
      </button>
      {current && (
        <button type="button" role="menuitem" onClick={() => { onClose(); onOpenSpaceSettings(); }} className={`${item} text-zinc-300 hover:bg-zinc-800/60`}>
          <Settings size={15} aria-hidden="true" /> {current.name} settings
        </button>
      )}
      <button type="button" role="menuitem" onClick={() => { onClose(); onOpenArchived(); }} className={`${item} text-zinc-300 hover:bg-zinc-800/60`}>
        <Archive size={15} aria-hidden="true" /> Archived spaces
      </button>
    </div>
  );

  if (sheet) {
    return (
      <div className="fixed inset-0 z-50 bg-black/60 flex items-end" onMouseDown={(e) => e.target === e.currentTarget && onClose()}>
        <div className="kw-sheet w-full max-h-[80vh] overflow-y-auto rounded-t-2xl bg-zinc-900 border-t border-zinc-800 p-3 pb-6 shadow-2xl">
          <div className="mx-auto mb-2 h-1 w-10 rounded-full bg-zinc-700" aria-hidden="true" />
          <p className="px-2.5 pb-2 text-[11px] font-semibold uppercase tracking-wider text-zinc-500">Spaces</p>
          {list}
        </div>
      </div>
    );
  }
  return <div className="absolute left-2 right-2 top-full mt-1 z-40 rounded-xl border border-zinc-700/70 bg-zinc-900 p-1.5 shadow-2xl">{list}</div>;
};

// ---------------------------------------------------------------------------
// Dialog frame shared by the space dialogs

const Dialog: React.FC<{
  title: string;
  subtitle?: string;
  labelId: string;
  onClose: () => void;
  children: React.ReactNode;
  wide?: boolean;
  /** Changes Escape would lose: it does nothing while there are some. */
  unsaved?: boolean;
}> = ({ title, subtitle, labelId, onClose, children, wide, unsaved }) => {
  const rootRef = useRef<HTMLDivElement>(null);
  useEscapeLayer(rootRef, true, onClose, { modal: true, unsaved });
  return (
    <div className="kw-dialog kw-dialog-bottom fixed inset-0 z-50 bg-black/70 backdrop-blur-sm flex items-end sm:items-center justify-center sm:p-4" onMouseDown={(e) => e.target === e.currentTarget && onClose()}>
      <div
        ref={rootRef}
        tabIndex={-1}
        role="dialog"
        aria-modal="true"
        aria-labelledby={labelId}
        className={`kw-dialog-panel outline-none bg-zinc-900 border border-zinc-800 rounded-t-2xl sm:rounded-2xl w-full ${wide ? 'sm:max-w-xl' : 'sm:max-w-md'} max-h-[92vh] overflow-y-auto p-4 sm:p-6 text-zinc-100 shadow-2xl space-y-5`}
      >
        <div className="flex items-start justify-between gap-3">
          <div>
            <h2 id={labelId} className="text-sm font-semibold text-zinc-100">{title}</h2>
            {subtitle && <p className="mt-0.5 text-xs text-zinc-400">{subtitle}</p>}
          </div>
          <button type="button" onClick={onClose} aria-label="Close" className="p-1.5 rounded-lg text-zinc-400 hover:text-zinc-100 hover:bg-zinc-800 transition-colors">
            <X size={18} />
          </button>
        </div>
        {children}
      </div>
    </div>
  );
};

// ---------------------------------------------------------------------------
// New space

interface NewSpaceProps {
  /** Every space, archived ones too, for taken names and colours. */
  spaces: Space[];
  onClose: () => void;
  /** `addAccounts`: open the new space's Accounts next. */
  onCreated: (space: Space, addAccounts: boolean) => void;
}

export const NewSpaceModal: React.FC<NewSpaceProps> = ({ spaces, onClose, onCreated }) => {
  const [name, setName] = useState('');
  // Like the colour, the first icon no other space uses, so two spaces don't look alike on the rail.
  const [icon, setIcon] = useState(() => SPACE_ICONS.find((i) => !spaces.some((s) => s.icon === i)) ?? SPACE_ICONS[0]);
  const [color, setColor] = useState(() => firstUnusedColor(spaces));
  const [thisMachine, setThisMachine] = useState(true);
  const [addAccounts, setAddAccounts] = useState(true);
  const [touched, setTouched] = useState(false);
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const nameErr = spaceNameError(name, spaces.map((s) => s.name));
  const nameRef = useRef<HTMLInputElement>(null);
  useEffect(() => nameRef.current?.focus(), []);

  const submit = async (e: React.FormEvent) => {
    e.preventDefault();
    setTouched(true);
    if (nameErr || saving) return;
    setSaving(true);
    setError(null);
    try {
      const space = await spacesApi.create({ name: name.trim(), icon, color, this_machine_enabled: thisMachine });
      playSfx('coin');
      onCreated(space, addAccounts);
    } catch (err) {
      setError(err instanceof Error ? err.message : 'Couldn’t create the space.');
      setSaving(false);
    }
  };

  const preview = { id: 'preview', icon, color };
  return (
    <Dialog title="New space" subtitle="Its own bots, teams, accounts and chats. Nothing is shared with your other spaces." labelId="new-space-title" onClose={onClose} unsaved={!!name.trim()}>
      <form onSubmit={submit} className="space-y-4" noValidate>
        <div className="flex items-center gap-3">
          <SpaceTile space={preview} size={48} />
          <div className="flex-1 min-w-0">
            <label htmlFor="space-name" className="block text-xs font-medium text-zinc-300 mb-1">Name</label>
            <input
              id="space-name"
              ref={nameRef}
              value={name}
              maxLength={60}
              onChange={(e) => setName(e.target.value)}
              onBlur={() => setTouched(true)}
              placeholder="Personal"
              aria-invalid={touched && !!nameErr}
              aria-describedby="space-name-error"
              className={`w-full bg-zinc-950 border rounded-lg px-3 py-2 text-sm text-zinc-100 placeholder-zinc-600 focus:outline-none ${
                touched && nameErr ? 'border-rose-500/70' : 'border-zinc-800 focus:border-sky-500'
              }`}
            />
          </div>
        </div>
        <p id="space-name-error" className="-mt-2 text-[11px] text-rose-300 min-h-[1em]">{touched ? nameErr : ''}</p>

        <fieldset>
          <legend className="text-xs font-medium text-zinc-300 mb-1.5">Icon</legend>
          <div className="flex flex-wrap gap-1.5">
            {SPACE_ICONS.map((i) => (
              <button
                key={i}
                type="button"
                aria-pressed={icon === i}
                aria-label={`Icon ${i}`}
                onClick={() => setIcon(i)}
                className={`w-11 h-11 sm:w-9 sm:h-9 rounded-lg text-lg flex items-center justify-center border transition-colors ${
                  icon === i ? 'border-sky-400 bg-sky-500/10' : 'border-zinc-800 hover:border-zinc-600'
                }`}
              >
                {i}
              </button>
            ))}
          </div>
        </fieldset>

        <fieldset>
          <legend className="text-xs font-medium text-zinc-300 mb-1.5">Colour</legend>
          <div className="flex flex-wrap gap-2">
            {SPACE_COLORS.map((c) => (
              <button
                key={c}
                type="button"
                aria-pressed={color === c}
                aria-label={`Colour ${c}`}
                onClick={() => setColor(c)}
                className={`w-7 h-7 rounded-full transition-transform ${color === c ? 'ring-2 ring-white ring-offset-2 ring-offset-zinc-900 scale-110' : 'hover:scale-110'}`}
                style={{ background: c }}
              />
            ))}
          </div>
        </fieldset>

        <div className="flex items-start gap-3 bg-zinc-950/60 p-3 rounded-xl border border-zinc-800">
          <div className="flex-1">
            <p id="space-this-machine" className="text-xs font-medium text-zinc-200">Use This machine</p>
            <p className="text-[11px] text-zinc-500">
              Bots here can use the CLIs signed in on this computer. Turn it off to keep this space on its own accounts only; bots never fall back to it.
            </p>
          </div>
          <button
            type="button"
            role="switch"
            aria-checked={thisMachine}
            aria-labelledby="space-this-machine"
            onClick={() => setThisMachine((v) => !v)}
            className={`relative shrink-0 w-9 h-5 rounded-full transition-colors focus-visible:outline focus-visible:outline-2 focus-visible:outline-sky-500 ${thisMachine ? 'bg-sky-600' : 'bg-zinc-700'}`}
          >
            <span className={`absolute top-0.5 left-0.5 w-4 h-4 rounded-full bg-white transition-transform motion-reduce:transition-none ${thisMachine ? 'translate-x-4' : ''}`} />
          </button>
        </div>

        <label className="flex items-start gap-3 bg-zinc-950/60 p-3 rounded-xl border border-zinc-800 cursor-pointer">
          <input type="checkbox" checked={addAccounts} onChange={(e) => setAddAccounts(e.target.checked)} className="mt-0.5 accent-sky-500" aria-describedby="space-add-accounts-detail" />
          <span className="flex-1">
            <span className="flex items-center gap-1.5 text-xs font-medium text-zinc-200">
              <KeyRound size={12} className="text-sky-400" aria-hidden="true" /> Add accounts next
            </span>
            <span id="space-add-accounts-detail" className="block text-[11px] text-zinc-500">
              Sign in to more Claude, Codex or Grok accounts, or add an Anthropic, OpenAI or xAI API key. They stay in this space.
            </span>
          </span>
        </label>

        <fieldset className="space-y-1.5">
          <legend className="text-xs font-medium text-zinc-300 mb-1.5">Start with</legend>
          <label className="flex items-center gap-2.5 p-2.5 rounded-lg border border-sky-500/40 bg-sky-500/5 text-xs text-zinc-200">
            <input type="radio" name="space-start" checked readOnly className="accent-sky-500" />
            An empty space
          </label>
          <label className="flex items-center gap-2.5 p-2.5 rounded-lg border border-zinc-800 text-xs text-zinc-500 cursor-not-allowed">
            <input type="radio" name="space-start" disabled className="accent-sky-500" />
            <span className="flex-1">Copy bots from another space</span>
            <span className="px-1.5 py-px rounded bg-zinc-800 text-[10px] text-zinc-400">Coming soon</span>
          </label>
        </fieldset>

        {error && (
          <div role="alert" className="bg-rose-500/10 border border-rose-500/20 text-rose-300 px-3 py-2 rounded-lg text-xs flex items-center gap-2">
            <AlertTriangle size={14} /> <span>{error}</span>
          </div>
        )}

        <div className="flex justify-end gap-2 pt-1">
          <button type="button" onClick={onClose} className="px-3.5 py-2 rounded-lg text-xs font-medium text-zinc-300 hover:text-zinc-100 hover:bg-zinc-800">
            Cancel
          </button>
          <button
            type="submit"
            disabled={saving}
            className="px-4 py-2 rounded-lg text-xs font-medium text-white bg-sky-600 hover:bg-sky-500 disabled:opacity-50 flex items-center gap-1.5"
          >
            {saving && <Loader2 size={13} className="animate-spin motion-reduce:animate-none" aria-hidden="true" />}
            {saving ? 'Creating…' : 'Create space'}
          </button>
        </div>
      </form>
    </Dialog>
  );
};

// ---------------------------------------------------------------------------
// Archived spaces

export const ArchivedSpacesModal: React.FC<{ onClose: () => void; onRestored: (space: Space) => void }> = ({ onClose, onRestored }) => {
  const [list, setList] = useState<Space[] | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState<string | null>(null);
  useEffect(() => {
    spacesApi.archived().then(setList, (e) => setError(e instanceof Error ? e.message : 'Couldn’t load archived spaces.'));
  }, []);

  const restore = async (s: Space) => {
    setBusy(s.id);
    setError(null);
    try {
      const restored = await spacesApi.patch(s.id, { archived: false });
      setList((prev) => prev?.filter((x) => x.id !== s.id) ?? null);
      playSfx('coin');
      onRestored(restored);
    } catch (e) {
      setError(e instanceof Error ? e.message : 'Couldn’t restore the space.');
    } finally {
      setBusy(null);
    }
  };

  return (
    <Dialog title={`Archived spaces${list ? ` (${list.length})` : ''}`} subtitle="Their routines are paused and nothing new runs until you restore them." labelId="archived-title" onClose={onClose}>
      {error && <p role="alert" className="text-xs text-rose-300">{error}</p>}
      {!list && !error && (
        <p className="flex items-center gap-2 text-xs text-zinc-400"><Loader2 size={13} className="animate-spin motion-reduce:animate-none" /> Loading…</p>
      )}
      {list?.length === 0 && <p className="text-xs text-zinc-500">No archived spaces.</p>}
      <ul className="space-y-1.5">
        {list?.map((s) => (
          <li key={s.id} className="flex items-center gap-3 p-2.5 rounded-xl border border-zinc-800 bg-zinc-950/50">
            <SpaceTile space={s} size={32} />
            <div className="flex-1 min-w-0">
              <p className="text-xs font-medium text-zinc-200 truncate">{s.name}</p>
              <p className="text-[11px] text-zinc-500">{spaceSummary(s.counts)}</p>
              {s.paused_routines ? (
                <p className="text-[11px] text-amber-300/80">
                  {s.paused_routines} {s.paused_routines === 1 ? 'routine' : 'routines'} paused
                </p>
              ) : null}
            </div>
            <button
              type="button"
              disabled={busy === s.id}
              onClick={() => restore(s)}
              className="flex items-center gap-1.5 px-2.5 py-1.5 rounded-lg text-xs font-medium text-sky-300 border border-sky-500/30 bg-sky-500/10 hover:bg-sky-500/20 disabled:opacity-50"
            >
              {busy === s.id ? <Loader2 size={12} className="animate-spin motion-reduce:animate-none" /> : <RotateCcw size={12} />}
              Restore
            </button>
          </li>
        ))}
      </ul>
    </Dialog>
  );
};
