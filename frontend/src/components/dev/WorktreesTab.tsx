// 0.9.0 H1 + P3: the Dev desk's Worktrees tab (Mira, designs/h1-worktrees.md). One group per linked repo,
// a row per worktree, New worktree (new or existing branch) and Remove (refuses a dirty checkout until
// "Remove anyway" is ticked; never deletes the branch). Finished worktrees carry the P3 cleanup line.

import React, { useCallback, useEffect, useId, useMemo, useRef, useState, useSyncExternalStore } from 'react';
import { AlertTriangle, Check, Clock, Copy, Eraser, Folder, FolderGit2, GitBranch, Info, Loader2, MessageSquare, MoreHorizontal, Plus, Trash2 } from 'lucide-react';
import {
  BranchInfo,
  OtherWorktree,
  P3,
  Worktree,
  WorktreeError,
  WorktreeRepo,
  WorktreesData,
  branchProblem,
  branchSlug,
  changesLabel,
  createErrorCopy,
  keptNotes,
  p3State,
  removeErrorCopy,
  rowStatus,
  rowTask,
  localNameOf,
  pickableBranches,
  createBodyFor,
  rowTime,
  shortPath,
  suggestBranch,
  upToDate,
  worktreeApi,
  worktreeStore,
} from '../../dev/worktrees';
import { ModernAvatar } from '../ModernAvatar';
import { threadsApi } from '../../threads';
import { trapFocusTarget } from '../../imports';

const ring = 'focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-sky-300';
const FOCUSABLE = 'button:not([disabled]), input:not([disabled]), select:not([disabled]), a[href]';

export interface WtBot {
  id: string;
  name: string;
  role?: string | null;
}

export function useWorktrees(space: string, on: boolean): WorktreesData {
  const data = useSyncExternalStore(worktreeStore.subscribe, () => worktreeStore.get(space), () => worktreeStore.get(space));
  useEffect(() => {
    if (on) void worktreeStore.load(space);
  }, [space, on]);
  return data;
}

interface Toast {
  id: number;
  text: string;
  action?: { label: string; run: () => void };
}

export interface WorktreesPanelProps {
  space: string;
  data: WorktreesData;
  bots: WtBot[];
  /** "Last bot you chatted with in Dev mode": the create dialog's default. */
  defaultBotId?: string | null;
  onOpenChat?: (botId: string, threadId: string | null) => void;
  /** Phone layout (below 768 px): ⋯ menus and bottom sheets. */
  phone?: boolean;
  now?: number;
  home?: string | null;
}

