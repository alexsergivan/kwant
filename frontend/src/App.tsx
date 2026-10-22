import React, { Suspense, lazy, useState, useEffect, useRef, useCallback, useMemo } from 'react';
import { Sidebar, Kwant, Channel, Team, memberIdsOf } from './components/Sidebar';
import { SpaceSettings, SpaceSettingsTab } from './components/SpaceSettings';
import { SpaceTile } from './components/Spaces';
import { ChatView, ChatMessage, DiscussionState, RoutingState, QueuedState } from './components/ChatView';
import { ThreadBar, ThreadHeader } from './components/ThreadBar';
import { DelegationCard } from './components/DelegationCard';
import { ApprovalsOffBadge } from './components/ApprovalsSetting';
import { APPROVALS_CHANGED_EVENT, approvalsOff } from './approvalMode';
import { QuestionCard } from './components/QuestionCard';
import { McpSuggestionCard } from './components/McpSuggestionCard';
import { Suggestion, SuggestionMeta, applySuggestion, suggestionsApi } from './mcpSuggestions';
import { ApiError, BotActionMeta, HireMeta, applyBotAction, applyHire, botActionsApi, changesBots, decisionError, hiresApi } from './botActions';
import { BotActionCard, type BotActionDecision } from './components/BotActionCard';
import { HireCard, type HireDecision } from './components/HireCard';
import type { RoutineFocus } from './components/RoutinesPanel';
import { AwayPage } from './components/AwayView';
import { LooseEndsPage } from './components/LooseEndsView';
import { ParallelPage } from './components/ParallelUniverseView';
import { AlertTriangle, Bot, Check, HelpCircle, Inbox, KeyRound, ListChecks, Loader2, MessageSquare, Settings } from 'lucide-react';
import { ChatApproval, OpenChat, approvalFromEvent, lateAnswerText, approvalsForChat, approvalsFromAway, syncApprovals, upsertApproval, withoutApproval } from './approvals';
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
import { AwayItem, affectsAway, awayApi, awayTarget, badgeText } from './away';
import { Space, eventsUrl, pickInitialChat, spaceColor } from './spaces';
import { canRunNow } from './engines';
import { useEngines } from './useEngines';
import { claudeNeedsSignIn } from './claudeLogin';
import { personFetch } from './auth';

// Kwant and Tools editors are their own chunks. They stay mounted (rendering nothing while closed),
// so both load right after start-up and opening them never waits.
const DeleteBotDialog = lazy(() => import('./components/DeleteBotDialog').then((m) => ({ default: m.DeleteBotDialog })));
const NewAgentModal = lazy(() => import('./components/NewAgentModal').then((m) => ({ default: m.NewAgentModal })));
const McpServersModal = lazy(() => import('./components/McpServersModal').then((m) => ({ default: m.McpServersModal })));
const ChannelInstructionsModal = lazy(() =>
  import('./components/ChannelInstructionsModal').then((m) => ({ default: m.ChannelInstructionsModal })),
);
const SearchModal = lazy(() => import('./components/SearchModal').then((m) => ({ default: m.SearchModal })));
const RoutinesModal = lazy(() => import('./components/RoutinesModal').then((m) => ({ default: m.RoutinesModal })));

/** Where to land when a space opens: a channel or a bot's direct chat. */
export interface SpaceTarget {
  channelId?: string;
  botId?: string;
}

export interface SpaceAppProps {
  space: Space;
  spaces: Space[];
  initialTarget: SpaceTarget | null;
  /** Space settings open on this tab when the space mounts (Accounts, right after creating it). */
  initialSettingsTab?: SpaceSettingsTab | null;
  onSwitchSpace: (id: string, to?: SpaceTarget | null) => void;
  onCreateSpace: () => void;
  onOpenArchived: () => void;
  onOpenAppSettings: () => void;
  /** Re-reads the space list (counts, #general appearing, a rename). */
  onSpacesChanged: () => void;
  onSpaceArchived: (space: Space) => void;
  /** The open chat changed, for the address bar and reopening it later. */
  onChatChange: (target: SpaceTarget) => void;
  onTokensUsed: (n: number) => void;
}

const PHONE_QUERY = '(max-width: 767px)';

/** Below 768px the space is one column: a list or a chat, with a tab bar. */
function useIsPhone(): boolean {
  const [phone, setPhone] = useState(() => typeof window !== 'undefined' && !!window.matchMedia?.(PHONE_QUERY).matches);
  useEffect(() => {
    const mq = window.matchMedia?.(PHONE_QUERY);
    if (!mq) return;
    const on = () => setPhone(mq.matches);
    mq.addEventListener('change', on);
    return () => mq.removeEventListener('change', on);
  }, []);
  return phone;
}

type PhoneTab = 'chats' | 'bots' | 'inbox' | 'space';

