// 0.7.7 T2 preview sheet and D4 suggestions: one sheet, one Create, one Undo. Desktop: a centred
// 576 px dialog. Phone: a bottom sheet. Design: next-077/design/design.md §T2, §D1–D7.
import React, { useCallback, useEffect, useId, useRef, useState } from 'react';
import {
  type LucideIcon,
  AlertTriangle,
  Check,
  CheckCircle2,
  ChevronDown,
  Clock,
  Code2,
  Crown,
  FileText,
  FolderOpen,
  Handshake,
  Inbox,
  Info,
  Loader2,
  Lock,
  Pencil,
  Plug,
  Plus,
  RefreshCw,
  ScanSearch,
  ShieldCheck,
  Sparkles,
  Undo2,
  WifiOff,
  CircleAlert,
  X,
} from 'lucide-react';
import { ModernAvatar } from './ModernAvatar';
import { useEscapeLayer } from '../escape';
import { forgeHost, looseApi } from '../looseEnds';
import { useMedia } from '../useMedia';
import { PHONE_QUERY } from '../layout';
import {
  MAX_BOTS,
  MIN_BOTS,
  ROLE_OPTIONS,
  TeamsError,
  carryEdits,
  cleanHandle,
  fieldKey,
  connectForgeFor,
  serverErrorText,
  needError,
  initialEdits,
  nameError,
  newBotCount,
  botNameByKey,
  cantRead,
  folderInUse,
  partialBody,
  partialHead,
  plural,
  readLine,
  resolveClashes,
  roleOf,
  safeFolder,
  slugHandle,
  sizeNote,
  sourceName,
  summaryLine,
  swapRole,
  teamsApi,
  tickedCount,
  type Clash,
  type CreatedTeam,
  type CreateRequest,
  type RowSend,
  type SuggestRequest,
  newIdempotencyKey,
  type RowEdit,
  type SuggestedBot,
  type Suggestion,
  type TeamTemplate,
} from '../teams/teamTemplates';

export const TEMPLATE_ICONS: Record<string, LucideIcon> = {
  inbox: Inbox,
  handshake: Handshake,
  'code-xml': Code2,
};
export const TemplateGlyph: React.FC<{ icon: string; size?: number; box?: string; className?: string }> = ({ icon, size = 17, box = 'w-8 h-8 rounded-[9px]', className = '' }) => {
  const Icon = TEMPLATE_ICONS[icon] ?? ScanSearch;
  return (
    <span aria-hidden="true" className={`${box} shrink-0 grid place-items-center bg-sky-400/15 text-sky-400 ring-1 ring-inset ring-sky-300/20 ${className}`}>
      <Icon size={size} aria-hidden="true" />
    </span>
  );
};

const FOCUS = 'focus-visible:outline focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-sky-400';
const SEC = 'py-4 border-t border-zinc-800 first:border-t-0';
const H = 'mb-2.5 flex items-baseline gap-2 text-[11px] leading-4 font-semibold uppercase tracking-[.08em] text-zinc-400';
const CNT = 'ml-auto normal-case tracking-normal font-normal text-[11px]';
const CODE = 'px-1 py-px rounded bg-black/30 border border-white/5 font-mono text-[11.5px] whitespace-nowrap text-zinc-200';

export const LeadsBadge = () => (
  <span className="inline-flex items-center gap-[3px] h-[18px] px-1.5 rounded-[5px] text-[10.5px] font-semibold text-amber-300 bg-amber-300/10">
    <Crown size={10} aria-hidden="true" />
    Leads
  </span>
);

const Banner: React.FC<{ tone: 'bad' | 'warn' | 'info'; head: string; children?: React.ReactNode; icon?: 'offline' | 'alert' }> = ({ tone, head, children, icon = 'offline' }) => {
  const c =
    tone === 'bad'
      ? 'border-rose-400/30 bg-rose-500/10 text-rose-100'
      : tone === 'warn'
        ? 'border-amber-300/30 bg-amber-300/10 text-amber-50'
        : 'border-sky-300/25 bg-sky-400/10 text-zinc-100';
  const Icon = tone === 'bad' ? AlertTriangle : tone === 'warn' ? (icon === 'alert' ? CircleAlert : WifiOff) : Info;
  const size = tone === 'warn' && icon === 'alert' ? 14 : 15;
  return (
    <div role={tone === 'bad' ? 'alert' : 'status'} className={`mt-3 flex gap-2.5 p-3 rounded-[10px] border text-[12.5px] leading-[18px] ${c}`}>
      <Icon size={size} className={`${size === 14 ? 'mt-0.5' : 'mt-px'} shrink-0`} aria-hidden="true" />
      <p>
        <strong className="font-semibold">{head}</strong> {children}
      </p>
    </div>
  );
};

/** "a", "a and b", "a, b and c": the server's own phrases, joined, never rewritten. */
export const listWords = (xs: string[]) => (xs.length <= 1 ? xs.join('') : `${xs.slice(0, -1).join(', ')} and ${xs[xs.length - 1]}`);

const Note: React.FC<{ children: React.ReactNode; id?: string }> = ({ children, id }) => (
  <p id={id} className="mt-1 flex items-start gap-1.5 text-xs text-zinc-300">
    <Info size={12} className="mt-0.5 shrink-0 text-sky-300" aria-hidden="true" />
    <span>{children}</span>
  </p>
);

const Skeleton: React.FC<{ rows?: number; tick?: boolean }> = ({ rows = 3, tick = false }) => (
  <div aria-hidden="true" className="py-4 space-y-4">
    {Array.from({ length: rows }, (_, i) => (
      <div key={i} className="flex items-center gap-3">
        {tick && <span className="w-5 h-5 rounded-md bg-white/[.06]" />}
        <span className="w-9 h-9 rounded-lg bg-white/[.06]" />
        <span className="flex-1 space-y-1.5">
          <span className="block h-3 w-1/3 rounded-md bg-white/[.06]" />
          <span className="block h-3 w-2/3 rounded-md bg-white/[.06]" />
        </span>
      </div>
    ))}
  </div>
);

/** A D reason: text with paths as inline code. Every part is plain text for screen readers. */
export const Reason: React.FC<{ parts: SuggestedBot['reason'] }> = ({ parts }) => (
  <>
    {parts.map((p, i) =>
      'path' in p ? (
        <code key={i} className={CODE}>
          {p.path}
        </code>
      ) : (
        <React.Fragment key={i}>{p.text}</React.Fragment>
      ),
    )}
  </>
);

// ---------------------------------------------------------------------------------------------------

export type SheetMode =
  | { kind: 'template'; templateId: string }
  /** D: from New team (no team), Manage team (team), or the Coding template with a folder or repo. */
  | {
      kind: 'suggest';
      workspace_path?: string | null;
      repo_url?: string | null;
      handleSeed?: string;
      team?: { id: string; name: string };
      viaCoding?: boolean;
      /** A team from your own words: the person's words, sent as `{ need }`. Try again sends them again. */
      need?: string;
      /** The answer the need box already has, so the sheet opens on it without asking twice. */
      initial?: Suggestion;
    };

export interface TeamPreviewSheetProps {
  open: boolean;
  space: string;
  mode: SheetMode;
  /** The template list (cached or fresh); null while it loads. */
  templates: TeamTemplate[] | null;
  templatesError?: boolean;
  onRetryTemplates?: () => void;
  takenNames: string[];
  takenHandles: string[];
  /** Bots in this space, for D6's "Add Frontend (already in this space)" avatar. */
  spaceBots?: { id: string; name: string; role?: string }[];
  onClose: () => void;
  /** `t` is null for a team from your own words, which has no template. */
  onCreated: (r: CreatedTeam, t: TeamTemplate | null, handle: string) => void;
  /** No ready account: "Add an AI account" opens Accounts. */
  onOpenAccounts?: () => void;
  /** D7: "Use the Coding roster" switches the sheet to the plain Coding template. */
  onUseCoding?: () => void;
  onPickFolder?: () => void;
  onConnectForge?: (repoUrl: string, forge: 'GitHub' | 'GitLab') => void;
}

type DPhase = 'reading' | 'ready' | 'cantread' | 'offline' | 'error';

export const TeamPreviewSheet: React.FC<TeamPreviewSheetProps> = (p) => {
  if (!p.open) return null;
  return <Sheet {...p} />;
};