export const WorktreesPanel: React.FC<WorktreesPanelProps> = ({ space, data, bots, defaultBotId, onOpenChat, phone = false, now: nowProp, home = null }) => {
  const now = nowProp ?? Date.now();
  const botName = useCallback((id: string | null) => (id ? bots.find((b) => b.id === id)?.name ?? null : null), [bots]);
  const [creating, setCreating] = useState<WorktreeRepo | null>(null);
  const [removing, setRemoving] = useState<Worktree | null>(null);
  const [menu, setMenu] = useState<Worktree | null>(null);
  const [announce, setAnnounce] = useState('');
  const [toasts, setToasts] = useState<Toast[]>([]);
  const [busy, setBusy] = useState<Record<string, 'cleaning' | 'keeping' | undefined>>({});
  const [failed, setFailed] = useState<Record<string, string | undefined>>({});
  const [rowError, setRowError] = useState<Record<string, string | undefined>>({});
  const returnFocus = useRef<HTMLElement | null>(null);

  const toast = useCallback((text: string, action?: Toast['action']) => {
    const id = Date.now() + Math.random();
    setToasts((t) => [...t.slice(-2), { id, text, action }]);
    setAnnounce(text);
    setTimeout(() => setToasts((t) => t.filter((x) => x.id !== id)), 7000);
  }, []);

  const copyPath = useCallback((p: string) => {
    const done = () => setAnnounce('Path copied');
    navigator.clipboard?.writeText(p).then(done, done) ?? done();
  }, []);

  const keep = useCallback(
    (w: Worktree, on: boolean) => {
      setBusy((b) => ({ ...b, [w.id]: 'keeping' }));
      worktreeApi
        .keep(w.id, on)
        .then((n) => n && worktreeStore.patch(space, n))
        .catch((e: unknown) => setRowError((r) => ({ ...r, [w.id]: e instanceof Error ? e.message : 'Couldn’t change it.' })))
        .finally(() => setBusy((b) => ({ ...b, [w.id]: undefined })));
    },
    [space],
  );

  const clean = useCallback(
    (w: Worktree) => {
      setBusy((b) => ({ ...b, [w.id]: 'cleaning' }));
      setFailed((f) => ({ ...f, [w.id]: undefined }));
      worktreeApi
        .clean(w.id)
        .then((n) => n && worktreeStore.patch(space, n))
        .catch((e: unknown) => {
          const code = e instanceof WorktreeError ? e.code : null;
          const reason = code === 'working' ? `${botName(w.kwant_id) ?? 'a bot'} is working here` : code === 'missing' ? 'the folder is gone' : code === 'not_ours' ? 'the folder wasn’t made here' : e instanceof Error ? e.message.replace(/\.$/, '') : 'it failed';
          setFailed((f) => ({ ...f, [w.id]: reason }));
        })
        .finally(() => setBusy((b) => ({ ...b, [w.id]: undefined })));
    },
    [space, botName],
  );

  const forget = useCallback(
    (w: Worktree) => {
      setRowError((r) => ({ ...r, [w.id]: undefined }));
      worktreeApi.remove(w.id, false).then(
        () => void worktreeStore.load(space),
        (e: unknown) => setRowError((r) => ({ ...r, [w.id]: removeErrorCopy({ code: e instanceof WorktreeError ? e.code : null, message: e instanceof Error ? e.message : 'it failed' }, botName(w.kwant_id)) })),
      );
    },
    [space, botName],
  );

  const openRemove = (w: Worktree, from: HTMLElement | null) => {
    returnFocus.current = from;
    setMenu(null);
    setRemoving(w);
  };
  const openCreate = (r: WorktreeRepo, from: HTMLElement | null) => {
    returnFocus.current = from;
    setCreating(r);
  };
  const closeDialog = () => {
    setCreating(null);
    setRemoving(null);
    setMenu(null);
    const el = returnFocus.current;
    returnFocus.current = null;
    if (el?.isConnected) el.focus();
  };

  const list = data.list;
  const rowProps = { now, home, botName, busy, failed, rowError, phone, onOpenChat, copyPath, keep, clean, forget, openRemove, openMenu: (w: Worktree, from: HTMLElement) => ((returnFocus.current = from), setMenu(w)) };

  let body: React.ReactNode;
  if (!list && data.loading) body = <Loading />;
  else if (!list && data.error)
    body = (
      <div role="alert" className="flex items-center gap-2 rounded-[14px] border border-hairline bg-surface-raised px-3.5 py-3 text-[13px] text-zinc-200" data-wt-error>
        <AlertTriangle size={15} className="shrink-0 text-zinc-400" aria-hidden="true" />
        <span className="min-w-0 flex-1">Couldn’t read the worktrees. {data.error}</span>
        <button type="button" onClick={() => void worktreeStore.load(space)} className={`shrink-0 rounded-md px-2 py-1 text-sky-300 hover:bg-sky-500/10 ${ring}`}>
          Try again
        </button>
      </div>
    );
  else if (!list || !list.repos.length)
    body = <EmptyTerm out="(no repo yet)" title="Worktrees show up once there’s a repo." line="Link a repo from Dev mode in the sidebar." data="no-repo" />;
  else
    body = list.repos.map((r) => (
      <RepoGroup key={r.repo} space={space} repo={r} home={home} phone={phone} onNew={(el) => openCreate(r, el)}>
        {r.worktrees.map((w) => (
          <WorktreeRow key={w.id} w={w} repo={r.repo} {...rowProps} />
        ))}
        {r.other.map((o) => (
          <OtherRow key={o.path} o={o} home={home} phone={phone} copyPath={copyPath} />
        ))}
      </RepoGroup>
    ));

  return (
    <div className="space-y-6" data-wt-panel>
      <p className="hidden sm:block -mt-2 text-right text-[12px] text-zinc-500" data-wt-note>
        Cleans build output an hour after a task finishes
      </p>
      {body}
      <p className="sr-only" aria-live="polite" data-wt-live>
        {announce}
      </p>
      {toasts.length > 0 && (
        <div className="fixed inset-x-0 bottom-4 z-[70] flex flex-col items-center gap-2 px-4 pointer-events-none" data-wt-toasts>
          {toasts.map((t) => (
            <div key={t.id} className="kw-toast pointer-events-auto flex items-center gap-3 max-w-[480px] w-full sm:w-auto rounded-xl border border-zinc-700 bg-zinc-900/95 px-3.5 py-2.5 text-[13px] text-zinc-100 shadow-xl" data-wt-toast>
              <span className="min-w-0 flex-1">{t.text}</span>
              {t.action && (
                <button type="button" onClick={t.action.run} className={`shrink-0 rounded-md px-2 py-1 font-medium text-sky-300 hover:bg-sky-500/10 ${ring}`}>
                  {t.action.label}
                </button>
              )}
            </div>
          ))}
        </div>
      )}
      {creating && (
        <CreateDialog
          space={space}
          repo={creating}
          bots={bots}
          defaultBotId={defaultBotId ?? null}
          home={home}
          phone={phone}
          existing={creating.worktrees}
          onClose={closeDialog}
          onCreated={(w) => {
            const who = botName(w.kwant_id);
            closeDialog();
            void worktreeStore.load(space);
            toast(who ? `Worktree ready for ${who}` : 'Worktree ready', who && onOpenChat && w.kwant_id ? { label: 'Open chat', run: () => onOpenChat(w.kwant_id as string, w.thread_id) } : undefined);
          }}
        />
      )}
      {removing && (
        <RemoveDialog
          w={removing}
          bot={botName(removing.kwant_id)}
          home={home}
          phone={phone}
          onClose={closeDialog}
          onRemoved={(w) => {
            const who = botName(w.kwant_id);
            closeDialog();
            void worktreeStore.load(space);
            toast(`Removed ${who ? `${who}’s` : 'the'} worktree. The branch ${w.branch} stays.`);
          }}
        />
      )}
      {menu && (
        <RowSheet
          w={menu}
          bot={botName(menu.kwant_id)}
          onClose={closeDialog}
          onOpenChat={onOpenChat}
          copyPath={copyPath}
          keep={keep}
          clean={clean}
          openRemove={(w) => {
            setMenu(null);
            setRemoving(w);
          }}
        />
      )}
    </div>
  );
};

// ---------------------------------------------------------------------------
// Groups and rows

const RepoGroup: React.FC<{ space: string; repo: WorktreeRepo; home: string | null; phone: boolean; onNew: (el: HTMLElement) => void; children: React.ReactNode }> = ({ space, repo, home, phone, onNew, children }) => {
  const n = repo.worktrees.length + repo.other.length;
  const headId = useId();
  return (
    <section aria-labelledby={headId} data-wt-repo={repo.repo}>
      <div className="mb-2 flex items-center gap-2">
        <FolderGit2 size={15} className="shrink-0 text-zinc-300" aria-hidden="true" />
        <h3 id={headId} className="min-w-0 flex flex-wrap items-baseline gap-x-2 text-[14px]">
          <span className="font-semibold text-zinc-50">{repo.repo}</span>
          <span className="text-zinc-500" data-wt-count>
            {n === 1 ? '1 worktree' : `${n} worktrees`}
          </span>
          {!phone && repo.checkout && (
            <span className="font-mono text-[11.5px] text-zinc-500">
              {shortPath(repo.checkout, home)} · main checkout
            </span>
          )}
        </h3>
        <span className="flex-1" />
        {!repo.error && (
          <button
            type="button"
            onClick={(e) => onNew(e.currentTarget)}
            aria-label={phone ? `New worktree in ${repo.repo}` : undefined}
            data-wt-new
            className={`kw-btn-ghost shrink-0 ${phone ? 'w-11 h-11 px-0 justify-center' : ''} ${ring}`}
          >
            <Plus size={14} aria-hidden="true" />
            {!phone && <span>New worktree</span>}
          </button>
        )}
      </div>
      {repo.error ? (
        <RepoError space={space} repo={repo.repo} />
      ) : n === 0 ? (
        <EmptyTerm
          out={`${repo.checkout ? shortPath(repo.checkout, home) : repo.repo}  [${repo.default_branch ?? 'main'}]`}
          title="Each bot task can get its own folder."
          line="Bots then work side by side without touching each other’s files. kwant cleans build output an hour after a task finishes."
          data="empty"
        >
          <button type="button" onClick={(e) => onNew(e.currentTarget)} className={`kw-btn-primary mt-3 ${ring}`} data-wt-empty-new>
            <Plus size={14} aria-hidden="true" /> New worktree
          </button>
        </EmptyTerm>
      ) : (
        <ul className="rounded-[14px] border border-hairline bg-surface-raised divide-y divide-[color:var(--kw-hairline)]">{children}</ul>
      )}
    </section>
  );
};

