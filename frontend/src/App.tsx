import React, { Suspense, lazy, useState, useEffect, useRef, useCallback } from 'react';
import { AppHeader } from './components/AppHeader';
import { Sidebar, Kwant, Channel, Team, memberIdsOf } from './components/Sidebar';
import { ChatView, ChatMessage, DiscussionState, RoutingState, QueuedState } from './components/ChatView';
import { ThreadBar, ThreadHeader } from './components/ThreadBar';
import { DelegationCard } from './components/DelegationCard';
import { QuestionCard } from './components/QuestionCard';
import { HelpCircle, ListChecks, Loader2 } from 'lucide-react';
import { PendingApproval } from './components/ModernApprovalCard';
import type { AgentFormData } from './components/NewAgentModal';
import { NewTeamModal } from './components/NewTeamModal';
import { ManageTeamModal } from './components/ManageTeamModal';
import { playSfx } from './audio/chiptune';
import { Toasts, Toast } from './components/Toasts';
import { clearAllTurns, resetTurns, turnEnded, turnStarted } from './kwantActivity';
import {
  Delegation,
  DelegationMeta,
  Thread,
  TurnMap,
  applyDelegation,
  applyTurnEvent,
  backgroundTurns,
  hydrateTurns,
  messageContextId,
  parseDelegationMeta,
  queuePosition,
  shouldRenderDelta,
  streamTarget,
  threadBadge,
  threadsApi,
  turnsIn,
  upsertThread,
} from './threads';
import { Profile, describeProfileSwitch, profilesApi } from './profiles';
import {
  NotifyEvent,
  SearchHit,
  TOAST_KINDS,
  browserNotifyEnabled,
  channelLabel,
  continuityApi,
  emitContinuity,
  notificationPermission,
  shouldShowOsNotification,
} from './continuity';
import type { PinnedItem } from './components/Instructions';
import type { EditTab } from './components/NewAgentModal';
import { Skill, applySkillEvent, pendingByKwant, skillsApi } from './skills';
import { AttachmentRef, sendAttachmentErrorText } from './attachments';
import {
  Question,
  QuestionMeta,
  applyQuestion,
  hydrateQuestions,
  latestOpenQuestion,
  questionsApi,
  turnsAwaitingAnswer,
} from './questions';

// Kwant and Tools editors are their own chunks. They stay mounted (rendering nothing while closed),
// so both load right after start-up and opening them never waits.
const NewAgentModal = lazy(() => import('./components/NewAgentModal').then((m) => ({ default: m.NewAgentModal })));
const McpServersModal = lazy(() => import('./components/McpServersModal').then((m) => ({ default: m.McpServersModal })));
const ProfilesModal = lazy(() => import('./components/ProfilesModal').then((m) => ({ default: m.ProfilesModal })));
const SettingsModal = lazy(() => import('./components/SettingsModal').then((m) => ({ default: m.SettingsModal })));
const ChannelInstructionsModal = lazy(() =>
  import('./components/ChannelInstructionsModal').then((m) => ({ default: m.ChannelInstructionsModal })),
);
const SearchModal = lazy(() => import('./components/SearchModal').then((m) => ({ default: m.SearchModal })));
const RoutinesModal = lazy(() => import('./components/RoutinesModal').then((m) => ({ default: m.RoutinesModal })));