const Sheet: React.FC<TeamPreviewSheetProps> = ({
  space,
  mode,
  templates,
  templatesError,
  onRetryTemplates,
  takenNames,
  takenHandles,
  spaceBots = [],
  onClose,
  onCreated,
  onUseCoding,
  onPickFolder,
  onConnectForge,
  onOpenAccounts,
}) => {
  const phone = useMedia(PHONE_QUERY);
  const rootRef = useRef<HTMLDivElement>(null);
  const createRef = useRef<HTMLButtonElement>(null);
  const firstRenameRef = useRef<HTMLButtonElement | null>(null);
  const titleId = useId();
  const subId = useId();
  const isD = mode.kind === 'suggest';
  const manage = isD && !!mode.team;
  /** A team from your own words (brief "Flow"): no facts, read line, partial or stack line. */
  const isN = mode.kind === 'suggest' && !!mode.need && !mode.team;
  const need = mode.kind === 'suggest' ? mode.need ?? '' : '';
  const templateId = mode.kind === 'template' ? mode.templateId : 'coding';
  const t = templates?.find((x) => x.id === templateId) ?? null;

  const [online, setOnline] = useState(() => (typeof navigator === 'undefined' ? true : navigator.onLine !== false));
  useEffect(() => {
    const on = () => setOnline(true);
    const off = () => setOnline(false);
    window.addEventListener('online', on);
    window.addEventListener('offline', off);
    return () => {
      window.removeEventListener('online', on);
      window.removeEventListener('offline', off);
    };
  }, []);

  // ---- T state
  const [names, setNames] = useState<Record<string, string>>({});
  const [notes, setNotes] = useState<Record<string, string>>({});
  const [handle, setHandle] = useState('');
  const [handleNote, setHandleNote] = useState<string | null>(null);
  const [renaming, setRenaming] = useState<string | null>(null);
  const [draft, setDraft] = useState('');
  const [reading, setReading] = useState<Set<number>>(new Set());
  const [phase, setPhase] = useState<'idle' | 'creating' | 'failed'>('idle');
  /** U-5: the server's `error` for the last failed create; null keeps "The server stopped…". */
  const [createError, setCreateError] = useState<string | null>(null);
  const [live, setLive] = useState('');
  const seeded = useRef(false);
  /** Rows (and '@handle') the person changed, which a late preview answer must not overwrite. */
  const touched = useRef(new Set<string>());
  // api.md §0.3: one idempotency key per sheet open. A retry, a clash fix or a re-read after 410 reuses
  // it; the server only replays a create that went through, so nothing is made twice.
  const idemKey = useRef<string>(newIdempotencyKey());

  // ---- D state
  const [dPhase, setDPhase] = useState<DPhase>('reading');
  /** U-5: the server's `error` when Suggest was refused (bad_path, folder_in_use, …). */
  const [readError, setReadError] = useState<string | null>(null);
  /** N-5: the last read or create was refused with `folder_in_use`, which a retry can't change. */
  const [inUse, setInUse] = useState(false);
  /** U-3: the 422's `forge`, and the GitLab hosts already connected, for which Connect to offer. */
  const [cantForge, setCantForge] = useState<unknown>(null);
  const [gitlabHosts, setGitlabHosts] = useState<string[]>([]);
  const [sug, setSug] = useState<Suggestion | null>(null);
  const [edits, setEdits] = useState<Record<string, RowEdit>>({});
  const [elapsed, setElapsed] = useState(0);
  const [expiredNote, setExpiredNote] = useState(false);
  const readAbort = useRef<AbortController | null>(null);
  const editsRef = useRef(edits);
  editsRef.current = edits;
  const sugRef = useRef(sug);
  sugRef.current = sug;
  /** Need mode: why the last ask failed, in the person's words. */
  const [needErr, setNeedErr] = useState<ReturnType<typeof needError> | null>(null);
  /** Need mode: the answer the box handed over is used once, for the first read. */
  const usedInitial = useRef(false);
  /** Need mode: Try again keeps the last team here, so Cancel or Escape while asking goes back to it. */
  const [again, setAgain] = useState(false);
  const [openInstr, setOpenInstr] = useState<Set<string>>(new Set());

  const close = useCallback(() => {
    if (phase === 'creating') return;
    readAbort.current?.abort();
    if (isN && again && dPhase === 'reading' && sugRef.current) {
      // Stop the second ask and keep the team that was here.
      setAgain(false);
      setDPhase('ready');
      setLive('Stopped. The last suggestion is still here.');
      return;
    }
    onClose();
  }, [phase, onClose, isN, again, dPhase]);
  useEscapeLayer(rootRef, true, close, { modal: true });

  // Seed names and handle once the template is here (T9 clash rules).
  useEffect(() => {
    if (isD || !t || seeded.current) return;
    seeded.current = true;
    const c = resolveClashes(t.bots, t.handle, takenNames, takenHandles);
    setNames(c.names);
    setNotes(c.notes);
    setHandle(c.handle);
    setHandleNote(c.handleNote);
    // api.md §0.10: the server's own free names, when it answers; ours stand otherwise. Applied only to
    // rows the person hasn't renamed yet.
    const tid = t.id;
    teamsApi.preview(space, tid).then(
      (p) => {
        if (!p || p.template_id !== tid) return;
        const fresh = (k: string) => !touched.current.has(k);
        setNames((n) => {
          const out = { ...n };
          for (const b of t.bots) if (fresh(b.key) && p.names?.[b.key]) out[b.key] = p.names[b.key];
          return out;
        });
        setNotes(() => {
          const out: Record<string, string> = {};
          for (const b of t.bots) {
            const v = p.names?.[b.key];
            if (fresh(b.key) && v && v !== b.name) out[b.key] = `${b.name} is taken in this space, so this one is ${v}.`;
          }
          return out;
        });
        if (fresh('@handle') && p.handle) {
          setHandle(p.handle);
          setHandleNote(p.handle !== t.handle ? `@${t.handle} is taken here, so this team is @${p.handle}.` : null);
        }
      },
      () => {},
    );
  }, [isD, t, takenNames, takenHandles, space]);

  // D: read the project.
  const read = useCallback(
    async (keep = false) => {
      if (mode.kind !== 'suggest') return;
      readAbort.current?.abort();
      const ac = new AbortController();
      readAbort.current = ac;
      setDPhase('reading');
      setInUse(false);
      setElapsed(0);
      setNeedErr(null);
      setLive(isN ? 'Suggesting a team' : 'Reading the project');
      const first = isN && !usedInitial.current && mode.initial ? mode.initial : null;
      usedInitial.current = true;
      if (!first && !navigator.onLine) {
        setDPhase('offline');
        setLive(isN ? 'You’re offline. Your words are kept.' : 'You’re offline. Nothing was read.');
        return;
      }
      try {
        // api.md §4: exactly one source. Manage team reads the team's own folder or repo. A team from
        // your own words sends only `{ need }` (brief "Server contract").
        const req: SuggestRequest = isN
          ? { space, need: need.trim() }
          : mode.team
            ? { space, team_id: mode.team.id }
            : mode.workspace_path
              ? { space, workspace_path: mode.workspace_path }
              : { space, repo_url: mode.repo_url ?? '' };
        const s = first ?? (await teamsApi.suggest(req, isN ? 'need' : mode.workspace_path || mode.repo_url || (mode.team ? `@${mode.team.name}` : '')));
        if (ac.signal.aborted) return;
        const fresh = initialEdits(s, takenNames);
        const old = sugRef.current;
        setEdits(keep && old ? carryEdits(editsRef.current, fresh, old, s) : fresh);
        setSug(s);
        if (!keep) {
          const seed = cleanHandle(mode.handleSeed || '') || cleanHandle(s.handle || '') || cleanHandle(sourceName(mode.workspace_path || mode.repo_url)) || 'team';
          const c = resolveClashes([], seed, [], takenHandles);
          setHandle(c.handle);
          setHandleNote(c.handleNote);
        }
        setDPhase('ready');
        setAgain(false);
        setLive(
          isN
            ? `Suggested ${plural(s.bots.length, 'bot')}. Nothing is made until you create it.`
            : s.bots.length + s.also_possible.length === 0
            ? 'Nothing new to add.'
            : `Suggested ${plural(s.bots.length, 'bot')}${s.also_possible.length ? `, ${s.also_possible.length} also possible` : ''}`,
        );
      } catch (e) {
        if (ac.signal.aborted) return;
        if (isN) {
          const ne = needError(e);
          setNeedErr(ne);
          setAgain(false);
          setDPhase(e instanceof TeamsError && e.status === 0 ? 'offline' : 'error');
          setLive(`${ne.head}${ne.reason ? ` ${ne.reason}` : ''}`);
          return;
        }
        // The live region still says "Reading the project" otherwise.
        if (cantRead(e)) {
          setCantForge(e instanceof TeamsError ? e.extra?.forge ?? null : null);
          looseApi.sources().then(
            (r) =>
              !ac.signal.aborted &&
              setGitlabHosts(
                (r.sources ?? [])
                  .filter((x) => x.kind === 'forge' && x.config?.forge === 'gitlab' && typeof x.config?.url === 'string')
                  .map((x) => forgeHost(x.config!.url as string))
                  .filter(Boolean),
              ),
            () => undefined,
          );
          setDPhase('cantread');
          setLive('Can’t read this project yet. Nothing was read and nothing was created.');
        } else if (e instanceof TeamsError && e.status === 0) {
          setDPhase('offline');
          setLive('You’re offline. Nothing was read.');
        } else {
          const said = serverErrorText(e);
          setReadError(said);
          setInUse(folderInUse(e));
          setDPhase('error');
          setLive(`Couldn’t read the project. ${said ? `${said} ` : ''}Nothing was created.`);
        }
      }
    },
    // eslint-disable-next-line react-hooks/exhaustive-deps
    [space, mode.kind === 'suggest' ? mode.workspace_path : null, mode.kind === 'suggest' ? mode.repo_url : null, need],
  );
  useEffect(() => {
    if (isD) void read();
    return () => readAbort.current?.abort();
  }, [isD, read]);

  // D2's meter fills against the 10 s limit, not the file count.
  useEffect(() => {
    if (dPhase !== 'reading' || !isD) return;
    const start = Date.now();
    const id = window.setInterval(() => setElapsed(Math.min(10_000, Date.now() - start)), 250);
    return () => window.clearInterval(id);
  }, [dPhase, isD]);

  // Focus on open: the first Rename on a clash, otherwise Create.
  const ready = isD ? dPhase === 'ready' : !!t;
  const focused = useRef(false);
  useEffect(() => {
    if (!ready || focused.current) return;
    focused.current = true;
    requestAnimationFrame(() => {
      const clash = !isD && Object.keys(notes).length > 0;
      (clash && firstRenameRef.current ? firstRenameRef.current : createRef.current)?.focus({ preventScroll: true });
    });
  }, [ready, isD, notes]);

  // ---- derived
  const tBots = t?.bots ?? [];
  const dRows = sug ? [...sug.bots, ...sug.also_possible] : [];
  const rowName = (key: string) => (isD ? edits[key]?.name : names[key]) ?? '';
  const otherNames = (key: string) => (isD ? dRows.filter((b) => b.key !== key && edits[b.key]?.ticked && !edits[b.key]?.useExisting) : tBots.filter((b) => b.key !== key)).map((b) => rowName(b.key));
  const nameErr = (key: string, v = rowName(key)) => {
    if (isD && edits[key]?.useExisting) return null;
    return nameError(v, takenNames, otherNames(key));
  };
  const ticked = isD ? tickedCount(edits) : tBots.length;
  const anyNameBad = isD ? dRows.some((b) => edits[b.key]?.ticked && nameErr(b.key)) : tBots.some((b) => nameErr(b.key));
  const handleBad = !manage && !cleanHandle(handle);
  const minBots = manage ? 1 : MIN_BOTS;
  const dSizeBad = isD && (ticked < minBots || ticked > MAX_BOTS);
  const creating = phase === 'creating';
  const canCreate = ready && online && !creating && !anyNameBad && !handleBad && !dSizeBad && renaming === null;
  const label = isN ? 'suggested' : t?.label ?? 'Coding';
  const busy = (!isD && !t && !templatesError) || (isD && dPhase === 'reading');

  // ---- rename
  const startRename = (key: string) => {
    setRenaming(key);
    setDraft(rowName(key));
  };
  const commitRename = (key: string) => {
    if (nameErr(key, draft.trim())) return;
    const v = draft.trim();
    if (isD) setEdits((e) => ({ ...e, [key]: { ...e[key], name: v, renamed: true } }));
    else {
      touched.current.add(key);
      setNames((n) => ({ ...n, [key]: v }));
      setNotes((n) => {
        const { [key]: _drop, ...rest } = n;
        return rest;
      });
    }
    setRenaming(null);
  };

  const applyClashes = (clashes: Clash[], h: string) => {
    for (const c of clashes) {
      if (c.field === 'handle') {
        setHandleNote(`@${c.wanted ?? h} is taken here, so this team is @${c.suggestion}.`);
        setHandle(c.suggestion);
        continue;
      }
      const key = fieldKey(c.field);
      if (!key) continue;
      const was = c.wanted ?? rowName(key);
      if (isD) setEdits((x) => (x[key] ? { ...x, [key]: { ...x[key], name: c.suggestion } } : x));
      else setNames((n) => ({ ...n, [key]: c.suggestion }));
      setNotes((n) => ({ ...n, [key]: `${was} is taken in this space, so this one is ${c.suggestion}.` }));
    }
  };

  // ---- create
  const create = async () => {
    if (!canCreate || (!t && !isD)) return;
    setPhase('creating');
    setLive(manage ? `Adding ${plural(newBotCount(edits), 'bot')} to @${mode.kind === 'suggest' ? mode.team?.name : ''}` : `Creating the ${label} team`);
    const h = cleanHandle(handle);
    let body: CreateRequest;
    if (isD && sug) {
      // api.md §3b: only on, name, preset and use_existing per row.
      const rows: Record<string, RowSend> = {};
      for (const b of dRows) {
        const e = edits[b.key];
        const row: RowSend = { on: !!e?.ticked };
        if (e?.ticked) {
          if (e.useExisting) row.use_existing = e.useExisting;
          else {
            row.name = e.name;
            if (e.preset !== b.preset) row.preset = e.preset;
          }
        }
        rows[b.key] = row;
      }
      body = {
        kind: 'suggestion',
        space,
        suggestion_id: sug.suggestion_id,
        rows,
        ...(manage && mode.kind === 'suggest' && mode.team ? { team_id: mode.team.id } : { handle: h }),
      };
    } else {
      const ns: Record<string, string> = {};
      for (const b of tBots) if (names[b.key] && names[b.key] !== b.name) ns[b.key] = names[b.key];
      if (!t) return;
      body = { kind: 'template', space, template_id: t.id, handle: h, names: ns };
    }
    try {
      const r = await teamsApi.create(body, idemKey.current);
      setLive(`Created @${h}. Undo is available.`);
      onCreated(r, t, h);
    } catch (e) {
      if (e instanceof TeamsError && e.status === 410 && isD) {
        // A17: read again, keep the edits on rows that come back.
        setPhase('idle');
        setExpiredNote(true);
        void read(true);
        return;
      }
      if (e instanceof TeamsError && e.status === 409 && e.clashes.length) {
        // A10 + api.md §0.4: names or the handle were taken meanwhile. Take every suggestion and say so.
        setPhase('idle');
        applyClashes(e.clashes, h);
        setLive(e.clashes.length > 1 ? `${e.clashes.length} names were taken. Check the new names and create again.` : 'A name was taken. Check the new name and create again.');
        return;
      }
      const said = serverErrorText(e);
      setCreateError(said);
      setInUse(folderInUse(e));
      setPhase('failed');
      setLive(`Couldn’t create the team. ${said ? `${said} ` : ''}Nothing was created.`);
    }
  };

  // ---- phone drag-to-close (past 30% of the sheet's height)
  const drag = useRef<{ y: number; h: number } | null>(null);
  const [dy, setDy] = useState(0);
  const onGrabDown = (e: React.PointerEvent) => {
    drag.current = { y: e.clientY, h: rootRef.current?.offsetHeight ?? 600 };
    (e.target as HTMLElement).setPointerCapture?.(e.pointerId);
  };
  const onGrabMove = (e: React.PointerEvent) => {
    if (drag.current) setDy(Math.max(0, e.clientY - drag.current.y));
  };
  const onGrabUp = () => {
    if (drag.current && dy > drag.current.h * 0.3) close();
    drag.current = null;
    setDy(0);
  };

  // ---- header
  const src = isD && mode.kind === 'suggest' ? (mode.workspace_path || mode.repo_url || '') : '';
  let title = `${label} team`;
  let sub: React.ReactNode = t ? summaryLine(t) : '';
  let glyph = t?.icon ?? 'code-xml';
  if (isN) {
    title = 'Suggested team';
    sub = 'Drafted from your words. Nothing is made until you create it.';
    glyph = 'sparkles';
  } else if (isD && mode.kind === 'suggest') {
    if (manage) {
      title = `Suggested for @${mode.team!.name}`;
      sub = (
        <>
          Only roles @{mode.team!.name} doesn’t have yet · from <code className={CODE}>{src}</code>
        </>
      );
      glyph = 'scan';
    } else if (mode.viaCoding) {
      title = 'Coding team';
      sub = (
        <>
          Members suggested from <code className={CODE}>{src}</code>
        </>
      );
    } else {
      title = `Suggested team for ${sourceName(src) || 'this project'}`;
      sub = (
        <>
          From <code className={CODE}>{src}</code>
        </>
      );
      glyph = 'scan';
    }
  }

  const iconBtn = `${phone ? 'w-11 h-11' : 'w-8 h-8'} shrink-0 grid place-items-center rounded-lg text-zinc-400 hover:text-zinc-100 hover:bg-white/5 ${FOCUS}`;

  // ---- sections
  const renameField = (key: string, title: string) => {
    const err = nameErr(key, draft.trim());
    const errId = `${titleId}-rn-${key}`;
    return (
      <div className="min-w-0">
        <input
          autoFocus
          aria-label={`Name for ${title}`}
          aria-invalid={!!err}
          aria-describedby={errId}
          value={draft}
          onFocus={(e) => e.currentTarget.select()}
          onChange={(e) => setDraft(e.target.value)}
          onBlur={() => (err ? undefined : commitRename(key))}
          onKeyDown={(e) => {
            if (e.key === 'Enter') {
              e.preventDefault();
              commitRename(key);
            } else if (e.key === 'Escape') {
              e.preventDefault();
              e.stopPropagation();
              setRenaming(null);
            }
          }}
          className={`h-8 w-full max-w-[240px] px-2 rounded-lg bg-zinc-950/60 border text-sm font-semibold text-zinc-50 outline-none ${err ? 'border-rose-400' : 'border-sky-400'}`}
        />
        <p id={errId} aria-live="polite" className={`mt-1 text-xs ${err ? 'text-rose-300' : 'text-zinc-400'}`}>
          {err ?? 'Enter to keep · Esc to cancel'}
        </p>
      </div>
    );
  };

  const whoT = t && (
    <section className={SEC} aria-labelledby={`${titleId}-who`}>
      <h4 className={H} id={`${titleId}-who`}>
        Who’s on it<span className={`${CNT} text-zinc-500`}>{names[tBots.find((b) => b.lead)?.key ?? ''] ?? ''} leads</span>
      </h4>
      <ul>
        {tBots.map((b) => (
          <li key={b.key} data-bot={b.key} className={`flex items-start gap-3 p-2 -mx-2 rounded-[10px] ${renaming === b.key ? 'bg-white/[.03]' : ''}`}>
            <ModernAvatar role={b.avatar_sprite} name={names[b.key]} size={36} />
            <div className="flex-1 min-w-0">
              {renaming === b.key ? (
                renameField(b.key, b.title)
              ) : (
                <div className="flex flex-wrap items-center gap-x-2 gap-y-0.5">
                  <span className="text-sm leading-5 font-semibold text-zinc-50">{names[b.key]}</span>
                  {b.lead && <LeadsBadge />}
                  <span className="text-xs text-zinc-400">{b.title}</span>
                </div>
              )}
              <p className="text-[13px] leading-[18px] text-zinc-300">{b.line}</p>
              {notes[b.key] && renaming !== b.key && <Note>{notes[b.key]}</Note>}
            </div>
            {renaming !== b.key && (
              <button
                type="button"
                ref={b.key === (tBots.find((x) => notes[x.key]) ?? tBots[0])?.key ? (el) => (firstRenameRef.current = el) : undefined}
                aria-label={`Rename ${names[b.key]}`}
                onClick={() => startRename(b.key)}
                className={iconBtn}
              >
                <Pencil size={14} aria-hidden="true" />
              </button>
            )}
          </li>
        ))}
      </ul>
    </section>
  );

  const handleSec = !manage && (
    <section className={SEC} aria-labelledby={`${titleId}-handle`}>
      <h4 className={H} id={`${titleId}-handle`}>
        Team handle
      </h4>
      <div className={`flex ${phone || handleNote ? 'flex-col items-stretch' : 'items-center'} ${handleNote && !phone ? 'gap-0' : 'gap-2'}`}>
        <label className={`flex items-center ${phone ? 'h-11' : 'h-9 w-[200px]'} rounded-lg border ${handleBad ? 'border-rose-400' : 'border-zinc-700'} bg-zinc-950/60 px-2 focus-within:border-sky-400`}>
          <span className="text-zinc-400 text-sm" aria-hidden="true">
            @
          </span>
          <input
            aria-label="Team handle"
            aria-describedby={`${titleId}-hh`}
            value={handle}
            onChange={(e) => {
              touched.current.add('@handle');
              setHandle(slugHandle(e.target.value));
              setHandleNote(null);
            }}
            className="flex-1 min-w-0 w-full bg-transparent outline-none text-sm text-zinc-50 pl-0.5"
          />
        </label>
        {handleNote ? (
          <span id={`${titleId}-hh`} className={phone ? '' : 'mt-0.5'}>
            <Note>{handleNote}</Note>
          </span>
        ) : (
          <span id={`${titleId}-hh`} className={`text-xs ${handleBad ? 'text-rose-300' : 'text-zinc-400'}`}>
            {handleBad ? 'Give the team a handle.' : 'How you and your bots mention the team.'}
          </span>
        )}
      </div>
    </section>
  );

  const needsSec = t && !isD && (
    <section className={SEC} aria-labelledby={`${titleId}-needs`}>
      <h4 className={H} id={`${titleId}-needs`}>
        What it needs
      </h4>
      <ul className="space-y-2">
        {t.needs.map((n) => (
          <li key={n.id} className="flex items-start gap-2.5">
            {n.opens === 'composer_prompt' ? <Plus size={14} className="mt-0.5 text-sky-300 shrink-0" aria-hidden="true" /> : <Plug size={14} className="mt-0.5 text-sky-300 shrink-0" aria-hidden="true" />}
            <div className="min-w-0">
              <p className="text-[13px] leading-[18px]">
                <span className="font-semibold text-zinc-50">{n.label}</span>
                {!n.required && <span className="ml-2 text-[11px] text-zinc-400">Optional</span>}
              </p>
              <p className="text-xs text-zinc-400">{n.why}</p>
            </div>
          </li>
        ))}
      </ul>
      {t.id === 'sales' && <p className="mt-2 text-xs text-zinc-400">Uses the built-in browser to look things up. Nothing to connect first.</p>}
    </section>
  );

  const neverSec = t && !manage && (
    <section className={SEC} aria-labelledby={`${titleId}-never`}>
      <h4 className={H} id={`${titleId}-never`}>
        Never without you
      </h4>
      <ul className="space-y-1.5">
        {t.never.map((l, i) => (
          <li key={i} className="flex items-start gap-2 text-[13px] leading-[18px] text-zinc-200">
            <ShieldCheck size={14} className="mt-0.5 shrink-0" style={{ color: 'var(--kw-ok)' }} aria-hidden="true" />
            {l}
          </li>
        ))}
      </ul>
      <p className="mt-2 text-[11.5px] text-zinc-400">It gets no new tools and your approvals stay as they are.</p>
    </section>
  );

  const comesSec = t && !isD && (
    <section className={SEC} aria-labelledby={`${titleId}-comes`}>
      <h4 className={H} id={`${titleId}-comes`}>
        Comes with
      </h4>
      <ul className="space-y-3">
        {t.skills.map((s, i) => {
          const open = reading.has(i);
          return (
            <li key={s.name}>
              <div className="flex items-start gap-2.5">
                <FileText size={14} className="mt-0.5 shrink-0 text-zinc-400" aria-hidden="true" />
                <div className="flex-1 min-w-0">
                  <p className="flex flex-wrap items-center gap-2 text-[13px] leading-[18px]">
                    <span className="font-semibold text-zinc-50">{s.name}</span>
                    <span className="inline-flex items-center h-[18px] px-1.5 rounded-[5px] text-[10.5px] font-medium text-sky-200 bg-sky-400/15">From the {t.label} template</span>
                  </p>
                  <p className="text-xs text-zinc-400">
                    {botNameByKey(t, names, s.owner, s.owner_name)} · on · {s.description}
                  </p>
                </div>
                <button
                  type="button"
                  aria-expanded={open}
                  aria-controls={`${titleId}-sk-${i}`}
                  aria-label={`${open ? 'Hide' : 'Read'} ${s.name}`}
                  onClick={() => setReading((r) => (r.has(i) ? new Set([...r].filter((x) => x !== i)) : new Set([...r, i])))}
                  className={`${phone ? 'min-h-11 px-3' : 'min-h-7 px-1.5'} rounded-md text-[12.5px] font-medium text-sky-300 hover:bg-white/5 ${FOCUS}`}
                >
                  {open ? 'Hide' : 'Read'}
                </button>
              </div>
              {open && (
                <div id={`${titleId}-sk-${i}`} className="relative mt-2 ml-6">
                  <pre className="max-h-[180px] overflow-auto whitespace-pre-wrap p-3 rounded-lg bg-black/30 border border-white/5 text-xs leading-[18px] text-zinc-300 font-sans">{s.body}</pre>
                </div>
              )}
            </li>
          );
        })}
        {t.routine && (
          <li className="flex items-start gap-2.5">
            <Clock size={14} className="mt-0.5 shrink-0 text-zinc-400" aria-hidden="true" />
            <div className="min-w-0">
              <p className="flex flex-wrap items-center gap-2 text-[13px] leading-[18px]">
                <span className="font-semibold text-zinc-50">{t.routine.name}</span>
                <span className="inline-flex items-center h-[18px] px-1.5 rounded-[5px] text-[10.5px] font-medium text-zinc-300 bg-white/[.06]">Off</span>
              </p>
              <p className="text-xs text-zinc-400">
                {t.routine.when} · {botNameByKey(t, names, t.routine.bot)}. Turn it on in Routines when you want it.
              </p>
            </div>
          </li>
        )}
      </ul>
    </section>
  );

  // ---- D sections
  const atMax = ticked >= MAX_BOTS;
  const sgRow = (b: SuggestedBot, also: boolean) => {
    const e = edits[b.key];
    if (!e) return null;
    const role = roleOf(e.preset);
    const sprite = e.preset === b.preset ? b.avatar_sprite : role?.sprite ?? b.avatar_sprite;
    const titleText = b.lead ? b.title : e.preset === b.preset ? b.title : role?.title ?? b.title;
    const off = !e.ticked;
    const existing = b.existing_kwant_id ? spaceBots.find((x) => x.id === b.existing_kwant_id) : null;
    const existingName = existing?.name ?? b.existing_name ?? b.name;
    // Code suggestions own folders, so only safe folder names show. A team from words owns plain jobs.
    const owns = isN ? b.owns.filter((o) => o.trim()) : b.owns.filter(safeFolder);
    const instrOpen = openInstr.has(b.key);
    const instrId = `${titleId}-in-${b.key}`;
    const disabledTick = !b.lead && off && atMax;
    const choiceBlock = (
      <>
            {b.existing_kwant_id && e.ticked && !also && (
              <Choice
                phone={phone}
                existingName={existingName}
                existingRole={existing?.role ?? b.existing_avatar ?? b.avatar_sprite}
                value={e.useExisting ? 'existing' : 'new'}
                onChange={(v) =>
                  setEdits((x) => {
                    const cur = x[b.key];
                    if (v === 'existing') return { ...x, [b.key]: { ...cur, useExisting: b.existing_kwant_id ?? null, name: existingName } };
                    const taken = [...takenNames, ...Object.entries(x).filter(([k, r]) => k !== b.key && r.ticked && !r.useExisting).map(([, r]) => r.name)];
                    return { ...x, [b.key]: { ...cur, useExisting: null, name: nameError(cur.name, taken) ? swapRole({ ...cur, renamed: false }, cur.preset, taken).name : cur.name } };
                  })
                }
              />
            )}
            {b.existing_leads_team && e.ticked && e.useExisting && <Note>{existingName} leads @{b.existing_leads_team}. Here it joins as a member.</Note>}
      </>
    );
    return (
      <li key={b.key} data-row={b.key} className={`flex items-start ${phone ? 'flex-wrap gap-x-3 gap-y-0' : 'gap-3'} py-3 border-t border-white/5 first:border-t-0 ${renaming === b.key ? 'bg-white/[.03] -mx-2 px-2 rounded-[10px]' : ''}`}>
        {b.lead ? (
          <span role="checkbox" aria-checked="true" aria-disabled="true" aria-label={`${e.name} leads and stays in the team`} className={`${phone ? 'w-6 h-6' : 'w-5 h-5'} mt-2 shrink-0 grid place-items-center rounded-md bg-[#3f3868] text-white/80`}>
            <Check size={12} strokeWidth={3} aria-hidden="true" />
          </span>
        ) : (
          <button
            type="button"
            role="checkbox"
            aria-checked={e.ticked}
            aria-label={`Add ${e.useExisting ? existingName : e.name}`}
            disabled={disabledTick}
            onClick={() => setEdits((x) => ({ ...x, [b.key]: { ...x[b.key], ticked: !x[b.key].ticked } }))}
            className={`${phone ? 'w-11 h-11 -m-2.5' : 'w-5 h-5 mt-2'} shrink-0 grid place-items-center rounded-md disabled:opacity-40 ${FOCUS}`}
          >
            <span className={`${phone ? 'w-6 h-6' : 'w-5 h-5'} grid place-items-center rounded-md border-[1.5px] ${e.ticked ? 'bg-sky-600 border-sky-600 text-white' : 'border-[#5a5383]'}`}>
              {e.ticked && <Check size={12} strokeWidth={3} aria-hidden="true" />}
            </span>
          </button>
        )}
        <div className={`flex-1 min-w-0 flex items-start gap-3 transition-opacity duration-[120ms] motion-reduce:transition-none ${off ? 'opacity-60' : ''}`}>
          <ModernAvatar role={sprite} name={e.name} size={36} />
          <div className="flex-1 min-w-0">
            {renaming === b.key ? (
              renameField(b.key, titleText)
            ) : (
              <div className="flex flex-wrap items-center gap-x-2 gap-y-0.5">
                <span className="text-sm leading-5 font-semibold text-zinc-50">{e.useExisting ? existingName : e.name}</span>
                {b.lead && <LeadsBadge />}
                <span className="text-xs text-zinc-400">{titleText}</span>
              </div>
            )}
            <p className={`text-[12.5px] leading-[18px] ${also ? 'text-zinc-400' : 'text-zinc-300'}`}>
              <Reason parts={b.reason} />
            </p>
            {notes[b.key] && renaming !== b.key && <Note>{notes[b.key]}</Note>}
            {isN && owns.length > 0 && (
              <p className="mt-1 text-xs text-zinc-300" data-takes-care>
                Takes care of {listWords(owns)}
              </p>
            )}
            {isN && b.instructions && (
              <div className="mt-1.5">
                <button
                  type="button"
                  aria-expanded={instrOpen}
                  aria-controls={instrId}
                  onClick={() => setOpenInstr((r) => (r.has(b.key) ? new Set([...r].filter((x) => x !== b.key)) : new Set([...r, b.key])))}
                  className={`inline-flex items-center gap-1 ${phone ? 'min-h-11' : 'min-h-7'} -ml-1 px-1 rounded-md text-[12.5px] font-medium text-sky-300 hover:bg-white/5 ${FOCUS}`}
                >
                  <ChevronDown size={12} aria-hidden="true" className={`transition-transform motion-reduce:transition-none ${instrOpen ? 'rotate-180' : ''}`} />
                  Instructions<span className="sr-only"> for {e.name}</span>
                </button>
                {instrOpen && (
                  <div id={instrId} className="mt-1">
                    <pre className="max-h-[180px] overflow-auto whitespace-pre-wrap break-words p-3 rounded-lg bg-black/30 border border-white/5 text-xs leading-[18px] text-zinc-300 font-sans">{b.instructions}</pre>
                    <p className="mt-1 text-[11.5px] text-zinc-400">You can change these in the bot’s settings after you create it.</p>
                  </div>
                )}
              </div>
            )}
            {!isN && owns.length > 0 && (
              <p className="mt-1 text-xs text-zinc-400">
                Owns{' '}
                {owns.map((o) => (
                  <code key={o} className={`${CODE} mr-1`}>
                    {o}
                  </code>
                ))}
              </p>
            )}
            {!phone && choiceBlock}
          </div>
        </div>
        {!also && renaming !== b.key && (
          <span className={`flex items-center gap-1 ${phone ? 'flex-col' : ''}`}>
            {!b.lead && !isN && (
              <RoleSelect
                phone={phone}
                name={e.name}
                value={e.preset}
                disabled={!!e.useExisting}
                onChange={(preset) =>
                  setEdits((x) => {
                    const taken = [...takenNames, ...Object.entries(x).filter(([k, r]) => k !== b.key && r.ticked && !r.useExisting).map(([, r]) => r.name)];
                    return { ...x, [b.key]: swapRole(x[b.key], preset, taken) };
                  })
                }
              />
            )}
            {!e.useExisting && (
              <button type="button" aria-label={`Rename ${e.name}`} onClick={() => startRename(b.key)} className={iconBtn}>
                <Pencil size={14} aria-hidden="true" />
              </button>
            )}
          </span>
        )}
        {phone && b.existing_kwant_id && e.ticked && !also && (
          <div data-choice-row className={`basis-full pl-9 ${off ? 'opacity-60' : ''}`}>
            {choiceBlock}
          </div>
        )}
      </li>
    );
  };

  const reasonUnknown = sug?.fallback === true;
  const asked = isN && (
    <section className={SEC} aria-labelledby={`${titleId}-asked`}>
      <h4 className={H} id={`${titleId}-asked`}>
        You asked
      </h4>
      <p data-asked className="max-h-[96px] overflow-auto whitespace-pre-wrap break-words p-2.5 rounded-lg bg-black/30 border border-white/5 text-[13px] leading-[18px] text-zinc-200">
        {need.trim()}
      </p>
    </section>
  );
  const nBody = () => {
    if (dPhase === 'reading')
      return (
        <>
          {asked}
          <div role="status" aria-live="polite" className="flex flex-col items-center text-center pt-6 pb-6 px-3" data-suggesting>
            <Loader2 size={22} className="text-sky-400 animate-spin motion-reduce:animate-none" aria-hidden="true" />
            <p className="mt-3 text-sm font-semibold text-zinc-50">Suggesting…</p>
            {elapsed >= 8000 && <p className="mt-1 text-xs text-zinc-400">Usually under a minute.</p>}
          </div>
          <Skeleton rows={3} tick />
        </>
      );
    if (dPhase === 'offline' || dPhase === 'error')
      return (
        <>
          <Banner tone={dPhase === 'offline' ? 'warn' : 'bad'} icon={dPhase === 'offline' ? 'offline' : 'alert'} head={needErr?.head ?? 'Couldn’t suggest a team this time. Try again, or start from a ready-made team.'}>
            {needErr?.reason ? `${needErr.reason} ` : ''}Nothing was created.
          </Banner>
          {asked}
        </>
      );
    if (!sug) return null;
    const lead = sug.bots.find((b) => b.lead);
    return (
      <>
        {expiredNote && (
          <p role="status" className="mt-3 text-xs text-zinc-300">
            Suggested again, it had been an hour.
          </p>
        )}
        {asked}
        <section className={SEC} aria-labelledby={`${titleId}-who`}>
          <h4 className={H} id={`${titleId}-who`}>
            Who’s on it{lead && <span className={`${CNT} text-zinc-500`}>{edits[lead.key]?.name} leads</span>}
          </h4>
          <ul>{sug.bots.map((b) => sgRow(b, false))}</ul>
        </section>
        {handleSec}
      </>
    );
  };
  const dBody = () => {
    if (isN) return nBody();
    if (dPhase === 'reading') {
      const near = elapsed >= 7000;
      return (
        <>
          {expiredNote && (
            <p role="status" className="mt-3 text-xs text-zinc-300">
              Read the project again, it had been an hour.
            </p>
          )}
          <div role="status" aria-live="polite" className="flex flex-col items-center text-center pt-8 pb-6 px-3">
            <span className="w-10 h-10 rounded-[11px] grid place-items-center bg-sky-400/15 text-sky-400 ring-1 ring-inset ring-sky-300/20" aria-hidden="true">
              <ScanSearch size={20} />
            </span>
            <p className="mt-3 text-sm font-semibold text-zinc-50">Reading the project</p>
            <p className="mt-1 font-mono text-xs leading-[18px] text-zinc-300">{near ? 'Still reading file names and manifests' : 'Reading file names and manifests'}</p>
            <span className="mt-3 block w-[240px] max-w-full h-1 rounded-full bg-white/[.07] overflow-hidden" role="progressbar" aria-label="Time used of the 10-second limit" aria-valuemin={0} aria-valuemax={10} aria-valuenow={Math.round(elapsed / 1000)}>
              <span className="block h-full bg-sky-400 transition-[width] duration-200 motion-reduce:transition-none" style={{ width: `${(elapsed / 10_000) * 100}%` }} />
            </span>
            <p className="mt-3 text-xs text-zinc-400">{near ? 'Almost at the 10-second limit. It will suggest from what it has read.' : 'File names and manifests only. No model, up to 10 seconds.'}</p>
          </div>
          <Skeleton rows={3} tick />
        </>
      );
    }
    if (dPhase === 'cantread') {
      const url = mode.kind === 'suggest' ? mode.repo_url ?? '' : '';
      const forge = connectForgeFor(url, cantForge, gitlabHosts);
      return (
        <div className="flex flex-col items-center text-center py-10 px-4">
          <Lock size={22} className="text-zinc-300" aria-hidden="true" />
          <p className="mt-3 text-sm font-semibold text-zinc-50">Can’t read this repo yet</p>
          <p className="mt-1.5 max-w-[420px] text-[12.5px] leading-[18px] text-zinc-300">
            {forge
              ? `${url.replace(/^https?:\/\//, '')} is private. Connect ${forge} so kwant can read file names and manifests. It’s the same connection Dev mode uses for this repo.`
              : `${url.replace(/^https?:\/\//, '')} may be private, or on a host you haven’t connected.`}
          </p>
          <div className={`mt-4 flex ${phone ? 'flex-col w-full' : ''} gap-2`}>
            {forge && (
              <button type="button" onClick={() => onConnectForge?.(url, forge)} className={`${phone ? 'h-11' : 'h-9'} px-4 rounded-lg bg-sky-600 hover:bg-sky-500 text-white text-sm font-medium ${FOCUS}`}>
                Connect {forge}
              </button>
            )}
            {onPickFolder && (
              <button type="button" onClick={onPickFolder} className={`${phone ? 'h-11' : 'h-9'} px-4 rounded-lg border border-zinc-700 text-zinc-200 text-sm hover:bg-white/5 ${FOCUS}`}>
                Pick a local folder instead
              </button>
            )}
          </div>
          <p className="mt-3 text-xs text-zinc-400">Nothing was read and nothing was created.</p>
        </div>
      );
    }
    if (dPhase === 'offline')
      return (
        <Banner tone="warn" head="You’re offline.">
          Suggest members works when you’re back. Nothing was read.
        </Banner>
      );
    if (dPhase === 'error')
      return (
        <Banner tone="bad" head="Couldn’t read the project.">
          {readError ?? 'The server stopped before it finished.'} Nothing was created.
        </Banner>
      );
    if (!sug) return null;
    const ph = partialHead(sug.partial);
    const also = sug.also_possible;
    return (
      <>
        {expiredNote && (
          <p role="status" className="mt-3 text-xs text-zinc-300">
            Read the project again, it had been an hour.
          </p>
        )}
        {ph && (
          <Banner tone="warn" icon="alert" head={ph}>
            {partialBody(sug.partial)}
          </Banner>
        )}
        {reasonUnknown && (
          <Banner tone="info" head="Couldn’t tell what this project uses.">
            It has {plural(sug.read.files, 'file')} and no manifests kwant knows, so this is the Coding template’s team. Change anything before you create it.
          </Banner>
        )}
        {!reasonUnknown && sug.facts.length > 0 && (
          <section className={SEC}>
            <h4 className={H}>
              What it found<span className={`${CNT} text-zinc-500`}>no model, nothing written</span>
            </h4>
            <div className="flex flex-wrap gap-1.5">
              {sug.facts.map((f) => (
                <span key={f.name + f.detail} className="inline-flex items-center gap-1.5 h-6 px-2 rounded-md bg-black/30 border border-white/5 font-mono text-[11.5px] text-zinc-200">
                  {f.name}
                  {f.detail && <small className="text-[11.5px] text-zinc-400">{f.detail}</small>}
                </span>
              ))}
            </div>
            <p className="mt-2 flex items-center gap-1.5 text-xs text-zinc-300">
              <CheckCircle2 size={13} style={{ color: 'var(--kw-ok)' }} aria-hidden="true" />
              {readLine(sug.read, sug.partial)}
            </p>
          </section>
        )}
        <section className={SEC} aria-labelledby={`${titleId}-who`}>
          <h4 className={H} id={`${titleId}-who`}>
            Who’s on it{sug.bots.find((b) => b.lead) && <span className={`${CNT} text-zinc-500`}>{edits[sug.bots.find((b) => b.lead)!.key]?.name} leads</span>}
          </h4>
          {sug.bots.length === 0 && sug.also_possible.length === 0 ? (
            <p className="py-3 text-sm text-zinc-300" data-nothing-new>
              {manage && mode.kind === 'suggest' && mode.team ? `@${mode.team.name} already has every role the code suggests.` : 'The code doesn’t suggest any roles.'} Nothing to add.
            </p>
          ) : sug.bots.length === 0 ? (
            <p className="py-3 text-sm text-zinc-300" data-nothing-core>
              {manage && mode.kind === 'suggest' && mode.team ? `@${mode.team.name} already has the roles the code suggests.` : 'The code doesn’t suggest a core role.'} Tick one below if it helps.
            </p>
          ) : (
            <ul>{sug.bots.map((b) => sgRow(b, false))}</ul>
          )}
          {also.length > 0 && (
            <details open={also.length <= 3} className="mt-2 pt-2 border-t border-dashed border-white/10">
              <summary className={`cursor-pointer list-none ${phone ? 'min-h-11' : 'min-h-8'} flex items-center gap-1.5 text-[11px] font-semibold uppercase tracking-[.08em] text-zinc-400 rounded ${FOCUS}`} role="button">
                <ChevronDown size={12} aria-hidden="true" /> Also possible · {also.length}
              </summary>
              <ul>{also.map((b) => sgRow(b, true))}</ul>
            </details>
          )}
        </section>
        {sug.stack_line && !reasonUnknown && (
          <section className={SEC}>
            <h4 className={H}>In every persona</h4>
            <p className="p-2.5 rounded-lg bg-black/30 border border-white/5 font-mono text-xs leading-[18px] text-zinc-300">
              This team’s code: <strong className="text-zinc-100 font-semibold">{sug.stack_line}</strong>
            </p>
            <p className="mt-1.5 text-xs text-zinc-400">Plus the folders each bot owns. Only names kwant knows and folder names go in, never text from the repo.</p>
          </section>
        )}
        {handleSec}
        {neverSec}
      </>
    );
  };

  const tBody = () => {
    if (templatesError && !t)
      return (
        <Banner tone="bad" head="Couldn’t load this template.">
          Nothing was created. Check the connection and try again.
        </Banner>
      );
    if (!t) return <Skeleton rows={3} />;
    return (
      <>
        {whoT}
        {handleSec}
        {needsSec}
        {neverSec}
        {comesSec}
      </>
    );
  };

  // ---- footer
  const nNew = isD ? newBotCount(edits) : tBots.length;
  // Manage with nothing new to suggest: the body says so; no "Pick at least" nag.
  const nothingNew = manage && dPhase === 'ready' && !!sug && sug.bots.length === 0 && sug.also_possible.length === 0;
  const sNote = isD && dPhase === 'ready' && !nothingNew ? sizeNote(ticked, minBots) : null;
  const fallbackFooter = isD && (dPhase === 'cantread' || dPhase === 'offline' || dPhase === 'error');
  const noAccount = isN && dPhase === 'error' && !!needErr?.noAccount && !!onOpenAccounts;
  // N-5: the same folder gets the same refusal, so offer another folder instead of "Try again".
  const pickOther = inUse && !!onPickFolder && mode.kind === 'suggest' && !mode.team && (phase === 'failed' || dPhase === 'error');
  const retry = !pickOther && ((!isD && templatesError && !t) || phase === 'failed' || (isD && (dPhase === 'error' || dPhase === 'offline')));
  const primaryLabel = creating ? 'Creating…' : noAccount ? 'Add an AI account first' : pickOther ? 'Pick another folder' : retry ? 'Try again' : manage ? (nothingNew || ticked === 0 ? 'Add bots' : `Add ${plural(ticked, 'bot')}`) : isD && dPhase === 'reading' ? 'Create team' : `Create team · ${plural(ticked, 'bot')}`;
  const onPrimary = () => {
    if (noAccount) return onOpenAccounts?.();
    if (pickOther) return onPickFolder?.();
    if (retry) {
      if (!isD && templatesError && !t) return onRetryTemplates?.();
      if (isD && dPhase !== 'ready') return void read();
      setPhase('idle');
      return void create();
    }
    void create();
  };
  const primaryDisabled = creating || (pickOther ? false : retry ? (isD && dPhase === 'offline' && !online) || (phase === 'failed' && !online) : !canCreate);
  // Reused bots are left alone by Undo (api.md §6); with none new, say what it does instead of "the 0 new bots".
  const footNote = nothingNew
    ? null
    : sNote ??
    ((isD && nNew !== ticked) || manage
      ? nNew === 0
        ? manage
          ? 'Undo takes them out of the team again.'
          : 'Undo removes the team. Your bots stay.'
        : `Undo removes the ${plural(nNew, 'new bot')}.`
      : 'Undo removes all of it.');

  const btnH = phone ? 'h-11 flex-1' : 'h-9';
  const panel = (
    <div
      ref={rootRef}
      tabIndex={-1}
      role="dialog"
      aria-modal="true"
      aria-labelledby={titleId}
      aria-describedby={subId}
      aria-busy={busy || undefined}
      data-team-sheet={isD ? 'suggest' : t?.id ?? templateId}
      onClick={(e) => e.stopPropagation()}
      style={phone && dy ? { transform: `translateY(${dy}px)` } : undefined}
      className={
        phone
          ? 'absolute inset-x-0 bottom-0 max-h-[92%] flex flex-col rounded-t-2xl bg-zinc-900 border-t border-zinc-800 outline-none animate-[t-sheet_200ms_ease-out] motion-reduce:animate-none'
          : 'w-[576px] max-w-[calc(100vw-32px)] max-h-[calc(100vh-48px)] flex flex-col overflow-hidden rounded-2xl bg-zinc-900 border border-zinc-800 shadow-[0_25px_50px_-12px_rgb(0_0_0/.6)] outline-none animate-[t-rise_180ms_ease-out] motion-reduce:animate-none'
      }
    >
      {phone && (
        <div aria-hidden="true" className="h-5 shrink-0 grid place-items-center touch-none cursor-grab" onPointerDown={onGrabDown} onPointerMove={onGrabMove} onPointerUp={onGrabUp} onPointerCancel={onGrabUp}>
          <span className="w-9 h-1 rounded-full bg-[#3f3868]" />
        </div>
      )}
      <header className={`flex items-center gap-3 ${phone ? 'px-4 pt-1 pb-3' : 'px-5 py-4'} border-b border-zinc-800`}>
        {glyph === 'sparkles' ? (
          <span aria-hidden="true" className="w-10 h-10 rounded-[11px] shrink-0 grid place-items-center bg-sky-400/15 text-sky-400 ring-1 ring-inset ring-sky-300/20">
            <Sparkles size={20} />
          </span>
        ) : glyph === 'scan' ? (
          <span aria-hidden="true" className="w-10 h-10 rounded-[11px] shrink-0 grid place-items-center bg-sky-400/15 text-sky-400 ring-1 ring-inset ring-sky-300/20">
            <ScanSearch size={20} />
          </span>
        ) : (
          <TemplateGlyph icon={glyph} size={20} box="w-10 h-10 rounded-[11px]" />
        )}
        <div className="flex-1 min-w-0">
          <h3 id={titleId} className="text-[17px] leading-6 font-semibold text-zinc-50 truncate">
            {title}
          </h3>
          <p id={subId} className="text-xs text-zinc-400 truncate">
            {sub}
          </p>
        </div>
        <button type="button" aria-label="Close" onClick={close} disabled={creating} className={iconBtn}>
          <X size={16} aria-hidden="true" />
        </button>
      </header>
      <div
        className={`flex-1 min-h-0 overflow-y-auto ${phone ? 'px-4' : 'px-5'} pt-1 pb-4 transition-opacity ${creating ? 'opacity-60' : ''}`}
        // inert while creating: React 18 doesn't know the prop, so set it as an attribute.
        {...(creating ? { inert: '' } : {})}
      >
        {!online && !isD && (
          <Banner tone="warn" head="You’re offline.">
            You can look, and Create team works when you’re back.
          </Banner>
        )}
        {phase === 'failed' && (
          <Banner tone="bad" head="Couldn’t create the team. Nothing was created.">
            {createError ? `${createError} ` : 'The server stopped before it finished. '}Your names are kept.
          </Banner>
        )}
        {isD ? dBody() : tBody()}
      </div>
      <footer className={`flex ${phone ? 'flex-wrap' : 'items-center'} gap-2 ${phone ? 'px-4 pt-3 pb-[max(12px,env(safe-area-inset-bottom))]' : 'px-5 py-3.5'} border-t border-zinc-800 bg-zinc-900`}>
        <p className={`${phone ? 'w-full' : 'flex-1 min-w-0'} flex items-center gap-1.5 text-xs ${sNote ? 'text-amber-200' : 'text-zinc-400'}`} aria-live="polite">
          {!fallbackFooter && !sNote && footNote && <Undo2 size={13} aria-hidden="true" />}
          {fallbackFooter ? null : footNote}
        </p>
        <button type="button" onClick={close} disabled={creating} className={`${btnH} px-3.5 rounded-lg text-sm text-zinc-200 hover:bg-white/5 disabled:opacity-50 ${FOCUS}`}>
          Cancel
        </button>
        {isN && dPhase === 'ready' && (
          <button
            type="button"
            onClick={() => {
              setAgain(true);
              void read();
            }}
            disabled={creating || !online}
            className={`${btnH} inline-flex items-center justify-center gap-1.5 px-3.5 rounded-lg border border-zinc-700 text-sm text-zinc-100 hover:bg-white/5 disabled:opacity-50 ${FOCUS}`}
          >
            <RefreshCw size={14} aria-hidden="true" />
            Try again
          </button>
        )}
        {fallbackFooter && onUseCoding && !isN && (
          <button type="button" onClick={onUseCoding} className={`${btnH} px-3.5 rounded-lg border border-zinc-700 text-sm text-zinc-100 hover:bg-white/5 ${FOCUS}`}>
            Use the Coding roster
          </button>
        )}
        {!(isD && dPhase === 'cantread') && (
          <button
            ref={createRef}
            type="button"
            onClick={onPrimary}
            disabled={primaryDisabled}
            className={`${btnH} inline-flex items-center justify-center gap-2 px-4 rounded-lg bg-sky-600 hover:bg-sky-500 text-white text-sm font-medium disabled:opacity-50 disabled:hover:bg-sky-600 ${FOCUS}`}
          >
            {creating && <Loader2 size={14} className="animate-spin motion-reduce:animate-none" aria-hidden="true" />}
            {pickOther && !creating && <FolderOpen size={14} aria-hidden="true" />}
            {retry && !creating && !noAccount && <RefreshCw size={14} aria-hidden="true" />}
            {primaryLabel}
          </button>
        )}
      </footer>
      <p className="sr-only" aria-live="polite">
        {live}
      </p>
    </div>
  );

  return (
    <div className={`fixed inset-0 z-[70] ${phone ? '' : 'grid place-items-center p-6'} bg-[rgb(6_3_20/.62)]`} onClick={close} data-team-sheet-scrim>
      {panel}
    </div>
  );
};