const RepoError: React.FC<{ space: string; repo: string }> = ({ space, repo }) => (
  <div className="rounded-[14px] border border-hairline bg-surface-raised px-3.5 py-3 flex items-center gap-2.5 text-[13px] text-zinc-200" role="alert" data-wt-repo-error>
    <AlertTriangle size={15} className="shrink-0 text-zinc-400" aria-hidden="true" />
    <span className="min-w-0 flex-1">Couldn’t read the worktrees of {repo}.</span>
    <button type="button" onClick={() => void worktreeStore.load(space)} className={`shrink-0 rounded-md px-2 py-1 font-medium text-sky-300 hover:bg-sky-500/10 min-h-[32px] ${ring}`}>
      Try again
    </button>
  </div>
);

interface RowProps {
  w: Worktree;
  repo: string;
  now: number;
  home: string | null;
  phone: boolean;
  botName: (id: string | null) => string | null;
  busy: Record<string, 'cleaning' | 'keeping' | undefined>;
  failed: Record<string, string | undefined>;
  rowError: Record<string, string | undefined>;
  onOpenChat?: (botId: string, threadId: string | null) => void;
  copyPath: (p: string) => void;
  keep: (w: Worktree, on: boolean) => void;
  clean: (w: Worktree) => void;
  forget: (w: Worktree) => void;
  openRemove: (w: Worktree, from: HTMLElement | null) => void;
  openMenu: (w: Worktree, from: HTMLElement) => void;
}

const IconBtn: React.FC<{ label: string; title?: string; disabled?: boolean; describe?: string; onClick: (e: React.MouseEvent<HTMLButtonElement>) => void; children: React.ReactNode; data?: string }> = ({ label, title, disabled, describe, onClick, children, data }) => {
  const descId = useId();
  return (
    <>
      <button
        type="button"
        aria-label={label}
        title={title ?? label}
        aria-disabled={disabled || undefined}
        aria-describedby={describe ? descId : undefined}
        onClick={(e) => (disabled ? e.preventDefault() : onClick(e))}
        data-wt-act={data}
        className={`w-[30px] h-[30px] inline-flex items-center justify-center rounded-lg text-zinc-400 ${disabled ? 'opacity-40 cursor-not-allowed' : 'hover:text-zinc-100 hover:bg-white/[0.06]'} ${ring}`}
      >
        {children}
      </button>
      {describe && (
        <span id={descId} className="sr-only">
          {describe}
        </span>
      )}
    </>
  );
};

