import React, { useEffect, useRef, useState } from 'react';
import {
  AlertTriangle,
  Archive,
  ArrowLeft,
  Bot,
  CalendarClock,
  Check,
  Download,
  FileUp,
  Globe,
  KeyRound,
  Link2,
  Loader2,
  Pencil,
  Plug,
  Plus,
  Settings2,
  Sparkles,
  Users,
  X,
} from 'lucide-react';
import { playSfx } from '../audio/chiptune';
import { SPACE_COLORS, SPACE_ICONS, Space, spaceColor, spaceNameError, spacesApi } from '../spaces';
import { useEngines } from '../useEngines';
import { useProfiles } from '../useProfiles';
import type { Profile } from '../profiles';
import { EnginesPanel } from './EnginesPanel';
import { ApiKeysPanel } from './ApiKeysPanel';
import { RoutinesPanel, type RoutineFocus } from './RoutinesPanel';
import { HirePolicyPanel } from './HirePolicyPanel';
import { ModernAvatar } from './ModernAvatar';
import { SpaceTile } from './Spaces';
import { ImportDialog, RecentImports } from './ImportDialog';
import { ImportError, ImportRecord, downloadExport, importApi, importErrorText } from '../imports';
import { memberIdsOf, type Channel, type Kwant, type Team } from './Sidebar';
import { SpaceApprovalsSetting } from './ApprovalsSetting';

import type { SettingsScope, SpaceSettingsTab } from '../nav';
import { InstallAppButton } from './InstallAppButton';

export type { SettingsScope, SpaceSettingsTab };

const TABS: Array<{ id: SpaceSettingsTab; label: string; icon: React.ReactNode }> = [
  { id: 'general', label: 'General', icon: <Settings2 size={15} /> },
  { id: 'accounts', label: 'Accounts', icon: <KeyRound size={15} /> },
  { id: 'bots', label: 'Kwant bots', icon: <Bot size={15} /> },
  { id: 'teams', label: 'Teams', icon: <Users size={15} /> },
  { id: 'connections', label: 'Connections', icon: <Link2 size={15} /> },
  { id: 'tools', label: 'Tools', icon: <Plug size={15} /> },
  { id: 'skills', label: 'Skills', icon: <Sparkles size={15} /> },
  { id: 'routines', label: 'Routines', icon: <CalendarClock size={15} /> },
  { id: 'import', label: 'Import & export', icon: <FileUp size={15} /> },
];

interface Props {
  space: Space;
  /** Every space, for taken names. */
  spaces: Space[];
  /** This space, or All spaces (app-wide settings). */
  scope: SettingsScope;
  onScope: (scope: SettingsScope) => void;
  tab: SpaceSettingsTab;
  onTab: (tab: SpaceSettingsTab) => void;
  onClose: () => void;
  /** This space › Connections: the space's connected apps and tools. */
  connections: React.ReactNode;
  /** All spaces: the app-wide settings. */
  appSettings: React.ReactNode;
  onOpenArchived: () => void;
  kwants: Kwant[];
  channels: Channel[];
  teams: Team[];
  /** Changes to focus "Sign in with Claude" in Accounts. */
  focusClaudeSignIn?: number;
  onProfilesChanged?: (profiles: Profile[]) => void;
  pendingSkills?: Record<string, number>;
  onSpaceSaved: (space: Space) => void;
  onArchived: (space: Space) => void;
  onNewBot: () => void;
  onEditBot: (k: Kwant) => void;
  onOpenSkills: (k: Kwant) => void;
  onNewTeam: () => void;
  onManageTeam: (t: Team) => void;
  onOpenTools: () => void;
  /** Set by a routine card's Edit or Pause: opens that routine in Routines. */
  routineFocus?: RoutineFocus | null;
  /** After an import or its undo: load bots, teams and chats again. */
  onImported?: () => void;
  /** After the approvals switch changes: load bots and spaces again (no event fires). */
  onApprovalsChanged?: () => void;
}