// ---------------------------------------------------------------------------------------------------

const RoleSelect: React.FC<{ phone: boolean; name: string; value: string; disabled?: boolean; onChange: (preset: string) => void }> = ({ phone, name, value, disabled, onChange }) => {
  const [open, setOpen] = useState(false);
  const [active, setActive] = useState(0);
  const [up, setUp] = useState(false);
  const btn = useRef<HTMLButtonElement>(null);
  const list = useRef<HTMLUListElement>(null);
  const cur = roleOf(value);
  const id = useId();
  const show = () => {
    const r = btn.current?.getBoundingClientRect();
    setUp(!!r && window.innerHeight - r.bottom < 320);
    setActive(Math.max(0, ROLE_OPTIONS.findIndex((o) => o.preset === value)));
    setOpen(true);
  };
  useEffect(() => {
    if (open) list.current?.focus();
  }, [open]);
  const pick = (i: number) => {
    onChange(ROLE_OPTIONS[i].preset);
    setOpen(false);
    btn.current?.focus();
  };
  return (
    <span className="relative">
      <button
        ref={btn}
        type="button"
        aria-haspopup="listbox"
        aria-expanded={open}
        aria-controls={open ? id : undefined}
        aria-label={`Role: ${cur?.label ?? value}. Change`}
        disabled={disabled}
        onClick={() => (open ? setOpen(false) : show())}
        className={`${phone ? 'h-11' : 'h-7'} inline-flex items-center gap-1 px-2 rounded-md border border-white/10 bg-white/[.03] text-xs text-zinc-200 hover:bg-white/[.06] disabled:opacity-40 ${FOCUS}`}
      >
        {cur?.label ?? value}
        <ChevronDown size={12} aria-hidden="true" />
      </button>
      {open && (
        <ul
          ref={list}
          id={id}
          role="listbox"
          tabIndex={-1}
          aria-label={`Role for ${name}`}
          aria-activedescendant={`${id}-${active}`}
          onBlur={(e) => {
            if (!e.currentTarget.contains(e.relatedTarget as Node)) setOpen(false);
          }}
          onKeyDown={(e) => {
            if (e.key === 'ArrowDown') {
              e.preventDefault();
              setActive((a) => (a + 1) % ROLE_OPTIONS.length);
            } else if (e.key === 'ArrowUp') {
              e.preventDefault();
              setActive((a) => (a - 1 + ROLE_OPTIONS.length) % ROLE_OPTIONS.length);
            } else if (e.key === 'Enter' || e.key === ' ') {
              e.preventDefault();
              pick(active);
            } else if (e.key === 'Escape') {
              e.preventDefault();
              e.stopPropagation();
              setOpen(false);
              btn.current?.focus();
            }
          }}
          className={`absolute right-0 z-10 w-[200px] p-1 rounded-[10px] bg-[#231d4b] border border-white/10 shadow-xl outline-none animate-[t-rise_120ms_ease-out] motion-reduce:animate-none ${up ? 'bottom-full mb-1' : 'top-full mt-1'}`}
        >
          {ROLE_OPTIONS.map((o, i) => (
            <li
              key={o.preset}
              id={`${id}-${i}`}
              role="option"
              aria-selected={o.preset === value}
              onMouseDown={(e) => e.preventDefault()}
              onClick={() => pick(i)}
              className={`flex items-center gap-2 h-[34px] px-2 rounded-md text-[13px] cursor-pointer ${i === active ? 'bg-white/[.06]' : ''} text-zinc-100`}
            >
              <ModernAvatar role={o.sprite} name={o.name} size={22} />
              {o.label}
              {o.preset === value && <Check size={12} className="ml-auto text-sky-300" aria-hidden="true" />}
            </li>
          ))}
        </ul>
      )}
    </span>
  );
};