const WorktreeRow: React.FC<RowProps> = ({ w, repo, now, home, phone, botName, busy, failed, rowError, onOpenChat, copyPath, keep, clean, forget, openRemove, openMenu }) => {
  const st = rowStatus(w, now);
  const bot = botName(w.kwant_id);
  const who = bot ?? 'You';
  const path = shortPath(w.path, home);
  const err = rowError[w.id];

  if (st.kind === 'missing' || st.kind === 'leftover' || st.kind === 'leftover_foreign') {
    const missing = st.kind === 'missing';
    return (
      <li className="grid grid-cols-[22px_1fr_auto] gap-3 px-3.5 py-3 items-start" data-wt-row={w.id} data-wt-state={st.kind}>
        <Folder size={18} className="mt-0.5 text-zinc-400" aria-hidden="true" />
        <div className="min-w-0">
          <p className="flex flex-wrap items-center gap-x-2 gap-y-1 text-[14px]">
            <span className="dv-led" data-s={st.led} aria-hidden="true" />
            <span className="font-semibold text-zinc-50">{st.text}</span>
            {missing ? <Branch name={w.branch} /> : <span className="font-mono text-[12px] text-zinc-400 break-all">{path}</span>}
          </p>
          <p className="mt-1 text-[12.5px] text-zinc-300">
            {missing ? `Git still lists ${path}, but the folder is gone.` : st.kind === 'leftover' ? `Git doesn’t list it any more. It was a worktree of ${repo}, so kwant can remove it.` : `This folder isn’t from ${repo}, so kwant won’t delete it.`}
          </p>
          {err && (
            <p role="alert" className="mt-1 text-[12px] text-rose-300">
              {err}
            </p>
          )}
        </div>
        {st.kind !== 'leftover_foreign' && (
          <button type="button" onClick={(e) => (missing ? forget(w) : openRemove(w, e.currentTarget))} className={`kw-btn-ghost ${phone ? 'min-h-[44px]' : ''} ${ring}`} data-wt-act={missing ? 'forget' : 'remove-folder'}>
            {missing ? 'Forget it' : 'Remove folder'}
          </button>
        )}
      </li>
    );
  }

  const changes = changesLabel(w.dirty);
  const time = rowTime(w, now);
  const p3 = p3State(w, { cleaning: busy[w.id] === 'cleaning', failed: failed[w.id] });
  const blocked = w.working ? `${who} is working here. Stop ${who} first.` : undefined;
  const removeLabel = bot ? `Remove ${bot}’s worktree` : 'Remove this worktree';
  return (
    <li className={`grid ${phone ? 'grid-cols-[24px_1fr_44px]' : 'grid-cols-[22px_1fr_auto]'} gap-3 px-3.5 py-3 items-start`} data-wt-row={w.id} data-wt-state={st.kind}>
      <ModernAvatar name={who} role={w.kwant_id ? undefined : 'person'} size={phone ? 24 : 22} kwantId={w.kwant_id ?? undefined} />
      <div className="min-w-0">
        <p className="flex flex-wrap items-center gap-x-2 gap-y-1 text-[14px]">
          <span className="dv-led" data-s={st.led} aria-hidden="true" />
          <span className="font-semibold text-zinc-50">{who}</span>
          <span className="text-zinc-400" data-wt-status>
            {st.text}
          </span>
          <span className={phone ? 'basis-full' : ''}>
            <Branch name={w.branch} />
          </span>
        </p>
        <p className="mt-1 text-[12.5px] text-zinc-300 break-words" data-wt-task>
          {rowTask(w, now)}
        </p>
        <p className="mt-1 flex flex-wrap items-center gap-x-3 gap-y-0.5 text-[11.5px] text-zinc-500" data-wt-facts>
          {changes &&
            (w.dirty === 0 ? (
              <span className="inline-flex items-center gap-1">
                <Check size={12} aria-hidden="true" /> {changes}
              </span>
            ) : (
              <span className="text-zinc-200" data-wt-dirty>
                <span aria-hidden="true">● </span>
                {changes}
              </span>
            ))}
          {upToDate(w) ? (
            <span className="font-mono">up to date</span>
          ) : (
            <>
              {!!w.ahead && (
                <span style={{ color: 'var(--dv-add)' }} aria-label={`${w.ahead} ahead of ${w.base ?? 'the default branch'}`}>
                  ↑{w.ahead}
                </span>
              )}
              {!!w.behind && (
                <span style={{ color: 'var(--dv-del)' }} aria-label={`${w.behind} behind ${w.base ?? 'the default branch'}`}>
                  ↓{w.behind}
                </span>
              )}
            </>
          )}
          {!phone && <span className="font-mono text-zinc-400 break-all">{path}</span>}
          {time && <span data-wt-time>{time}</span>}
        </p>
        {p3 && <P3Line p={p3} w={w} busy={!!busy[w.id]} keep={keep} clean={clean} />}
        {err && (
          <p role="alert" className="mt-1 text-[12px] text-rose-300">
            {err}
          </p>
        )}
      </div>
      {phone ? (
        <button type="button" aria-label={`More for ${bot ? `${bot}’s` : 'this'} worktree`} aria-haspopup="dialog" onClick={(e) => openMenu(w, e.currentTarget)} className={`w-11 h-11 -mr-1.5 inline-flex items-center justify-center rounded-lg text-zinc-300 hover:bg-white/[0.06] ${ring}`} data-wt-more>
          <MoreHorizontal size={18} aria-hidden="true" />
        </button>
      ) : (
        <div className="flex items-center gap-1">
          {w.kwant_id && onOpenChat && (
            <IconBtn label={`Open ${bot ?? 'the bot'}’s chat`} onClick={() => onOpenChat(w.kwant_id as string, w.thread_id)} data="chat">
              <MessageSquare size={15} aria-hidden="true" />
            </IconBtn>
          )}
          <IconBtn label="Copy the folder path" onClick={() => copyPath(w.path)} data="copy">
            <Copy size={15} aria-hidden="true" />
          </IconBtn>
          <IconBtn label={removeLabel} title={blocked ?? removeLabel} disabled={w.working} describe={blocked} onClick={(e) => openRemove(w, e.currentTarget)} data="remove">
            <Trash2 size={15} aria-hidden="true" />
          </IconBtn>
        </div>
      )}
    </li>
  );
};

const OtherRow: React.FC<{ o: OtherWorktree; home: string | null; phone: boolean; copyPath: (p: string) => void }> = ({ o, home, phone, copyPath }) => (
  <li className={`grid ${phone ? 'grid-cols-[24px_1fr_44px]' : 'grid-cols-[22px_1fr_auto]'} gap-3 px-3.5 py-3 items-start`} data-wt-row={o.path} data-wt-state={o.missing ? 'outside-missing' : 'outside'}>
    <Folder size={18} className="mt-0.5 text-zinc-400" aria-hidden="true" />
    <div className="min-w-0">
      <p className="flex flex-wrap items-center gap-x-2 gap-y-1 text-[14px]">
        <span className="dv-led" data-s={o.missing ? 'warn' : 'off'} aria-hidden="true" />
        <span className="font-semibold text-zinc-50">{o.missing ? 'Folder missing' : o.folder_name}</span>
        <span className="text-zinc-400">made outside kwant</span>
        {o.branch && <Branch name={o.branch} />}
      </p>
      <p className="mt-1 text-[12.5px] text-zinc-300 break-words">
        {o.missing ? `Git still lists ${shortPath(o.path, home)}, but the folder is gone.` : <span className="font-mono text-[11.5px] text-zinc-400 break-all">{shortPath(o.path, home)}</span>}
      </p>
    </div>
    {!o.missing && (
      <div className="flex items-center">
        <IconBtn label="Copy the folder path" onClick={() => copyPath(o.path)} data="copy">
          <Copy size={15} aria-hidden="true" />
        </IconBtn>
      </div>
    )}
  </li>
);

const Branch: React.FC<{ name: string }> = ({ name }) => (
  <code className="dv-branch max-w-full" data-wt-branch>
    <GitBranch className="i" aria-hidden="true" />
    <span className="h">{name}</span>
  </code>
);