/** One space: its bots, teams, channels and chats. Remounted for every space, so nothing carries over. */
export const SpaceApp: React.FC<SpaceAppProps> = ({
  space,
  spaces,
  initialTarget,
  initialSettingsTab = null,
  onSwitchSpace,
  onCreateSpace,
  onOpenArchived,
  onOpenAppSettings,
  onSpacesChanged,
  onSpaceArchived,
  onChatChange,
  onTokensUsed,
}) => {
  const isPhone = useIsPhone();
  // Phones: the list (one tab of it) or the open chat.
  const [phoneView, setPhoneView] = useState<'list' | 'chat'>(() => (initialTarget?.channelId || initialTarget?.botId ? 'chat' : 'list'));
  const [phoneTab, setPhoneTab] = useState<PhoneTab>('chats');
  const [loaded, setLoaded] = useState(false);
  const [kwants, setKwants] = useState<Kwant[]>([]);
  // Whether the bot list is as new as the last bot change seen. Until then cards can't tell a
  // deleted bot from one not fetched yet (a card for a new bot arrives before the list reloads).
  const [rosterFresh, setRosterFresh] = useState(false);
  const botIds = useMemo(() => (rosterFresh ? new Set(kwants.map((k) => k.id)) : null), [kwants, rosterFresh]);
  const [channels, setChannels] = useState<Channel[]>([]);
  const [teams, setTeams] = useState<Team[]>([]);
  const [selectedId, setSelectedId] = useState<string>('');
  const [isChannel, setIsChannel] = useState(true);
  const [selectedTeamId, setSelectedTeamId] = useState<string | null>(null);
  const [messages, setMessages] = useState<ChatMessage[]>([]);
  const [messagesError, setMessagesError] = useState<string | null>(null);
  // The open thread of the selected bot; null for its direct chat (and for channels).
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
  // Every tool approval still waiting, from live events and, after a reload or reconnect, the server.
  const [pendingApprovals, setPendingApprovals] = useState<ChatApproval[]>([]);
  // Seen live since the last server list: kept even if that list was fetched just before.
  const liveApprovalIdsRef = useRef<Set<string>>(new Set());
  const dropApproval = (id: string) => {
    liveApprovalIdsRef.current.delete(id);
    setPendingApprovals((prev) => withoutApproval(prev, id));
  };
  const [tokensUsed, setTokensUsed] = useState(0);

  // Space settings: open on one tab, or closed.
  const [spaceSettingsTab, setSpaceSettingsTab] = useState<SpaceSettingsTab | null>(initialSettingsTab);
  // Bumped when Space settings closes, so open bot forms read account status again.
  const [settingsClosedCount, setSettingsClosedCount] = useState(0);
  // Set when Accounts opens from a "Sign in with Claude" hint, to focus that button.
  const [claudeSignInFocus, setClaudeSignInFocus] = useState(0);
  // Set by a routine card's Edit or Pause, so Routines opens at that routine.
  const [routineFocus, setRoutineFocus] = useState<RoutineFocus | null>(null);
  const openSpaceSettings = (tab: SpaceSettingsTab = 'general') => {
    setClaudeSignInFocus(0);
    if (tab !== 'routines') setRoutineFocus(null);
    setSpaceSettingsTab(tab);
    playSfx('click');
  };
  const openSettings = () => openSpaceSettings('accounts');
  const openClaudeSignIn = () => {
    setClaudeSignInFocus(Date.now());
    setSpaceSettingsTab('accounts');
  };
  // This space's engine checks, shared with the sidebar: the setup banner, the "can't answer"
  // card and the bots' online dots all read whether anything can run.
  const engines = useEngines(true);
  const canRun = canRunNow(engines.engines);
  const offerClaude = canRun === false && claudeNeedsSignIn(engines.engines);
  const closeSpaceSettings = () => {
    setSpaceSettingsTab(null);
    setSettingsClosedCount((n) => n + 1);
    // Accounts may have changed: the getting-started list counts the ready ones.
    onSpacesChanged();
  };
  const [isMcpOpen, setIsMcpOpen] = useState(false);
  const [isSearchOpen, setIsSearchOpen] = useState(false);
  const [isRoutinesOpen, setIsRoutinesOpen] = useState(false);
  const [editingChannelInstructions, setEditingChannelInstructions] = useState<Channel | null>(null);
  const [highlightMessageId, setHighlightMessageId] = useState<string | null>(null);
  // Older messages shown instead of the latest (a search hit beyond them). Kept apart from the
  // cache, which live updates keep current underneath.
  const [historyWindow, setHistoryWindow] = useState<{ ctx: string; messages: ChatMessage[] } | null>(null);
  const [profiles, setProfiles] = useState<Profile[]>([]);
  const [toasts, setToasts] = useState<Toast[]>([]);
  // "While you were away": open instead of a chat, and its unseen count for the sidebar badge.
  const [showAway, setShowAway] = useState(false);
  const [showLoose, setShowLoose] = useState(false);
  const [showParallel, setShowParallel] = useState(false);
  const [awayCount, setAwayCount] = useState(0);
  const awaySinceRef = useRef<string | null>(null);
  const awayTimerRef = useRef<ReturnType<typeof setTimeout> | undefined>(undefined);
  const refreshAwayCount = (delay = 0) => {
    clearTimeout(awayTimerRef.current);
    awayTimerRef.current = setTimeout(() => {
      awayApi.get().then((r) => {
        setAwayCount(r.unseen.total);
        const fromServer = approvalsFromAway(r.waiting);
        const live = liveApprovalIdsRef.current;
        setPendingApprovals((prev) => syncApprovals(prev, fromServer, live));
        for (const a of fromServer) live.delete(a.id);
      }, () => undefined);
    }, delay);
  };
  const toastIdRef = useRef(0);
  const dismissToast = useCallback((id: number) => setToasts((prev) => prev.filter((t) => t.id !== id)), []);
  // Set each render once the select handlers exist; the SSE handler and notification clicks go through it.
  // Opens a bot's settings on the Skills tab (set once the bots are known).
  const openSkillsRef = useRef<(kwantId: string) => void>(() => undefined);
  // Opens the chat of a bot's newest open question; false when it has none.
  const openQuestionRef = useRef<(kwantId: string) => boolean>(() => false);
  const openTargetRef = useRef<(t: { channel_id: string | null; kwant_id: string | null }) => boolean>(() => false);
  const showNotification = (n: NotifyEvent) => {
    // Another space's news is the shell's to show, with a way to switch there.
    if (n.space_id && n.space_id !== space.id) return;
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
  // Bots, teams and the space's counts again after a bot was hired, made, undone or deleted elsewhere, so the sidebar
  // matches the server without a reload. Bursts of events share one load; a late reply never
  // overwrites a newer one.
  const onSpacesChangedRef = useRef(onSpacesChanged);
  onSpacesChangedRef.current = onSpacesChanged;
  const rosterTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  const rosterSeqRef = useRef(0);
  const refreshRoster = useCallback(() => {
    if (rosterTimerRef.current) clearTimeout(rosterTimerRef.current);
    rosterTimerRef.current = setTimeout(async () => {
      rosterTimerRef.current = null;
      const seq = ++rosterSeqRef.current;
      try {
        const [kRes, tRes] = await Promise.all([
          fetch('/api/kwants').then((r) => (r.ok ? r.json() : Promise.reject(new Error(`HTTP ${r.status}`)))),
          fetch('/api/teams').then((r) => (r.ok ? r.json() : null)).catch(() => null),
        ]);
        if (seq !== rosterSeqRef.current || !Array.isArray(kRes)) return;
        setKwants(kRes);
        setRosterFresh(true);
        // The space header counts bots too.
        onSpacesChangedRef.current();
        if (Array.isArray(tRes)) {
          setTeams(tRes.map((t: any) => ({
            ...t,
            member_kwants: Array.isArray(t.member_kwants)
              ? t.member_kwants
              : typeof t.member_kwants === 'string'
              ? JSON.parse(t.member_kwants || '[]')
              : [],
          })));
        }
      } catch {
        // The next change or a reload tries again; the list on screen stays as it was.
      }
    }, 150);
  }, []);
  useEffect(() => () => {
    if (rosterTimerRef.current) clearTimeout(rosterTimerRef.current);
  }, []);
  // "Turn approvals back on" from a password screen: the markers and space header catch up.
  useEffect(() => {
    window.addEventListener(APPROVALS_CHANGED_EVENT, refreshRoster);
    return () => window.removeEventListener(APPROVALS_CHANGED_EVENT, refreshRoster);
  }, [refreshRoster]);
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
    refreshAwayCount();
    questionsApi.listOpen().then((list) => {
      // Closed ones seen live stay; open ones come from the server.
      setQuestions((prev) => {
        const next = hydrateQuestions(list);
        for (const q of Object.values(prev)) if (q.status !== 'open' && !next[q.id]) next[q.id] = q;
        return next;
      });
    }, () => undefined);
    suggestionsApi.list().then((list) => setSuggestions((prev) => list.reduce(applySuggestion, prev)), () => undefined);
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
  // Team that a newly created bot should join (set by a team's "Add member")
  const [newAgentTeam, setNewAgentTeam] = useState<Team | null>(null);
  const [isNewTeamOpen, setIsNewTeamOpen] = useState(false);
  const [editingTeam, setEditingTeam] = useState<Team | null>(null);
  const [editingKwant, setEditingKwant] = useState<Kwant | null>(null);
  const [deletingKwant, setDeletingKwant] = useState<Kwant | null>(null);
  const [editingTab, setEditingTab] = useState<EditTab>('profile');
  // Skills waiting for review, for the counts on the bot's header and Skills tab.
  const [pendingSkills, setPendingSkills] = useState<Skill[]>([]);
  // Bots' questions by id: open ones from the server, plus every question_updated since.
  const [questions, setQuestions] = useState<Record<string, Question>>({});
  const questionsRef = useRef(questions);
  questionsRef.current = questions;
  // Bots' tool suggestions by id (from the list and mcp_suggestion_updated), and installed
  // servers that still need the person to sign in.
  const [suggestions, setSuggestions] = useState<Record<string, Suggestion>>({});
  const [needsSignIn, setNeedsSignIn] = useState<Record<string, boolean>>({});
  // Cards for changes bots made (bot_action_updated) and hire proposals (hire_updated), by id.
  const [botActions, setBotActions] = useState<Record<string, BotActionMeta>>({});
  const [hires, setHires] = useState<Record<string, HireMeta>>({});

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
        if (Array.isArray(kRes)) setRosterFresh(true);
        setChannels(cRes);
        setTeams(normalizedTeams);

        // The chat asked for (a link, a reload, a notification, the last one open here) if it still exists.
        const first = pickInitialChat(initialTarget, Array.isArray(kRes) ? (kRes as Kwant[]) : [], Array.isArray(cRes) ? (cRes as Channel[]) : [], space.general_channel_id);
        if (first?.kind === 'bot') {
          setSelectedId(first.bot.id);
          selectedIdRef.current = first.bot.id;
          setIsChannel(false);
          isChannelRef.current = false;
          setSelectedTeamId(null);
        } else if (first?.kind === 'channel') {
          const init = first.channel;
          setSelectedId(init.id);
          selectedIdRef.current = init.id;
          setIsChannel(true);
          isChannelRef.current = true;
          const matchedTeam = normalizedTeams.find(
            (t) => `team-${t.name}` === init.name || t.id === init.id
          );
          setSelectedTeamId(matchedTeam ? matchedTeam.id : null);
        }
      } catch (err) {
        console.error('Failed to load initial data:', err);
      } finally {
        setLoaded(true);
      }
    }
    loadInitialData();
    refreshProfiles();
  }, [refreshProfiles]);

  // The open chat (channel, direct chat or thread): show what's cached, then load the latest.
  useEffect(() => {
    setHistoryWindow((w) => (w && w.ctx !== openCtx ? null : w));
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

  // A bot's conversations load when its chat opens; SSE keeps them current from then on.
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
    const sse = new EventSource(eventsUrl(space.id));
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
    // whatever changed meanwhile in the open chat and the open bot's conversations.
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
        if (affectsAway(event.type)) refreshAwayCount(800);
        if (changesBots(event.type, event.data)) {
          setRosterFresh(false);
          refreshRoster();
        }
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
          case 'mcp_suggestion_updated':
          case 'McpSuggestionUpdated': {
            const sg: Suggestion | undefined = event.data?.suggestion;
            if (sg?.id) setSuggestions((prev) => applySuggestion(prev, sg));
            break;
          }
          case 'bot_action_updated':
          case 'BotActionUpdated': {
            const a = event.data?.action;
            if (a?.id) setBotActions((prev) => applyBotAction(prev, a));
            break;
          }
          case 'hire_updated':
          case 'HireUpdated': {
            const h = event.data?.hire;
            if (h?.id) {
              setHires((prev) => applyHire(prev, h));
              window.dispatchEvent(new Event('kwnt:hire-updated'));
            }
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
              candidateCount: typeof event.data?.candidate_count === 'number' ? event.data.candidate_count : prev?.candidateCount ?? null,
              fallback: !!event.data?.fallback,
              matchedTerms: Array.isArray(event.data?.matched_terms) ? event.data.matched_terms : undefined,
              basis: typeof event.data?.basis === 'string' ? event.data.basis : undefined,
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
          case 'ApprovalRequired': {
            const approval = approvalFromEvent(event.data);
            liveApprovalIdsRef.current.add(approval.id);
            setPendingApprovals((prev) => upsertApproval(prev, approval));
            playSfx('alert');
            break;
          }
          case 'ApprovalWithdrawn':
            dropApproval(event.data.id);
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

  const handleSendMessage = async (text: string, attachments: AttachmentRef[] = []): Promise<boolean> => {
    setHistoryWindow(null);
    const targetId = openCtx;
    const threadId = openThreadId;
    const isTargetChannel = isChannel;
    const targetTeam = selectedTeamId;
    const speakerName = !isTargetChannel ? (currentKwant?.name || 'Bot') : chatTitle;

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
      return true;
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
      return false;
    }
  };

  const handleResolveApproval = async (
    ticketId: string,
    decision: 'allow_once' | 'always_allow' | 'deny'
  ) => {
    try {
      const res = await fetch(`/api/approvals/${ticketId}/resolve`, {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ decision }),
      });
      // 409/404: it lapsed meanwhile (turn ended, expired, answered elsewhere).
      if (res.ok || res.status === 409 || res.status === 404) {
        const card = pendingApprovals.find((a) => a.id === ticketId);
        dropApproval(ticketId);
        if (!res.ok) pushToast('This request expired', 'The bot is no longer waiting for this answer.');
        else if (card?.waiting && decision !== 'deny') {
          const bot = typeof card.details.kwant_name === 'string' ? card.details.kwant_name : null;
          const text = lateAnswerText(await res.json().catch(() => null), bot);
          if (text) pushToast('Card answered', text);
        }
        refreshAwayCount(300);
      } else if (res.status === 401) {
        // Person-only: the token dialog is open; the card stays so the person can answer again.
        pushToast('Enter your access token', 'Answering approval cards needs your token. Then answer the card again.');
      } else {
        pushToast("Couldn't send your answer", `Request failed (HTTP ${res.status}). Try again.`);
      }
    } catch (err) {
      console.error('Failed resolving approval:', err);
      pushToast("Couldn't send your answer", 'Check the connection and try again.');
    }
  };

  const handleSelectTeam = (team: Team) => {
    setOpenThreadId(null);
    setShowAway(false);
    setShowLoose(false);
    setShowParallel(false);
    setPhoneView('chat');
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
    setShowAway(false);
    setShowLoose(false);
    setShowParallel(false);
    setPhoneView('chat');
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
    setShowAway(false);
    setShowLoose(false);
    setShowParallel(false);
    setPhoneView('chat');
    setSelectedId(kwant.id);
    selectedIdRef.current = kwant.id;
    setIsChannel(false);
    isChannelRef.current = false;
    setSelectedTeamId(null);
    setMessages(messagesCacheRef.current[kwant.id] || []);
    playSfx('click');
  };

  /** Opens a channel, or the direct chat with a bot when there's no channel. False if it's gone. */
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

  /** Opens one of a bot's threads (from the thread list, a delegation card or a badge). */
  const openThread = (kwantId: string, threadId: string) => {
    const kwant = kwants.find((k) => k.id === kwantId);
    if (!kwant) {
      pushToast('That bot no longer exists.');
      return;
    }
    knownThreadsRef.current.add(threadId);
    setShowAway(false);
    setShowLoose(false);
    setShowParallel(false);
    if (selectedIdRef.current !== kwant.id || isChannelRef.current) handleSelectKwant(kwant);
    else playSfx('click');
    setOpenThreadId(threadId);
    if (threadsByKwantRef.current[kwantId] === undefined) loadThreads(kwantId);
  };

  /** Opens the thread, channel or direct chat an away item links to. */
  const openAwayItem = (item: AwayItem) => {
    const t = awayTarget(item);
    if ('thread' in t) return openThread(t.kwant, t.thread);
    const ok = 'channel' in t ? openTarget({ channel_id: t.channel, kwant_id: null }) : openTarget({ channel_id: null, kwant_id: t.direct });
    if (!ok) return pushToast('That chat no longer exists.');
    setShowAway(false);
    setShowLoose(false);
    setShowParallel(false);
  };

  const openDirectChat = () => {
    if (!openThreadId) return;
    setOpenThreadId(null);
    playSfx('click');
  };

  const errText = (err: unknown) => (err instanceof Error ? err.message : 'Something went wrong.');
  /** The server's own `error` text, e.g. why a team name isn't allowed. */
  const serverError = async (res: Response): Promise<string | null> => {
    try {
      const body = await res.clone().json();
      const text = typeof body?.error === 'string' ? body.error.trim() : '';
      // The server writes lowercase fragments ("a team called x already exists…"); the UI shows sentences.
      return text ? text[0].toUpperCase() + text.slice(1) : null;
    } catch {
      return null;
    }
  };

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

  const handleInstallSuggestion = async (id: string, body: { env?: Record<string, string>; headers?: Record<string, string> }) => {
    try {
      const res = await suggestionsApi.install(id, body);
      setSuggestions((prev) => applySuggestion(prev, res.suggestion));
      if (res.oauth_required && res.server?.id) setNeedsSignIn((prev) => ({ ...prev, [res.server.id]: true }));
      return {};
    } catch (err) {
      return { error: errText(err) };
    }
  };
  const handleDismissSuggestion = async (id: string): Promise<string | void> => {
    try {
      const sg = await suggestionsApi.dismiss(id);
      setSuggestions((prev) => applySuggestion(prev, sg));
    } catch (err) {
      return `Couldn't dismiss it: ${errText(err)}`;
    }
  };
  // Same flow as the servers screen: the window opens inside the click, the address comes after.
  const handleSuggestionSignIn = async (serverId: string): Promise<string | void> => {
    const popup = window.open('', 'kwnt-mcp-oauth', 'popup,width=520,height=720');
    let url: string;
    try {
      const res = await fetch(`/api/mcp/servers/${encodeURIComponent(serverId)}/oauth/start`, {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ origin: window.location.origin }),
      });
      const data = await res.json().catch(() => null);
      if (!res.ok) throw new Error(typeof data?.error === 'string' ? data.error : `Request failed (HTTP ${res.status}).`);
      if (typeof data?.authorize_url !== 'string' || !/^https?:\/\//.test(data.authorize_url)) throw new Error('The server didn’t return a sign-in page.');
      url = data.authorize_url;
    } catch (err) {
      popup?.close();
      return `Couldn't start the sign-in: ${errText(err)}`;
    }
    if (!popup || popup.closed) {
      window.location.assign(url);
      return;
    }
    popup.location.href = url;
    popup.focus();
    await new Promise<void>((resolve) => {
      const t = window.setInterval(() => {
        if (popup.closed) {
          window.clearInterval(t);
          resolve();
        }
      }, 500);
    });
    try {
      const test = await suggestionsApi.testServer(serverId);
      if (test.ok) {
        setNeedsSignIn((prev) => ({ ...prev, [serverId]: false }));
        return;
      }
      return test.auth_required ? 'The sign-in didn’t finish.' : test.error || 'The server still isn’t answering.';
    } catch (err) {
      return errText(err);
    }
  };
  const renderMcpSuggestion = (msg: ChatMessage, meta: SuggestionMeta, flash: boolean) => {
    const live = suggestions[meta.suggestion_id];
    const owner = live?.kwant_id ?? meta.kwant_id ?? undefined;
    const name = kwantName(owner);
    const serverId = live?.server_id ?? meta.server_id;
    return (
      <McpSuggestionCard
        meta={meta}
        messageId={msg.id}
        kwantName={name ?? (msg.sender_id && msg.sender_id.length <= 20 ? msg.sender_id : chatTitle)}
        kwantId={owner}
        live={live}
        needsSignIn={!!serverId && !!needsSignIn[serverId]}
        onInstall={handleInstallSuggestion}
        onDismiss={handleDismissSuggestion}
        onSignIn={handleSuggestionSignIn}
        flash={flash}
      />
    );
  };

  // A routine card's Edit or Pause: open Space settings → Routines at that routine.
  const handleOpenRoutine = (id: string, mode: 'edit' | 'pause') => {
    setRoutineFocus({ id, mode, nonce: Date.now() });
    openSpaceSettings('routines');
  };
  const handleBotActionDecision = async (actionId: string, d: BotActionDecision): Promise<string | void> => {
    try {
      const a = d === 'undo' ? await botActionsApi.undo(actionId) : d === 'confirm' ? await botActionsApi.confirm(actionId) : await botActionsApi.decline(actionId);
      setBotActions((prev) => applyBotAction(prev, a));
      playSfx('click');
    } catch (err) {
      // Already decided elsewhere: pick up the real state so the card stops offering the button.
      if (err instanceof ApiError && err.status === 409) {
        // "The bot no longer exists": the bot list drops it and the card settles.
        setRosterFresh(false);
        refreshRoster();
        botActionsApi
          .list()
          .then((list) => setBotActions((prev) => list.reduce((m, a) => applyBotAction(m, a), prev)))
          .catch(() => {});
      }
      return decisionError(err, d === 'undo' ? 'undo it' : d === 'confirm' ? 'confirm it' : 'decline it');
    }
  };
  const refreshHires = () =>
    hiresApi
      .list()
      .then((list) => setHires((prev) => list.reduce((m, h) => applyHire(m, h), prev)))
      .catch(() => {});
  const handleHireDecision = async (hire: HireMeta, d: HireDecision): Promise<string | void> => {
    try {
      if (d === 'undo') {
        if (!hire.action_id) return 'Couldn’t undo it: this hire has nothing to undo.';
        await botActionsApi.undo(hire.action_id);
        refreshHires();
      } else {
        const h = d === 'deny' ? await hiresApi.deny(hire.hire_id) : await hiresApi.accept(hire.hire_id, d === 'always_accept');
        setHires((prev) => applyHire(prev, h));
      }
      window.dispatchEvent(new Event('kwnt:hire-updated'));
      playSfx('click');
    } catch (err) {
      if (err instanceof ApiError && err.status === 409) {
        refreshHires();
        setRosterFresh(false);
        refreshRoster();
      }
      return decisionError(err, d === 'deny' ? 'say no' : d === 'undo' ? 'undo the hire' : 'hire it');
    }
  };
  const renderBotAction = (msg: ChatMessage, meta: BotActionMeta, flash: boolean) => {
    const owner = meta.kwant_id ?? undefined;
    return (
      <BotActionCard
        meta={meta}
        messageId={msg.id}
        kwantName={kwantName(owner) ?? (msg.sender_id && msg.sender_id.length <= 20 ? msg.sender_id : chatTitle)}
        kwantId={owner}
        live={botActions[meta.action_id]}
        onDecide={handleBotActionDecision}
        onOpenRoutine={handleOpenRoutine}
        flash={flash}
        botIds={botIds}
      />
    );
  };
  const renderHire = (msg: ChatMessage, meta: HireMeta, flash: boolean) => {
    const owner = meta.kwant_id ?? undefined;
    return (
      <HireCard
        meta={meta}
        messageId={msg.id}
        kwantName={meta.proposer_name || kwantName(owner) || chatTitle}
        kwantId={owner}
        live={hires[meta.hire_id]}
        onDecide={handleHireDecision}
        flash={flash}
        botIds={botIds}
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

  // Search result picked: open its chat and scroll to the message, loading the messages around it
  // when it's older than the latest page.
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
      if (data.some((m) => m.id === hit.message_id)) {
        setHistoryWindow(null);
        setHighlightMessageId(hit.message_id);
        return;
      }
      const around = await fetch(
        `/api/channels/${encodeURIComponent(targetId)}/messages?around=${encodeURIComponent(hit.message_id)}`,
      );
      if (around.status === 404) {
        pushToast('Opened the chat', 'That message is no longer there.');
        return;
      }
      if (!around.ok) throw new Error(`HTTP ${around.status}`);
      const older: ChatMessage[] = await around.json();
      if (selectedIdRef.current !== targetId) return;
      setHistoryWindow({ ctx: targetId, messages: older });
      setHighlightMessageId(hit.message_id);
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
      if (res.status === 409) return (await serverError(res)) ?? `Another bot in ${space.name} is already named "${agentData.name}".`;
      if (res.status === 400) return (await serverError(res)) ?? 'Check the name (no spaces or @) and that the account belongs to the chosen engine.';
      if (!res.ok) return (await serverError(res)) ?? `Couldn't save the bot (HTTP ${res.status}).`;

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
      console.error('Failed updating bot:', err);
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
        // The first bot opens the space's #general; counts change either way.
        onSpacesChanged();
        fetch('/api/channels')
          .then((r) => (r.ok ? r.json() : null))
          .then((fresh: Channel[] | null) => fresh && setChannels(fresh))
          .catch(() => undefined);
        setSelectedId(created.id);
        selectedIdRef.current = created.id;
        setPhoneView('chat');
        setIsChannel(false);
        isChannelRef.current = false;
        setSelectedTeamId(null);
        messagesCacheRef.current[created.id] = [];
        setMessages([]);
        return;
      }
      if (res.status === 409) return (await serverError(res)) ?? `Another bot in ${space.name} is already named "${agentData.name}".`;
      if (res.status === 400) return (await serverError(res)) ?? 'Check the name (no spaces or @) and that the account belongs to the chosen engine.';
      return (await serverError(res)) ?? `Couldn't create the bot (HTTP ${res.status}).`;
    } catch (err) {
      console.error('Failed creating bot:', err);
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
        onSpacesChanged();
        return;
      }
      throw new Error((await serverError(res)) ?? `Couldn't create the team (HTTP ${res.status}).`);
    } catch (err) {
      console.error('Failed creating team:', err);
      throw err instanceof Error ? err : new Error('Could not reach the server.');
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
        return;
      }
      throw new Error((await serverError(res)) ?? `Couldn't save the team (HTTP ${res.status}).`);
    } catch (err) {
      console.error('Failed updating team:', err);
      throw err instanceof Error ? err : new Error('Could not reach the server.');
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
      if (!res.ok) throw new Error((await serverError(res)) ?? `HTTP ${res.status}`);
      const cRes = await fetch('/api/channels');
      if (cRes.ok) setChannels(await cRes.json());
      playSfx('click');
    } catch (err) {
      console.error('Failed updating team members:', err);
      setTeams(previous);
      pushToast("Couldn't change the team.", err instanceof Error ? err.message : undefined);
    }
  };

  // Throws when the team wasn't deleted, so the dialog stays open and says why.
  const handleDeleteTeam = async (id: string) => {
    // Person-only: after "Confirm it's you" the delete goes through without pressing it again.
    const res = await personFetch(`/api/teams/${encodeURIComponent(id)}`, { method: 'DELETE' });
    if (!res.ok && res.status !== 404) throw new Error(`Couldn’t delete the team (HTTP ${res.status}).`);
    setTeams((prev) => prev.filter((t) => t.id !== id));
    if (selectedTeamId === id) {
      setSelectedTeamId(null);
      if (channels.length > 0) {
        setSelectedId(channels[0].id);
        setIsChannel(true);
      }
    }
    setEditingTeam(null);
    playSfx('click');
    // Refresh channels
    try {
      const cRes = await fetch('/api/channels');
      if (cRes.ok) {
        const freshChannels: Channel[] = await cRes.json();
        setChannels(freshChannels);
      }
    } catch (err) {
      console.error('Failed refreshing channels after deleting a team:', err);
    }
  };

  const currentChannel = isChannel ? channels.find((c) => c.id === selectedId) : null;
  const currentKwant = !isChannel ? kwants.find((k) => k.id === selectedId) : null;
  // Approvals belong in the asking bot's chat, or a channel it's in.
  const openChat: OpenChat | null = isChannel
    ? currentChannel
      ? { kind: 'channel', channelId: currentChannel.id, memberIds: [currentChannel.lead_kwant_id, ...(currentChannel.member_kwants ?? [])] }
      : null
    : selectedId
      ? { kind: 'kwant', kwantId: selectedId, threadId: openThreadId }
      : null;
  const approvalsHere = approvalsForChat(pendingApprovals, openChat);
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
    ? currentKwant?.name || 'Bot'
    : currentTeam
    ? `@${currentTeam.name}`
    : currentChannel?.name.startsWith('team-')
    ? `@${currentChannel.name.replace(/^team-/, '')}`
    : `#${currentChannel?.name || 'general'}`;

  const isGeneral = isChannel && !currentTeam && currentChannel?.name === 'general';
  const chatSubtitle = !isChannel
    ? currentKwant?.title || 'Kwant bot'
    : isGeneral
      ? `Laya picks who answers, from the ${kwants.length} ${kwants.length === 1 ? 'bot' : 'bots'} in ${space.name}`
      : currentTeam?.description || currentChannel?.description || `A team in ${space.name}`;

  // A different team or bot: forget the last one's page
  useEffect(() => {
    setScreenshotBase64(null);
    setActiveUrl('');
    setPageTitle('');
    setBrowserError(null);
  }, [activeSandboxId]);

  // Load container status and screenshot once the pane is open. Both start the bot's browser
  // container, which can take half a minute, and two such requests on every chat opened tie up
  // the browser's few connections to this server, so the rest of the page loads late.
  useEffect(() => {
    const sid = activeSandboxId;
    let cancelled = false;
    if (!isVmPaneOpen || !sid || sid === 'default') return;

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
  }, [activeSandboxId, isVmPaneOpen]);

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


  useEffect(() => onTokensUsed(tokensUsed), [tokensUsed, onTokensUsed]);

  // The address bar and "reopen where I was" follow the open chat.
  const chatChangeRef = useRef(onChatChange);
  chatChangeRef.current = onChatChange;
  useEffect(() => {
    if (!loaded || !selectedId) return;
    chatChangeRef.current(isChannel ? { channelId: selectedId } : { botId: selectedId });
  }, [loaded, selectedId, isChannel]);

  const isCurrentContextStreaming = openCtx in live;
  const openTurns = turnsIn(turns, openCtx);
  // Turns paused on a question wait for the person; they don't make the bot "busy".
  const awaitingTurns = turnsAwaitingAnswer(questions);
  const awaitingHere = openTurns.some((t) => t.state === 'running' && awaitingTurns.has(t.turn_id)) && !openTurns.some((t) => t.state === 'running' && !awaitingTurns.has(t.turn_id));
  const waiting = openTurns.some((t) => t.state === 'running') ? undefined : openTurns.find((t) => t.state === 'queued');
  const queuedHere: QueuedState | null = waiting
    ? { position: queuePosition(turns, waiting.turn_id), reason: waiting.reason ?? 'agent_busy', speaker: kwantName(waiting.kwant_id) ?? live[openCtx] ?? 'The bot' }
    : null;

  // The bot's conversations, the open thread's controls, and "working on…" elsewhere.
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
  const approvalsBadge = approvalsOff(currentKwant) ? (
    <ApprovalsOffBadge onClick={() => currentKwant && openAgentSettings(currentKwant)} title={`${currentKwant?.name} runs commands, file writes and tools without asking. Open its settings to change it.`} />
  ) : null;
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
  // Sidebar: bots busy somewhere other than the open chat.
  const busyKwants: Record<string, string> = {};
  for (const t of Object.values(turns)) {
    if (t.context_id === openCtx || busyKwants[t.kwant_id]) continue;
    busyKwants[t.kwant_id] = `${t.state === 'queued' ? 'Waiting to work on' : awaitingTurns.has(t.turn_id) ? 'Asked you a question in' : 'Working on'} ${describeContext(t.context_id, t.kwant_id)}`;
  }

  // Delete is off while the bot runs anywhere, the open chat included.
  const workingKwants = new Set(Object.values(turns).map((t) => t.kwant_id));

  /** Drops a deleted bot from the lists; leaves its chat if it was open. */
  const handleKwantDeleted = async (id: string) => {
    const gone = kwants.find((k) => k.id === id);
    setDeletingKwant(null);
    setEditingKwant(null);
    setKwants((prev) => prev.filter((k) => k.id !== id));
    if (!isChannel && selectedId === id) {
      setOpenThreadId(null);
      if (channels.length > 0) {
        setSelectedId(channels[0].id);
        setIsChannel(true);
      }
    }
    try {
      const [cRes, tRes] = await Promise.all([fetch('/api/channels'), fetch('/api/teams')]);
      if (cRes.ok) setChannels(await cRes.json());
      if (tRes.ok) setTeams(await tRes.json());
    } catch {
      /* the next refresh picks it up */
    }
    playSfx('click');
    pushToast(gone ? `Deleted ${gone.name}.` : 'Bot deleted.');
  };

  const viewingHistory = !!historyWindow && historyWindow.ctx === openCtx;
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

  const needsSetup = loaded && kwants.length === 0;
  const readyAccounts = space.counts?.ready_accounts ?? 0;
  // The space has bots but nothing can run them (a fresh install's starter bots): the checklist's
  // first step, kept above the chat until an account works.
  const setupBanner =
    loaded && kwants.length > 0 && canRun === false ? (
      <div
        role="status"
        data-setup-banner
        className="shrink-0 px-3 sm:px-6 py-2.5 border-b border-amber-500/20 bg-amber-500/5 flex flex-wrap items-center gap-x-3 gap-y-2"
      >
        <AlertTriangle size={15} className="shrink-0 text-amber-300" aria-hidden="true" />
        <div className="flex-1 min-w-[12rem]">
          <p className="text-xs font-medium text-zinc-100">Get {space.name} ready: add an account</p>
          <p className="text-[11px] text-zinc-400">Bots here can't answer until one account works. Sign in with Claude, Codex or Grok, or add an API key.</p>
        </div>
        <button
          type="button"
          onClick={offerClaude ? openClaudeSignIn : openSettings}
          className="shrink-0 flex items-center gap-1.5 px-3 py-1.5 rounded-lg text-xs font-medium text-white bg-sky-600 hover:bg-sky-500 focus-visible:outline focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-sky-300"
        >
          <KeyRound size={13} aria-hidden="true" />
          {offerClaude ? 'Sign in with Claude' : 'Add an account'}
        </button>
      </div>
    ) : null;
  const setupChecklist = (
    <main className="flex-1 min-w-0 overflow-y-auto flex items-start sm:items-center justify-center p-4 sm:p-8" data-space-checklist>
      <div className="w-full max-w-md rounded-2xl border border-zinc-800 bg-zinc-900/60 p-5 sm:p-6 space-y-4">
        <div className="flex items-center gap-3">
          <SpaceTile space={space} size={40} />
          <div className="min-w-0">
            <h1 className="text-base font-semibold text-zinc-100 truncate">Get {space.name} ready</h1>
            <p className="text-xs text-zinc-400">Bots, teams and accounts here stay in {space.name}.</p>
          </div>
        </div>
        <ol className="space-y-2">
          {[
            readyAccounts > 0
              ? {
                  done: true,
                  // Still offered once This machine is ready: more accounts and API keys go here too.
                  keepAction: true,
                  title: readyAccounts === 1 ? '1 account ready' : `${readyAccounts} accounts ready`,
                  detail: 'Add another Claude, Codex or Grok sign-in, or an API key.',
                  action: { label: 'Add an account', icon: <KeyRound size={13} />, onClick: () => openSpaceSettings('accounts') },
                }
              : {
                  done: false,
                  title: 'Add an account',
                  detail: 'Sign in with Claude, Codex or Grok, add an API key, or use this machine.',
                  action: { label: 'Add an account', icon: <KeyRound size={13} />, onClick: () => openSpaceSettings('accounts') },
                },
            {
              done: kwants.length > 0,
              title: 'Add your first bot',
              detail: `${space.name}'s #general opens with it.`,
              action: {
                label: 'New bot',
                icon: <Bot size={13} />,
                onClick: () => {
                  setNewAgentTeam(null);
                  setIsNewAgentOpen(true);
                  playSfx('click');
                },
              },
            },
            { done: false, title: 'Message #general', detail: `Laya picks who answers, only from ${space.name}'s bots.` },
          ].map((step: { done: boolean; keepAction?: boolean; title: string; detail: string; action?: { label: string; icon: React.ReactNode; onClick: () => void } }, i) => (
            <li key={step.title} className="flex items-start gap-3 rounded-xl border border-zinc-800 bg-zinc-950/50 p-3">
              <span
                className={`mt-0.5 w-5 h-5 shrink-0 rounded-full flex items-center justify-center text-[11px] font-semibold ${
                  step.done ? 'bg-emerald-500 text-zinc-950' : 'bg-zinc-800 text-zinc-300'
                }`}
                aria-hidden="true"
              >
                {step.done ? <Check size={12} /> : i + 1}
              </span>
              <span className="flex-1 min-w-0">
                <span className={`block text-xs font-medium ${step.done ? (step.keepAction ? 'text-zinc-300' : 'text-zinc-400 line-through') : 'text-zinc-100'}`}>
                  {step.title}
                  {step.done && <span className="sr-only"> (done)</span>}
                </span>
                <span className="block text-[11px] text-zinc-500">{step.detail}</span>
              </span>
              {step.action && (!step.done || step.keepAction) && (
                <button
                  type="button"
                  onClick={step.action.onClick}
                  className={`shrink-0 flex items-center gap-1 px-2.5 py-1 rounded-lg text-[11px] font-medium focus-visible:outline focus-visible:outline-2 focus-visible:outline-sky-300 ${
                    step.done ? 'text-zinc-200 border border-zinc-700 hover:border-sky-500/50 hover:text-sky-200' : 'text-white bg-sky-600 hover:bg-sky-500'
                  }`}
                >
                  {step.action.icon}
                  {step.action.label}
                </button>
              )}
            </li>
          ))}
        </ol>
      </div>
    </main>
  );

  const sidebar = (section: 'all' | 'chats' | 'bots') => (
    <Sidebar
      space={space}
      spaces={spaces}
      onSwitchSpace={onSwitchSpace}
      onCreateSpace={onCreateSpace}
      onOpenSpaceSettings={() => openSpaceSettings('general')}
      onOpenArchived={onOpenArchived}
      onOpenSearch={() => setIsSearchOpen(true)}
      onOpenRoutines={() => setIsRoutinesOpen(true)}
      section={section}
      phone={isPhone}
      onOpenAccounts={openSettings}
      onSignInClaude={openClaudeSignIn}
      enginesRefreshKey={settingsClosedCount}
      engines={engines}
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
      awayCount={awayCount}
      awayActive={showAway}
      looseActive={showLoose}
      parallelActive={showParallel}
      onOpenParallel={() => {
        setShowParallel(true);
        setShowLoose(false);
        setShowAway(false);
        setPhoneView('chat');
        playSfx('click');
      }}
      onOpenLooseEnds={() => {
        setShowLoose(true);
        setShowParallel(false);
        setShowAway(false);
        setPhoneView('chat');
        playSfx('click');
      }}
      onOpenAway={() => {
        setShowAway(true);
        setShowLoose(false);
        setShowParallel(false);
        setPhoneView('chat');
        playSfx('click');
      }}
    />
  );

  const away = (
    <AwayPage since={awaySinceRef.current} onSince={(t) => (awaySinceRef.current = t)} onOpen={openAwayItem} onCounts={setAwayCount} onApprovalResolved={dropApproval}
      onBack={isPhone ? () => { setShowAway(false); setPhoneView('list'); } : undefined}
    />
  );
  const parallel = <ParallelPage onBack={isPhone ? () => { setShowParallel(false); setPhoneView('list'); } : undefined} />;
  const loose = <LooseEndsPage onBack={isPhone ? () => { setShowLoose(false); setPhoneView('list'); } : undefined} onOpenChat={openThread} />;
  const chat = (
        <ChatView
          title={chatTitle}
          subtitle={chatSubtitle}
          spaceName={space.name}
          onBack={isPhone ? () => setPhoneView('list') : undefined}
          messages={viewingHistory ? historyWindow.messages : messages}
          viewingHistory={viewingHistory}
          onJumpToLatest={() => setHistoryWindow(null)}
          streamingText={isCurrentContextStreaming ? streamingText : ''}
          streamingThinking={isCurrentContextStreaming ? streamingThinking : ''}
          pendingApprovals={approvalsHere}
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
          vmApprovalsOff={space.approvals === 'off' || kwants.some(approvalsOff)}
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
          subHeader={
            setupBanner ? (
              <>
                {subHeader}
                {setupBanner}
              </>
            ) : (
              subHeader
            )
          }
          onOpenAccounts={openSettings}
          canRun={canRun}
          titleExtra={
            workingBadge || skillsBadge || approvalsBadge ? (
              <>
                {approvalsBadge}
                {workingBadge}
                {skillsBadge}
              </>
            ) : null
          }
          queued={queuedHere}
          renderDelegation={renderDelegation}
          renderQuestion={renderQuestion}
          renderMcpSuggestion={renderMcpSuggestion}
          renderBotAction={renderBotAction}
          renderHire={renderHire}
          awaitingAnswer={awaitingHere}
          contextKey={openCtx}
        />
  );
  const mainPane = needsSetup ? setupChecklist : showParallel ? parallel : showLoose ? loose : showAway ? away : !loaded ? (
    <main className="flex-1 flex items-center justify-center text-xs text-zinc-500">
      <Loader2 size={14} className="mr-2 animate-spin motion-reduce:animate-none" aria-hidden="true" /> Opening {space.name}…
    </main>
  ) : chat;

  const phoneTabs: Array<{ id: PhoneTab; label: string; icon: React.ReactNode; badge?: number }> = [
    { id: 'chats', label: 'Chats', icon: <MessageSquare size={18} /> },
    { id: 'bots', label: 'Bots', icon: <Bot size={18} /> },
    { id: 'inbox', label: 'Inbox', icon: <Inbox size={18} />, badge: awayCount },
    { id: 'space', label: 'Space', icon: <Settings size={18} /> },
  ];
  const showPhoneList = isPhone && phoneView === 'list' && !needsSetup;

  return (
    <div className="flex-1 min-w-0 h-full flex flex-col bg-zinc-950 text-zinc-100 overflow-hidden" data-space={space.id}>
      {/* The open space's colour, so it's clear where a message goes */}
      <div className="h-[3px] shrink-0" style={{ background: spaceColor(space) }} aria-hidden="true" />

      {isPhone ? (
        <>
          {showPhoneList || needsSetup ? (
            <div className="flex-1 min-h-0 flex flex-col">
              {needsSetup ? (
                <>
                  {sidebar('chats')}
                  {setupChecklist}
                </>
              ) : (
                sidebar(phoneTab === 'bots' ? 'bots' : 'chats')
              )}
            </div>
          ) : (
            <div className="flex-1 min-h-0 flex">{mainPane}</div>
          )}
          {(showPhoneList || needsSetup) && (
            <nav aria-label={`${space.name} sections`} className="shrink-0 grid grid-cols-4 border-t border-zinc-800 bg-zinc-900/95 pb-[env(safe-area-inset-bottom)]">
              {phoneTabs.map((t) => {
                const active = t.id === phoneTab;
                return (
                  <button
                    key={t.id}
                    type="button"
                    aria-current={active ? 'page' : undefined}
                    onClick={() => {
                      playSfx('click');
                      if (t.id === 'inbox') {
                        setShowAway(true);
                        setShowLoose(false);
                        setShowParallel(false);
                        setPhoneView('chat');
                        return;
                      }
                      if (t.id === 'space') {
                        // Nothing can run yet: the fix is in Accounts, so start there
                        openSpaceSettings(canRun === false ? 'accounts' : 'general');
                        return;
                      }
                      setPhoneTab(t.id);
                    }}
                    className={`relative flex flex-col items-center gap-0.5 py-2 text-[11px] font-medium focus-visible:outline focus-visible:outline-2 focus-visible:-outline-offset-2 focus-visible:outline-sky-500 ${
                      active ? 'text-sky-300' : 'text-zinc-400 hover:text-zinc-200'
                    }`}
                  >
                    {t.icon}
                    {t.label}
                    {!!t.badge && (
                      <span className="absolute top-1 left-1/2 ml-2 min-w-[16px] px-1 rounded-full bg-amber-500 text-zinc-950 text-[9px] font-semibold text-center">
                        {badgeText(t.badge)}
                      </span>
                    )}
                  </button>
                );
              })}
            </nav>
          )}
        </>
      ) : (
        <div className="flex-1 min-h-0 flex overflow-hidden">
          {sidebar('all')}
          {mainPane}
        </div>
      )}

      {/* New bot */}
      <Suspense fallback={null}>
        <NewAgentModal
          isOpen={isNewAgentOpen}
          onClose={() => {
            setIsNewAgentOpen(false);
            setNewAgentTeam(null);
          }}
          joinTeamName={newAgentTeam?.name}
          approvalsOff={space.approvals === 'off'}
          takenNames={kwants.map((k) => k.name)}
          profiles={profiles}
          onManageProfiles={openSettings}
          onManageServers={() => setIsMcpOpen(true)}
          onOpenSettings={openSettings}
          onSignInClaude={openClaudeSignIn}
          enginesRefreshKey={settingsClosedCount}
          onSubmit={handleCreateAgent}
        />

        {/* Edit a bot */}
        <NewAgentModal
          isOpen={!!editingKwant}
          initial={editingKwant}
          kwantId={editingKwant?.id}
          kwants={kwants}
          channels={channels}
          profiles={profiles}
          onManageProfiles={openSettings}
          onManageServers={() => setIsMcpOpen(true)}
          onOpenSettings={openSettings}
          onSignInClaude={openClaudeSignIn}
          enginesRefreshKey={settingsClosedCount}
          initialTab={editingTab}
          pendingSkills={editingKwant ? pendingSkillCounts[editingKwant.id] ?? 0 : 0}
          onClose={() => setEditingKwant(null)}
          onSubmit={(data) => (editingKwant ? handleUpdateAgent(editingKwant, data) : undefined)}
          onRequestDelete={editingKwant ? () => setDeletingKwant(editingKwant) : undefined}
          onApprovalsChanged={refreshRoster}
          approvalsOff={approvalsOff(editingKwant)}
        />

        {deletingKwant && (
          <DeleteBotDialog
            kwant={deletingKwant}
            busy={workingKwants.has(deletingKwant.id) ? busyKwants[deletingKwant.id] ?? 'Working' : null}
            onClose={() => setDeletingKwant(null)}
            onDeleted={handleKwantDeleted}
          />
        )}
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

      {spaceSettingsTab && (
        <SpaceSettings
          space={space}
          spaces={spaces}
          tab={spaceSettingsTab}
          onTab={setSpaceSettingsTab}
          onClose={closeSpaceSettings}
          kwants={kwants}
          channels={channels}
          teams={teams}
          focusClaudeSignIn={claudeSignInFocus}
          onProfilesChanged={setProfiles}
          pendingSkills={pendingSkillCounts}
          onSpaceSaved={() => onSpacesChanged()}
          onArchived={(s) => {
            setSpaceSettingsTab(null);
            onSpaceArchived(s);
          }}
          onNewBot={() => {
            setNewAgentTeam(null);
            setIsNewAgentOpen(true);
          }}
          onEditBot={(k) => openAgentSettings(k)}
          onOpenSkills={(k) => openAgentSettings(k, 'skills')}
          onNewTeam={() => setIsNewTeamOpen(true)}
          onManageTeam={(t) => setEditingTeam(t)}
          onOpenTools={() => setIsMcpOpen(true)}
          onOpenAppSettings={onOpenAppSettings}
          routineFocus={routineFocus}
          onApprovalsChanged={refreshRoster}
          onImported={() => {
            // New bots, teams and their team chats.
            refreshRoster();
            fetch('/api/channels')
              .then((r) => (r.ok ? r.json() : null))
              .then((c) => Array.isArray(c) && setChannels(c))
              .catch(() => undefined);
          }}
        />
      )}

      {/* MCP tool servers (opened last so it sits above the bot form) */}
      <Suspense fallback={null}>
        <McpServersModal isOpen={isMcpOpen} onClose={() => setIsMcpOpen(false)} kwants={kwants} />
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