const Choice: React.FC<{ phone: boolean; existingName: string; existingRole: string; value: 'existing' | 'new'; onChange: (v: 'existing' | 'new') => void }> = ({ phone, existingName, existingRole, value, onChange }) => {
  const refs = [useRef<HTMLButtonElement>(null), useRef<HTMLButtonElement>(null)];
  const opts: ('existing' | 'new')[] = ['existing', 'new'];
  const onKey = (e: React.KeyboardEvent) => {
    if (!['ArrowLeft', 'ArrowRight', 'ArrowUp', 'ArrowDown'].includes(e.key)) return;
    e.preventDefault();
    const next = value === 'existing' ? 'new' : 'existing';
    onChange(next);
    refs[opts.indexOf(next)].current?.focus();
  };
  const cls = (on: boolean) => `${phone ? 'min-h-11 w-full justify-start text-left gap-2 px-3 text-[13px] whitespace-nowrap' : 'h-7 gap-1.5 px-2 text-xs'} inline-flex items-center rounded-md ${on ? 'bg-[#2c2552] text-zinc-50' : 'text-zinc-300 hover:bg-white/5'} ${FOCUS}`;
  return (
    <div role="radiogroup" aria-label={`${existingName} for this team`} onKeyDown={onKey} className={`mt-2 ${phone ? 'flex flex-col w-full' : 'inline-flex'} gap-1 p-0.5 rounded-lg border border-white/10`}>
      <button ref={refs[0]} type="button" role="radio" aria-checked={value === 'existing'} tabIndex={value === 'existing' ? 0 : -1} onClick={() => onChange('existing')} className={cls(value === 'existing')}>
        <ModernAvatar role={existingRole} name={existingName} size={22} />
        Add {existingName} (already in this space)
      </button>
      <button ref={refs[1]} type="button" role="radio" aria-checked={value === 'new'} tabIndex={value === 'new' ? 0 : -1} onClick={() => onChange('new')} className={cls(value === 'new')}>
        <Plus size={12} aria-hidden="true" />
        New bot
      </button>
    </div>
  );
};