/** What the tabs get on top of the props: opening the Import dialog. */
type TabProps = Props & { onOpenImport: () => void; importsSeq: number };

const card = 'rounded-xl border border-zinc-800 bg-zinc-950/60';
const rowBtn =
  'flex items-center gap-1.5 px-2.5 py-1.5 rounded-lg text-xs font-medium text-zinc-300 border border-zinc-700/60 bg-zinc-800/60 hover:bg-zinc-700/70 hover:text-zinc-100 transition-colors';
const primaryBtn =
  'flex items-center gap-1.5 px-3 py-1.5 rounded-lg text-xs font-medium text-white bg-sky-600 hover:bg-sky-500 transition-colors disabled:opacity-50';

/** Settings, in one place: "This space" (only the open space) and "All spaces" (the app). */
export const SpaceSettings: React.FC<Props> = (p) => {
  const { space, tab, onTab, onClose, scope, onScope } = p;
  const toApp = () => {
    playSfx('click');
    onScope('app');
  };
  const [importOpen, setImportOpen] = useState(false);
  // Bumped after the dialog imports or undoes, so the Import tab's list loads again.
  const [importsSeq, setImportsSeq] = useState(0);
  const tp: TabProps = { ...p, onOpenImport: () => setImportOpen(true), importsSeq };
  const closeRef = useRef(onClose);
  closeRef.current = onClose;
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      // Dialogs opened from here (bot editor, tools) close themselves first.
      if (e.key === 'Escape' && !document.querySelector('[role="dialog"]:not([data-space-settings])')) closeRef.current();
    };
    window.addEventListener('keydown', onKey);
    return () => window.removeEventListener('keydown', onKey);
  }, []);

  return (
    <div
      role="dialog"
      aria-modal="true"
      aria-labelledby="space-settings-title"
      data-space-settings
      className="fixed inset-0 z-40 bg-zinc-950 text-zinc-100 flex flex-col md:flex-row overflow-hidden"
    >
      {/* Tab list: a column on desktop, scrolling chips on phones */}
      <div className="md:w-60 shrink-0 border-b md:border-b-0 md:border-r border-zinc-800/80 bg-zinc-900/40 flex flex-col">
        <div className="flex items-center gap-2.5 p-3 md:p-4 pb-2 md:pb-3">
          <button type="button" onClick={onClose} aria-label="Back to chats" className="p-1.5 -ml-1 rounded-lg text-zinc-400 hover:text-zinc-100 hover:bg-zinc-800">
            <ArrowLeft size={17} />
          </button>
          <h2 id="space-settings-title" className="text-sm font-semibold truncate">Settings</h2>
        </div>
        {/* Which settings: this space's, or the ones every space shares */}
        <div role="tablist" aria-label="Settings scope" className="mx-3 md:mx-2 mb-3 grid grid-cols-2 gap-1 rounded-lg bg-zinc-900 border border-zinc-800 p-0.5">
          {(
            [
              { id: 'space', label: 'This space', icon: <SpaceTile space={space} size={16} /> },
              { id: 'app', label: 'All spaces', icon: <Globe size={13} aria-hidden="true" /> },
            ] as const
          ).map((s) => (
            <button
              key={s.id}
              type="button"
              role="tab"
              aria-selected={scope === s.id}
              data-settings-scope={s.id}
              onClick={() => {
                if (scope === s.id) return;
                playSfx('click');
                onScope(s.id);
              }}
              className={`flex items-center justify-center gap-1.5 min-w-0 px-2 py-1.5 rounded-md text-xs font-medium transition-colors focus-visible:outline focus-visible:outline-2 focus-visible:outline-sky-500 ${
                scope === s.id ? 'bg-zinc-800 text-zinc-100 shadow-sm' : 'text-zinc-400 hover:text-zinc-200'
              }`}
            >
              <span className="shrink-0 flex">{s.icon}</span>
              <span className="truncate">{s.label}</span>
            </button>
          ))}
        </div>
        {scope === 'space' && (
          <nav aria-label={`${space.name} settings`} className="flex md:flex-col gap-1 px-3 pb-3 md:px-2 overflow-x-auto md:overflow-visible">
            {TABS.map((t) => (
              <button
                key={t.id}
                type="button"
                aria-current={tab === t.id ? 'page' : undefined}
                onClick={() => {
                  playSfx('click');
                  onTab(t.id);
                }}
                className={`shrink-0 flex items-center gap-2 px-3 py-1.5 md:py-2 rounded-full md:rounded-lg text-xs font-medium transition-colors ${
                  tab === t.id ? 'bg-zinc-800 text-zinc-100 border border-zinc-700/60' : 'text-zinc-400 hover:text-zinc-200 hover:bg-zinc-800/50 border border-transparent'
                }`}
              >
                <span className={tab === t.id ? 'text-sky-400' : 'text-zinc-500'} aria-hidden="true">{t.icon}</span>
                {t.label}
              </button>
            ))}
          </nav>
        )}
      </div>

      <div className="flex-1 min-w-0 overflow-y-auto">
        <div className="max-w-2xl mx-auto p-4 sm:p-6 space-y-5">
          <div className="flex items-center justify-between gap-3">
            <div>
              <h3 className="text-base font-semibold">{scope === 'app' ? 'All spaces' : TABS.find((t) => t.id === tab)?.label}</h3>
              <p className="text-[11px] text-zinc-500">
                {scope === 'app' ? 'Applies to every space and to this app' : `Only affects this space (${space.name})`}
              </p>
            </div>
            <button type="button" onClick={onClose} aria-label="Close settings" className="hidden md:block p-1.5 rounded-lg text-zinc-400 hover:text-zinc-100 hover:bg-zinc-800">
              <X size={18} />
            </button>
          </div>
          {scope === 'app' ? (
            <>
              <div className={`${card} p-4 space-y-3`} data-all-spaces-spaces>
                <p className="text-xs font-medium text-zinc-200">Spaces</p>
                <p className="text-[11px] text-zinc-500">Switch spaces on the left rail (Ctrl or ⌘ with 1–9). Archived spaces keep their bots and chats until you restore them.</p>
                <div className="flex flex-wrap gap-2">
                  <button type="button" onClick={p.onOpenArchived} className={rowBtn}>
                    <Archive size={13} /> Archived spaces
                  </button>
                  <InstallAppButton className={rowBtn} />
                </div>
              </div>
              {p.appSettings}
            </>
          ) : (
            <>
              {tab === 'general' && <GeneralTab {...p} />}
              {tab === 'accounts' && <AccountsTab {...p} />}
              {tab === 'bots' && <BotsTab {...tp} />}
              {tab === 'teams' && <TeamsTab {...p} />}
              {tab === 'connections' && <div data-settings-connections>{p.connections}</div>}
              {tab === 'tools' && (
                <div className={`${card} p-4 space-y-3`}>
                  <p className="text-xs text-zinc-300">Tool servers (MCP) give the bots in {space.name} abilities like files, web pages or GitHub.</p>
                  <button type="button" onClick={p.onOpenTools} className={primaryBtn}>
                    <Plug size={13} /> Manage tools
                  </button>
                </div>
              )}
              {tab === 'skills' && <SkillsTab {...p} />}
              {tab === 'routines' && <RoutinesPanel kwants={p.kwants} channels={p.channels} focus={p.routineFocus} onOpenSettings={toApp} />}
              {tab === 'import' && <ImportTab {...tp} />}
            </>
          )}
        </div>
      </div>
      {importOpen && (
        <ImportDialog
          spaceName={space.name}
          onClose={() => setImportOpen(false)}
          onChanged={() => {
            setImportsSeq((n) => n + 1);
            p.onImported?.();
          }}
        />
      )}
    </div>
  );
};