const P3Line: React.FC<{ p: P3; w: Worktree; busy: boolean; keep: (w: Worktree, on: boolean) => void; clean: (w: Worktree) => void }> = ({ p, w, busy, keep, clean }) => {
  const link = `rounded px-0.5 font-medium text-sky-300 hover:underline disabled:opacity-60 ${ring}`;
  const notes = p.kind === 'cleaned' ? keptNotes(w) : [];
  const Icon = p.kind === 'due' || p.kind === 'kept' ? Clock : p.kind === 'cleaning' ? Loader2 : p.kind === 'cleaned' ? Eraser : AlertTriangle;
  return (
    <p className="mt-2 inline-flex max-w-full items-start gap-1.5 rounded-md border border-hairline bg-white/[.03] px-2 py-[3px] text-[11.5px] leading-[18px] text-zinc-400" data-wt-p3={p.kind}>
      <Icon size={12} className={`mt-[3px] shrink-0 ${p.kind === 'cleaning' ? 'animate-spin motion-reduce:animate-none' : ''}`} aria-hidden="true" />
      <span className="min-w-0">
        {p.kind === 'due' && (
          <>
            Build output{p.dirs.length ? ` (${p.dirs.join(', ')})` : ''} is cleaned at <strong className="font-semibold text-zinc-100">{p.at}</strong>
            {' · '}
            <button type="button" disabled={busy} onClick={() => keep(w, true)} className={link} data-wt-keep>
              Keep it
            </button>
          </>
        )}
        {p.kind === 'kept' && (
          <>
            Build output kept{' · '}
            <button type="button" disabled={busy || w.working} onClick={() => clean(w)} className={link} data-wt-clean>
              Clean now
            </button>
          </>
        )}
        {p.kind === 'cleaning' && <span role="status">Cleaning build output…</span>}
        {p.kind === 'cleaned' && (
          <>
            Cleaned <strong className="font-semibold text-zinc-100">{p.size}</strong> of build output at {p.at} ({p.dirs.join(', ')})
            {notes.map((n) => (
              <span key={n}>
                {' · '}
                <Info size={11} className="inline -mt-0.5" aria-hidden="true" /> {n}
              </span>
            ))}
          </>
        )}
        {p.kind === 'failed' && (
          <>
            Couldn’t clean build output: {p.reason}. Nothing else was touched.{' · '}
            <button type="button" disabled={busy} onClick={() => clean(w)} className={link} data-wt-retry>
              Try again
            </button>
          </>
        )}
      </span>
    </p>
  );
};

const Loading: React.FC = () => (
  <div data-wt-loading className="rounded-[14px] border border-hairline bg-surface-raised px-3.5 py-3.5">
    <p role="status" className="flex items-center gap-2 text-[13px] text-zinc-300">
      <Loader2 size={13} className="motion-safe:animate-spin" aria-hidden="true" /> Reading worktrees…
    </p>
    <div className="mt-3 space-y-2" aria-hidden="true">
      {[62, 40, 58].map((w, i) => (
        <span key={i} className={`kw-dv-skel-bar block h-2.5 ${i === 2 ? 'mt-4' : ''}`} style={{ width: `${w}%` }} />
      ))}
    </div>
  </div>
);

const EmptyTerm: React.FC<{ out: string; title: string; line: string; data: string; children?: React.ReactNode }> = ({ out, title, line, data, children }) => (
  <div className="rounded-2xl border border-dashed border-hairline-strong px-5 py-6" data-dev-empty data-wt-empty={data}>
    <p className="font-mono text-[12px] text-zinc-500 break-words" aria-hidden="true">
      <span className="text-sky-300">$</span> git worktree list
      <br />
      {out}
      <span className="kw-dv-intro-caret" />
    </p>
    <p className="mt-3 text-body font-medium text-zinc-200">{title}</p>
    <p className="mt-1 text-meta text-zinc-400">{line}</p>
    {children}
  </div>
);

// ---------------------------------------------------------------------------
// Dialogs: a centred 520 px box on desktop, a bottom sheet on the phone. Focus is trapped; Esc closes.

const Dialog: React.FC<{ role: 'dialog' | 'alertdialog'; labelledBy: string; describedBy?: string; phone: boolean; onClose: () => void; busy?: boolean; children: React.ReactNode; data: string }> = ({ role, labelledBy, describedBy, phone, onClose, busy, children, data }) => {
  const box = useRef<HTMLDivElement>(null);
  const onKeyDown = (e: React.KeyboardEvent) => {
    if (e.key === 'Escape') {
      e.stopPropagation();
      if (!busy) onClose();
      return;
    }
    if (e.key !== 'Tab' || !box.current) return;
    const items = Array.from(box.current.querySelectorAll<HTMLElement>(FOCUSABLE));
    const to = trapFocusTarget(items.length, items.indexOf(document.activeElement as HTMLElement), e.shiftKey);
    if (to !== null) {
      e.preventDefault();
      items[to]?.focus();
    }
  };
  return (
    <div className={`fixed inset-0 z-[60] flex ${phone ? 'items-end' : 'items-center justify-center p-4'} bg-black/60 backdrop-blur-sm`} onMouseDown={(e) => e.target === e.currentTarget && !busy && onClose()} data-wt-dialog-backdrop>
      <div
        ref={box}
        role={role}
        aria-modal="true"
        aria-labelledby={labelledBy}
        aria-describedby={describedBy}
        onKeyDown={onKeyDown}
        data-wt-dialog={data}
        className={
          phone
            ? 'kw-sheet w-full max-h-[90vh] overflow-y-auto rounded-t-2xl border-t border-zinc-800 bg-zinc-900 px-4 pt-4 pb-[calc(1rem+env(safe-area-inset-bottom))] shadow-2xl'
            : 'w-full max-w-[520px] max-h-[90vh] overflow-y-auto rounded-2xl border border-hairline-strong bg-surface-overlay p-5 shadow-2xl'
        }
      >
        {children}
      </div>
    </div>
  );
};

const btnRow = (phone: boolean) => (phone ? 'mt-5 grid grid-cols-2 gap-2 [&>button]:h-[46px] [&>button]:justify-center' : 'mt-5 flex justify-end gap-2');

