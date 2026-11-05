import React, { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import { ArrowLeft, ArrowRight, Check, ChevronDown, Clock, Eye, Inbox, Settings, Sparkles, SquareKanban } from 'lucide-react';
import {
  type Idea,
  type IdeaList,
  type IdeaTake,
  IdeaError,
  type SectionKey,
  SECTION_ORDER,
  SECTION_TITLE,
  countIdeas,
  emitIdeaUpdated,
  forSpace,
  groupIdeas,
  ideasApi,
  jarNotice,
  lookRefusalText,
  onIdeaUpdated,
  parseIdea,
  peeksLeft,
  publishReadyCount,
  takeArrivedText,
  clip,
} from '../../ideas';
import { AvatarsContext, UndoToast } from '../MissionBoardView';
import { Sticker } from '../Sticker';
import { usePhoneLayout } from '../../usePhoneLayout';
import { JarArt, JarIcon } from './JarArt';
import { IdeaQuickAdd, type IdeaQuickAddHandle } from './IdeaQuickAdd';
import { IdeaSlip, type SlipBusy } from './IdeaSlip';
import { type PickState, TakeCard, TakeRow } from './TakeCard';
import { InfoLine, btn, focusRing } from './parts';

/** A bot in the space, for names ("Lin is peeking") and the "Who should do it" card. */
export interface IdeaJarBot {
  id: string;
  name: string;
  role?: string | null;
}

export interface IdeaJarViewProps {
  spaceId: string;
  /** Open on board: the board with this (draft) mission selected. */
  onOpenMission: (missionId: string) => void;
  /** For copy like "Bots have had their 3 peeks for today in Trailhead". */
  spaceName?: string;
  /** The space's bots; without them the view says "A bot". */
  bots?: IdeaJarBot[];
  /** Missions App already has, for the Picked chips (by mission_id). */
  missions?: { id: string; status: string }[];
  /** Header Settings and "Turn on in Settings": open Settings focused on the idea-jar row. */
  onOpenSettings?: () => void;
  /** kwant id → avatar URL, as the board gets them. Initials without. */
  avatars?: Record<string, string>;
}

type Busy = SlipBusy | 'pick';
type Notice = { tone: 'sky' | 'rose'; text: string };
type Toast = { kind: 'later' | 'delete'; id: string; text: string };
type Ready = Idea & { take: IdeaTake };

const SECTION_ICON: Record<SectionKey, React.ReactNode> = {
  ready: <Sparkles size={15} aria-hidden="true" />,
  peeking: <Eye size={15} aria-hidden="true" />,
  jar: <JarIcon size={15} />,
  later: <Clock size={15} aria-hidden="true" />,
  picked: <SquareKanban size={15} aria-hidden="true" />,
};
const PICKED_FOLD = 3;
const TOAST_MS = 6000;

const MISSION_CHIP: Record<string, { cls: string; text: string }> = {
  draft: { cls: 'kw-chip kw-chip-warn', text: 'Waiting for your OK' },
  active: { cls: 'kw-chip kw-chip-info', text: 'Working' },
  done: { cls: 'kw-chip kw-chip-ok', text: 'Done' },
};

const isReady = (i: Idea): i is Ready => i.status === 'ready' && !!i.take;

/**
 * The idea jar (spec §2–§5): the person drops ideas in, free bots peek and write a read-only take, and
 * Do it makes a draft mission. Plain text only; numbers shown are the server's.
 */
export const IdeaJarView: React.FC<IdeaJarViewProps> = ({ spaceId, onOpenMission, spaceName, bots = [], missions = [], onOpenSettings, avatars }) => {
  const phone = usePhoneLayout();
  const [list, setList] = useState<IdeaList | null>(null);
  const [load, setLoad] = useState<'loading' | 'error' | 'ok'>('loading');
  const [slow, setSlow] = useState(false);
  const [now, setNow] = useState(() => Date.now());
  const [busy, setBusy] = useState<Record<string, Busy>>({});
  const [notice, setNotice] = useState<Record<string, Notice>>({});
  const [pick, setPick] = useState<Record<string, PickState>>({});
  /** Just picked this session: the card keeps its done panel in place (focus lands on Open on board). */
  const [kept, setKept] = useState<Record<string, Ready>>({});
  const [editing, setEditing] = useState<string | null>(null);
  const [editError, setEditError] = useState<string | null>(null);
  const [hidden, setHidden] = useState<Set<string>>(() => new Set());
  const [toast, setToast] = useState<Toast | null>(null);
  const [openId, setOpenId] = useState<string | null>(null);
  const [phoneTake, setPhoneTake] = useState<string | null>(null);
  const [fresh, setFresh] = useState<Set<string>>(() => new Set());
  const [laterOpen, setLaterOpen] = useState(false);
  const [pickedAll, setPickedAll] = useState(false);
  const [live, setLive] = useState('');
  const [drop, setDrop] = useState(0);
  const [learned, setLearned] = useState<Record<string, string>>({});
  const root = useRef<HTMLDivElement | null>(null);
  const quick = useRef<IdeaQuickAddHandle | null>(null);
  const openRef = useRef<HTMLButtonElement | null>(null);
  /** Where focus goes after the next commit. `from` is the card the action came from: focus only moves if it is
   * on body or still inside that card, never from wherever the person went meanwhile (M1). */
  const focusNext = useRef<{ kind: 'open'; from: string } | { kind: 'sel'; sel: string[]; from?: string } | null>(null);
  const editFrom = useRef<'button' | 'menu'>('button');
  const prevStatus = useRef<Map<string, string>>(new Map());
  const loaded = useRef(false);
  const expandedRef = useRef<string | null>(null);
  const toastRef = useRef<Toast | null>(null);
  toastRef.current = toast;

  const names = useMemo(() => {
    const m: Record<string, string> = { ...learned };
    for (const b of bots) if (b && b.id && b.name) m[b.id] = b.name;
    return m;
  }, [bots, learned]);
  const roles = useMemo(() => Object.fromEntries(bots.filter((b) => b.role).map((b) => [b.id, b.role as string])), [bots]);
  const botNames = useMemo(() => bots.map((b) => b.name).filter(Boolean), [bots]);
  const nameOf = useCallback((id: string | null) => (id ? (names[id] ?? null) : null), [names]);

  // ---- data
  const fetchList = useCallback(
    (quiet = false) => {
      if (!quiet) {
        setLoad((l) => (l === 'ok' ? l : 'loading'));
        setSlow(false);
      }
      return ideasApi.list(spaceId).then(
        (l) => {
          setList(l);
          setLoad('ok');
        },
        () => {
          if (!quiet || !loaded.current) setLoad('error');
        },
      );
    },
    [spaceId],
  );

  useEffect(() => {
    loaded.current = false;
    prevStatus.current = new Map();
    setList(null);
    setLoad('loading');
    void fetchList();
  }, [fetchList]);

  useEffect(() => {
    if (load !== 'loading') return;
    const t = setTimeout(() => setSlow(true), 10_000);
    return () => clearTimeout(t);
  }, [load]);

  useEffect(() => {
    const t = setInterval(() => setNow(Date.now()), 30_000);
    return () => clearInterval(t);
  }, []);

  // Live: upsert what this tab knows, refetch (debounced) for the rest.
  useEffect(() => {
    let timer: ReturnType<typeof setTimeout> | null = null;
    const off = onIdeaUpdated((e) => {
      if (!forSpace(e, spaceId)) return;
      if (e.status === 'deleted') {
        setList((l) => (l ? withIdeas(l, l.ideas.filter((i) => i.id !== e.id)) : l));
        return;
      }
      if (e.idea) {
        const idea = parseIdea(e.idea);
        if (idea) setList((l) => (l ? upsert(l, idea) : l));
      }
      if (timer) clearTimeout(timer);
      timer = setTimeout(() => void fetchList(true), 250);
    });
    return () => {
      if (timer) clearTimeout(timer);
      off();
    };
  }, [spaceId, fetchList]);

  const ideas = useMemo(() => (list ? list.ideas.filter((i) => !hidden.has(i.id)) : []), [list, hidden]);

  // Badge, "New" chips and the live region, from status changes.
  useEffect(() => {
    if (!list) return;
    // A just-picked idea stays in Takes ready for the session, but it is not a take waiting any more (ruling 2).
    publishReadyCount(spaceId, countIdeas(ideas.filter((i) => !kept[i.id])).ready);
    const prev = prevStatus.current;
    const first = !loaded.current;
    const arrived: Ready[] = [];
    const failed: Idea[] = [];
    for (const i of ideas) {
      const was = prev.get(i.id);
      if (!first && was !== i.status) {
        if (isReady(i) && was !== 'later') arrived.push(i);
        if (i.status === 'failed' && was === 'peeking') failed.push(i);
      }
    }
    prevStatus.current = new Map(ideas.map((i) => [i.id, i.status]));
    loaded.current = true;
    if (arrived.length) {
      // Keep the open card open: a take that arrives waits as a row with its New chip (no layout jump).
      const wasOpen = expandedRef.current;
      if (wasOpen && !arrived.some((a) => a.id === wasOpen)) setOpenId((o) => o ?? wasOpen);
      setFresh((f) => new Set([...f, ...arrived.map((i) => i.id)]));
      setLive(takeArrivedText(nameOf(arrived[0].kwant_id), arrived[0].take, arrived[0].text));
    } else if (failed.length) {
      setLive(`${nameOf(failed[0].kwant_id) ?? 'A bot'} couldn’t read “${clip(failed[0].text)}”. It’s back in the jar.`);
    }
  }, [list, ideas, kept, spaceId, nameOf]);

  const groups = useMemo(() => {
    const g = groupIdeas(ideas.filter((i) => !kept[i.id]));
    const keptOnes = Object.values(kept).filter((k) => !hidden.has(k.id));
    if (keptOnes.length) g.ready = [...keptOnes, ...g.ready];
    return g;
  }, [ideas, kept, hidden]);

  // Desktop: the first ready take is open; opening another closes it.
  const expanded = useMemo(() => {
    if (openId && groups.ready.some((i) => i.id === openId)) return openId;
    return groups.ready[0]?.id ?? null;
  }, [openId, groups.ready]);

  // The card open as of the last commit (read by the arrival effect above, which runs first).
  useEffect(() => {
    expandedRef.current = expanded;
  });

  // Focus after a card leaves, or after Do it.
  useEffect(() => {
    const f = focusNext.current;
    if (!f) return;
    focusNext.current = null;
    const a = document.activeElement;
    const from = f.from ? root.current?.querySelector(f.from) : null;
    if (a && a !== document.body && !from?.contains(a)) return;
    if (f.kind === 'open') openRef.current?.focus();
    else {
      for (const sel of f.sel) {
        const el = root.current?.querySelector<HTMLElement>(sel);
        if (el) {
          el.focus();
          break;
        }
      }
    }
  });

  const setB = (id: string, b: Busy) => setBusy((m) => ({ ...m, [id]: b }));
  const clearB = (id: string) => setBusy(({ [id]: _, ...m }) => m);
  const setN = (id: string, n: Notice | null) => setNotice(({ [id]: _, ...m }) => (n ? { ...m, [id]: n } : m));
  const replace = (idea: Idea) => setList((l) => (l ? upsert(l, idea) : l));

  /** Where focus goes when `id` leaves its section: the next card's first button, else the heading. */
  const leaving = (id: string) => {
    const sec = SECTION_ORDER.find((k) => groups[k].some((i) => i.id === id));
    if (!sec) return;
    const rest = groups[sec].filter((i) => i.id !== id);
    const at = groups[sec].findIndex((i) => i.id === id);
    const next = rest[Math.min(at, rest.length - 1)];
    const after = SECTION_ORDER.slice(SECTION_ORDER.indexOf(sec)).map((k) => `#jar-sec-${k}`);
    focusNext.current = { kind: 'sel', sel: [...(next ? firstAction(next.id) : []), ...after, '[data-section] h3 > *', '[data-quick-add] input'] };
  };

  /** A card's first action: Do it, else Look (S1), else Open or any enabled button. */
  const firstAction = (id: string) => {
    const c = `[data-card="${cssId(id)}"]`;
    return [`${c} [data-action="do-it"]`, `${c} [data-action="look"]`, `${c} [data-action="open"]`, `${c} button:not([disabled])`];
  };

  // ---- actions
  const look = async (idea: Idea) => {
    if (busy[idea.id]) return;
    setB(idea.id, 'look');
    setN(idea.id, null);
    try {
      const r = await ideasApi.look(idea.id);
      if (r.kwant_id && r.kwant_name) setLearned((m) => ({ ...m, [r.kwant_id as string]: r.kwant_name as string }));
      if (kept[idea.id]) return;
      replace({ ...idea, status: 'peeking', kwant_id: r.kwant_id, peeked_at: new Date().toISOString(), take: null, error: null });
      emitIdeaUpdated({ id: idea.id, status: 'peeking', space_id: spaceId });
    } catch (e) {
      // A 409 leaves the idea where it is (Ada, 11:12).
      if (e instanceof IdeaError && e.status === 409) {
        setN(idea.id, { tone: 'sky', text: lookRefusalText(e.code, nameOf(idea.kwant_id), e.message) });
        if (e.code === 'already_peeking' || e.code === 'picked') void fetchList(true);
      } else setN(idea.id, { tone: 'rose', text: e instanceof IdeaError && e.status === 403 ? e.message : 'Couldn’t ask a bot just now. Try again.' });
    } finally {
      clearB(idea.id);
    }
  };

  const doIt = async (idea: Ready) => {
    if (busy[idea.id] || pick[idea.id]?.kind === 'done') return;
    setB(idea.id, 'pick');
    setPick((m) => ({ ...m, [idea.id]: { kind: 'busy' } }));
    setN(idea.id, null);
    try {
      const result = await ideasApi.pick(idea.id);
      setKept((m) => ({ ...m, [idea.id]: idea }));
      setPick((m) => ({ ...m, [idea.id]: { kind: 'done', result } }));
      setLive(`Draft mission made${idea.take.bot ? ` for ${idea.take.bot}` : ''}. Nothing starts until you approve it.`);
      focusNext.current = { kind: 'open', from: `[data-take-card="${cssId(idea.id)}"]` };
      emitIdeaUpdated({ id: idea.id, status: 'picked', space_id: spaceId });
    } catch {
      setPick((m) => ({ ...m, [idea.id]: { kind: 'error' } }));
      void fetchList(true);
    } finally {
      clearB(idea.id);
    }
  };

  const later = async (idea: Idea) => {
    leaving(idea.id);
    setN(idea.id, null);
    replace({ ...idea, status: 'later', updated_at: new Date().toISOString() });
    setToast({ kind: 'later', id: idea.id, text: 'Moved to Later · Undo' });
    try {
      const r = await ideasApi.later(idea.id);
      replace(r);
    } catch {
      replace(idea);
      setToast(null);
      setN(idea.id, { tone: 'rose', text: 'Couldn’t move it. Try again.' });
    }
  };

  /** `undo`: from the toast, so focus goes to the restored card itself (S2), not to the next one in Later. */
  const restore = async (idea: Idea, undo = false) => {
    if (busy[idea.id]) return;
    setB(idea.id, 'restore');
    setN(idea.id, null);
    try {
      const r = await ideasApi.restore(idea.id);
      if (undo) focusNext.current = { kind: 'sel', sel: firstAction(idea.id) };
      else if (idea.status === 'later') leaving(idea.id);
      replace(r);
    } catch {
      setN(idea.id, { tone: 'rose', text: 'Couldn’t restore it. Try again.' });
    } finally {
      clearB(idea.id);
    }
  };

  const saveEdit = async (idea: Idea, text: string) => {
    setB(idea.id, 'edit');
    setEditError(null);
    try {
      const r = await ideasApi.edit(idea.id, text);
      setEditing(null);
      replace(r);
      setFresh((f) => {
        const n = new Set(f);
        n.delete(idea.id);
        return n;
      });
      focusNext.current = { kind: 'sel', sel: firstAction(idea.id) };
    } catch (e) {
      setEditError(
        e instanceof IdeaError && e.code === 'already_peeking'
          ? `${nameOf(idea.kwant_id) ?? 'A bot'} is peeking at this one. Try again when the take is in.`
          : e instanceof IdeaError && e.code === 'picked'
            ? 'This idea is already a mission.'
            : e instanceof IdeaError && (e.status === 400 || e.status === 403)
              ? e.message
              : 'Couldn’t save it. Try again.',
      );
    } finally {
      clearB(idea.id);
    }
  };

  const remove = (idea: Idea) => {
    leaving(idea.id);
    if (toastRef.current?.kind === 'delete') commitDelete(toastRef.current.id);
    setHidden((h) => new Set(h).add(idea.id));
    setToast({ kind: 'delete', id: idea.id, text: 'Idea deleted · Undo' });
    if (phoneTake === idea.id) setPhoneTake(null);
  };

  const unhide = (id: string) =>
    setHidden((h) => {
      const n = new Set(h);
      n.delete(id);
      return n;
    });

  const commitDelete = (id: string) => {
    ideasApi.remove(id).then(
      () => {
        setList((l) => (l ? withIdeas(l, l.ideas.filter((i) => i.id !== id)) : l));
        unhide(id);
        setKept(({ [id]: _, ...m }) => m);
        emitIdeaUpdated({ id, status: 'deleted', space_id: spaceId });
      },
      () => {
        unhide(id);
        setN(id, { tone: 'rose', text: 'Couldn’t delete it. Try again.' });
      },
    );
  };

  const closeToast = useCallback(() => {
    const t = toastRef.current;
    setToast(null);
    if (t?.kind === 'delete') commitDelete(t.id);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  const undoToast = () => {
    const t = toastRef.current;
    setToast(null);
    if (!t) return;
    if (t.kind === 'delete') {
      unhide(t.id);
      focusNext.current = { kind: 'sel', sel: firstAction(t.id) };
      return;
    }
    const idea = list?.ideas.find((i) => i.id === t.id);
    if (idea) void restore(idea, true);
  };

  useEffect(() => {
    if (!toast) return;
    const t = setTimeout(closeToast, TOAST_MS);
    return () => clearTimeout(t);
  }, [toast, closeToast]);

  // A pending delete still goes through when the view closes.
  useEffect(
    () => () => {
      const t = toastRef.current;
      if (t?.kind === 'delete') void ideasApi.remove(t.id).catch(() => undefined);
    },
    [],
  );

  const onDropped = (idea: Idea) => {
    replace(idea);
    setDrop((d) => d + 1);
    setLive(`Dropped in the jar: ${clip(idea.text)}.`);
  };

  const openTake = (id: string) => {
    setFresh((f) => {
      const n = new Set(f);
      n.delete(id);
      return n;
    });
    if (phone) setPhoneTake(id);
    else {
      setOpenId(id);
      focusNext.current = { kind: 'sel', sel: [`[data-card="${cssId(id)}"] [data-action="do-it"]`] };
    }
  };

  const startEdit = (id: string, from: 'button' | 'menu') => {
    editFrom.current = from;
    setEditing(id);
    setEditError(null);
  };

  /** Esc and Cancel put focus back on Edit, or on ⋯ when Edit came from the menu (M2). */
  const cancelEdit = (id: string) => {
    setEditing(null);
    setEditError(null);
    const c = `[data-card="${cssId(id)}"]`;
    const t = `[data-take-card="${cssId(id)}"]`;
    focusNext.current = {
      kind: 'sel',
      sel: editFrom.current === 'button' ? [`${t} [data-action="edit"]`, `${c} [data-idea-menu]`] : [`${t} [data-idea-menu]`, `${c} [data-idea-menu]`, `${t} [data-action="edit"]`],
    };
  };

  // ---- render pieces
  const takeCard = (idea: Ready, asPhone: boolean) => (
    <TakeCard
      idea={idea}
      authorName={nameOf(idea.kwant_id)}
      suggestedRole={idea.take.bot_id ? (roles[idea.take.bot_id] ?? null) : null}
      botNames={botNames}
      now={now}
      phone={asPhone}
      pick={pick[idea.id] ?? null}
      busy={busy[idea.id] === 'look' ? 'look' : busy[idea.id] === 'edit' ? 'edit' : null}
      notice={notice[idea.id] ?? null}
      editing={editing === idea.id}
      editError={editing === idea.id ? editError : null}
      onDoIt={() => void doIt(idea)}
      onLater={() => void later(idea)}
      onEdit={(from) => startEdit(idea.id, from)}
      onSaveEdit={(t) => void saveEdit(idea, t)}
      onCancelEdit={() => cancelEdit(idea.id)}
      onLook={() => void look(idea)}
      onDelete={() => remove(idea)}
      onOpenMission={onOpenMission}
      openRef={pick[idea.id]?.kind === 'done' ? openRef : undefined}
    />
  );

  const slip = (idea: Idea) => (
    <IdeaSlip
      idea={idea}
      botName={nameOf(idea.kwant_id)}
      botNames={botNames}
      now={now}
      busy={busy[idea.id] === 'pick' ? null : ((busy[idea.id] as SlipBusy) ?? null)}
      notice={notice[idea.id] ?? null}
      editing={editing === idea.id}
      editError={editing === idea.id ? editError : null}
      onLook={() => void look(idea)}
      onEdit={(from) => startEdit(idea.id, from)}
      onSaveEdit={(t) => void saveEdit(idea, t)}
      onCancelEdit={() => cancelEdit(idea.id)}
      onDelete={() => remove(idea)}
      onRestore={() => void restore(idea)}
    />
  );

  const pickedRow = (idea: Idea) => {
    const m = idea.mission_id ? missions.find((x) => x.id === idea.mission_id) : undefined;
    const chip = m ? MISSION_CHIP[m.status] : undefined;
    return (
      <div className="flex flex-wrap items-center gap-2 rounded-xl border border-white/[0.08] bg-white/[0.02] px-3 py-2 min-w-0" data-picked-row={idea.id}>
        <Sticker tone="holo" size="sm" icon={<Check aria-hidden="true" />}>
          Picked
        </Sticker>
        <span className="min-w-0 flex-1 basis-40 text-[13px] text-[#bcb7de] truncate">{idea.take?.title || idea.text}</span>
        {chip && (
          <span className={chip.cls} data-mission-chip={m!.status}>
            {chip.text}
          </span>
        )}
        {idea.mission_id && (
          <button type="button" onClick={() => onOpenMission(idea.mission_id as string)} className={btn.quiet} data-action="open-board">
            Open on board
            <ArrowRight size={14} aria-hidden="true" />
          </button>
        )}
      </div>
    );
  };

  const section = (key: SectionKey) => {
    const items = groups[key];
    if (!items.length) return null;
    const foldable = key === 'later' || (key === 'picked' && items.length > PICKED_FOLD);
    const open = key === 'later' ? laterOpen : true;
    const shown = key === 'picked' && !pickedAll ? items.slice(0, PICKED_FOLD) : items;
    const headId = `jar-sec-${key}`;
    const title = (
      <>
        <span className="text-[#9d8fff]">{SECTION_ICON[key]}</span>
        <span>{SECTION_TITLE[key]}</span>
        <span className="text-[#8d87b8] font-normal tabular-nums" data-count>
          {items.length}
        </span>
      </>
    );
    return (
      <section key={key} aria-labelledby={headId} className="flex flex-col gap-2.5" data-section={key}>
        <h3 className="m-0">
          {key === 'later' ? (
            <button
              id={headId}
              type="button"
              aria-expanded={open}
              onClick={() => setLaterOpen((v) => !v)}
              className={`flex items-center gap-2 min-h-[44px] sm:min-h-[32px] text-[13px] font-semibold text-[#ecebfa] rounded ${focusRing}`}
            >
              {title}
              <ChevronDown size={14} aria-hidden="true" className={`transition-transform motion-reduce:transition-none ${open ? 'rotate-180' : ''}`} />
            </button>
          ) : (
            <span id={headId} tabIndex={-1} className="flex items-center gap-2 text-[13px] font-semibold text-[#ecebfa] outline-none">
              {title}
            </span>
          )}
        </h3>
        {open && (
          <ul className="flex flex-col gap-2.5 list-none m-0 p-0">
            {shown.map((i) => (
              <li key={i.id} data-card={i.id} className="min-w-0">
                {key === 'ready' && isReady(i)
                  ? !phone && expanded === i.id
                    ? takeCard(i, false)
                    : <TakeRow idea={i} isNew={fresh.has(i.id)} botNames={botNames} authorName={nameOf(i.kwant_id)} onOpen={() => openTake(i.id)} />
                  : key === 'picked'
                    ? pickedRow(i)
                    : slip(i)}
              </li>
            ))}
          </ul>
        )}
        {foldable && key === 'picked' && (
          <button type="button" aria-expanded={pickedAll} onClick={() => setPickedAll((v) => !v)} className={`${btn.quiet} self-start`} data-picked-toggle>
            {pickedAll ? 'Show fewer' : `Show all ${items.length}`}
            <ChevronDown size={14} aria-hidden="true" className={pickedAll ? 'rotate-180' : ''} />
          </button>
        )}
      </section>
    );
  };

  const left = list ? peeksLeft(list) : null;
  const jn = list ? jarNotice(list) : null;
  const space = spaceName || 'this space';
  const empty = load === 'ok' && ideas.length === 0 && Object.keys(kept).length === 0;
  const inJar = groups.jar.length + groups.peeking.length + groups.ready.length;

  // Phone: a ready take opens on its own screen.
  const phoneIdea = phone && phoneTake ? (groups.ready.find((i) => i.id === phoneTake) as Ready | undefined) : undefined;
  if (phoneIdea) {
    return (
      <AvatarsContext.Provider value={avatars ?? {}}>
        <div ref={root} className="flex flex-col flex-1 min-h-0 min-w-0 w-full bg-[#120e26] text-[#ecebfa]" data-idea-jar data-take-screen={phoneIdea.id}>
          <div className="flex items-center gap-2 px-2 py-1.5 border-b border-white/[0.08]">
            <button type="button" onClick={() => setPhoneTake(null)} className={`${btn.quiet} !px-2`} data-action="back" autoFocus>
              <ArrowLeft size={16} aria-hidden="true" />
              Idea jar
            </button>
          </div>
          <div className="p-3 pb-0 flex-1 min-h-0 overflow-y-auto" data-jar-scroll>{takeCard(phoneIdea, true)}</div>
          <div aria-live="polite" className="sr-only" data-live>
            {live}
          </div>
        </div>
      </AvatarsContext.Provider>
    );
  }

  return (
    <AvatarsContext.Provider value={avatars ?? {}}>
      {/* Its own scroller: App mounts it in a flex-1 min-h-0 pane that doesn't scroll. */}
      <div className="flex-1 min-h-0 min-w-0 w-full overflow-y-auto" data-jar-scroll>
      <div ref={root} className="flex flex-col gap-5 min-h-full w-full max-w-[920px] mx-auto px-3 sm:px-6 py-4 sm:py-6 text-[#ecebfa] min-w-0" data-idea-jar>
        {/* Below sm: line 1 is the title and an icon-only Settings in the corner, line 2 the chips (ruling 3). */}
        <header className="flex flex-wrap items-center gap-2" data-jar-header>
          <h2 className="flex items-center gap-2 text-[17px] font-semibold text-[#f7f6ff] m-0">
            <JarIcon size={18} className="text-[#b8adff]" />
            Idea jar
          </h2>
          {(spaceName || (left !== null && list)) && (
            <div className="flex flex-wrap items-center gap-2 min-w-0 basis-full order-last sm:basis-auto sm:order-none" data-jar-chips>
              {spaceName && <span className="kw-chip !text-[11px]">{spaceName}</span>}
              {left !== null && list && (
                <span className="kw-chip !text-[11px]" data-peeks-left>
                  {left} of {list.daily_cap} peeks left today
                </span>
              )}
            </div>
          )}
          {onOpenSettings && (
            <button type="button" onClick={onOpenSettings} aria-label="Settings" className={`${btn.quiet} ml-auto max-sm:w-11 max-sm:h-11 max-sm:!px-0`} data-action="settings">
              <Settings size={16} aria-hidden="true" />
              <span className="hidden sm:inline">Settings</span>
            </button>
          )}
        </header>

        {load === 'loading' && !list ? (
          <div aria-busy="true" aria-label="Loading the idea jar" className="flex flex-col gap-3" data-loading>
            <div className="h-[120px] rounded-2xl bg-white/[0.04]" />
            {[0, 1, 2].map((k) => (
              <div key={k} className="h-[64px] rounded-xl bg-white/[0.04]" />
            ))}
            {slow && (
              <InfoLine
                tone="zinc"
                action={
                  <button type="button" onClick={() => void fetchList()} className={btn.secondary} data-action="retry">
                    Retry
                  </button>
                }
              >
                Still loading the jar…
              </InfoLine>
            )}
          </div>
        ) : load === 'error' && !list ? (
          <InfoLine
            tone="rose"
            action={
              <button type="button" onClick={() => void fetchList()} className={btn.secondary} data-action="retry">
                Retry
              </button>
            }
          >
            Couldn’t open the jar. Try again.
          </InfoLine>
        ) : empty ? (
          <div className="flex flex-col items-center text-center gap-3 py-6" data-empty>
            <JarArt size={92} slips={0} lid="on" />
            <h3 className="text-[16px] font-semibold text-[#f7f6ff] m-0">The jar is empty</h3>
            <p className="max-w-[440px] text-[13px] leading-5 text-[#a39dcc]">
              Drop in anything: a feature, a fix, a hunch. When a bot is free, it takes a look and tells you if it’s worth it, how big it is, and how it would go.
            </p>
            <div className="w-full max-w-[520px] text-left">
              <IdeaQuickAdd ref={quick} spaceId={spaceId} variant={phone ? 'phone' : 'hero'} onDropped={onDropped} />
            </div>
            <div className="flex flex-wrap items-center justify-center gap-2 text-[12px] text-[#8d87b8]">
              <span>Try one:</span>
              {['A weekly summary of what the team did', 'A faster way to find old chats'].map((t) => (
                <button key={t} type="button" onClick={() => quick.current?.fill(t)} className={`${btn.secondary} !min-h-[44px] sm:!min-h-[32px] !text-[12px]`} data-example>
                  {t}
                </button>
              ))}
            </div>
            {jn?.kind === 'off' && offLine(onOpenSettings)}
          </div>
        ) : (
          <>
            <div className="flex flex-col gap-3 rounded-2xl border border-white/[0.08] bg-[#0e0a21] p-4 sm:p-5" data-hero>
              <div className="flex items-start gap-4">
                <div className="relative shrink-0 hidden min-[380px]:block" data-drop-anim={drop || undefined}>
                  {drop > 0 && <span key={`s${drop}`} className="kw-jar-slip" aria-hidden="true" />}
                  <JarArt key={`j${drop}`} size={phone ? 48 : 64} slips={inJar} lid={groups.peeking.length ? 'open' : 'on'} className={drop ? 'kw-jar-wobble-late' : ''} />
                  {drop > 0 && groups.peeking[0] && (
                    <span key={`p${drop}`} className="kw-jar-peek absolute -right-2 -top-2" aria-hidden="true">
                      <span className="block w-3 h-3 rounded-full bg-[#9d8fff]" />
                    </span>
                  )}
                </div>
                <div className="min-w-0 flex-1 flex flex-col gap-1">
                  <h3 className="text-[15px] sm:text-[16px] font-semibold text-[#f7f6ff] m-0">Drop an idea in. A free bot will take a look.</h3>
                  <p className="text-[13px] leading-5 text-[#a39dcc]">
                    When a bot has nothing on, it peeks in, takes the oldest idea and writes its take: is it worth it, how big, and a plan. Nothing starts until you press Do it.
                  </p>
                </div>
              </div>
              <IdeaQuickAdd ref={quick} spaceId={spaceId} variant={phone ? 'phone' : 'hero'} onDropped={onDropped} />
              <p className="text-[12px] text-[#8d87b8]">Mention @a bot to ask them first.</p>
              {jn?.kind === 'cap' && (
                <InfoLine tone="sky" className="!text-[12px]">
                  Bots have had their {jn.cap} peeks for today in {space}. The rest wait for tomorrow, or press Look now on one.
                </InfoLine>
              )}
              {jn?.kind === 'off' && offLine(onOpenSettings)}
              {load === 'error' && (
                <InfoLine
                  tone="rose"
                  action={
                    <button type="button" onClick={() => void fetchList()} className={btn.secondary} data-action="retry">
                      Retry
                    </button>
                  }
                >
                  Couldn’t open the jar. Try again.
                </InfoLine>
              )}
            </div>
            <div className="flex flex-col gap-6" data-sections>
              {SECTION_ORDER.map(section)}
              {groups.ready.length + groups.peeking.length + groups.jar.length === 0 && (
                <p className="flex items-center gap-2 text-[13px] text-[#8d87b8]">
                  <Inbox size={14} aria-hidden="true" />
                  Nothing waiting in the jar.
                </p>
              )}
            </div>
          </>
        )}

        <div aria-live="polite" className="sr-only" data-live>
          {live}
        </div>
        {toast && <UndoToast text={toast.text} onUndo={undoToast} onClose={closeToast} />}
      </div>
      </div>
    </AvatarsContext.Provider>
  );
};

const offLine = (onOpenSettings?: () => void) => (
  <InfoLine
    tone="zinc"
    className="!text-[12px] text-left"
    action={
      onOpenSettings ? (
        <button type="button" onClick={onOpenSettings} className={`${btn.quiet} !min-h-[28px] text-[#b8adff]`} data-action="turn-on">
          Turn on in Settings
        </button>
      ) : undefined
    }
  >
    Bots don’t peek in this jar on their own. Ideas wait here until you press Look now.
  </InfoLine>
);

const withIdeas = (l: IdeaList, ideas: Idea[]): IdeaList => ({ ...l, ideas, counts: countIdeas(ideas) });
const upsert = (l: IdeaList, idea: Idea): IdeaList => {
  const at = l.ideas.findIndex((i) => i.id === idea.id);
  const ideas = at < 0 ? [...l.ideas, idea] : l.ideas.map((i) => (i.id === idea.id ? idea : i));
  return withIdeas(l, ideas);
};
const cssId = (id: string) => (typeof CSS !== 'undefined' && CSS.escape ? CSS.escape(id) : id.replace(/["\\]/g, '\\$&'));