/** Export one bot or the whole space; shows a short error under the button if it fails. */
function useExport() {
  const [busy, setBusy] = useState<string | null>(null);
  const [error, setError] = useState<{ key: string; text: string } | null>(null);
  const run = async (key: string, kwantId?: string) => {
    setBusy(key);
    setError(null);
    try {
      await downloadExport(kwantId);
    } catch (err) {
      setError({ key, text: importErrorText(err).detail || 'The export didn’t work.' });
    } finally {
      setBusy(null);
    }
  };
  return { busy, error, run };
}

const ImportTab: React.FC<TabProps> = ({ space, kwants, onOpenImport, onImported, importsSeq }) => {
  const exp = useExport();
  const [records, setRecords] = useState<ImportRecord[] | null>(null);
  const [listError, setListError] = useState<unknown>(null);
  const [undoing, setUndoing] = useState<string | null>(null);
  const [confirm, setConfirm] = useState<{ record: ImportRecord; names: string[] } | null>(null);
  const [notice, setNotice] = useState<string | null>(null);
  const load = () => importApi.list().then((r) => { setRecords(r); setListError(null); }, setListError);
  useEffect(() => {
    load();
  }, [space.id, importsSeq]);

  const undo = async (r: ImportRecord, force = false) => {
    setUndoing(r.id);
    setConfirm(null);
    setNotice(null);
    try {
      await importApi.undo(r.id, force);
      setNotice('Import undone.');
      onImported?.();
    } catch (err) {
      if (err instanceof ImportError && err.code === 'edited') setConfirm({ record: r, names: err.names });
      else if (err instanceof ImportError && err.code === 'in_use')
        setNotice(`${err.names.join(', ') || 'Something it made'} has already been used, so this import can’t be undone in one step. Delete those bots yourself in Kwant bots.`);
      else setNotice(importErrorText(err).detail || importErrorText(err).title);
    } finally {
      setUndoing(null);
      load();
    }
  };

  return (
    <>
      <div className={`${card} p-4 space-y-3`}>
        <p className="text-xs font-medium text-zinc-200">Bring bots from another app</p>
        <p className="text-[11px] text-zinc-500">From Grok, ChatGPT, Claude or any other bot, or from a file. You check a preview first, and you can undo it.</p>
        <button type="button" onClick={onOpenImport} className={primaryBtn}>
          <FileUp size={13} /> Import bots
        </button>
      </div>
      <div className={`${card} p-4 space-y-3`}>
        <p className="text-xs font-medium text-zinc-200">Export {space.name}</p>
        <p className="text-[11px] text-zinc-500">
          Downloads {kwants.length === 1 ? 'its bot' : `its ${kwants.length} bots`}, teams and shared skills as one file you can import again. Chat history, files, tools and keys are never included.
        </p>
        <button type="button" onClick={() => exp.run('space')} disabled={exp.busy === 'space' || kwants.length === 0} className={rowBtn}>
          {exp.busy === 'space' ? <Loader2 size={13} className="animate-spin motion-reduce:animate-none" /> : <Download size={13} />} Export space
        </button>
        {exp.error && <p role="alert" className="text-[11px] text-rose-300">{exp.error.text}</p>}
      </div>
      <div className="space-y-2">
        <p className="text-xs font-medium text-zinc-300">Recent imports</p>
        {confirm && (
          <div role="alert" className="rounded-lg border border-amber-500/40 bg-amber-500/10 p-3 space-y-2">
            <p className="text-xs text-amber-100">
              {confirm.names.join(', ')} changed since the import. Undo anyway removes {confirm.names.length === 1 ? 'it' : 'them'} with your changes.
            </p>
            <div className="flex flex-wrap gap-2">
              <button type="button" onClick={() => setConfirm(null)} className={rowBtn}>Keep them</button>
              <button type="button" onClick={() => undo(confirm.record, true)} className="flex items-center gap-1.5 px-3 py-1.5 rounded-lg text-xs font-medium text-white bg-rose-600 hover:bg-rose-500">
                Undo anyway
              </button>
            </div>
          </div>
        )}
        {notice && <p role="status" className="text-[11px] text-zinc-300">{notice}</p>}
        {listError ? (
          <p role="alert" className="text-[11px] text-rose-300">{importErrorText(listError).detail || 'Couldn’t load the imports.'}</p>
        ) : records ? (
          <RecentImports records={records} busyId={undoing} onUndo={(r) => undo(r)} />
        ) : (
          <p className="text-[11px] text-zinc-500">Loading…</p>
        )}
      </div>
    </>
  );
};