const CreateDialog: React.FC<{
  space: string;
  repo: WorktreeRepo;
  bots: WtBot[];
  defaultBotId: string | null;
  home: string | null;
  phone: boolean;
  existing: Worktree[];
  onClose: () => void;
  onCreated: (w: Worktree) => void;
}> = ({ space, repo, bots, defaultBotId, home, phone, existing, onClose, onCreated }) => {
  const ids = { title: useId(), sub: useId(), branch: useId(), hint: useId(), err: useId(), base: useId(), pick: useId() };
  const forRef = useRef<HTMLSelectElement>(null);
  const [bot, setBot] = useState<string>(() => (defaultBotId && bots.some((b) => b.id === defaultBotId) ? defaultBotId : bots[0]?.id ?? ''));
  const botName = bots.find((b) => b.id === bot)?.name ?? null;
  const [mode, setMode] = useState<'new' | 'existing'>('new');
  const [branch, setBranch] = useState(() => suggestBranch(botName, null));
  const [touched, setTouched] = useState(false);
  const [base, setBase] = useState(repo.default_branch ?? 'main');
  const [pick, setPick] = useState('');
  const [branches, setBranches] = useState<BranchInfo[] | null>(null);
  const [saving, setSaving] = useState(false);
  const [serverErr, setServerErr] = useState<{ field: boolean; text: string } | null>(null);

  useEffect(() => forRef.current?.focus(), []);
  useEffect(() => {
    if (!repo.team_id) return;
    let live = true;
    worktreeApi.branches(space, repo.team_id).then(
      (b) => live && setBranches(b),
      () => live && setBranches(null),
    );
    return () => {
      live = false;
    };
  }, [space, repo.team_id]);
  // The suggestion follows the bot until the person types their own name.
  useEffect(() => {
    if (!touched) setBranch(suggestBranch(botName, null));
  }, [botName, touched]);

  const pickedRemote = mode === 'existing' && branches?.find((b) => b.remote && b.name === pick) ? pick : null;
  const name = (mode === 'new' ? branch : pickedRemote ? localNameOf(pick) : pick).trim();
  const taken = useMemo(() => {
    const w = existing.find((x) => x.branch === name && x.state !== 'removed');
    if (w) return w.path;
    return branches?.find((b) => b.name === name)?.checked_out_at ?? null;
  }, [existing, branches, name]);
  const slug = branchSlug(name);
  const folderPath = `${shortPath(existing[0]?.path.replace(/\/[^/]+\/[^/]+$/, '') ?? '', home) || '…/worktrees'}/${repo.repo}/${slug}`;
  const folderTaken = existing.some((x) => x.folder.endsWith(`/${slug}`) && x.branch !== name);
  const exists = mode === 'new' && !!branches?.some((b) => !b.remote && b.name === name);
  const problem = branchProblem(name);
  const localErr = !name
    ? null
    : problem?.kind === 'spaces'
      ? `Branch names can’t have spaces. Try ${problem.suggestion}.`
      : problem
        ? 'That isn’t a valid branch name.'
        : taken
          ? `${name} already has a worktree: ${shortPath(taken, home)}.`
          : folderTaken
            ? `${folderPath} already exists and isn’t empty. kwant won’t write into it.`
            : exists
              ? `${name} already exists. Pick Existing branch to use it, or choose another name.`
              : null;
  const fieldErr = localErr ?? (serverErr?.field ? serverErr.text : null);
  const ok = !!name && !fieldErr && !saving && (mode === 'new' ? !!base.trim() : true);

  const submit = () => {
    if (!ok || !repo.team_id) return;
    setSaving(true);
    setServerErr(null);
    // A bot's main 1:1 has no thread id and v1 keeps it in the team folder, so the worktree gets its own
    // chat on that bot, titled with the branch (Lin, #486). Until `new_thread: true` lands there, make the
    // thread first; if create then fails, the empty thread stays (harmless).
    const fail = (e: unknown) => {
      setSaving(false);
        setServerErr(createErrorCopy({ code: e instanceof WorktreeError ? e.code : null, message: e instanceof Error ? e.message : 'it failed' }, { branch: name, path: folderPath, checkedOutAt: taken }));
    };
    const team = repo.team_id;
    (bot ? threadsApi.create(bot, name).then((t) => t.id) : Promise.resolve(null))
      .then((thread) => worktreeApi.create(space, createBodyFor({ team, mode, name, base: base.trim(), pickedRemote, bot, thread })))
      .then(onCreated, fail);
  };

  const field = 'w-full h-10 rounded-lg border bg-black/30 px-3 text-[13.5px] text-zinc-100 placeholder:text-zinc-600 focus:outline-none focus-visible:ring-2 focus-visible:ring-sky-300';
  const seg = (on: boolean) => `text-left rounded-xl border px-3 py-2 ${on ? 'border-violet-400/70 bg-violet-500/15' : 'border-hairline-strong hover:bg-white/[0.04]'} ${ring}`;
  const localBranches = branches?.filter((b) => !b.remote) ?? [];

  return (
    <Dialog role="dialog" labelledBy={ids.title} describedBy={ids.sub} phone={phone} onClose={onClose} busy={saving} data="create">
      <h2 id={ids.title} className="flex items-center gap-2 text-[17px] font-semibold text-zinc-50">
        <FolderGit2 size={17} aria-hidden="true" /> New worktree in {repo.repo}
      </h2>
      <p id={ids.sub} className="mt-1 text-[13px] text-zinc-400">
        A separate folder for one bot’s task, so bots don’t work in each other’s files.
      </p>
      <form
        className="mt-4 space-y-4"
        onSubmit={(e) => {
          e.preventDefault();
          submit();
        }}
      >
        <label className="block">
          <span className="block mb-1.5 text-[13px] font-medium text-zinc-200">For</span>
          <select ref={forRef} value={bot} onChange={(e) => setBot(e.target.value)} className={`${field} border-hairline-strong`} data-wt-for>
            {bots.map((b) => (
              <option key={b.id} value={b.id}>
                {b.name}
              </option>
            ))}
            <option value="">Nobody (just me)</option>
          </select>
        </label>
        <fieldset>
          <legend className="mb-1.5 text-[13px] font-medium text-zinc-200">Start from</legend>
          <div className="grid grid-cols-2 gap-2" role="radiogroup">
            {(
              [
                ['new', 'New branch', 'from a base branch'],
                ['existing', 'Existing branch', 'check out one that’s there'],
              ] as const
            ).map(([k, t, s]) => (
              <button key={k} type="button" role="radio" aria-checked={mode === k} onClick={() => (setMode(k), setServerErr(null))} className={seg(mode === k)} data-wt-mode={k}>
                <span className="block text-[13.5px] font-semibold text-zinc-100">{t}</span>
                <span className="block text-[12.5px] text-zinc-400">{s}</span>
              </button>
            ))}
          </div>
        </fieldset>
        {mode === 'new' ? (
          <div className={phone ? 'space-y-3' : 'grid grid-cols-[1fr_150px] gap-3'}>
            <div>
              <label htmlFor={ids.branch} className="block mb-1.5 text-[13px] font-medium text-zinc-200">
                New branch
              </label>
              <input
                id={ids.branch}
                value={branch}
                onChange={(e) => (setBranch(e.target.value), setTouched(true), setServerErr(null))}
                spellCheck={false}
                autoCapitalize="off"
                autoComplete="off"
                aria-invalid={!!fieldErr || undefined}
                aria-describedby={fieldErr ? `${ids.err} ${ids.hint}` : ids.hint}
                className={`${field} font-mono ${fieldErr ? 'border-rose-400/70' : 'border-violet-400/60'}`}
                data-wt-branch-input
              />
              {fieldErr && (
                <p id={ids.err} className="mt-1.5 text-[12.5px] text-rose-300 break-words" data-wt-field-error>
                  {fieldErr}
                </p>
              )}
              <p id={ids.hint} className="mt-1 text-[12px] text-zinc-500">
                Lowercase, no spaces. kwant suggests one from the task.
              </p>
            </div>
            <div>
              <label htmlFor={ids.base} className="block mb-1.5 text-[13px] font-medium text-zinc-200">
                From
              </label>
              {localBranches.length ? (
                <select id={ids.base} value={base} onChange={(e) => setBase(e.target.value)} className={`${field} font-mono border-hairline-strong`} data-wt-base>
                  {localBranches.map((b) => (
                    <option key={b.name} value={b.name}>
                      {b.name}
                    </option>
                  ))}
                </select>
              ) : (
                <input id={ids.base} value={base} onChange={(e) => setBase(e.target.value)} spellCheck={false} autoCapitalize="off" className={`${field} font-mono border-hairline-strong`} data-wt-base />
              )}
            </div>
          </div>
        ) : (
          <div>
            <label htmlFor={ids.pick} className="block mb-1.5 text-[13px] font-medium text-zinc-200">
              Branch
            </label>
            {branches && branches.length ? (
              <select id={ids.pick} value={pick} onChange={(e) => (setPick(e.target.value), setServerErr(null))} aria-invalid={!!fieldErr || undefined} aria-describedby={fieldErr ? ids.err : undefined} className={`${field} font-mono border-hairline-strong`} data-wt-pick>
                <option value="">Pick a branch</option>
                {pickableBranches(branches).map((b) => (
                  <option key={`${b.remote}-${b.name}`} value={b.name} disabled={!!b.checked_out_at}>
                    {b.name}
                    {b.remote ? ' — new local branch' : ''}
                    {b.checked_out_at ? ` — checked out at ${shortPath(b.checked_out_at, home)}` : ''}
                  </option>
                ))}
              </select>
            ) : (
              <input id={ids.pick} value={pick} onChange={(e) => (setPick(e.target.value), setServerErr(null))} placeholder="the branch name" spellCheck={false} autoCapitalize="off" aria-invalid={!!fieldErr || undefined} aria-describedby={fieldErr ? ids.err : undefined} className={`${field} font-mono border-hairline-strong`} data-wt-pick />
            )}
            {fieldErr && (
              <p id={ids.err} className="mt-1.5 text-[12.5px] text-rose-300 break-words" data-wt-field-error>
                {fieldErr}
              </p>
            )}
          </div>
        )}
        {name && !problem && (
          <div className="rounded-lg border border-hairline bg-black/30 px-3 py-2 font-mono text-[12px] leading-5 text-zinc-300 break-all" data-wt-preview>
            <div>
              <span className="text-zinc-500">folder</span> {folderPath}
            </div>
            <div>
              <span className="text-zinc-500">branch</span> {name} {mode === 'new' ? `(new, from ${base.trim() || '…'})` : pickedRemote ? `(new, from ${pickedRemote})` : '(existing)'}
            </div>
          </div>
        )}
        {serverErr && !serverErr.field && (
          <p role="alert" className="text-[12.5px] text-rose-300" data-wt-dialog-error>
            {serverErr.text}
          </p>
        )}
        <div className={btnRow(phone)}>
          <button type="button" onClick={onClose} disabled={saving} className={`kw-btn-ghost ${ring}`}>
            Cancel
          </button>
          <button type="submit" disabled={!ok} className={`kw-btn-primary ${ring}`} data-wt-create>
            {saving ? (
              <>
                <Loader2 size={14} className="motion-safe:animate-spin" aria-hidden="true" /> Creating…
              </>
            ) : (
              'Create worktree'
            )}
          </button>
        </div>
      </form>
    </Dialog>
  );
};