export const App: React.FC = () => {
  const [kwants, setKwants] = useState<Kwant[]>([]);
  const [channels, setChannels] = useState<Channel[]>([]);
  const [teams, setTeams] = useState<Team[]>([]);
  const [selectedId, setSelectedId] = useState<string>('');
  const [isChannel, setIsChannel] = useState(true);
  const [selectedTeamId, setSelectedTeamId] = useState<string | null>(null);
  const [messages, setMessages] = useState<ChatMessage[]>([]);
  const [messagesError, setMessagesError] = useState<string | null>(null);
  // The open thread of the selected agent; null for its direct chat (and for channels).
  const [openThreadId, setOpenThreadId] = useState<string | null>(null);
  const openCtx = openThreadId || selectedId;
  const openCtxRef = useRef(openCtx);
  openCtxRef.current = openCtx;
  // What the open chat is streaming; other chats' streams are kept in streamsRef until opened.
  const [streamingText, setStreamingText] = useState('');
  const [streamingThinking, setStreamingThinking] = useState('');
  const streamsRef = useRef<Record<string, { text: string; thinking: string }>>({});
  // Chats with a reply in flight -> who is replying. Several can stream at once.
  const [live, setLive] = useState<Record<string, string>>({});
  const liveRef = useRef(live);
  // Chat the last send or discussion started in; untagged events (team discussions) go here.
  const streamingContextIdRef = useRef<string | null>(null);
  const [turns, setTurns] = useState<TurnMap>({});
  const turnsRef = useRef(turns);
  turnsRef.current = turns;
  const [threadsByKwant, setThreadsByKwant] = useState<Record<string, Thread[] | undefined>>({});
  const threadsByKwantRef = useRef(threadsByKwant);
  threadsByKwantRef.current = threadsByKwant;
  const [threadErrors, setThreadErrors] = useState<Record<string, string | null>>({});
  const [showArchived, setShowArchived] = useState(false);
  const [delegations, setDelegations] = useState<Record<string, Delegation>>({});
  const delegationsRef = useRef(delegations);
  delegationsRef.current = delegations;
  /** Ids known to be threads, so their messages load from the thread route. */
  const knownThreadsRef = useRef<Set<string>>(new Set());
  const threadsLoadingRef = useRef<Set<string>>(new Set());
  const delegationsFetchedRef = useRef<Set<string>>(new Set());
  const selectedIdRef = useRef<string>(selectedId);
  selectedIdRef.current = selectedId;
  const isChannelRef = useRef<boolean>(isChannel);
  isChannelRef.current = isChannel;
  const messagesCacheRef = useRef<Record<string, ChatMessage[]>>({});
  const [discussionState, setDiscussionState] = useState<DiscussionState>({ inProgress: false });
  const [routingState, setRoutingState] = useState<RoutingState | null>(null);
  const routingStartedAtRef = useRef<number>(0);
  const discussionInProgressRef = useRef(false);
  const kwantsRef = useRef<Kwant[]>(kwants);
  kwantsRef.current = kwants;
  const channelsRef = useRef<Channel[]>(channels);
  channelsRef.current = channels;
  const teamsRef = useRef<Team[]>(teams);
  teamsRef.current = teams;
  const activeTeamIdRef = useRef<string | null>(selectedTeamId);
  activeTeamIdRef.current = selectedTeamId;
  const activeSandboxIdRef = useRef<string>('default');
  const currentKwantRef = useRef<Kwant | null>(null);
  const currentTeamRef = useRef<Team | null>(null);
  const [pendingApproval, setPendingApproval] = useState<PendingApproval | null>(null);
  const [tokensUsed, setTokensUsed] = useState(0);

  // Modals state
  const [isSettingsOpen, setIsSettingsOpen] = useState(false);
  const [isMcpOpen, setIsMcpOpen] = useState(false);
  const [isProfilesOpen, setIsProfilesOpen] = useState(false);
  const [isSearchOpen, setIsSearchOpen] = useState(false);
  const [isRoutinesOpen, setIsRoutinesOpen] = useState(false);
  const [editingChannelInstructions, setEditingChannelInstructions] = useState<Channel | null>(null);
  const [highlightMessageId, setHighlightMessageId] = useState<string | null>(null);
  const [profiles, setProfiles] = useState<Profile[]>([]);
  const [toasts, setToasts] = useState<Toast[]>([]);
  const toastIdRef = useRef(0);
  const dismissToast = useCallback((id: number) => setToasts((prev) => prev.filter((t) => t.id !== id)), []);
  // Set each render once the select handlers exist; the SSE handler and notification clicks go through it.
  // Opens an agent's settings on the Skills tab (set once the agents are known).
  const openSkillsRef = useRef<(kwantId: string) => void>(() => undefined);
  // Opens the chat of an agent's newest open question; false when it has none.
  const openQuestionRef = useRef<(kwantId: string) => boolean>(() => false);
  const openTargetRef = useRef<(t: { channel_id: string | null; kwant_id: string | null }) => boolean>(() => false);
  const showNotification = (n: NotifyEvent) => {
    const target = { channel_id: n.channel_id, kwant_id: n.kwant_id };
    // Failed routines and finished or slow delegations show in the app; the OS notification is for a hidden tab.
    if (!document.hidden && (TOAST_KINDS as string[]).includes(n.kind)) {
      if (n.kind !== 'routine_failed' && n.kind !== 'skill_proposed' && openCtxRef.current === (n.channel_id ?? n.kwant_id)) return;
      const id = ++toastIdRef.current;
      const action =
        n.kind === 'routine_failed'
          ? undefined
          : n.kind === 'skill_proposed'
            ? { label: 'Review', onClick: () => openSkillsRef.current(n.kwant_id) }
            : n.kind === 'question_asked'
              ? { label: 'Answer', onClick: () => openQuestionRef.current(n.kwant_id) || openTargetRef.current(target) }
              : { label: 'Open', onClick: () => openTargetRef.current(target) };
      setToasts((prev) => [...prev.slice(-2), { id, title: n.title, detail: n.body, action }]);
      if (n.kind !== 'routine_failed') playSfx('alert');
      return;
    }
    if (!shouldShowOsNotification({ enabled: browserNotifyEnabled(), permission: notificationPermission(), hidden: document.hidden, kind: n.kind })) return;
    try {
      const note = new Notification(n.title, { body: n.body, tag: `kwnt-${n.kind}-${n.channel_id ?? n.kwant_id}` });
      note.onclick = () => {
        window.focus();
        if (n.kind === 'skill_proposed') openSkillsRef.current(n.kwant_id);
        else if (!(n.kind === 'question_asked' && openQuestionRef.current(n.kwant_id))) openTargetRef.current(target);
        note.close();
      };
    } catch {
      // Some mobile browsers only allow notifications from a service worker.
    }
  };
  const showNotificationRef = useRef(showNotification);
  showNotificationRef.current = showNotification;
  const refreshProfiles = useCallback(() => {
    profilesApi.list().then(setProfiles, () => undefined);
  }, []);
  const markLive = (ctx: string | null | undefined, speaker?: string) => {
    if (!ctx) return;
    const name = speaker ?? liveRef.current[ctx] ?? '';
    if (liveRef.current[ctx] === name) return;
    liveRef.current = { ...liveRef.current, [ctx]: name };
    setLive(liveRef.current);
  };
  const clearLive = (ctx: string | null | undefined) => {
    if (ctx) delete streamsRef.current[ctx];
    if (ctx === openCtxRef.current) {
      setStreamingText('');
      setStreamingThinking('');
    }
    if (!ctx || !(ctx in liveRef.current)) return;
    const next = { ...liveRef.current };
    delete next[ctx];
    liveRef.current = next;
    setLive(next);
  };
  const kwantName = (id: unknown) => (typeof id === 'string' ? kwantsRef.current.find((k) => k.id === id)?.name : undefined);

  /** Loads a chat's latest messages into the cache, and on screen if it's open. */
  const refreshContext = async (ctx: string | null | undefined): Promise<void> => {
    if (!ctx) return;
    try {
      let data: ChatMessage[];
      if (knownThreadsRef.current.has(ctx)) {
        data = await threadsApi.messages<ChatMessage>(ctx);
      } else {
        const res = await fetch(`/api/channels/${encodeURIComponent(ctx)}/messages`);
        if (!res.ok) throw new Error(`HTTP ${res.status}`);
        data = await res.json();
      }
      messagesCacheRef.current[ctx] = data;
      if (openCtxRef.current === ctx) {
        setMessages(data);
        setMessagesError(null);
      }
    } catch {
      if (openCtxRef.current === ctx) setMessagesError('Could not load this chat.');
    }
  };

  const rememberThreads = (list: Thread[]) => list.forEach((t) => knownThreadsRef.current.add(t.id));

  const loadThreads = async (kwantId: string, includeArchived = false) => {
    if (!kwantId || threadsLoadingRef.current.has(kwantId)) return;
    threadsLoadingRef.current.add(kwantId);
    setThreadErrors((prev) => ({ ...prev, [kwantId]: null }));
    try {
      const list = await threadsApi.list(kwantId, includeArchived);
      rememberThreads(list);
      setThreadsByKwant((prev) => ({ ...prev, [kwantId]: list }));
      setDelegations((prev) => {
        const next = { ...prev };
        for (const t of list) if (t.delegation) next[t.delegation.id] = t.delegation;
        return next;
      });
    } catch (err) {
      setThreadErrors((prev) => ({ ...prev, [kwantId]: err instanceof Error ? err.message : 'Failed' }));
    } finally {
      threadsLoadingRef.current.delete(kwantId);
    }
  };

  /** Running turns after a (re)connect; chats whose reply ended meanwhile stop spinning. */
  const hydrateFromServer = async () => {
    skillsApi.list({ status: 'pending' }).then(setPendingSkills, () => undefined);
    questionsApi.listOpen().then((list) => {
      // Closed ones seen live stay; open ones come from the server.
      setQuestions((prev) => {
        const next = hydrateQuestions(list);
        for (const q of Object.values(prev)) if (q.status !== 'open' && !next[q.id]) next[q.id] = q;
        return next;
      });
    }, () => undefined);
    try {
      const { turns: list } = await threadsApi.turns();
      const map = hydrateTurns(list);
      for (const t of Object.values(map)) if (t.thread_id) knownThreadsRef.current.add(t.thread_id);
      turnsRef.current = map;
      setTurns(map);
      resetTurns(Object.values(map).filter((t) => t.state === 'running'));
      const busy = new Set(Object.values(map).map((t) => t.context_id));
      for (const ctx of Object.keys(liveRef.current)) {
        if (!busy.has(ctx) && !discussionInProgressRef.current) clearLive(ctx);
      }
      for (const t of Object.values(map)) markLive(t.context_id, kwantName(t.kwant_id));
      // Titles for the "working on" badges
      for (const t of Object.values(map)) {
        if (t.thread_id && !threadsByKwantRef.current[t.kwant_id]) loadThreads(t.kwant_id);
      }
    } catch {
      clearAllTurns();
    }
  };

  const [isNewAgentOpen, setIsNewAgentOpen] = useState(false);
  // Team that a newly created agent should join (set by a team's "Add member")
  const [newAgentTeam, setNewAgentTeam] = useState<Team | null>(null);
  const [isNewTeamOpen, setIsNewTeamOpen] = useState(false);
  const [editingTeam, setEditingTeam] = useState<Team | null>(null);
  const [editingKwant, setEditingKwant] = useState<Kwant | null>(null);
  const [editingTab, setEditingTab] = useState<EditTab>('profile');
  // Skills waiting for review, for the counts on the agent's header and Skills tab.
  const [pendingSkills, setPendingSkills] = useState<Skill[]>([]);
  // Agents' questions by id: open ones from the server, plus every question_updated since.
  const [questions, setQuestions] = useState<Record<string, Question>>({});
  const questionsRef = useRef(questions);
  questionsRef.current = questions;

  // Virtual Machine & Live Browser state
  const [isVmPaneOpen, setIsVmPaneOpen] = useState(false);
  const [activeUrl, setActiveUrl] = useState('');
  const [pageTitle, setPageTitle] = useState('');
  const [screenshotBase64, setScreenshotBase64] = useState<string | null>(null);
  const [isVmRunning, setIsVmRunning] = useState(true);
  const [browserError, setBrowserError] = useState<string | null>(null);
  const [terminalLogs, setTerminalLogs] = useState<Array<{ command: string; output: string; timestamp?: string }>>([]);

  // Load initial Kwants, Channels, and Teams
  useEffect(() => {
    async function loadInitialData() {
      try {
        const [kRes, cRes, tRes] = await Promise.all([
          fetch('/api/kwants').then((r) => r.json()),
          fetch('/api/channels').then((r) => r.json()),
          fetch('/api/teams')
            .then((r) => (r.ok ? r.json() : []))
            .catch(() => []),
        ]);

        const normalizedTeams: Team[] = (tRes || []).map((t: any) => ({
          ...t,
          member_kwants: Array.isArray(t.member_kwants)
            ? t.member_kwants
            : typeof t.member_kwants === 'string'
            ? JSON.parse(t.member_kwants || '[]')
            : [],
        }));

        setKwants(kRes);
        setChannels(cRes);
        setTeams(normalizedTeams);

        if (cRes.length > 0) {
          const initId = cRes[0].id;
          setSelectedId(initId);
          selectedIdRef.current = initId;
          setIsChannel(true);
          isChannelRef.current = true;
          const matchedTeam = normalizedTeams.find(
            (t) => `team-${t.name}` === cRes[0].name || t.id === cRes[0].id
          );
          setSelectedTeamId(matchedTeam ? matchedTeam.id : null);
        } else if (kRes.length > 0) {
          const initId = kRes[0].id;
          setSelectedId(initId);
          selectedIdRef.current = initId;
          setIsChannel(false);
          isChannelRef.current = false;
          setSelectedTeamId(null);
        }
      } catch (err) {
        console.error('Failed to load initial data:', err);
      }
    }
    loadInitialData();
    refreshProfiles();
  }, [refreshProfiles]);

  // The open chat (channel, direct chat or thread): show what's cached, then load the latest.
  useEffect(() => {
    if (!openCtx) return;
    const b = streamsRef.current[openCtx];
    setStreamingText(b?.text ?? '');
    setStreamingThinking(b?.thinking ?? '');
    setMessages(messagesCacheRef.current[openCtx] || []);
    setMessagesError(null);
    refreshContext(openCtx).then(() => {
      if (openCtxRef.current !== openCtx) return;
      const data = messagesCacheRef.current[openCtx] || [];
      setTokensUsed(data.reduce((acc, m) => acc + (m.tokens_used || 0), 0));
    });
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [openCtx]);

  // An agent's conversations load when its chat opens; SSE keeps them current from then on.
  useEffect(() => {
    if (!selectedId || isChannel || threadsByKwantRef.current[selectedId] !== undefined) return;
    loadThreads(selectedId);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [selectedId, isChannel]);

  // Delegation notes need the live record for their status; fetch each unknown one once.
  useEffect(() => {
    for (const m of messages) {
      const meta = parseDelegationMeta(m.meta);
      if (!meta || delegationsRef.current[meta.delegation_id] || delegationsFetchedRef.current.has(meta.delegation_id)) continue;
      delegationsFetchedRef.current.add(meta.delegation_id);
      knownThreadsRef.current.add(meta.thread_id);
      threadsApi.delegation(meta.delegation_id).then(
        (d) => setDelegations((prev) => (prev[d.id] ? prev : { ...prev, [d.id]: d })),
        () => undefined,
      );
    }
  }, [messages]);

  // Connect to SSE stream
  useEffect(() => {
    const sse = new EventSource('/api/events');
    let dropped = false;
    /** TurnComplete already refreshed these turns' chats; their turn_ended needn't again. */
    const completed = new Set<string>();

    /** After a reply: drop the chat's stream once nothing else runs or waits there. */
    const settle = (ctx: string | null) => {
      if (!ctx) return;
      if (discussionInProgressRef.current && ctx === streamingContextIdRef.current) return;
      if (turnsIn(turnsRef.current, ctx).length > 0) {
        streamsRef.current[ctx] = { text: '', thinking: '' };
        if (ctx === openCtxRef.current) {
          setStreamingText('');
          setStreamingThinking('');
        }
        return;
      }
      clearLive(ctx);
      if (ctx === streamingContextIdRef.current) {
        streamingContextIdRef.current = null;
        setRoutingState(null);
      }
    };

    const setTurnMap = (next: TurnMap) => {
      if (next === turnsRef.current) return;
      turnsRef.current = next;
      setTurns(next);
    };

    sse.onerror = () => {
      dropped = true;
    };
    // First connect and every reconnect: running turns from the server. After a drop, also
    // whatever changed meanwhile in the open chat and the open agent's conversations.
    sse.onopen = () => {
      hydrateFromServer();
      if (dropped) {
        refreshContext(openCtxRef.current);
        if (!isChannelRef.current && selectedIdRef.current) loadThreads(selectedIdRef.current);
      }
      dropped = false;
    };

    sse.onmessage = (e) => {
      try {
        const event = JSON.parse(e.data);
        switch (event.type) {
          case 'turn_queued':
          case 'TurnQueued':
          case 'turn_started':
          case 'TurnStarted':
          case 'turn_ended':
          case 'TurnEnded': {
            const d = event.data ?? {};
            const ctx: string | null = d.context_id ?? d.thread_id ?? d.kwant_id ?? null;
            setTurnMap(applyTurnEvent(turnsRef.current, event));
            if (d.thread_id) {
              knownThreadsRef.current.add(d.thread_id);
              const list = threadsByKwantRef.current[d.kwant_id];
              if (list && !list.some((t) => t.id === d.thread_id)) loadThreads(d.kwant_id);
            }
            const key = d.turn_id ?? d.context_id;
            if (event.type === 'turn_ended' || event.type === 'TurnEnded') {
              turnEnded(d.kwant_id, key);
              if (d.turn_id && completed.delete(d.turn_id)) settle(ctx);
              else refreshContext(ctx).then(() => settle(ctx));
            } else {
              if (event.type === 'turn_started' || event.type === 'TurnStarted') turnStarted(d.kwant_id, key);
              if (!discussionInProgressRef.current || ctx !== streamingContextIdRef.current) markLive(ctx, kwantName(d.kwant_id));
            }
            break;
          }
          case 'thread_updated':
          case 'ThreadUpdated': {
            const t: Thread | undefined = event.data?.thread;
            if (!t?.id) break;
            knownThreadsRef.current.add(t.id);
            setThreadsByKwant((prev) =>
              prev[t.kwant_id] === undefined ? prev : { ...prev, [t.kwant_id]: upsertThread(prev[t.kwant_id], t) },
            );
            break;
          }
          case 'delegation_updated':
          case 'DelegationUpdated': {
            const dl: Delegation | undefined = event.data?.delegation;
            if (!dl?.id) break;
            knownThreadsRef.current.add(dl.thread_id);
            setDelegations((prev) => ({ ...prev, [dl.id]: dl }));
            const list = threadsByKwantRef.current[dl.specialist_kwant_id];
            if (list && !list.some((t) => t.id === dl.thread_id)) loadThreads(dl.specialist_kwant_id);
            else setThreadsByKwant((prev) => {
              const next = applyDelegation(prev[dl.specialist_kwant_id], dl);
              return next === prev[dl.specialist_kwant_id] ? prev : { ...prev, [dl.specialist_kwant_id]: next };
            });
            break;
          }
          case 'message_added':
          case 'MessageAdded': {
            const m: ChatMessage | undefined = event.data?.message;
            const ctx = m ? messageContextId(m) : null;
            if (!m?.id || !ctx) break;
            if (m.thread_id) knownThreadsRef.current.add(m.thread_id);
            const list = messagesCacheRef.current[ctx];
            if (!list) {
              if (ctx === openCtxRef.current) refreshContext(ctx);
              break;
            }
            if (list.some((x) => x.id === m.id)) break;
            // The person's own message replaces its optimistic copy.
            let base = list;
            if (m.sender_role === 'user') {
              const i = list.findIndex((x) => x.id.startsWith('temp-') && x.text === m.text);
              if (i >= 0) base = list.filter((_, j) => j !== i);
            }
            const updated = [...base, m];
            messagesCacheRef.current[ctx] = updated;
            if (ctx === openCtxRef.current) setMessages(updated);
            break;
          }
          case 'profile_switched':
          case 'ProfileSwitched': {
            const kwantName = kwantsRef.current.find((k) => k.id === event.data?.kwant_id)?.name;
            const { title, detail } = describeProfileSwitch(event.data, kwantName);
            const id = ++toastIdRef.current;
            setToasts((prev) => [...prev.slice(-2), { id, title, detail }]);
            playSfx('alert');
            refreshProfiles();
            break;
          }
          case 'skill_updated':
          case 'SkillUpdated': {
            const sk = event.data?.skill;
            if (!sk?.id) break;
            emitContinuity({ type: 'skill_updated', data: sk });
            setPendingSkills((prev) => applySkillEvent(prev, sk).filter((x) => x.status === 'pending'));
            break;
          }
          case 'question_updated':
          case 'QuestionUpdated': {
            const q: Question | undefined = event.data?.question;
            if (q?.id) setQuestions((prev) => applyQuestion(prev, q));
            break;
          }
          case 'memory_changed':
          case 'MemoryChanged':
            emitContinuity({ type: 'memory_changed', data: event.data });
            break;
          case 'routine_run':
          case 'RoutineRun':
            emitContinuity({ type: 'routine_run', data: event.data });
            break;
          case 'notify':
          case 'Notify':
            showNotificationRef.current(event.data as NotifyEvent);
            break;
          case 'TextDelta':
          case 'ThinkingDelta': {
            // Each chat streams into its own buffer; only the open chat's is drawn.
            const ctx = streamTarget(event, streamingContextIdRef.current);
            if (!ctx) break;
            const buf = (streamsRef.current[ctx] ??= { text: '', thinking: '' });
            if (event.type === 'TextDelta') buf.text += event.data;
            else buf.thinking = event.data;
            if (!(ctx in liveRef.current)) markLive(ctx, kwantName(event.kwant_id));
            if (shouldRenderDelta(event, openCtxRef.current, streamingContextIdRef.current)) {
              if (event.type === 'TextDelta') {
                setStreamingText(buf.text);
                playSfx('blip');
              } else setStreamingThinking(buf.thinking);
            }
            break;
          }
          case 'routing_started':
          case 'RoutingStarted': {
            const ctx = event.data?.context_id;
            const targetId = streamingContextIdRef.current;
            if (ctx && targetId && ctx !== targetId) break;
            routingStartedAtRef.current = Date.now();
            setRoutingState({
              phase: 'deciding',
              contextId: ctx || targetId || undefined,
              candidates: event.data?.candidates || [],
            });
            markLive(ctx || targetId, 'Laya');
            break;
          }
          case 'routing_decided':
          case 'RoutingDecided': {
            const ctx = event.data?.context_id;
            const targetId = streamingContextIdRef.current;
            if (ctx && targetId && ctx !== targetId) break;
            const name: string = event.data?.kwant_name;
            const startedAt = routingStartedAtRef.current;
            const applyDecision = () => {
            if (routingStartedAtRef.current !== startedAt) return;
            setRoutingState((prev) => (startedAt && !prev) ? prev : ({
              phase: 'decided',
              contextId: ctx || targetId || undefined,
              candidates: prev?.candidates || [],
              kwantId: event.data?.kwant_id,
              kwantName: name,
              kwantTitle: event.data?.kwant_title,
              reason: event.data?.reason,
              mode: event.data?.mode,
            }));
            if (name && (ctx || targetId) && (ctx || targetId) in liveRef.current) markLive(ctx || targetId, name);
            playSfx('blip');
            };
            // Keep Laya's "deciding" visible briefly so instant heuristic routing doesn't just flash.
            const remaining = 700 - (Date.now() - startedAt);
            if (startedAt && remaining > 0) setTimeout(applyDecision, remaining);
            else applyDecision();
            break;
          }
          case 'ApprovalRequired':
            setPendingApproval({
              id: event.data.id,
              tool_name: event.data.tool_name,
              details: event.data.details,
            });
            playSfx('alert');
            break;
          case 'ApprovalWithdrawn':
            setPendingApproval((current) => (current?.id === event.data.id ? null : current));
            break;
          case 'discussion_started':
          case 'DiscussionStarted': {
            discussionInProgressRef.current = true;
            const teamId = event.data?.team_id;
            const initialSpeakerId = event.data?.speakers?.[0];
            const initialSpeakerName =
              kwantsRef.current.find((k) => k.id === initialSpeakerId)?.name || initialSpeakerId;

            const matchedChannel = channelsRef.current.find(
              (c) => c.name === `team-${event.data?.team_name}` || c.id === teamId
            );
            const contextId = matchedChannel ? matchedChannel.id : (teamId || selectedIdRef.current);

            streamingContextIdRef.current = contextId;
            markLive(contextId, initialSpeakerName);

            setDiscussionState({
              inProgress: true,
              teamId,
              contextId,
              teamName: event.data?.team_name,
              totalTurns: event.data?.total_turns,
              currentSpeaker: initialSpeakerName,
              currentTurn: 1,
              kind: event.data?.kind,
            });
            playSfx('alert');
            break;
          }
          case 'discussion_turn_started':
          case 'DiscussionTurnStarted': {
            discussionInProgressRef.current = true;
            const spkName = event.data?.speaker_name;
            setDiscussionState((prev) => ({
              ...prev,
              inProgress: true,
              currentSpeaker: spkName,
              askedBy: event.data?.asked_by,
              phase: event.data?.phase,
              currentTurn: event.data?.turn_index,
              totalTurns: event.data?.total_turns ?? prev.totalTurns,
            }));
            const targetId = streamingContextIdRef.current || selectedIdRef.current;
            if (targetId) {
              refreshContext(targetId).then(() => {
                streamsRef.current[targetId] = { text: '', thinking: '' };
                if (openCtxRef.current === targetId) {
                  setStreamingText('');
                  setStreamingThinking('');
                }
              });
              markLive(targetId, spkName);
            }
            break;
          }
          case 'discussion_completed':
          case 'DiscussionCompleted': {
            discussionInProgressRef.current = false;
            setDiscussionState({ inProgress: false });
            setRoutingState(null);
            const targetId = streamingContextIdRef.current || selectedIdRef.current;
            streamingContextIdRef.current = null;
            playSfx('coin');
            refreshContext(targetId).then(() => settle(targetId));
            break;
          }
          case 'TurnComplete': {
            const ctx = streamTarget(event, streamingContextIdRef.current);
            if (ctx === openCtxRef.current) {
              playSfx('coin');
              setTokensUsed((prev) => prev + (event.data?.tokens || 0));
            }
            if (typeof event.turn_id === 'string') completed.add(event.turn_id);
            // The stream stays up until the saved reply is on screen, so nothing jumps.
            refreshContext(ctx).then(() => {
              if (!discussionInProgressRef.current) settle(ctx);
            });
            break;
          }
          case 'browser_navigated':
          case 'BrowserNavigated': {
            const teamId = event.data?.team_id;
            const currentTid = activeSandboxIdRef.current;
            const k = currentKwantRef.current;
            const t = currentTeamRef.current;
            const isMatch =
              !currentTid ||
              !teamId ||
              currentTid === teamId ||
              teamId === 'default' ||
              teamId === selectedIdRef.current ||
              teamId === streamingContextIdRef.current ||
              (k && (k.id === teamId || k.name.toLowerCase() === teamId.toLowerCase())) ||
              (t && (t.id === teamId || t.name.toLowerCase() === teamId.toLowerCase()));
            if (isMatch) {
              if (event.data?.url) setActiveUrl(event.data.url);
              if (event.data?.title) setPageTitle(event.data.title);
              setIsVmPaneOpen(true);
              playSfx('blip');
            }
            break;
          }
          case 'browser_screenshot':
          case 'BrowserScreenshot': {
            const teamId = event.data?.team_id;
            const currentTid = activeSandboxIdRef.current;
            const k = currentKwantRef.current;
            const t = currentTeamRef.current;
            const isMatch =
              !currentTid ||
              !teamId ||
              currentTid === teamId ||
              teamId === 'default' ||
              teamId === selectedIdRef.current ||
              teamId === streamingContextIdRef.current ||
              (k && (k.id === teamId || k.name.toLowerCase() === teamId.toLowerCase())) ||
              (t && (t.id === teamId || t.name.toLowerCase() === teamId.toLowerCase()));
            if (isMatch) {
              setBrowserError(null);
              setIsVmRunning(true);
              if (event.data?.image_base64) setScreenshotBase64(event.data.image_base64);
              if (event.data?.url) setActiveUrl(event.data.url);
              setIsVmPaneOpen(true);
              playSfx('blip');
            }
            break;
          }
          case 'sandbox_status':
          case 'SandboxStatusChanged': {
            const teamId = event.data?.team_id;
            const currentTid = activeSandboxIdRef.current;
            const k = currentKwantRef.current;
            const t = currentTeamRef.current;
            const isMatch =
              !currentTid ||
              !teamId ||
              currentTid === teamId ||
              teamId === 'default' ||
              teamId === selectedIdRef.current ||
              teamId === streamingContextIdRef.current ||
              (k && (k.id === teamId || k.name.toLowerCase() === teamId.toLowerCase())) ||
              (t && (t.id === teamId || t.name.toLowerCase() === teamId.toLowerCase()));
            if (isMatch) {
              if (typeof event.data?.is_running === 'boolean') {
                setIsVmRunning(event.data.is_running);
              }
            }
            break;
          }
          case 'ToolCallCompleted': {
            const result = event.data?.result;
            if (result && typeof result === 'object' && ('command' in result || 'stdout' in result)) {
              const cmd = result.command || 'bash';
              const output = result.stdout || result.stderr || JSON.stringify(result);
              setTerminalLogs((prev) => [
                ...prev,
                { command: cmd, output, timestamp: new Date().toLocaleTimeString() },
              ]);
            }
            break;
          }
          case 'Error': {
            const ctx = streamTarget(event, streamingContextIdRef.current);
            if (ctx && ctx === streamingContextIdRef.current) {
              discussionInProgressRef.current = false;
              setDiscussionState({ inProgress: false });
              setRoutingState(null);
            }
            refreshContext(ctx).then(() => settle(ctx));
            break;
          }
        }
      } catch (err) {
        console.error('Failed parsing SSE event:', err);
      }
    };

    return () => {
      sse.close();
      clearAllTurns();
    };
  }, [refreshProfiles]);

  /** Stop in the composer: the open thread's turn, the direct chat's, or the channel's. */
  const handleCancelDiscussion = async () => {
    const ctx = openCtxRef.current;
    const threadId = openThreadId;
    try {
      if (threadId) {
        await threadsApi.cancel(threadId);
      } else {
        const running = turnsIn(turnsRef.current, ctx).find((t) => t.state === 'running');
        const direct = !isChannelRef.current;
        const res = await fetch('/api/chat/cancel', {
          method: 'POST',
          headers: { 'Content-Type': 'application/json' },
          body: JSON.stringify(
            direct
              ? { kwant_id: ctx, turn_id: running?.turn_id }
              : { channel_id: ctx, team_id: activeTeamIdRef.current || null },
          ),
        });
        if (!res.ok) throw new Error(`HTTP ${res.status}`);
      }
    } catch (err) {
      console.error('Failed cancelling chat/discussion:', err);
      pushToast('Could not stop the reply.', err instanceof Error ? err.message : undefined);
      return;
    }
    if (ctx === streamingContextIdRef.current) {
      discussionInProgressRef.current = false;
      setDiscussionState({ inProgress: false });
      setRoutingState(null);
      streamingContextIdRef.current = null;
    }
    refreshContext(ctx).then(() => {
      if (turnsIn(turnsRef.current, ctx).length === 0) clearLive(ctx);
    });
  };

  const handleSendMessage = async (text: string, attachments: AttachmentRef[] = []) => {
    const targetId = openCtx;
    const threadId = openThreadId;
    const isTargetChannel = isChannel;
    const targetTeam = selectedTeamId;
    const speakerName = !isTargetChannel ? (currentKwant?.name || 'Agent') : chatTitle;

    if (!threadId) {
      discussionInProgressRef.current = false;
      setDiscussionState({ inProgress: false });
      setRoutingState(null);
    }
    streamingContextIdRef.current = targetId;
    if (!(targetId in liveRef.current)) {
      streamsRef.current[targetId] = { text: '', thinking: '' };
      setStreamingText('');
      setStreamingThinking('');
    }
    markLive(targetId, speakerName);

    // Optimistically add user message
    const tempId = `temp-${Date.now()}`;
    const userMsg: ChatMessage = {
      id: tempId,
      sender_role: 'user',
      sender_id: 'PlayerHero',
      text,
      meta: attachments.length ? JSON.stringify({ attachments }) : null,
    };
    const cached = messagesCacheRef.current[targetId] || messages;
    const updated = [...cached, userMsg];
    messagesCacheRef.current[targetId] = updated;
    if (openCtxRef.current === targetId) {
      setMessages(updated);
    }

    const attachment_ids = attachments.map((a) => a.id);
    try {
      const res = await fetch('/api/chat/send', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify(
          threadId
            ? { thread_id: threadId, prompt: text, sender_id: 'PlayerHero', attachment_ids }
            : {
                channel_id: isTargetChannel ? targetId : null,
                kwant_id: !isTargetChannel ? targetId : null,
                team_id: targetTeam || null,
                prompt: text,
                sender_id: 'PlayerHero',
                attachment_ids,
              },
        ),
      });
      if (!res.ok) {
        let message = attachments.length ? sendAttachmentErrorText(res.status) ?? `HTTP ${res.status}` : `HTTP ${res.status}`;
        try {
          const data = await res.json();
          if (typeof data?.error === 'string') message = data.error;
        } catch {
          // no JSON body
        }
        throw new Error(message);
      }
    } catch (err) {
      console.error('Failed sending chat:', err);
      const rest = (messagesCacheRef.current[targetId] || []).filter((m) => m.id !== tempId);
      messagesCacheRef.current[targetId] = rest;
      if (openCtxRef.current === targetId) setMessages(rest);
      pushToast('Message not sent.', err instanceof Error ? err.message : undefined);
      if (turnsIn(turnsRef.current, targetId).length === 0) clearLive(targetId);
      if (streamingContextIdRef.current === targetId) {
        setRoutingState(null);
        streamingContextIdRef.current = null;
      }
    }
  };

  const handleResolveApproval = async (
    ticketId: string,
    decision: 'allow_once' | 'always_allow' | 'deny'
  ) => {
    try {
      await fetch(`/api/approvals/${ticketId}/resolve`, {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ decision }),
      });
      setPendingApproval(null);
    } catch (err) {
      console.error('Failed resolving approval:', err);
    }
  };

  const handleSelectTeam = (team: Team) => {
    setOpenThreadId(null);
    setSelectedTeamId(team.id);
    const targetChannel = channels.find(
      (c) => c.name === `team-${team.name}` || c.id === team.id
    );
    if (targetChannel) {
      setSelectedId(targetChannel.id);
      selectedIdRef.current = targetChannel.id;
      setIsChannel(true);
      isChannelRef.current = true;
      setMessages(messagesCacheRef.current[targetChannel.id] || []);
    } else {
      setSelectedId(team.id);
      selectedIdRef.current = team.id;
      setIsChannel(true);
      isChannelRef.current = true;
      setMessages(messagesCacheRef.current[team.id] || []);
      fetch('/api/channels')
        .then((r) => (r.ok ? r.json() : []))
        .then((freshChannels: Channel[]) => {
          setChannels(freshChannels);
          const match = freshChannels.find(
            (c) => c.name === `team-${team.name}` || c.id === team.id
          );
          if (match) {
            setSelectedId(match.id);
            selectedIdRef.current = match.id;
            setIsChannel(true);
            isChannelRef.current = true;
            setMessages(messagesCacheRef.current[match.id] || []);
          }
        })
        .catch(() => {});
    }
    playSfx('click');
  };

  const handleSelectChannel = (channel: Channel) => {
    setOpenThreadId(null);
    setSelectedId(channel.id);
    selectedIdRef.current = channel.id;
    setIsChannel(true);
    isChannelRef.current = true;
    const matchedTeam = teams.find(
      (t) => `team-${t.name}` === channel.name || t.id === channel.id
    );
    setSelectedTeamId(matchedTeam ? matchedTeam.id : null);
    setMessages(messagesCacheRef.current[channel.id] || []);
    playSfx('click');
  };

  const handleSelectKwant = (kwant: Kwant) => {
    setOpenThreadId(null);
    setSelectedId(kwant.id);
    selectedIdRef.current = kwant.id;
    setIsChannel(false);
    isChannelRef.current = false;
    setSelectedTeamId(null);
    setMessages(messagesCacheRef.current[kwant.id] || []);
    playSfx('click');
  };

  /** Opens a channel, or the direct chat with a kwant when there's no channel. False if it's gone. */
  const openTarget = (t: { channel_id: string | null; kwant_id: string | null }): boolean => {
    if (t.channel_id) {
      const channel = channels.find((c) => c.id === t.channel_id);
      if (!channel) return false;
      if (selectedIdRef.current !== channel.id) handleSelectChannel(channel);
      return true;
    }
    const kwant = kwants.find((k) => k.id === t.kwant_id);
    if (!kwant) return false;
    if (selectedIdRef.current !== kwant.id || openThreadId) handleSelectKwant(kwant);
    return true;
  };
  openTargetRef.current = openTarget;
  const openAgentSettings = (kwant: Kwant, tab: EditTab = 'profile') => {
    setEditingTab(tab);
    setEditingKwant(kwant);
    playSfx('click');
  };
  openQuestionRef.current = (kwantId: string) => {
    const q = latestOpenQuestion(questionsRef.current, kwantId);
    if (!q) return false;
    if (q.thread_id) openThread(q.kwant_id, q.thread_id);
    else openTarget({ channel_id: q.channel_id, kwant_id: q.channel_id ? null : q.kwant_id });
    if (q.message_id) setHighlightMessageId(q.message_id);
    return true;
  };
  openSkillsRef.current = (kwantId: string) => {
    const k = kwants.find((x) => x.id === kwantId);
    if (k) openAgentSettings(k, 'skills');
  };

  const pushToast = (title: string, detail?: string) => {
    const id = ++toastIdRef.current;
    setToasts((prev) => [...prev.slice(-2), { id, title, detail }]);
  };

  /** Opens one of an agent's threads (from the thread list, a delegation card or a badge). */
  const openThread = (kwantId: string, threadId: string) => {
    const kwant = kwants.find((k) => k.id === kwantId);
    if (!kwant) {
      pushToast('That agent no longer exists.');
      return;
    }
    knownThreadsRef.current.add(threadId);
    if (selectedIdRef.current !== kwant.id || isChannelRef.current) handleSelectKwant(kwant);
    else playSfx('click');
    setOpenThreadId(threadId);
    if (threadsByKwantRef.current[kwantId] === undefined) loadThreads(kwantId);
  };

  const openDirectChat = () => {
    if (!openThreadId) return;
    setOpenThreadId(null);
    playSfx('click');
  };

  const errText = (err: unknown) => (err instanceof Error ? err.message : 'Something went wrong.');

  const handleNewThread = async (): Promise<string | void> => {
    const kwantId = selectedIdRef.current;
    try {
      const t = await threadsApi.create(kwantId);
      knownThreadsRef.current.add(t.id);
      messagesCacheRef.current[t.id] = [];
      setThreadsByKwant((prev) => ({ ...prev, [kwantId]: upsertThread(prev[kwantId], t) }));
      if (selectedIdRef.current === kwantId) setOpenThreadId(t.id);
    } catch (err) {
      return `Couldn't start a conversation: ${errText(err)}`;
    }
  };

  /** Stops a thread's work: its delegation when one is open (so the lead hears), else its turns. */
  const handleStopThread = async (t: Thread): Promise<string | void> => {
    const d = t.delegation ? delegationsRef.current[t.delegation.id] ?? t.delegation : null;
    try {
      if (d && (d.status === 'queued' || d.status === 'running')) {
        const next = await threadsApi.cancelDelegation(d.id);
        setDelegations((prev) => ({ ...prev, [next.id]: next }));
      } else {
        await threadsApi.cancel(t.id);
      }
    } catch (err) {
      return `Couldn't stop it: ${errText(err)}`;
    }
  };

  const handleArchiveThread = async (t: Thread, archived: boolean): Promise<string | void> => {
    try {
      const next = await threadsApi.patch(t.id, { archived });
      setThreadsByKwant((prev) => ({ ...prev, [next.kwant_id]: upsertThread(prev[next.kwant_id], next) }));
      if (archived && openThreadId === t.id) {
        setOpenThreadId(null);
        pushToast(`Archived “${t.title}”`, 'Find it under Archived in the conversation list.');
      }
    } catch (err) {
      return `Couldn't ${archived ? 'archive' : 'unarchive'} it: ${errText(err)}`;
    }
  };

  const handleCancelDelegation = async (id: string): Promise<string | void> => {
    try {
      const next = await threadsApi.cancelDelegation(id);
      setDelegations((prev) => ({ ...prev, [next.id]: next }));
    } catch (err) {
      return `Couldn't stop it: ${errText(err)}`;
    }
  };

  const handleAnswerQuestion = async (id: string, answer: { choice: number } | { text: string }): Promise<string | void> => {
    try {
      const { question } = await questionsApi.answer(id, answer);
      setQuestions((prev) => applyQuestion(prev, question));
    } catch (err) {
      // Answered from another tab, dismissed, or its turn was stopped: show what happened.
      try {
        const { question } = await questionsApi.get(id);
        setQuestions((prev) => applyQuestion(prev, question));
        if (question.status === 'answered') return 'This was already answered.';
        if (question.status === 'cancelled') return 'This question was closed.';
      } catch {
        // keep the first error
      }
      return `Couldn't send your answer: ${errText(err)}`;
    }
  };
  const handleDismissQuestion = async (id: string): Promise<string | void> => {
    try {
      const q = await questionsApi.cancel(id);
      setQuestions((prev) => applyQuestion(prev, q));
    } catch (err) {
      return `Couldn't skip it: ${errText(err)}`;
    }
  };
  const renderQuestion = (msg: ChatMessage, meta: QuestionMeta, flash: boolean) => {
    const live = questions[meta.question_id];
    const turnId = live?.turn_id;
    const name = live ? kwantName(live.kwant_id) : undefined;
    return (
      <QuestionCard
        meta={meta}
        messageId={msg.id}
        kwantName={name ?? (msg.sender_id && msg.sender_id.length <= 20 ? msg.sender_id : chatTitle)}
        kwantId={live?.kwant_id}
        live={live}
        waiting={!!turnId && !!turns[turnId]}
        onAnswer={handleAnswerQuestion}
        onDismiss={handleDismissQuestion}
        flash={flash}
      />
    );
  };

  const renderDelegation = (msg: ChatMessage, meta: DelegationMeta, flash: boolean) => {
    const queued = turnsIn(turns, meta.thread_id).find((t) => t.state === 'queued');
    return (
      <DelegationCard
        meta={meta}
        text={msg.text}
        messageId={msg.id}
        live={delegations[meta.delegation_id]}
        queuePos={queued ? queuePosition(turns, queued.turn_id) : null}
        onOpenThread={(m) => openThread(m.specialist_id, m.thread_id)}
        onCancel={handleCancelDelegation}
        flash={flash}
      />
    );
  };

  // Search result picked: open its chat, then scroll to the message if it's among the ones loaded.
  const handleJump = async (hit: SearchHit) => {
    const targetId = hit.channel_id ?? hit.kwant_id;
    if (!targetId || !openTarget(hit)) {
      pushToast('That chat no longer exists.');
      return;
    }
    try {
      const res = await fetch(`/api/channels/${encodeURIComponent(targetId)}/messages`);
      if (!res.ok) throw new Error(`HTTP ${res.status}`);
      const data: ChatMessage[] = await res.json();
      messagesCacheRef.current[targetId] = data;
      if (selectedIdRef.current === targetId) setMessages(data);
      if (data.some((m) => m.id === hit.message_id)) setHighlightMessageId(hit.message_id);
      else pushToast('Opened the chat', 'That message is older than the latest 100 shown here.');
    } catch {
      pushToast('Could not load that chat.');
    }
  };

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if ((e.metaKey || e.ctrlKey) && !e.altKey && !e.shiftKey && e.key.toLowerCase() === 'k') {
        e.preventDefault();
        setIsSearchOpen((v) => !v);
      }
    };
    window.addEventListener('keydown', onKey);
    return () => window.removeEventListener('keydown', onKey);
  }, []);

  const saveChannelInstructions = async (text: string): Promise<string | void> => {
    const channel = editingChannelInstructions;
    if (!channel) return;
    try {
      const updated = await continuityApi.setChannelInstructions(channel.id, text);
      setChannels((prev) => prev.map((c) => (c.id === channel.id ? { ...c, instructions: updated.instructions } : c)));
    } catch (err) {
      return err instanceof Error ? err.message : 'Could not save.';
    }
  };

  const saveKwantMcpServers = async (kwantId: string, serverIds: string[] | undefined): Promise<string | void> => {
    if (serverIds === undefined) return;
    const res = await fetch(`/api/kwants/${kwantId}/mcp-servers`, {
      method: 'PUT',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ server_ids: serverIds }),
    });
    if (!res.ok) return `Saved, but the tool servers couldn't be updated (HTTP ${res.status}).`;
  };

  const handleUpdateAgent = async (kwant: Kwant, agentData: AgentFormData): Promise<string | void> => {
    try {
      const { mcp_server_ids, ...kwantData } = agentData;
      const res = await fetch(`/api/kwants/${kwant.id}`, {
        method: 'PUT',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify(kwantData),
      });
      if (res.status === 409) return `Another agent is already named "${agentData.name}".`;
      if (res.status === 400) return 'Check the name (no spaces or @) and that the profile belongs to the chosen engine.';
      if (!res.ok) return `Saving failed (HTTP ${res.status}).`;

      const updated: Kwant = await res.json();
      setKwants((prev) => prev.map((k) => (k.id === updated.id ? updated : k)));
      if (updated.name !== kwant.name) {
        const rename = (list: ChatMessage[]) =>
          list.map((m) =>
            m.sender_role === 'kwant' && m.sender_id === kwant.name ? { ...m, sender_id: updated.name } : m,
          );
        for (const key of Object.keys(messagesCacheRef.current)) {
          messagesCacheRef.current[key] = rename(messagesCacheRef.current[key]);
        }
        setMessages((prev) => rename(prev));
      }
      const mcpError = await saveKwantMcpServers(updated.id, mcp_server_ids);
      if (mcpError) return mcpError;
      playSfx('click');
    } catch (err) {
      console.error('Failed updating Agent:', err);
      return 'Could not reach the server.';
    }
  };

  const handleCreateAgent = async (agentData: AgentFormData): Promise<string | void> => {
    try {
      const res = await fetch('/api/kwants', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({
          name: agentData.name,
          title: agentData.title,
          persona_prompt: agentData.persona_prompt,
          soul: agentData.soul ?? '',
          avatar_sprite: agentData.avatar_sprite,
          driver_type: 'api',
          preferred_model: agentData.preferred_model,
          ...(agentData.profile_id !== undefined ? { profile_id: agentData.profile_id } : {}),
          ...(agentData.profile_failover !== undefined ? { profile_failover: agentData.profile_failover } : {}),
        }),
      });

      if (res.ok) {
        const created: Kwant = await res.json();
        setKwants((prev) => [...prev, created]);
        // Saved even when empty: switching every server off means none.
        const mcpError = await saveKwantMcpServers(created.id, agentData.mcp_server_ids);
        if (mcpError) console.error(mcpError);
        setIsNewAgentOpen(false);
        playSfx('fanfare');
        const team = newAgentTeam && (teams.find((t) => t.id === newAgentTeam.id) ?? newAgentTeam);
        setNewAgentTeam(null);
        if (team) {
          const memberIds = memberIdsOf(team);
          await handleUpdateTeamMembers(team, [...memberIds.filter((id) => id !== created.id), created.id]);
          handleSelectTeam(team);
          return;
        }
        setSelectedId(created.id);
        selectedIdRef.current = created.id;
        setIsChannel(false);
        isChannelRef.current = false;
        setSelectedTeamId(null);
        messagesCacheRef.current[created.id] = [];
        setMessages([]);
        return;
      }
      if (res.status === 409) return `Another agent is already named "${agentData.name}".`;
      if (res.status === 400) return 'Check the name (no spaces or @) and that the profile belongs to the chosen engine.';
      return `Creating failed (HTTP ${res.status}).`;
    } catch (err) {
      console.error('Failed creating Agent:', err);
      return 'Could not reach the server.';
    }
  };

  const handleCreateTeam = async (teamData: {
    name: string;
    description?: string;
    workspace_path?: string;
    repo_url?: string;
    lead_kwant_id: string;
    member_kwants: string[];
    icon?: string;
  }) => {
    try {
      const res = await fetch('/api/teams', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify(teamData),
      });

      if (res.ok) {
        const createdRaw = await res.json();
        const created: Team = {
          ...createdRaw,
          member_kwants: Array.isArray(createdRaw.member_kwants)
            ? createdRaw.member_kwants
            : JSON.parse(createdRaw.member_kwants || '[]'),
        };
        setTeams((prev) => {
          const exists = prev.some((t) => t.id === created.id);
          return exists ? prev.map((t) => (t.id === created.id ? created : t)) : [...prev, created];
        });
        setSelectedTeamId(created.id);

        // Fetch refreshed channels because backend auto-creates team-${created.name}
        const cRes = await fetch('/api/channels');
        if (cRes.ok) {
          const freshChannels: Channel[] = await cRes.json();
          setChannels(freshChannels);
          const teamChan = freshChannels.find(
            (c) => c.name === `team-${created.name}` || c.id === created.id
          );
          if (teamChan) {
            setSelectedId(teamChan.id);
            selectedIdRef.current = teamChan.id;
            setIsChannel(true);
            isChannelRef.current = true;
            messagesCacheRef.current[teamChan.id] = [];
            setMessages([]);
          }
        }

        setIsNewTeamOpen(false);
        playSfx('fanfare');
      }
    } catch (err) {
      console.error('Failed creating team:', err);
    }
  };

  const handleUpdateTeam = async (
    id: string,
    data: {
      name: string;
      description?: string;
      workspace_path?: string;
      repo_url?: string;
      lead_kwant_id: string;
      member_kwants: string[];
      instructions?: string;
    }
  ) => {
    try {
      const res = await fetch(`/api/teams/${id}`, {
        method: 'PUT',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify(data),
      });

      if (res.ok) {
        const updatedRaw = await res.json();
        const updated: Team = {
          ...updatedRaw,
          member_kwants: Array.isArray(updatedRaw.member_kwants)
            ? updatedRaw.member_kwants
            : JSON.parse(updatedRaw.member_kwants || '[]'),
        };
        setTeams((prev) => prev.map((t) => (t.id === id ? updated : t)));

        // Refresh channels in case team name or description changed
        const cRes = await fetch('/api/channels');
        if (cRes.ok) {
          const freshChannels: Channel[] = await cRes.json();
          setChannels(freshChannels);
        }

        setEditingTeam(null);
        playSfx('fanfare');
      }
    } catch (err) {
      console.error('Failed updating team:', err);
    }
  };

  const handleUpdateTeamMembers = async (team: Team, memberIds: string[]) => {
    const previous = teams;
    // Optimistic update so the sidebar reacts instantly
    setTeams((prev) => prev.map((t) => (t.id === team.id ? { ...t, member_kwants: memberIds } : t)));
    try {
      const res = await fetch(`/api/teams/${team.id}`, {
        method: 'PUT',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({
          name: team.name,
          description: team.description,
          icon: team.icon,
          workspace_path: team.workspace_path,
          repo_url: team.repo_url,
          lead_kwant_id: team.lead_kwant_id,
          member_kwants: memberIds,
        }),
      });
      if (!res.ok) throw new Error(`HTTP ${res.status}`);
      const cRes = await fetch('/api/channels');
      if (cRes.ok) setChannels(await cRes.json());
      playSfx('click');
    } catch (err) {
      console.error('Failed updating team members:', err);
      setTeams(previous);
    }
  };

  const handleDeleteTeam = async (id: string) => {
    try {
      const res = await fetch(`/api/teams/${id}`, {
        method: 'DELETE',
      });

      if (res.ok) {
        setTeams((prev) => prev.filter((t) => t.id !== id));
        if (selectedTeamId === id) {
          setSelectedTeamId(null);
          if (channels.length > 0) {
            setSelectedId(channels[0].id);
            setIsChannel(true);
          }
        }

        // Refresh channels
        const cRes = await fetch('/api/channels');
        if (cRes.ok) {
          const freshChannels: Channel[] = await cRes.json();
          setChannels(freshChannels);
        }

        setEditingTeam(null);
        playSfx('click');
      }
    } catch (err) {
      console.error('Failed deleting team:', err);
    }
  };

  const currentChannel = isChannel ? channels.find((c) => c.id === selectedId) : null;
  const currentKwant = !isChannel ? kwants.find((k) => k.id === selectedId) : null;
  const currentTeam = selectedTeamId
    ? teams.find((t) => t.id === selectedTeamId)
    : isChannel && currentChannel?.name.startsWith('team-')
    ? teams.find((t) => `team-${t.name}` === currentChannel.name)
    : null;

  const activeSandboxId = currentTeam?.id || currentKwant?.id || selectedTeamId || (isChannel ? selectedId : 'default');
  const activeSandboxName = currentTeam?.name || currentKwant?.name || (isChannel ? currentChannel?.name.replace(/^team-/, '') : 'Sandbox');

  activeTeamIdRef.current = activeSandboxId;
  activeSandboxIdRef.current = activeSandboxId;
  currentKwantRef.current = currentKwant || null;
  currentTeamRef.current = currentTeam || null;

  const currentTeamMemberIds: string[] = currentTeam
    ? Array.isArray(currentTeam.member_kwants)
      ? currentTeam.member_kwants
      : typeof currentTeam.member_kwants === 'string'
      ? JSON.parse(currentTeam.member_kwants || '[]')
      : []
    : [];

  const currentTeamMembers = currentTeam
    ? kwants.filter((k) => currentTeamMemberIds.includes(k.id))
    : [];

  const chatTitle = !isChannel
    ? currentKwant?.name || 'Agent'
    : currentTeam
    ? `@${currentTeam.name}`
    : currentChannel?.name.startsWith('team-')
    ? `@${currentChannel.name.replace(/^team-/, '')}`
    : `#${currentChannel?.name || 'general'}`;

  const boundProfile = currentKwant?.profile_id ? profiles.find((p) => p.id === currentKwant.profile_id) : undefined;
  const chatSubtitle = !isChannel
    ? `${currentKwant?.title || 'Specialist'} [Laya Auto-Fleet]${boundProfile ? ` · profile: ${boundProfile.name}` : ''}`
    : currentTeam?.description || currentChannel?.description || 'Team collaboration room';

  // Load container status and screenshot for active team or bot
  useEffect(() => {
    const sid = activeSandboxId;
    let cancelled = false;
    setScreenshotBase64(null);
    setActiveUrl('');
    setPageTitle('');
    setBrowserError(null);
    if (!sid || sid === 'default') return;

    fetch(`/api/sandbox/${sid}/status`)
      .then((r) => (r.ok ? r.json() : null))
      .then((data) => {
        if (!cancelled && data && typeof data.is_running === 'boolean') {
          setIsVmRunning(data.is_running);
        }
      })
      .catch(() => {});

    fetch(`/api/sandbox/${sid}/screenshot`)
      .then(async (r) => {
        const data = await r.json();
        if (!r.ok) throw new Error(data.error || 'Could not load the sandbox browser.');
        return data;
      })
      .then((data) => {
        if (!cancelled && data) {
          setIsVmRunning(true);
          setBrowserError(null);
          if (data.image_base64 || data.screenshot) {
            setScreenshotBase64(data.image_base64 || data.screenshot);
          }
          if (data.url) {
            setActiveUrl(data.url);
          }
        }
      })
      .catch((err) => {
        if (!cancelled) {
          setIsVmRunning(false);
          setBrowserError(err instanceof Error ? err.message : 'Could not reach the sandbox browser.');
        }
      });
    return () => { cancelled = true; };
  }, [activeSandboxId]);

  const handleRefreshVm = async () => {
    const sid = activeSandboxId;
    if (!sid || sid === 'default') return;
    try {
      const res = await fetch(`/api/sandbox/${sid}/screenshot`);
      const data = await res.json();
      if (activeSandboxIdRef.current !== sid) return;
      if (!res.ok) throw new Error(data.error || 'Could not load the sandbox browser.');
      setBrowserError(null);
      setIsVmRunning(true);
      if (data.image_base64 || data.screenshot) {
        setScreenshotBase64(data.image_base64 || data.screenshot);
      }
      if (data.url) {
        setActiveUrl(data.url);
      }
    } catch (err) {
      if (activeSandboxIdRef.current !== sid) return;
      setScreenshotBase64(null);
      setIsVmRunning(false);
      setBrowserError(err instanceof Error ? err.message : 'Could not reach the sandbox browser.');
    }
  };

  const handleRestartVm = async () => {
    const sid = activeSandboxId;
    if (!sid || sid === 'default') return;
    setBrowserError(null);
    setScreenshotBase64(null);
    try {
      const res = await fetch(`/api/sandbox/${sid}/restart`, {
        method: 'POST',
      });
      const data = await res.json();
      if (activeSandboxIdRef.current !== sid) return;
      if (!res.ok || !data.is_running) throw new Error(data.error || 'Sandbox could not start. Check Docker and try again.');
      setIsVmRunning(true);
      setTerminalLogs((prev) => [
        ...prev,
        {
          command: 'docker restart sandbox',
          output: `Sandbox container (@${activeSandboxName}) reset and restarted.`,
          timestamp: new Date().toLocaleTimeString(),
        },
      ]);
      setTimeout(() => {
        handleRefreshVm();
      }, 1000);
    } catch (err) {
      if (activeSandboxIdRef.current !== sid) return;
      setIsVmRunning(false);
      setBrowserError(err instanceof Error ? err.message : 'Could not restart the sandbox browser.');
    }
  };

  const handleNavigateVm = async (url: string) => {
    const sid = activeSandboxId;
    if (!sid || sid === 'default') return;
    setBrowserError(null);
    setIsVmPaneOpen(true);
    try {
      const res = await fetch(`/api/sandbox/${sid}/navigate`, {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ url }),
      });
      const data = await res.json();
      if (activeSandboxIdRef.current !== sid) return;
      if (!res.ok) throw new Error(data.error || 'Browser navigation failed.');
      if (data.url) setActiveUrl(data.url);
      if (data.title) setPageTitle(data.title);
      setIsVmPaneOpen(true);
      handleRefreshVm();
    } catch (err) {
      if (activeSandboxIdRef.current !== sid) return;
      setBrowserError(err instanceof Error ? err.message : 'Browser navigation failed.');
    }
  };

  const closeProfiles = useCallback(() => setIsProfilesOpen(false), []);

  const isCurrentContextStreaming = openCtx in live;
  const openTurns = turnsIn(turns, openCtx);
  // Turns paused on a question wait for the person; they don't make the agent "busy".
  const awaitingTurns = turnsAwaitingAnswer(questions);
  const awaitingHere = openTurns.some((t) => t.state === 'running' && awaitingTurns.has(t.turn_id)) && !openTurns.some((t) => t.state === 'running' && !awaitingTurns.has(t.turn_id));
  const waiting = openTurns.some((t) => t.state === 'running') ? undefined : openTurns.find((t) => t.state === 'queued');
  const queuedHere: QueuedState | null = waiting
    ? { position: queuePosition(turns, waiting.turn_id), reason: waiting.reason ?? 'agent_busy', speaker: kwantName(waiting.kwant_id) ?? live[openCtx] ?? 'The agent' }
    : null;

  // The agent's conversations, the open thread's controls, and "working on…" elsewhere.
  const agentThreads = currentKwant ? threadsByKwant[currentKwant.id] : undefined;
  const shownThread = openThreadId ? agentThreads?.find((t) => t.id === openThreadId) ?? null : null;
  const elsewhere = currentKwant ? backgroundTurns(turns, currentKwant.id).filter((t) => t.context_id !== openCtx) : [];
  const describeContext = (ctx: string, kwantId: string): string =>
    threadsByKwant[kwantId]?.find((t) => t.id === ctx)?.title ??
    (ctx === kwantId ? 'the direct chat' : channels.find((c) => c.id === ctx) ? channelLabel(channels.find((c) => c.id === ctx)!.name) : 'another conversation');
  const workingBadge =
    currentKwant && elsewhere.length > 0 ? (
      <button
        type="button"
        onClick={() => {
          const t = elsewhere[0];
          if (t.thread_id) openThread(t.kwant_id, t.thread_id);
          else if (t.context_id === t.kwant_id) openDirectChat();
          else openTarget({ channel_id: t.context_id, kwant_id: null });
        }}
        className="inline-flex items-center gap-1.5 min-w-0 max-w-[16rem] sm:max-w-xs px-2 py-0.5 rounded-full border border-sky-500/30 bg-sky-500/10 text-[11px] text-sky-200 hover:bg-sky-500/20 focus-visible:outline focus-visible:outline-2 focus-visible:outline-sky-500"
        title={elsewhere.map((t) => describeContext(t.context_id, t.kwant_id)).join(', ')}
      >
        {awaitingTurns.has(elsewhere[0].turn_id) ? (
          <HelpCircle size={10} className="shrink-0 text-amber-300" aria-hidden="true" />
        ) : (
          <Loader2 size={10} className="shrink-0 animate-spin motion-reduce:animate-none" aria-hidden="true" />
        )}
        <span className="truncate">
          {elsewhere[0].state === 'queued' ? 'Waiting to work on ' : awaitingTurns.has(elsewhere[0].turn_id) ? 'Asked you a question in ' : 'Working on '}
          {describeContext(elsewhere[0].context_id, elsewhere[0].kwant_id)}
        </span>
        {elsewhere.length > 1 && <span className="shrink-0 text-sky-300/80">+{elsewhere.length - 1} more</span>}
      </button>
    ) : null;
  const pendingSkillCounts = pendingByKwant(pendingSkills);
  const pendingHere = currentKwant ? pendingSkillCounts[currentKwant.id] ?? 0 : 0;
  const skillsBadge =
    currentKwant && pendingHere > 0 ? (
      <button
        type="button"
        onClick={() => openAgentSettings(currentKwant, 'skills')}
        className="inline-flex items-center gap-1 shrink-0 px-2 py-0.5 rounded-full border border-amber-500/30 bg-amber-500/10 text-[11px] text-amber-200 hover:bg-amber-500/20 focus-visible:outline focus-visible:outline-2 focus-visible:outline-amber-400"
      >
        <ListChecks size={11} aria-hidden="true" />
        {pendingHere === 1 ? '1 skill to review' : `${pendingHere} skills to review`}
      </button>
    ) : null;
  // Sidebar: agents busy somewhere other than the open chat.
  const busyKwants: Record<string, string> = {};
  for (const t of Object.values(turns)) {
    if (t.context_id === openCtx || busyKwants[t.kwant_id]) continue;
    busyKwants[t.kwant_id] = `${t.state === 'queued' ? 'Waiting to work on' : awaitingTurns.has(t.turn_id) ? 'Asked you a question in' : 'Working on'} ${describeContext(t.context_id, t.kwant_id)}`;
  }

  const errorBar = messagesError ? (
    <div role="alert" className="px-3 sm:px-6 py-2 border-b border-rose-500/20 bg-rose-500/5 flex items-center gap-2 text-xs text-rose-300">
      <span>{messagesError}</span>
      <button
        type="button"
        onClick={() => refreshContext(openCtx)}
        className="px-2 py-0.5 rounded border border-rose-500/30 hover:bg-rose-500/10 focus-visible:outline focus-visible:outline-2 focus-visible:outline-rose-500"
      >
        Retry
      </button>
    </div>
  ) : null;
  const subHeader = currentKwant ? (
    <>
      <ThreadBar
        kwantName={currentKwant.name}
        threads={agentThreads}
        error={threadErrors[currentKwant.id] ?? null}
        openThreadId={openThreadId}
        turns={turns}
        awaiting={awaitingTurns}
        directBadge={threadBadge({ id: currentKwant.id, delegation: null }, turns, awaitingTurns)}
        showArchived={showArchived}
        onToggleArchived={() => setShowArchived((v) => !v)}
        onOpenDirect={openDirectChat}
        onOpenThread={(t) => openThread(t.kwant_id, t.id)}
        onNewThread={handleNewThread}
        onRetry={() => loadThreads(currentKwant.id)}
      />
      {shownThread && (
        <ThreadHeader
          thread={shownThread}
          badge={threadBadge(
            { id: shownThread.id, delegation: shownThread.delegation ? delegations[shownThread.delegation.id] ?? shownThread.delegation : null },
            turns,
            awaitingTurns,
          )}
          delegatedBy={kwantName(shownThread.delegation?.lead_kwant_id ?? shownThread.created_by)}
          onBack={openDirectChat}
          onStop={() => handleStopThread(shownThread)}
          onArchive={(archived) => handleArchiveThread(shownThread, archived)}
        />
      )}
      {errorBar}
    </>
  ) : (
    errorBar
  );
  const pinned: PinnedItem[] = [];
  if (currentTeam?.instructions?.trim()) pinned.push({ label: `Team @${currentTeam.name}`, text: currentTeam.instructions });
  if (isChannel && currentChannel?.instructions?.trim())
    pinned.push({ label: `Channel ${channelLabel(currentChannel.name)}`, text: currentChannel.instructions });

  const isDiscussionActiveHere =
    discussionState.inProgress &&
    (discussionState.contextId === selectedId ||
      discussionState.teamId === selectedTeamId ||
      (isChannel && currentTeam && discussionState.teamId === currentTeam.id));

  return (
    <div className="h-screen w-screen flex flex-col bg-zinc-950 text-zinc-100 overflow-hidden font-sans">
      {/* Modern App Header */}
      <AppHeader
        onOpenSettings={() => setIsSettingsOpen(true)}
        onOpenTools={() => setIsMcpOpen(true)}
        onOpenSearch={() => setIsSearchOpen(true)}
        onOpenRoutines={() => setIsRoutinesOpen(true)}
        tokensUsed={tokensUsed}
        activeModel={currentKwant?.preferred_model || 'auto'}
      />

      {/* Main Workspace Shell */}
      <div className="flex-1 flex overflow-hidden">
        <Sidebar
          kwants={kwants}
          channels={channels}
          teams={teams}
          selectedId={selectedId}
          isChannel={isChannel}
          selectedTeamId={selectedTeamId}
          busyKwants={busyKwants}
          onSelectKwant={handleSelectKwant}
          onSelectChannel={handleSelectChannel}
          onSelectTeam={handleSelectTeam}
          onUpdateTeamMembers={handleUpdateTeamMembers}
          onManageTeam={(team) => {
            setEditingTeam(team);
            playSfx('click');
          }}
          onEditKwant={(kwant) => {
            openAgentSettings(kwant);
          }}
          onSummonKwant={(team) => {
            setNewAgentTeam(team ?? null);
            setIsNewAgentOpen(true);
            playSfx('click');
          }}
          onCreateTeam={() => {
            setIsNewTeamOpen(true);
            playSfx('click');
          }}
        />

        <ChatView
          title={chatTitle}
          subtitle={chatSubtitle}
          messages={messages}
          streamingText={isCurrentContextStreaming ? streamingText : ''}
          streamingThinking={isCurrentContextStreaming ? streamingThinking : ''}
          pendingApproval={pendingApproval}
          isStreaming={isCurrentContextStreaming}
          streamingSpeakerName={live[openCtx] || undefined}
          onSendMessage={handleSendMessage}
          onResolveApproval={handleResolveApproval}
          kwants={kwants}
          teams={teams}
          discussionState={isDiscussionActiveHere ? discussionState : { inProgress: false }}
          routingState={isCurrentContextStreaming ? routingState : null}
          onCancelDiscussion={handleCancelDiscussion}
          teamMembers={currentTeamMembers}
          teamId={activeSandboxId}
          teamName={activeSandboxName}
          workspacePath={currentTeam?.workspace_path}
          repoUrl={currentTeam?.repo_url}
          onManageTeam={
            currentTeam
              ? () => {
                  setEditingTeam(currentTeam);
                  playSfx('click');
                }
              : undefined
          }
          onEditKwant={
            currentKwant
              ? () => {
                  openAgentSettings(currentKwant);
                }
              : undefined
          }
          isVmPaneOpen={isVmPaneOpen}
          onToggleVmPane={() => setIsVmPaneOpen((prev) => !prev)}
          activeUrl={activeUrl}
          pageTitle={pageTitle}
          screenshotBase64={screenshotBase64}
          browserError={browserError}
          isRunning={isVmRunning}
          terminalLogs={terminalLogs}
          onRefreshVm={handleRefreshVm}
          onRestartVm={handleRestartVm}
          onNavigateVm={handleNavigateVm}
          pinned={pinned}
          onEditInstructions={
            isChannel && currentChannel
              ? () => {
                  setEditingChannelInstructions(currentChannel);
                  playSfx('click');
                }
              : undefined
          }
          highlightMessageId={highlightMessageId}
          onHighlightDone={() => setHighlightMessageId(null)}
          subHeader={subHeader}
          titleExtra={
            workingBadge || skillsBadge ? (
              <>
                {workingBadge}
                {skillsBadge}
              </>
            ) : null
          }
          queued={queuedHere}
          renderDelegation={renderDelegation}
          renderQuestion={renderQuestion}
          awaitingAnswer={awaitingHere}
          contextKey={openCtx}
        />
      </div>

      {/* New Agent Modal */}
      <Suspense fallback={null}>
        <NewAgentModal
          isOpen={isNewAgentOpen}
          onClose={() => {
            setIsNewAgentOpen(false);
            setNewAgentTeam(null);
          }}
          joinTeamName={newAgentTeam?.name}
          takenNames={kwants.map((k) => k.name)}
          profiles={profiles}
          onManageProfiles={() => setIsProfilesOpen(true)}
          onManageServers={() => setIsMcpOpen(true)}
          onSubmit={handleCreateAgent}
        />

        {/* Edit Agent Modal */}
        <NewAgentModal
          isOpen={!!editingKwant}
          initial={editingKwant}
          kwantId={editingKwant?.id}
          kwants={kwants}
          channels={channels}
          profiles={profiles}
          onManageProfiles={() => setIsProfilesOpen(true)}
          onManageServers={() => setIsMcpOpen(true)}
          initialTab={editingTab}
          pendingSkills={editingKwant ? pendingSkillCounts[editingKwant.id] ?? 0 : 0}
          onClose={() => setEditingKwant(null)}
          onSubmit={(data) => (editingKwant ? handleUpdateAgent(editingKwant, data) : undefined)}
        />
      </Suspense>

      {/* New Team Modal */}
      <NewTeamModal
        isOpen={isNewTeamOpen}
        onClose={() => setIsNewTeamOpen(false)}
        kwants={kwants}
        onCreateTeam={handleCreateTeam}
      />

      {/* Manage Team Modal */}
      <ManageTeamModal
        isOpen={!!editingTeam}
        team={editingTeam}
        kwants={kwants}
        onClose={() => setEditingTeam(null)}
        onUpdateTeam={handleUpdateTeam}
        onDeleteTeam={handleDeleteTeam}
      />

      {/* Settings Modal */}
      <Suspense fallback={null}>
        <SettingsModal
          isOpen={isSettingsOpen}
          onClose={() => setIsSettingsOpen(false)}
          onOpenTools={() => {
            setIsSettingsOpen(false);
            setIsMcpOpen(true);
          }}
          onOpenProfiles={() => {
            setIsSettingsOpen(false);
            setIsProfilesOpen(true);
          }}
        />
      </Suspense>

      {/* MCP tool servers (opened last so it sits above the agent modal) */}
      <Suspense fallback={null}>
        <McpServersModal isOpen={isMcpOpen} onClose={() => setIsMcpOpen(false)} kwants={kwants} />
        <ProfilesModal isOpen={isProfilesOpen} onClose={closeProfiles} onProfilesChanged={setProfiles} />
        {isSearchOpen && <SearchModal isOpen onClose={() => setIsSearchOpen(false)} kwants={kwants} onJump={handleJump} />}
        {isRoutinesOpen && <RoutinesModal isOpen onClose={() => setIsRoutinesOpen(false)} kwants={kwants} channels={channels} />}
      </Suspense>

      {editingChannelInstructions && (
        <Suspense fallback={null}>
          <ChannelInstructionsModal
            channelLabel={channelLabel(editingChannelInstructions.name)}
            initial={editingChannelInstructions.instructions ?? ''}
            onClose={() => setEditingChannelInstructions(null)}
            onSave={saveChannelInstructions}
          />
        </Suspense>
      )}

      <Toasts toasts={toasts} onDismiss={dismissToast} />
    </div>
  );
};