const GeneralTab: React.FC<Props> = ({ space, spaces, kwants, onSpaceSaved, onArchived, onApprovalsChanged }) => {
  const [name, setName] = useState(space.name);
  const [icon, setIcon] = useState(space.icon);
  // "amber" from the server shows as its swatch.
  const initialColor = space.color ? spaceColor(space) : '';
  const [color, setColor] = useState(initialColor);
  const [saving, setSaving] = useState(false);
  const [saved, setSaved] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [confirmArchive, setConfirmArchive] = useState(false);
  const [archiving, setArchiving] = useState(false);
  const nameErr = spaceNameError(name, spaces.filter((s) => s.id !== space.id).map((s) => s.name));
  const dirty = name.trim() !== space.name || icon !== space.icon || color !== initialColor;

  const save = async (e: React.FormEvent) => {
    e.preventDefault();
    if (nameErr || !dirty) return;
    setSaving(true);
    setError(null);
    try {
      const next = await spacesApi.patch(space.id, { name: name.trim(), icon, color: color || null });
      onSpaceSaved(next);
      setSaved(true);
      setTimeout(() => setSaved(false), 2000);
    } catch (err) {
      setError(err instanceof Error ? err.message : 'Couldn’t save the space.');
    } finally {
      setSaving(false);
    }
  };

  const archive = async () => {
    setArchiving(true);
    setError(null);
    try {
      const next = await spacesApi.patch(space.id, { archived: true });
      onArchived(next);
    } catch (err) {
      setError(err instanceof Error ? err.message : 'Couldn’t archive the space.');
      setArchiving(false);
      setConfirmArchive(false);
    }
  };

  return (
    <>
      <form onSubmit={save} className={`${card} p-4 space-y-4`}>
        <div className="flex items-center gap-3">
          <SpaceTile space={{ id: space.id, icon, color: color || null }} size={44} />
          <div className="flex-1 min-w-0">
            <label htmlFor="space-rename" className="block text-xs font-medium text-zinc-300 mb-1">
              Name {space.is_default && <span className="ml-1 px-1.5 py-px rounded bg-zinc-800 text-[10px] text-zinc-400">Default</span>}
            </label>
            <input
              id="space-rename"
              value={name}
              maxLength={60}
              onChange={(e) => setName(e.target.value)}
              aria-invalid={!!nameErr}
              aria-describedby="space-rename-error"
              className={`w-full bg-zinc-950 border rounded-lg px-3 py-2 text-sm text-zinc-100 focus:outline-none ${nameErr ? 'border-rose-500/70' : 'border-zinc-800 focus:border-sky-500'}`}
            />
            <p id="space-rename-error" className="mt-1 text-[11px] text-rose-300 min-h-[1em]">{nameErr}</p>
          </div>
        </div>
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
                className={`w-9 h-9 max-md:w-11 max-md:h-11 rounded-lg text-lg flex items-center justify-center border ${icon === i ? 'border-sky-400 bg-sky-500/10' : 'border-zinc-800 hover:border-zinc-600'}`}
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
                className={`w-7 h-7 rounded-full ${color === c ? 'ring-2 ring-white ring-offset-2 ring-offset-zinc-900' : ''}`}
                style={{ background: c }}
              />
            ))}
          </div>
        </fieldset>
        {error && (
          <p role="alert" className="flex items-center gap-2 text-xs text-rose-300">
            <AlertTriangle size={13} /> {error}
          </p>
        )}
        <div className="flex justify-end items-center gap-2">
          {saved && (
            <span className="flex items-center gap-1 text-xs text-emerald-400" role="status">
              <Check size={13} /> Saved
            </span>
          )}
          <button type="submit" disabled={!dirty || !!nameErr || saving} className={primaryBtn}>
            {saving ? 'Saving…' : 'Save'}
          </button>
        </div>
      </form>

      <SpaceApprovalsSetting
        key={space.id}
        spaceId={space.id}
        spaceName={space.name}
        value={space.approvals ?? 'ask'}
        overrides={{
          ask: kwants.filter((k) => k.approvals === 'ask').map((k) => k.name),
          off: kwants.filter((k) => k.approvals === 'off').map((k) => k.name),
        }}
        onChanged={() => onApprovalsChanged?.()}
      />

      <div className={`${card} p-4 space-y-3`}>
        <p className="flex items-center gap-2 text-xs font-medium text-zinc-200">
          <Archive size={14} className="text-zinc-400" /> Archive space
        </p>
        {space.is_default ? (
          <p className="text-[11px] text-zinc-500">The Default space can be renamed but can’t be archived.</p>
        ) : confirmArchive ? (
          <div className="space-y-2.5" role="alert">
            <p className="text-xs text-zinc-300">
              Archive {space.name}? Its routines pause and nothing new can run in it. Its bots, chats and accounts stay, and you can restore it from Archived spaces.
            </p>
            <div className="flex gap-2">
              <button type="button" onClick={() => setConfirmArchive(false)} className={rowBtn}>
                Cancel
              </button>
              <button
                type="button"
                onClick={archive}
                disabled={archiving}
                className="flex items-center gap-1.5 px-3 py-1.5 rounded-lg text-xs font-medium text-white bg-rose-600 hover:bg-rose-500 disabled:opacity-50"
              >
                {archiving && <Loader2 size={12} className="animate-spin motion-reduce:animate-none" />}
                Archive {space.name}
              </button>
            </div>
          </div>
        ) : (
          <>
            <p className="text-[11px] text-zinc-500">Pauses its routines and stops anything new from running. You can restore it any time.</p>
            <button type="button" onClick={() => setConfirmArchive(true)} className={rowBtn}>
              <Archive size={13} /> Archive space…
            </button>
          </>
        )}
      </div>
    </>
  );
};