const RemoveDialog: React.FC<{ w: Worktree; bot: string | null; home: string | null; phone: boolean; onClose: () => void; onRemoved: (w: Worktree) => void }> = ({ w, bot, home, phone, onClose, onRemoved }) => {
  const ids = { title: useId(), body: useId(), box: useId() };
  const cancelRef = useRef<HTMLButtonElement>(null);
  const [dirty, setDirty] = useState<{ n: number; changes: Worktree['changes'] }>({ n: w.dirty ?? 0, changes: w.changes });
  const [force, setForce] = useState(false);
  const [saving, setSaving] = useState(false);
  const [err, setErr] = useState<string | null>(null);
  useEffect(() => cancelRef.current?.focus(), []);
  const leftover = w.state === 'leftover';
  const n = dirty.n;
  const needsForce = n > 0 && !leftover;
  const go = () => {
    if (saving || (needsForce && !force)) return;
    setSaving(true);
    setErr(null);
    worktreeApi.remove(w.id, needsForce && force).then(
      () => onRemoved(w),
      (e: unknown) => {
        setSaving(false);
        if (e instanceof WorktreeError && e.code === 'dirty') {
          // The checkout changed since the list was read: show what the server found and ask again.
          const b = e.body ?? {};
          const changes = Array.isArray(b.changes) ? (b.changes as Worktree['changes']) : [];
          setDirty({ n: typeof b.dirty === 'number' ? b.dirty : changes.length, changes });
          setForce(false);
          return;
        }
        setErr(removeErrorCopy({ code: e instanceof WorktreeError ? e.code : null, message: e instanceof Error ? e.message : 'it failed' }, bot));
      },
    );
  };
  const commits = w.ahead && w.ahead > 0 ? `, with its ${w.ahead === 1 ? '1 commit' : `${w.ahead} commits`}` : '';
  return (
    <Dialog role="alertdialog" labelledBy={ids.title} describedBy={ids.body} phone={phone} onClose={onClose} busy={saving} data="remove">
      <h2 id={ids.title} className="flex items-center gap-2 text-[17px] font-semibold text-zinc-50">
        <Trash2 size={16} aria-hidden="true" /> {leftover ? 'Remove this leftover folder?' : bot ? `Remove ${bot}’s worktree?` : 'Remove this worktree?'}
      </h2>
      <p id={ids.body} className="mt-1.5 text-[13px] leading-relaxed text-zinc-400">
        This deletes the folder <code className="rounded bg-black/40 px-1.5 py-0.5 font-mono text-[12px] text-zinc-200 break-all">{shortPath(w.path, home)}</code>. The branch <strong className="font-semibold text-zinc-100">{w.branch}</strong> stays{commits}.
      </p>
      {needsForce ? (
        <>
          <p className="mt-4 text-[13px] font-semibold text-zinc-100" data-wt-dirty-head>
            {n === 1 ? '1 file has changes nobody committed. Removing loses it.' : `${n} files have changes nobody committed. Removing loses them.`}
          </p>
          <ul className="mt-2 rounded-lg border border-hairline bg-black/30 px-3 py-2 font-mono text-[12px] leading-[21px] text-zinc-300" data-wt-changes>
            {dirty.changes.slice(0, 8).map((c) => (
              <li key={c.path} className="break-all">
                <span className={c.status === '?' ? 'text-zinc-500' : 'text-amber-300'}>{c.status === '??' ? '?' : c.status}</span> {c.path}
              </li>
            ))}
            {n > Math.min(8, dirty.changes.length) && <li className="text-zinc-500">and {n - Math.min(8, dirty.changes.length)} more</li>}
          </ul>
          <label className="mt-3 flex items-center gap-2.5 text-[13px] text-zinc-200 min-h-[32px] cursor-pointer">
            <input type="checkbox" checked={force} onChange={(e) => setForce(e.target.checked)} className={`h-[18px] w-[18px] rounded accent-rose-600 ${ring}`} data-wt-force />
            {n === 1 ? 'Remove anyway and lose this change' : `Remove anyway and lose these ${n} changes`}
          </label>
        </>
      ) : (
        <p className="mt-4 text-[13px] text-zinc-300">{leftover ? 'Git doesn’t list this folder any more.' : 'No uncommitted changes.'}</p>
      )}
      {err && (
        <p role="alert" className="mt-3 text-[12.5px] text-rose-300" data-wt-dialog-error>
          {err}
        </p>
      )}
      <div className={btnRow(phone)}>
        <button ref={cancelRef} type="button" onClick={onClose} disabled={saving} className={`kw-btn-ghost ${ring}`} data-wt-cancel>
          Cancel
        </button>
        <button
          type="button"
          onClick={go}
          disabled={saving || (needsForce && !force)}
          className={`inline-flex items-center gap-1.5 rounded-lg px-3.5 h-9 text-[13px] font-medium text-white bg-rose-700 hover:bg-rose-600 disabled:bg-rose-900/60 disabled:text-rose-200/50 disabled:cursor-not-allowed ${ring}`}
          data-wt-remove
        >
          {saving && <Loader2 size={14} className="motion-safe:animate-spin" aria-hidden="true" />}
          {leftover ? 'Remove folder' : 'Remove worktree'}
        </button>
      </div>
    </Dialog>
  );
};

