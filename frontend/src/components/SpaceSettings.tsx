import React, { useEffect, useRef, useState } from 'react';
import {
  AlertTriangle,
  Archive,
  ArrowLeft,
  Bell,
  Bot,
  CalendarClock,
  Check,
  Download,
  FileText,
  FileUp,
  Gauge,
  Globe,
  LayoutGrid,
  Loader2,
  Pencil,
  Plus,
  Search,
  Settings2,
  Sparkles,
  Users,
  X,
} from 'lucide-react';
import { ScrollRow } from './ScrollRow';
import { playSfx } from '../audio/chiptune';
import { SPACE_COLORS, SPACE_ICONS, Space, spaceColor, spaceNameError, spacesApi } from '../spaces';
import type { EnginesApi } from '../useEngines';
import type { ProfilesState } from '../useProfiles';
import type { Profile } from '../profiles';
import { EnginesPanel } from './EnginesPanel';
import { ApiKeysPanel } from './ApiKeysPanel';
import { RoutinesPanel, type RoutineFocus } from './RoutinesPanel';
import type { MessageTarget } from './DoneDetail';
import { HirePolicyPanel } from './HirePolicyPanel';
import { ModernAvatar } from './ModernAvatar';
import { PickerGlyph, SpaceTile } from './Spaces';
import { pixelIconFor } from '../spaceIcons';
import { ImportDialog, RecentImports } from './ImportDialog';
import { ImportError, ImportRecord, downloadExport, importApi, importErrorText } from '../imports';
import { memberIdsOf, type Channel, type Kwant, type Team } from './Sidebar';
import { SpaceApprovalsSetting } from './ApprovalsSetting';
import { DevModeSettings } from './DevModeSettings';
import { SpaceDocsTab } from './SpaceDocsTab';

import type { AppSettingsPage, AppsSection, SettingsScope, SpaceSettingsTab } from '../nav';
import { filterSettingRows } from '../settingsFilter';
import { AppsAccounts } from './AppsAccounts';
import { PhoneAppCard } from './PhoneAppCard';
import { editorProps, useEscapeLayer } from '../escape';
import { devTeamRefs } from '../devMode';
import type { NotifiedChat } from '../chatTarget';

export type { AppSettingsPage, SettingsScope, SpaceSettingsTab };

/** 36e: Settings › All spaces as four short pages under the same left nav as This space. */
export const APP_PAGES: Array<{ id: AppSettingsPage; label: string; icon: React.ReactNode }> = [
  { id: 'general', label: 'General', icon: <Settings2 size={15} /> },
  { id: 'bots', label: 'Bots on their own', icon: <Sparkles size={15} /> },
  { id: 'usage', label: 'Usage', icon: <Gauge size={15} /> },
  { id: 'notifications', label: 'Notifications', icon: <Bell size={15} /> },
];