const AccountsTab: React.FC<Props> = ({ space, focusClaudeSignIn, onProfilesChanged }) => {
  const engines = useEngines(true);
  const accounts = useProfiles(true, onProfilesChanged);
  return (
    <div className="space-y-3">
      <p className="text-xs text-zinc-400">What runs your kwant bots in {space.name}: the accounts here, and This machine when it’s on.</p>
      <EnginesPanel api={engines} accounts={accounts} focusClaudeSignIn={focusClaudeSignIn} />
      <ApiKeysPanel
        onSaved={() => {
          // Saving a key or the Ollama address re-checks engines on the server; show the result.
          window.setTimeout(engines.reload, 1500);
          window.setTimeout(engines.reload, 6000);
        }}
      />
    </div>
  );
};

const BotsTab: React.FC<TabProps> = ({ space, kwants, onNewBot, onEditBot, onOpenImport }) => {
  const exp = useExport();
  return (
  <div className="space-y-3">
    <div className="flex flex-wrap items-center justify-between gap-3">
      <p className="text-xs text-zinc-400">
        {kwants.length} {kwants.length === 1 ? 'bot' : 'bots'} in {space.name}. Laya only picks from these.
      </p>
      <div className="flex gap-2">
        <button type="button" onClick={onOpenImport} className={rowBtn}>
          <FileUp size={13} /> Import bots
        </button>
        <button type="button" onClick={onNewBot} className={primaryBtn}>
          <Plus size={13} /> New bot
        </button>
      </div>
    </div>
    {exp.error && <p role="alert" className="text-[11px] text-rose-300">{exp.error.text}</p>}
    <ul className="space-y-1.5">
      {kwants.map((k) => (
        <li key={k.id} className={`${card} flex items-center gap-3 p-2.5`}>
          <ModernAvatar role={k.avatar_sprite} name={k.name} size={30} kwantId={k.id} />
          <div className="flex-1 min-w-0">
            <p className="text-xs font-medium text-zinc-200 truncate">{k.name}</p>
            <p className="text-[11px] text-zinc-500 truncate">{k.title}</p>
          </div>
          <button type="button" onClick={() => exp.run(k.id, k.id)} disabled={exp.busy === k.id} className={rowBtn} aria-label={`Export ${k.name}`} title="Download as a file">
            {exp.busy === k.id ? <Loader2 size={12} className="animate-spin motion-reduce:animate-none" /> : <Download size={12} />}
            <span className="hidden sm:inline">Export</span>
          </button>
          <button type="button" onClick={() => onEditBot(k)} className={rowBtn} aria-label={`Edit ${k.name}`}>
            <Pencil size={12} /> Edit
          </button>
        </li>
      ))}
    </ul>
    {kwants.length === 0 && <p className="text-xs text-zinc-500">No bots yet. New spaces start empty. Make one, or import bots from another app.</p>}
    <HirePolicyPanel key={space.id} spaceId={space.id} spaceName={space.name} />
  </div>
  );
};