/** Phone: the row's ⋯ opens these actions as a bottom sheet. */
const RowSheet: React.FC<{
  w: Worktree;
  bot: string | null;
  onClose: () => void;
  onOpenChat?: (botId: string, threadId: string | null) => void;
  copyPath: (p: string) => void;
  keep: (w: Worktree, on: boolean) => void;
  clean: (w: Worktree) => void;
  openRemove: (w: Worktree) => void;
}> = ({ w, bot, onClose, onOpenChat, copyPath, keep, clean, openRemove }) => {
  const title = useId();
  const first = useRef<HTMLButtonElement>(null);
  useEffect(() => first.current?.focus(), []);
  const item = 'w-full flex items-center gap-3 rounded-xl px-3 min-h-[48px] text-left text-[15px] text-zinc-100 hover:bg-white/[0.05] disabled:opacity-40';
  const p3 = p3State(w);
  const who = bot ?? 'You';
  const act = (f: () => void) => () => {
    f();
    onClose();
  };
  return (
    <Dialog role="dialog" labelledBy={title} phone onClose={onClose} data="row-menu">
      <h2 id={title} className="px-3 pb-2 text-[13px] font-medium text-zinc-400">
        {bot ? `${bot}’s worktree` : 'Your worktree'} · <span className="font-mono">{w.branch}</span>
      </h2>
      {w.kwant_id && onOpenChat && (
        <button ref={first} type="button" className={`${item} ${ring}`} onClick={act(() => onOpenChat(w.kwant_id as string, w.thread_id))}>
          <MessageSquare size={17} aria-hidden="true" /> Open chat
        </button>
      )}
      <button ref={w.kwant_id && onOpenChat ? undefined : first} type="button" className={`${item} ${ring}`} onClick={act(() => copyPath(w.path))}>
        <Copy size={17} aria-hidden="true" /> Copy path
      </button>
      {p3?.kind === 'due' && (
        <button type="button" className={`${item} ${ring}`} onClick={act(() => keep(w, true))}>
          <Clock size={17} aria-hidden="true" /> Keep build output
        </button>
      )}
      {(p3?.kind === 'kept' || p3?.kind === 'due') && (
        <button type="button" disabled={w.working} className={`${item} ${ring}`} onClick={act(() => clean(w))}>
          <Eraser size={17} aria-hidden="true" /> Clean now
        </button>
      )}
      <button type="button" disabled={w.working} className={`${item} text-rose-300 ${ring}`} onClick={() => openRemove(w)} aria-describedby={w.working ? `${title}-why` : undefined}>
        <Trash2 size={17} aria-hidden="true" /> Remove
      </button>
      {w.working && (
        <p id={`${title}-why`} className="px-3 pt-1 text-[12.5px] text-zinc-400">
          {who} is working here. Stop {who} first.
        </p>
      )}
    </Dialog>
  );
};