const TABS: Array<{ id: SpaceSettingsTab; label: string; icon: React.ReactNode }> = [
  { id: 'general', label: 'General', icon: <Settings2 size={15} /> },
  { id: 'apps', label: 'Apps & accounts', icon: <LayoutGrid size={15} /> },
  { id: 'bots', label: 'Bots', icon: <Bot size={15} /> },
  { id: 'teams', label: 'Teams', icon: <Users size={15} /> },
  { id: 'docs', label: 'Documents', icon: <FileText size={15} /> },
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
  /** Apps & accounts › Advanced: tool servers, Telegram and notes from bots; gets Composio's "Choose bots". */
  connections: (chooseComposioBots: () => void) => React.ReactNode;
  /** Apps & accounts opens at this section (seq changes to scroll there again). */
  appsSection?: { section: AppsSection; seq: number } | null;
  /** A bot's tool note was asked for: Advanced opens on it. */
  toolNoteOpen?: boolean;
  /** A bot's "suggests a connection" card: Apps & accounts opens the Connect <App> sheet with these bots ticked (seq: a fresh sheet per press). */
  connectApp?: { slug: string; name?: string; initial: { picked: string[] }; seq: number } | null;
  /** All spaces: the app-wide settings, given the page to show (null while "Find a setting" has words: every page). */
  appSettings: React.ReactNode | ((page: AppSettingsPage | null) => React.ReactNode);
  /** All spaces: the open page (General when not given) and a way to change it (it shows in the address). */
  appPage?: AppSettingsPage;
  onAppPage?: (page: AppSettingsPage) => void;
  onOpenArchived: () => void;
  kwants: Kwant[];
  channels: Channel[];
  teams: Team[];
  /** Changes to focus "Sign in with Claude" in Apps & accounts. */
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
  /** 0.9.5 F: a routine run's History row opens its answer in the chat. */
  onOpenRunResult?: (t: MessageTarget) => void;
  /** 26d: a waiting routine run's "Open the card". */
  onOpenRunCard?: (t: NotifiedChat) => void;
  /** Phones get a full-screen dialog; desktop keeps the space rail beside Settings. */
  isPhone?: boolean;
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
  'flex items-center gap-1.5 px-3 py-1.5 rounded-lg text-xs font-medium text-white bg-sky-600 hover:bg-sky-700 transition-colors disabled:opacity-50';

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
  const rootRef = useRef<HTMLDivElement>(null);
  // Escape closes Settings, but not while a dialog opened from here is open (the bot editor, Tools),
  // and not while an editor in it holds typed text.
  useEscapeLayer(rootRef, true, onClose, { covered: () => !!document.querySelector('[role="dialog"]:not([data-space-settings])') });
  // Focus starts on the title (from the gear, Ctrl/⌘ 1–9 or a link), unless a tab already focused
  // something inside, or a space was picked on the rail, which keeps focus there.
  useEffect(() => {
    const a = document.activeElement;
    if (rootRef.current?.contains(a) || a?.closest('[data-rail-space]')) return;
    rootRef.current?.querySelector<HTMLElement>('#space-settings-title')?.focus();
  }, []);
  // Phone: the tab strip scrolls sideways, so bring the open tab into view (from the sidebar's
  // Routines row it would sit off the right edge; scroll-mx-6 keeps it clear of the edge fade). Instant: no motion.
  const currentTabRef = useRef<HTMLButtonElement>(null);
  const appPage = p.appPage ?? 'general';
  useEffect(() => {
    currentTabRef.current?.scrollIntoView?.({ block: 'nearest', inline: 'nearest' });
  }, [tab, scope, appPage]);
  // All spaces › "Find a setting": words typed show the matching rows of every page; empty, the open page again.
  const [find, setFind] = useState('');
  const findRef = useRef<HTMLInputElement>(null);
  const finding = scope === 'app' && find.trim() !== '';
  const contentRef = useRef<HTMLDivElement>(null);
  const [found, setFound] = useState<number | null>(null);
  // After every render while finding: rows load late (settings, usage), so the filter runs again on each.
  useEffect(() => {
    const el = contentRef.current;
    if (!el) return;
    const n = filterSettingRows(el, finding ? find : '');
    setFound((prev) => (finding ? (prev === n ? prev : n) : null));
  });
  const pageLabel = APP_PAGES.find((x) => x.id === appPage)?.label ?? 'General';
  const appBody = typeof p.appSettings === 'function' ? p.appSettings(finding ? null : appPage) : p.appSettings;
  const showGeneral = finding || appPage === 'general';

  return (
    // A full-screen dialog on phones. On desktop it fills the space's area beside the space rail,
    // which stays clickable: picking another space keeps Settings open there.
    <div
      ref={rootRef}
      role={p.isPhone ? 'dialog' : 'region'}
      aria-modal={p.isPhone ? true : undefined}
      aria-labelledby="space-settings-title"
      data-space-settings
      className={`${p.isPhone ? 'fixed' : 'absolute'} inset-0 z-40 bg-zinc-950 text-zinc-100 flex flex-col md:flex-row overflow-hidden`}
    >
      {/* Tab list: a column on desktop, scrolling chips on phones */}
      <div className="md:w-60 shrink-0 border-b md:border-b-0 md:border-r border-zinc-800/80 bg-zinc-900/40 flex flex-col md:sticky md:top-0 md:max-h-full md:overflow-y-auto" data-settings-nav>
        <div className="flex items-center gap-2.5 p-3 md:p-4 pb-2 md:pb-3">
          <button type="button" onClick={onClose} aria-label="Back to chats" className="p-1.5 -ml-1 rounded-lg text-zinc-400 hover:text-zinc-100 hover:bg-zinc-800">
            <ArrowLeft size={17} />
          </button>
          <h2 id="space-settings-title" tabIndex={-1} className="text-sm font-semibold truncate focus:outline-none">Settings</h2>
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
          <ScrollRow as="nav" aria-label={`${space.name} settings`} className="kw-scroll-row flex md:flex-col gap-1 px-3 pb-3 md:px-2 overflow-x-auto md:overflow-visible">
            {TABS.map((t) => (
              <button
                key={t.id}
                type="button"
                ref={tab === t.id ? currentTabRef : undefined}
                aria-current={tab === t.id ? 'page' : undefined}
                onClick={() => {
                  playSfx('click');
                  onTab(t.id);
                }}
                className={`shrink-0 scroll-mx-6 flex items-center gap-2 px-3 py-1.5 md:py-2 rounded-full md:rounded-lg text-xs font-medium transition-colors ${
                  tab === t.id ? 'bg-zinc-800 text-zinc-100 border border-zinc-700/60' : 'text-zinc-400 hover:text-zinc-200 hover:bg-zinc-800/50 border border-transparent'
                }`}
              >
                <span className={tab === t.id ? 'text-sky-400' : 'text-zinc-500'} aria-hidden="true">{t.icon}</span>
                {t.label}
              </button>
            ))}
          </ScrollRow>
        )}
        {scope === 'app' && (
          <>
            <div className="relative mx-3 md:mx-2 mb-2" role="search">
              <Search size={13} className="absolute left-2.5 top-1/2 -translate-y-1/2 text-zinc-500 pointer-events-none" aria-hidden="true" />
              <input
                ref={findRef}
                type="search"
                value={find}
                onChange={(e) => setFind(e.target.value)}
                onKeyDown={(e) => {
                  // Escape clears the words first; a second Escape closes Settings.
                  if (e.key === 'Escape' && find) {
                    e.preventDefault();
                    e.stopPropagation();
                    setFind('');
                  }
                }}
                placeholder="Find a setting"
                aria-label="Find a setting"
                aria-describedby={finding ? 'settings-find-count' : undefined}
                autoComplete="off"
                spellCheck={false}
                data-settings-find
                className="w-full bg-zinc-950 border border-zinc-800 rounded-lg pl-7 pr-9 py-1.5 text-xs text-zinc-100 placeholder-zinc-500 focus:border-sky-500 focus:outline-none [&::-webkit-search-cancel-button]:appearance-none"
              />
              {find && (
                <button
                  type="button"
                  aria-label="Clear"
                  data-settings-find-clear
                  onClick={() => {
                    setFind('');
                    findRef.current?.focus();
                  }}
                  className="absolute right-0.5 top-1/2 -translate-y-1/2 kw-hit kw-focus flex items-center justify-center w-7 h-7 rounded-md text-zinc-400 hover:text-zinc-100 hover:bg-zinc-800"
                >
                  <X size={14} aria-hidden="true" />
                </button>
              )}
            </div>
            <ScrollRow as="nav" aria-label="All spaces settings" className="kw-scroll-row flex flex-wrap md:flex-col gap-1 px-3 pb-3 md:px-2" data-app-settings-nav>
              {APP_PAGES.map((t) => {
                const on = !finding && appPage === t.id;
                return (
                  <button
                    key={t.id}
                    type="button"
                    ref={on ? currentTabRef : undefined}
                    aria-current={on ? 'page' : undefined}
                    data-app-page-link={t.id}
                    onClick={() => {
                      playSfx('click');
                      setFind('');
                      p.onAppPage?.(t.id);
                    }}
                    className={`shrink-0 scroll-mx-6 flex items-center gap-2 px-3 py-1.5 md:py-2 rounded-full md:rounded-lg text-xs font-medium transition-colors ${
                      on ? 'bg-zinc-800 text-zinc-100 border border-zinc-700/60' : 'text-zinc-400 hover:text-zinc-200 hover:bg-zinc-800/50 border border-transparent'
                    }`}
                  >
                    <span className={on ? 'text-sky-400' : 'text-zinc-500'} aria-hidden="true">{t.icon}</span>
                    {t.label}
                  </button>
                );
              })}
            </ScrollRow>
          </>
        )}
      </div>

      <div className="flex-1 min-w-0 overflow-y-auto">
        <div ref={contentRef} className="max-w-2xl mx-auto p-4 sm:p-6 space-y-5" data-app-page={scope === 'app' ? (finding ? 'find' : appPage) : undefined}>
          <div className="flex items-center justify-between gap-3">
            <div>
              <h3 className="text-base font-semibold">{scope === 'app' ? (finding ? 'Find a setting' : pageLabel) : TABS.find((t) => t.id === tab)?.label}</h3>
              <p className="text-[11px] text-zinc-500">
                {scope === 'app' ? 'Applies to every space and to this app' : `Only affects this space (${space.name})`}
              </p>
            </div>
            <button type="button" onClick={onClose} aria-label="Close settings" className="hidden md:block p-1.5 rounded-lg text-zinc-400 hover:text-zinc-100 hover:bg-zinc-800">
              <X size={18} />
            </button>
          </div>
          {finding && (
            <p id="settings-find-count" role="status" className="text-[11px] text-zinc-400" data-settings-find-count>
              {found === 0 ? `Nothing matches “${find.trim()}”. Try another word, or clear the field.` : found === null ? '' : `${found} ${found === 1 ? 'setting matches' : 'settings match'} “${find.trim()}”.`}
            </p>
          )}
          {scope === 'app' ? (
            <>
              <div hidden={!showGeneral} data-settings-page="general" data-setting-row>
                <PhoneAppCard phone={p.isPhone} />
              </div>
              <div hidden={!showGeneral} className={`${card} p-4 space-y-3`} data-all-spaces-spaces data-settings-page="general" data-setting-row>
                <p className="text-xs font-medium text-zinc-200">Spaces</p>
                <p className="text-[11px] text-zinc-500">Switch spaces on the left rail (Ctrl or ⌘ with 1–9). Archived spaces keep their bots and chats until you restore them.</p>
                <div className="flex flex-wrap gap-2">
                  <button type="button" onClick={p.onOpenArchived} className={rowBtn}>
                    <Archive size={13} /> Archived spaces
                  </button>
                </div>
              </div>
              {appBody}
            </>
          ) : (
            <>
              {tab === 'general' && <GeneralTab {...p} />}
              {tab === 'apps' && (
                <AppsAccounts
                  key={`${space.id}:${p.connectApp?.seq ?? 0}`}
                  spaceName={space.name}
                  spaceMode={space.approvals}
                  kwants={p.kwants}
                  section={p.appsSection?.section}
                  sectionSeq={p.appsSection?.seq}
                  focusClaudeSignIn={p.focusClaudeSignIn}
                  openAdvanced={p.toolNoteOpen}
                  initialSheet={p.connectApp ? { slug: p.connectApp.slug, name: p.connectApp.name, initial: p.connectApp.initial } : undefined}
                  onProfilesChanged={p.onProfilesChanged}
                  manageAccounts={(shared) => <AccountsTab {...p} {...shared} />}
                  advanced={p.connections}
                />
              )}
              {tab === 'bots' && <BotsTab {...tp} />}
              {tab === 'teams' && <TeamsTab {...p} />}
              {tab === 'docs' && <SpaceDocsTab key={space.id} spaceId={space.id} spaceName={space.name} isPhone={p.isPhone} />}
              {tab === 'skills' && <SkillsTab {...p} />}
              {tab === 'routines' && <RoutinesPanel kwants={p.kwants} channels={p.channels} focus={p.routineFocus} onOpenSettings={toApp} onOpenRunResult={p.onOpenRunResult} onOpenRunCard={p.onOpenRunCard} />}
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
        setNotice(`${err.names.join(', ') || 'Something it made'} has already been used, so this import can’t be undone in one step. Delete those bots yourself in Settings › This space › Bots.`);
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
              <button type="button" onClick={() => undo(confirm.record, true)} className="flex items-center gap-1.5 px-3 py-1.5 rounded-lg text-xs font-medium text-white bg-rose-600 hover:bg-rose-700">
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

const GeneralTab: React.FC<Props> = ({ space, spaces, kwants, teams, onSpaceSaved, onArchived, onApprovalsChanged }) => {
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
      {/* First: the one setting here that changes what the bots do (name and icon are set once). */}
      <DevModeSettings key={`dev-${space.id}`} spaceId={space.id} spaceName={space.name} approvals={space.approvals} teams={devTeamRefs(teams, kwants)} />

      <form onSubmit={save} {...editorProps(dirty)} className={`${card} p-4 space-y-4`}>
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
                aria-label={`Icon: ${pixelIconFor(i)?.name ?? i}`}
                onClick={() => setIcon(i)}
                className={`w-9 h-9 max-md:w-11 max-md:h-11 rounded-lg text-lg flex items-center justify-center border ${icon === i ? 'border-sky-400 bg-sky-500/10' : 'border-zinc-800 hover:border-zinc-600'}`}
              >
                <PickerGlyph icon={i} colour={spaceColor({ id: space.id, color })} />
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
            <span className="flex items-center gap-1 text-xs text-ok" role="status">
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
                className="flex items-center gap-1.5 px-3 py-1.5 rounded-lg text-xs font-medium text-white bg-rose-600 hover:bg-rose-700 disabled:opacity-50"
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

// Apps & accounts › Sign-ins and settings: each AI's sign-in, install and API keys. The order lives in the
// AI accounts list above (AccountPriority), so it isn't repeated here.
const AccountsTab: React.FC<Props & { engines: EnginesApi; accounts: ProfilesState }> = ({ focusClaudeSignIn, engines, accounts }) => {
  return (
    <div className="space-y-3">
      <EnginesPanel api={engines} accounts={accounts} focusClaudeSignIn={focusClaudeSignIn} hideRoutes />
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