const TeamsTab: React.FC<Props> = ({ space, teams, kwants, onNewTeam, onManageTeam }) => (
  <div className="space-y-3">
    <div className="flex items-center justify-between gap-3">
      <p className="text-xs text-zinc-400">
        {teams.length} {teams.length === 1 ? 'team' : 'teams'} in {space.name}.
      </p>
      <button type="button" onClick={onNewTeam} disabled={kwants.length === 0} className={primaryBtn} title={kwants.length === 0 ? 'Create a bot first' : undefined}>
        <Plus size={13} /> New team
      </button>
    </div>
    <ul className="space-y-1.5">
      {teams.map((t) => {
        const ids = memberIdsOf(t);
        const lead = kwants.find((k) => k.id === t.lead_kwant_id);
        return (
          <li key={t.id} className={`${card} flex items-center gap-3 p-2.5`}>
            <Users size={16} className="text-zinc-500 shrink-0" aria-hidden="true" />
            <div className="flex-1 min-w-0">
              <p className="text-xs font-medium text-zinc-200 truncate">@{t.name}</p>
              <p className="text-[11px] text-zinc-500 truncate">
                {ids.length} {ids.length === 1 ? 'member' : 'members'}
                {lead ? ` · lead ${lead.name}` : ''}
              </p>
            </div>
            <button type="button" onClick={() => onManageTeam(t)} className={rowBtn} aria-label={`Manage @${t.name}`}>
              <Settings2 size={12} /> Manage
            </button>
          </li>
        );
      })}
    </ul>
    {teams.length === 0 && <p className="text-xs text-zinc-500">No teams yet.</p>}
  </div>
);

const SkillsTab: React.FC<Props> = ({ space, kwants, pendingSkills = {}, onOpenSkills }) => (
  <div className="space-y-3">
    <p className="text-xs text-zinc-400">Skills are shared by the bots in {space.name}. Open a bot to see, approve or write its skills.</p>
    <ul className="space-y-1.5">
      {kwants.map((k) => (
        <li key={k.id} className={`${card} flex items-center gap-3 p-2.5`}>
          <ModernAvatar role={k.avatar_sprite} name={k.name} size={26} kwantId={k.id} />
          <p className="flex-1 min-w-0 text-xs font-medium text-zinc-200 truncate">{k.name}</p>
          {(pendingSkills[k.id] ?? 0) > 0 && (
            <span className="px-1.5 py-px rounded-full bg-amber-500 text-zinc-950 text-[10px] font-semibold">{pendingSkills[k.id]} to review</span>
          )}
          <button type="button" onClick={() => onOpenSkills(k)} className={rowBtn}>
            <Sparkles size={12} /> Skills
          </button>
        </li>
      ))}
    </ul>
    {kwants.length === 0 && <p className="text-xs text-zinc-500">Skills show up once this space has a bot.</p>}
  </div>
);
