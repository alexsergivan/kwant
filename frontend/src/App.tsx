import React, { Suspense, lazy, useState, useEffect, useRef, useCallback, useMemo } from 'react';
import { DecidingMap, applyDecideEvent, decisionsApi, hydrateDeciding } from './decideFirst';
import { Sidebar, Kwant, Channel, Team, memberIdsOf } from './components/Sidebar';
import { SpaceSettings, SpaceSettingsTab } from './components/SpaceSettings';
import { InboxPage } from './components/InboxPage';
import { InboxTab, SettingsScope, SpaceView, viewSegment } from './nav';
import { PHONE_QUERY } from './layout';
import { useSettingsCover } from './settingsFocus';
import { WhatsNewHost } from './components/WhatsNewSheet';
import { GetStarted, GetStartedBanner } from './components/GetStarted';
import { dismiss as dismissGetStarted, isDismissed as getStartedDismissed, showCard as showGetStarted, wasStarted as getStartedWasStarted, whatsNewEstablished as decideWhatsNewEstablished } from './gettingStarted';
import { useStartVersion } from './gettingStartedInstall';
import { SpaceTile } from './components/Spaces';
import { ChatView, ChatMessage, DiscussionState, RoutingState, QueuedState, SendOptions } from './components/ChatView';
import { DropPillHost } from './components/DropLayer';
import { ReactResult, Reaction, optimisticReplyRef, reactionOf, saveReaction, senderLabel } from './reactReply';
import { PhoneTabBar, type PhoneTabItem } from './components/PhoneTabBar';
import { ThreadBar, ThreadHeader, ThreadPicker, type ThreadBarProps } from './components/ThreadBar';
import { DelegationCard } from './components/DelegationCard';
import { ApprovalsOffBadge } from './components/ApprovalsSetting';
import { APPROVALS_CHANGED_EVENT, approvalsOff } from './approvalMode';
import { QuestionCard } from './components/QuestionCard';
import { HumanStepCard } from './components/HumanStepCard';
import { StarAsk } from './components/StarAsk';
import { starAskStore } from './starAsk';
import { LiveActivity, appendThinking, applyActivity, parseStep, seedActivity } from './activity';
import { liveNamingOrder, namesToFill } from './workingStrip';
import {
  HumanStep,
  HumanStepMeta,
  applyHumanStep,
  captchaStepFor,
  humanStepsApi,
  latestOpenStep,
  openStepFor,
  openStepsByKwant,
  turnsAwaitingStep,
} from './humanSteps';
import { McpSuggestionCard } from './components/McpSuggestionCard';
import { ConnectionSuggestionCard } from './components/ConnectionSuggestionCard';
import type { ConnectionSuggestionMeta } from './connectionSuggestions';
import { Suggestion, SuggestionMeta, applySuggestion, suggestionsApi } from './mcpSuggestions';
import { ApiError, BotActionMeta, HireMeta, applyBotAction, applyHire, botActionsApi, changesBots, decisionError, hiresApi } from './botActions';
import { BotActionCard, type BotActionDecision } from './components/BotActionCard';
import { HireCard, type HireDecision } from './components/HireCard';
import { RouteChoiceCard } from './components/RouteChoiceCard';
import { RouteChoiceMeta, applyRouteChoice, correctRoute, isAskingReply, rerouteTargetsFor, resolveRouteChoice } from './routeChoice';
import type { RoutineFocus } from './components/RoutinesPanel';
import { AwayPage } from './components/AwayView';
import { LooseEndsPage } from './components/LooseEndsView';
import { MissionBoardPage } from './components/MissionBoardView';
import { MISSION_BOARD_SETTING_EVENT, emitMission, parseMissionBoardSetting, sourceChat } from './missions';
import { type ChatTarget, type NotifiedChat, notifiedPlace, planOpenTarget } from './chatTarget';
import { ParallelPage } from './components/ParallelUniverseView';
import { ConnectionsPage } from './components/ConnectionsView';
import { IdeaJarView, deleteIdea, dropIdea, emitIdeaUpdated, isIdeaCommand, onIdeaUpdated, useIdeaJarBadge } from './components/ideas';
import { ideasApi, takeArrivedText } from './ideas';
import { JarIcon } from './components/JarIcon';
import { IdeaDropToast, type IdeaDropToastState } from './components/IdeaDropToast';
import { JAR_DROPPED, JAR_SEND_FAILED, ideaSendOutcome, useJarWobble, wobbleJar } from './ideaShell';
import { Bot, Check, HelpCircle, Inbox, KeyRound, ListChecks, Loader2, MessageSquare, Settings } from 'lucide-react';
import { TG_EVENT, TelegramEventDetail, forThisSpace } from './telegram';
import { ChatApproval, OpenChat, addSettled, settledFromRecords, withdrawApproval, approvalFromEvent, lateAnswerText, approvalsForChat, approvalsFromAway, syncApprovals, upsertApproval, withoutApproval } from './approvals';
import type { AgentFormData } from './components/NewAgentModal';
import { NewTeamModal } from './components/NewTeamModal';
import { ReadyTeamsProvider, TeamNeedsTop, type ReadyTeam } from './components/ReadyTeams';
import { NewChannelModal } from './components/NewChannelModal';
import type { NewChannelBody } from './channels';
import { ManageTeamModal } from './components/ManageTeamModal';
import { playSfx } from './audio/chiptune';
import { haptic, messageHaptic } from './haptics';
import { Toasts, Toast } from './components/Toasts';
import { clearAllTurns, resetTurns, turnEnded, turnStarted } from './kwantActivity';
import { setKnownBots } from './botLooks';
import { clearMoods, moodOutput, moodTurnEnded, moodTurnStarted, restingUntil, setMoodNeedsYou, setMoodResting } from './kwantMood';
import { browserTabDoc, createTabSignal, needsYouCount } from './tabSignal';
import { recentWork, type ChatHome } from './botHome';
import { approvalsDiffers } from './components/Sidebar';
import { botStatuses, busyLines } from './botStatus';
import { type H4Status, type HeldSend, applyBotStatusEvent, h4Api, heldFromReply, heldLine, stillHeld } from './h4Status';
import { HeldLine } from './components/HeldLine';
import { type BrowsingMap, expireBrowsing, globeRows, noteBrowser, ownerStillRunning, pruneBrowsing, seedBrowsing, switchEntries } from './watchLive';
import { LiveMissPage } from './components/WatchLive';
import {
  ChatLoads,
  Delegation,
  DelegationMeta,
  Thread,
  TrackedTurn,
  TurnMap,
  applyDelegation,
  applyTurnEvent,
  backgroundTurns,
  hydrateTurns,
  messageContextId,
  parseDelegationMeta,
  queuePosition,
  queuedInChat,
  reloadOnTurnEnd,
  shouldRenderDelta,
  streamTarget,
  waitsForOtherBot,
  threadBadge,
  threadsApi,
  turnsIn,
  upsertThread,
} from './threads';
import { Profile, profilesApi } from './profiles';
import { FailoverEvent, createOutageLog, failoverNotice, outageKey } from './failover';
import type { SwitchNote } from './failoverNotes';
import {
  NotifyEvent,
  NotifyKind,
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
import { AwayItem, affectsAway, awayApi, awayTarget } from './away';
import { needsYouRows, attentionRows } from './chatsTab';
import { Space, eventsUrl, getCurrentSpace, pickInitialChat } from './spaces';
import { canRunNow } from './engines';
import { useEngines } from './useEngines';
import { canInstall, setupApi } from './setupInstall';
import { claudeNeedsSignIn } from './claudeLogin';
import { personFetch } from './auth';
import { teamChannelOf, teamOfChannel } from './teamChannel';
import { PromptContext, suggestedPrompts } from './suggestedPrompts';
import { isNotificationOpen, placeOfNotification, showDeviceNotification } from './push';
import { requestSpendStopFocus } from './spendStop';
import { SpendStopNeedsYou } from './components/SpendStop';
import { EMPTY_PILL, NUDGES_SETTING_EVENT, browserIconDoc, createArrivals, createNudgeIcon, nudgesEnabled, pillAdd, pillPrune, setAppBadgeCount, shouldPill, type Nudge, type PillState } from './attention';
import { NeedsYouPill } from './components/NeedsYouPill';
import { DevModeError, devApi, devStore, devTeamRefs } from './devMode';
import { GitHubConnectHost, connectedToast, openGitHubConnect } from './components/GitHubConnect';
import { JiraSetupHost } from './components/JiraSetup';
import { useDevModeOn } from './components/DevModeSwitch';
import { DEV_CARD_EVENT, DevDeskView, type DevCardEvent } from './components/DevDesk';
import { DEV_TICKET_EVENT, DEV_TICKET_PROGRESS_EVENT } from './components/dev/TicketsTab';
import { PrepushNeedsYou, PrepushThread, threadCards, usePrepushCards, type PrepushEventDetail } from './components/dev/PrepushThread';
import { PrepushPointers } from './components/dev/PrepushPointers';
import { ModernAvatar } from './components/ModernAvatar';
import { PREPUSH_EVENT, messageNotSent as prepushMessageNotSent, needsYou as prepushNeedsYou, overrideMessage, prepushApi, retryMessage, type PrepushCard as PrepushCardData } from './dev/prepush';
import { StepReview } from './components/ShowOnce';
import { inDesktopApp, leaveForSignIn, mcpSignIn, pollUntil, signInChanged } from './externalLinks';

// Kwant and Tools editors are their own chunks. They stay mounted (rendering nothing while closed),
// so both load right after start-up and opening them never waits.
const DeleteBotDialog = lazy(() => import('./components/DeleteBotDialog').then((m) => ({ default: m.DeleteBotDialog })));
const NewAgentModal = lazy(() => import('./components/NewAgentModal').then((m) => ({ default: m.NewAgentModal })));
const McpServersModal = lazy(() => import('./components/McpServersModal').then((m) => ({ default: m.McpServersModal })));
const ChannelInstructionsModal = lazy(() =>
  import('./components/ChannelInstructionsModal').then((m) => ({ default: m.ChannelInstructionsModal })),
);
const SearchModal = lazy(() => import('./components/SearchModal').then((m) => ({ default: m.SearchModal })));
const SettingsModal = lazy(() => import('./components/SettingsModal').then((m) => ({ default: m.SettingsModal })));

/** Where to land when a space opens: a channel or a bot's direct chat. */
export interface SpaceTarget {
  channelId?: string;
  botId?: string;
  /** W2: the chat's `/live` address, with the browser panel open. */
  live?: boolean;
  /** 0.8.1 F6: one of the bot's conversations (`/b/<bot>/t/<thread>`). */
  threadId?: string;
  /** 0.8.1 F6: the card to show there (`?card=`), an approval's id. */
  cardId?: string;
}

export interface SpaceAppProps {
  space: Space;
  spaces: Space[];
  initialTarget: SpaceTarget | null;
  /** Where to open instead of a chat when the space mounts: an address like /inbox or
   * /settings/accounts (Accounts, right after creating a space). */
  initialView?: SpaceView | null;
  /** Asks the open space to show a view (the rail's gear, Back and Forward). `seq` makes a repeat count. */
  viewRequest?: { view: SpaceView | null; seq: number } | null;
  /** The view shown changed (null: a chat), for the address bar. */
  onViewChange?: (view: SpaceView | null) => void;
  onSwitchSpace: (id: string, to?: SpaceTarget | null) => void;
  onCreateSpace: () => void;
  onOpenArchived: () => void;
  /** Re-reads the space list (counts, #general appearing, a rename). */
  onSpacesChanged: () => void;
  onSpaceArchived: (space: Space) => void;
  /** The open chat changed, for the address bar and reopening it later. */
  onChatChange: (target: SpaceTarget) => void;
}

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

type PhoneTab = 'chats' | 'bots' | 'jar' | 'inbox' | 'space';

/** One space: its bots, teams, channels and chats. Remounted for every space, so nothing carries over. */
export const SpaceApp: React.FC<SpaceAppProps> = ({
  space,
  spaces,
  initialTarget,
  initialView = null,
  viewRequest = null,
  onViewChange,
  onSwitchSpace,
  onCreateSpace,
  onOpenArchived,
  onSpacesChanged,
  onSpaceArchived,
  onChatChange,
}) => {
  const isPhone = useIsPhone();
  // Phones: the list (one tab of it) or the open chat.
  // The Idea jar is a tab of the list on phones, so a link to it opens on the list. The Inbox opens
  // over it, as a tap does (a notification's Needs you lands there).
  const [phoneView, setPhoneView] = useState<'list' | 'chat'>(() =>
    initialView?.kind === 'inbox' || (initialView?.kind !== 'jar' && (initialTarget?.channelId || initialTarget?.botId || initialTarget?.live)) ? 'chat' : 'list',
  );
  const [phoneTab, setPhoneTab] = useState<PhoneTab>(() => (initialView?.kind === 'jar' ? 'jar' : 'chats'));
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
  /** The chat whose messages have finished loading at least once since it opened (What's new waits for it). */
  const [messagesLoadedFor, setMessagesLoadedFor] = useState<string | null>(null);
  // The open thread of the selected bot; null for its direct chat (and for channels).
  const [openThreadId, setOpenThreadId] = useState<string | null>(null);
  const openCtx = openThreadId || selectedId;
  const openCtxRef = useRef(openCtx);
  openCtxRef.current = openCtx;
  // What the open chat is streaming; other chats' streams are kept in streamsRef until opened.
  const [streamingText, setStreamingText] = useState('');
  const [streamingThinking, setStreamingThinking] = useState('');
  const streamsRef = useRef<Record<string, { text: string; thinking: string }>>({});
  // Running turns' live steps by turn id (with the chat they belong to), until the saved reply shows.
  const [activity, setActivity] = useState<Record<string, LiveActivity & { ctx: string }>>({});
  const activityRef = useRef(activity);
  const setActivityMap = (next: Record<string, LiveActivity & { ctx: string }>) => {
    activityRef.current = next;
    setActivity(next);
  };
  // Chats with a reply in flight -> who is replying. Several can stream at once.
  const [live, setLive] = useState<Record<string, string>>({});
  const liveRef = useRef(live);
  // Chat the last send or discussion started in; untagged events (team discussions) go here.
  const streamingContextIdRef = useRef<string | null>(null);
  const [turns, setTurns] = useState<TurnMap>({});
  // Chats where a held reply is being decided ("Team is deciding…"), from decide_updated.
  const [deciding, setDeciding] = useState<DecidingMap>({});
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
  const pendingRef = useRef<ChatApproval[]>([]);
  pendingRef.current = pendingApprovals;
  // Approvals allowed in Telegram, kept in their chat as one line (live, and from the server after a reload).
  const [settledApprovals, setSettledApprovals] = useState<ChatApproval[]>([]);
  // Seen live since the last server list: kept even if that list was fetched just before.
  const liveApprovalIdsRef = useRef<Set<string>>(new Set());
  const dropApproval = (id: string) => {
    liveApprovalIdsRef.current.delete(id);
    setPendingApprovals((prev) => withoutApproval(prev, id));
  };
  const [tokensUsed, setTokensUsed] = useState(0);

  // Settings: open on This space (one tab) or All spaces, or closed (tab null).
  const [spaceSettingsTab, setSpaceSettingsTab] = useState<SpaceSettingsTab | null>(() =>
    initialView?.kind === 'settings' ? (initialView.scope === 'space' ? initialView.tab : 'general') : null,
  );
  const [settingsScope, setSettingsScope] = useState<SettingsScope>(() => (initialView?.kind === 'settings' ? initialView.scope : 'space'));
  // Bumped when Space settings closes, so open bot forms read account status again.
  const [settingsClosedCount, setSettingsClosedCount] = useState(0);
  // Set when Accounts opens from a "Sign in with Claude" hint, to focus that button.
  const [claudeSignInFocus, setClaudeSignInFocus] = useState(0);
  // Set by a routine card's Edit or Pause, so Routines opens at that routine.
  const [routineFocus, setRoutineFocus] = useState<RoutineFocus | null>(null);
  const openSpaceSettings = (tab: SpaceSettingsTab = 'general') => {
    setClaudeSignInFocus(0);
    if (tab !== 'routines') setRoutineFocus(null);
    setSettingsScope('space');
    setSpaceSettingsTab(tab);
    playSfx('click');
  };
  // Settings › All spaces: what applies to every space and to the app.
  const openAppSettings = () => {
    setSettingsScope('app');
    setSpaceSettingsTab((t) => t ?? 'general');
    playSfx('click');
  };
  const openSettings = () => openSpaceSettings('accounts');
  // Settings › All spaces › Usage, at the daily spending stop's fields.
  const openUsageSettings = () => {
    setSettingsFocusRow('spend-stop');
    openAppSettings();
  };
  // Isolation notices: App settings at the Computer card, "Run Claude inside the VM" focused.
  const openComputerSettings = () => {
    setSettingsFocusRow('claude-isolation');
    openAppSettings();
  };
  const openConnections = () => openSpaceSettings('connections');
  const openClaudeSignIn = () => {
    setClaudeSignInFocus(Date.now());
    setSettingsScope('space');
    setSpaceSettingsTab('accounts');
  };
  // This space's engine checks, shared with the sidebar: the setup banner, the "can't answer"
  // card and the bots' online dots all read whether anything can run.
  const engines = useEngines(true);
  const canRun = canRunNow(engines.engines);
  const offerClaude = canRun === false && claudeNeedsSignIn(engines.engines);
  // Claude Code isn't here but kwant can install it: "Set up Claude" installs, then signs in, in one go.
  const claudeMissing = canRun === false && !!engines.engines?.some((e) => e.id === 'claude' && e.status === 'not_installed' && e.enabled !== false);
  const [claudeInstallable, setClaudeInstallable] = useState(false);
  useEffect(() => {
    if (!claudeMissing) return;
    let gone = false;
    setupApi
      .status()
      .then((items) => !gone && setClaudeInstallable(canInstall(items, 'claude') === true))
      .catch(() => !gone && setClaudeInstallable(false));
    return () => void (gone = true);
  }, [claudeMissing]);
  const offerSetup = claudeMissing && claudeInstallable;
  const setupAction = offerSetup || offerClaude ? openClaudeSignIn : openSettings;
  // O2: the Get started card in #general (design next-076/onboarding §3–§13). Re-renders when this
  // browser's getting-started state changes (choice, kept, dismissed).
  useStartVersion();
  const gsDismissed = getStartedDismissed(space.id);
  // C1: on a phone, open #general while getting started is unfinished (not the chats list). Once.
  const c1Done = useRef(false);
  useEffect(() => {
    if (c1Done.current || !isPhone || !loaded || canRun === null) return;
    c1Done.current = true;
    if (canRun !== false || gsDismissed || initialView || initialTarget?.channelId || initialTarget?.botId || initialTarget?.live) return;
    const general = channels.find((c) => c.id === space.general_channel_id) ?? channels.find((c) => c.name === 'general');
    if (general) handleSelectChannel(general);
    else setPhoneView('chat');
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [isPhone, loaded, canRun, channels]);
  // The sidebar and chat under Settings: inert while it's open (the space rail stays usable).
  const contentRef = useRef<HTMLDivElement>(null);
  useSettingsCover(!!spaceSettingsTab, contentRef);
  const closeSpaceSettings = () => {
    setSpaceSettingsTab(null);
    setSettingsFocusRow(null);
    setSettingsClosedCount((n) => n + 1);
    // Accounts may have changed: the getting-started list counts the ready ones.
    onSpacesChanged();
  };
  const [isMcpOpen, setIsMcpOpen] = useState(false);
  // Connections' "+ Add connection" opens Tools straight on adding a server.
  const [mcpStartAdding, setMcpStartAdding] = useState(false);
  const [isSearchOpen, setIsSearchOpen] = useState(false);
  const [editingChannelInstructions, setEditingChannelInstructions] = useState<Channel | null>(null);
  const [highlightMessageId, setHighlightMessageId] = useState<string | null>(null);
  // Older messages shown instead of the latest (a search hit beyond them). Kept apart from the
  // cache, which live updates keep current underneath.
  const [historyWindow, setHistoryWindow] = useState<{ ctx: string; messages: ChatMessage[] } | null>(null);
  const [profiles, setProfiles] = useState<Profile[]>([]);
  const [toasts, setToasts] = useState<Toast[]>([]);
  const [switchNotes, setSwitchNotes] = useState<SwitchNote[]>([]);
  const switchLogRef = useRef(createOutageLog());
  // What fills the main pane instead of a chat: the Inbox (New or Loose ends), the Mission board
  // or Parallel Universe. The Inbox's unseen count is the sidebar badge.
  const [page, setPage] = useState<'chat' | 'inbox' | 'board' | 'parallel' | 'dev' | 'jar'>(() =>
    initialView?.kind === 'inbox' || initialView?.kind === 'board' || initialView?.kind === 'parallel' || initialView?.kind === 'dev' || initialView?.kind === 'jar'
      ? initialView.kind
      : 'chat',
  );
  const [inboxTab, setInboxTab] = useState<InboxTab>(() => (initialView?.kind === 'inbox' ? initialView.tab : 'new'));
  const showPage = (next: 'inbox' | 'board' | 'parallel' | 'dev' | 'jar', tab: InboxTab = 'new') => {
    setPage(next);
    if (next === 'inbox') setInboxTab(tab);
    if (spaceSettingsTab) closeSpaceSettings();
    // Phones keep the tab bar under the jar (it's a tab, spec §2.2); every other page covers the list.
    if (next === 'jar' && isPhoneRef.current) {
      setPhoneTab('jar');
      setPhoneView('list');
    } else setPhoneView('chat');
  };
  const isPhoneRef = useRef(isPhone);
  isPhoneRef.current = isPhone;
  // Today's spending stop: Needs you (Inbox › New) with its Continue today, focused.
  const [spendStopSeq, setSpendStopSeq] = useState(0);
  const openSpendStop = () => {
    requestSpendStopFocus();
    setSpendStopSeq((n) => n + 1);
    showPage('inbox', 'new');
  };
  // Home: the chat list on a phone, the open chat elsewhere, with nothing on top.
  const openHome = () => {
    if (spaceSettingsTab) closeSpaceSettings();
    setPage('chat');
    if (isPhoneRef.current) setPhoneView('list');
  };
  // Dev mode Connect (0.7.3 B1): the one GitHub connect form opens in place, next to the button (a
  // sheet on the phone); the host re-reads this space's Dev mode once the login is added.
  const connectDevGitHub = (anchor?: HTMLElement) => openGitHubConnect({ space: space.id, anchor });
  // Idea jar (0.7): the ready-takes badge, Open on board's mission, the drop toast and the
  // app-wide live region (a take is announced on any screen; the open jar announces its own).
  const ideaJarCount = useIdeaJarBadge(space.id);
  const jarWobble = useJarWobble();
  const [pendingMissionId, setPendingMissionId] = useState<string | null>(null);
  const [ideaToast, setIdeaToast] = useState<{ id: string; state: IdeaDropToastState } | null>(null);
  const [jarAnnouncement, setJarAnnouncement] = useState('');
  const [settingsFocusRow, setSettingsFocusRow] = useState<string | null>(null);
  const closeIdeaToast = useCallback(() => setIdeaToast(null), []);
  // Open jar from the toast: land in the jar's own "Drop an idea…" field. The view may still be loading,
  // so look for it for a couple of seconds; give up quietly if focus has moved elsewhere meanwhile.
  const focusJarQuickAdd = useCallback(() => {
    let tries = 0;
    const tick = () => {
      const field = document.querySelector<HTMLElement>('[data-quick-add="hero"] input, [data-quick-add="phone"] input');
      const a = document.activeElement;
      if (a && a !== document.body) return;
      if (field) field.focus();
      else if (++tries < 40) setTimeout(tick, 50);
    };
    requestAnimationFrame(tick);
  }, []);
  const announceJar = (text: string) => {
    // Cleared first so the same words twice are read twice.
    setJarAnnouncement('');
    requestAnimationFrame(() => setJarAnnouncement(text));
  };
  useEffect(
    () =>
      onIdeaUpdated((e) => {
        if (e.status !== 'ready' || e.idea || (e.space_id && e.space_id !== space.id) || pageRef.current === 'jar') return;
        ideasApi.list(space.id).then(
          (l) => {
            const i = l.ideas.find((x) => x.id === e.id);
            if (i?.take) announceJar(takeArrivedText(kwantName(i.kwant_id) ?? null, i.take, i.text));
          },
          () => undefined,
        );
      }),
    // eslint-disable-next-line react-hooks/exhaustive-deps
    [space.id],
  );
  // Mission board: shown in the sidebar only when the server has it switched on.
  const [missionBoard, setMissionBoard] = useState(false);
  // Dev mode (0.7): the open space's state, read by the switch, the hats and the grid.
  useEffect(() => devStore.setCurrent(space.id), [space.id]);
  const devOn = useDevModeOn(space.id);
  // 0.8.0 B3: the space's pre-push cards (Dev mode only): the bot's thread and the Inbox read them.
  const prepush = usePrepushCards(space.id, devOn);
  const prepushWaiting = prepushNeedsYou(prepush.cards).length;
  const kwantNames = useMemo(() => Object.fromEntries(kwants.map((k) => [k.id, k.name])), [kwants]);
  // A task to open on the board once it shows (from a lesson on a bot's Memory tab).
  const [pendingTaskId, setPendingTaskId] = useState<string | null>(null);
  const [awayCount, setAwayCount] = useState(0);
  // "Needs you" nudges (attention.ts): what waits on the person, the pill, and the "Show nudges" setting.
  const [awayWaiting, setAwayWaiting] = useState(0);
  /** What waits on you right now (approvals, questions), for the phone Chats tab's Needs you. */
  const [awayItems, setAwayItems] = useState<AwayItem[]>([]);
  /** H5: unseen failed and finished items (ranks 2 and 3), and whether the first fetch has landed. */
  const [awayDone, setAwayDone] = useState<{ failed: AwayItem[]; finished: AwayItem[] } | null>(null);
  /** H5: when the person last opened each bot's chat (a finished item older than that is read). */
  const [openedAt, setOpenedAt] = useState<Record<string, number>>({});
  // H4 (#499): each bot's status from the server; null on an older server (today's lines stay).
  const [h4Status, setH4Status] = useState<Record<string, H4Status> | null>(null);
  // H3: messages the server held instead of starting a turn (a stuck bot, or one waiting on you).
  const [heldSends, setHeldSends] = useState<HeldSend[]>([]);
  useEffect(() => {
    let live = true;
    setH4Status(null);
    setHeldSends([]);
    h4Api.list(space.id).then((m) => live && setH4Status(m), () => {});
    return () => {
      live = false;
    };
  }, [space.id]);
  // A held line goes once the bot's status no longer lists its message (the turn started).
  useEffect(() => {
    if (!h4Status) return;
    setHeldSends((prev) => {
      const next = prev.filter((h) => stillHeld(h, h4Status[h.kwantId]));
      return next.length === prev.length ? prev : next;
    });
  }, [h4Status]);
  const [pill, setPill] = useState<PillState>(EMPTY_PILL);
  const [nudgesOn, setNudgesOn] = useState(nudgesEnabled);
  const nudgesOnRef = useRef(nudgesOn);
  nudgesOnRef.current = nudgesOn;
  const pageRef = useRef(page);
  pageRef.current = page;
  const arrivalsRef = useRef(createArrivals());
  const offerNudges = (arrivals: Nudge[]) => {
    const view = { openCtx: pageRef.current === 'chat' ? openCtxRef.current : null, onInbox: pageRef.current === 'inbox', hidden: document.hidden, enabled: nudgesOnRef.current };
    const shown = arrivals.filter((n) => shouldPill(n, view));
    if (!shown.length) return;
    setPill((prev) => pillAdd(prev, shown));
    haptic('warning');
  };
  // H5: the chat open now is read; when it closes, the time it was last seen stays.
  const readChat = page === 'chat' && !isChannel && selectedId ? selectedId : null;
  useEffect(() => {
    if (!readChat) return;
    setOpenedAt((prev) => ({ ...prev, [readChat]: Date.now() }));
    return () => setOpenedAt((prev) => ({ ...prev, [readChat]: Date.now() }));
  }, [readChat]);
  const awaySinceRef = useRef<string | null>(null);
  const awayTimerRef = useRef<ReturnType<typeof setTimeout> | undefined>(undefined);
  const refreshAwayCount = (delay = 0) => {
    clearTimeout(awayTimerRef.current);
    awayTimerRef.current = setTimeout(() => {
      awayApi.get().then((r) => {
        setAwayCount(r.unseen.total);
        setAwayWaiting(r.unseen.waiting);
        setAwayItems(r.waiting);
        setAwayDone({ failed: r.failed, finished: r.finished });
        const fresh = arrivalsRef.current.fromAway(r.waiting);
        setPill((prev) => pillPrune(prev, new Set(r.waiting.map((i) => `${i.kind}:${i.id}`))));
        offerNudges(fresh);
        const fromServer = approvalsFromAway(r.waiting);
        const live = liveApprovalIdsRef.current;
        setPendingApprovals((prev) => syncApprovals(prev, fromServer, live));
        for (const a of fromServer) live.delete(a.id);
      }, () => undefined);
    }, delay);
  };
  const toastIdRef = useRef(0);
  const applyReactionRef = useRef<((id: string, reaction: Reaction | null) => void) | null>(null);
  const dismissToast = useCallback((id: number) => setToasts((prev) => prev.filter((t) => t.id !== id)), []);
  // Set each render once the select handlers exist; the SSE handler and notification clicks go through it.
  // Opens a bot's settings on the Skills tab (set once the bots are known).
  const openSkillsRef = useRef<(kwantId: string) => void>(() => undefined);
  // Opens the chat of a bot's newest open question; false when it has none.
  const openQuestionRef = useRef<(kwantId: string) => boolean>(() => false);
  // Opens the chat of a bot's newest open browser step; false when it has none.
  const openHumanStepRef = useRef<(kwantId: string) => boolean>(() => false);
  const openTargetRef = useRef<(t: ChatTarget) => boolean>(() => false);
  // 0.8.1 F6: opens the chat a notification names (thread, channel or team chat, 1:1) at its card.
  const openPlaceRef = useRef<(n: NotifiedChat) => boolean>(() => false);
  // A tier request or a fact dispute is decided on the board's Needs-you list.
  const openBoardForTier = () => showPage('board');
  const showNotification = (n: NotifyEvent) => {
    // Another space's news is the shell's to show, with a way to switch there.
    if (n.space_id && n.space_id !== space.id) return;
    if (n.kind === 'automation_suggested') emitContinuity({ type: 'automation_suggested', data: n });
    // Failed routines and finished or slow delegations show in the app; the OS notification is for a hidden tab.
    // A question or browser step gets the "needs you" pill instead of a toast (approvals get it from the away list too).
    if (!document.hidden && nudgesOnRef.current && (n.kind === 'question_asked' || n.kind === 'human_step')) {
      if (pageRef.current === 'chat' && openCtxRef.current === (n.channel_id ?? (n.thread_id || n.kwant_id))) return;
      if (n.kind === 'human_step') {
        const nudge = arrivalsRef.current.fromHumanStep(n);
        if (nudge) offerNudges([nudge]);
      } else refreshAwayCount(300);
      playSfx('alert');
      return;
    }
    if (!document.hidden && (TOAST_KINDS as string[]).includes(n.kind)) {
      if (n.kind !== 'routine_failed' && n.kind !== 'skill_proposed' && n.kind !== 'automation_suggested' && n.kind !== 'tier_proposed' && n.kind !== 'fact_disputed' && openCtxRef.current === (n.channel_id ?? (n.thread_id || n.kwant_id))) return;
      const id = ++toastIdRef.current;
      const action =
        n.kind === 'routine_failed'
          ? undefined
          : n.kind === 'skill_proposed'
            ? { label: 'Review', onClick: () => openSkillsRef.current(n.kwant_id) }
            : n.kind === 'automation_suggested'
              ? { label: 'Review', onClick: () => openSpaceSettings('routines') }
              : n.kind === 'tier_proposed'
              ? { label: 'Review', onClick: openBoardForTier }
              : n.kind === 'fact_disputed'
              ? { label: 'Settle', onClick: openBoardForTier }
              : n.kind === 'question_asked'
              ? { label: 'Answer', onClick: () => openQuestionRef.current(n.kwant_id) || openPlaceRef.current(n) }
              : n.kind === 'human_step'
                ? { label: 'Open', onClick: () => openHumanStepRef.current(n.kwant_id) || openPlaceRef.current(n) }
                : { label: 'Open', onClick: () => openPlaceRef.current(n) };
      setToasts((prev) => [...prev.slice(-2), { id, title: n.title, detail: n.body, action }]);
      if (n.kind !== 'routine_failed') playSfx('alert');
      return;
    }
    if (!shouldShowOsNotification({ enabled: browserNotifyEnabled(), permission: notificationPermission(), hidden: document.hidden, kind: n.kind })) return;
    showDeviceNotification({ ...n, space_id: n.space_id ?? space.id }, () => openNotifiedRef.current(n)).catch(() => undefined);
  };
  // A tap on a notification: the bot's skill, question or browser step, else its chat. One about no
  // bot opens its place: today's spending stop opens Needs you on it, with Continue today; any other
  // kind (an app update, a loose-end reminder) opens home. 0.8.1 F6: an approval, and anything else
  // about a chat, opens the exact chat it came from (a thread, a channel or team chat, the 1:1) at
  // its card; when that chat is gone, the bot's direct chat.
  const openNotified = (n: Pick<NotifyEvent, 'kind' | 'kwant_id' | 'channel_id' | 'thread_id' | 'card_id'>) => {
    const place = placeOfNotification(n);
    if (place === 'spend_stop') openSpendStop();
    else if (place === 'home') openHome();
    else if (n.kind === 'skill_proposed') openSkillsRef.current(n.kwant_id);
    // A suggested routine is decided in Routines, not in the bot's chat
    else if (n.kind === 'automation_suggested') openSpaceSettings('routines');
    else if (n.kind === 'tier_proposed') openBoardForTier();
    else if (n.kind === 'fact_disputed') openBoardForTier();
    else if (n.kind === 'human_step' && openHumanStepRef.current(n.kwant_id)) {
      setIsVmPaneOpen(true);
      // A CAPTCHA lands straight in the solve view
      if (latestOpenStep(humanStepsRef.current, n.kwant_id)?.kind === 'captcha') setSolveRequest((v) => v + 1);
    }
    else if (!(n.kind === 'question_asked' && openQuestionRef.current(n.kwant_id))) openPlaceRef.current(n);
  };
  const openNotifiedRef = useRef(openNotified);
  const openSpendStopRef = useRef(openSpendStop);
  openSpendStopRef.current = openSpendStop;
  const openHomeRef = useRef(openHome);
  openHomeRef.current = openHome;
  openNotifiedRef.current = openNotified;
  // Taps on notifications shown through the service worker (pushes and this page's own) arrive
  // here; another space's are the shell's.
  useEffect(() => {
    const sw = 'serviceWorker' in navigator ? navigator.serviceWorker : null;
    if (!sw) return;
    const onMessage = (e: MessageEvent) => {
      const m = e.data;
      if (!isNotificationOpen(m)) return;
      // About no bot: the spending stop (server-wide) opens Needs you; other kinds open home.
      const place = placeOfNotification(m);
      if (place === 'spend_stop') return openSpendStopRef.current();
      if (m.space_id && m.space_id !== space.id) return;
      if (place === 'home') return openHomeRef.current();
      // 0.8.1 F6: a tap about a channel or team chat alone (no bot) opens it too
      openNotifiedRef.current({
        kind: m.kind as NotifyKind,
        kwant_id: m.kwant_id ?? '',
        channel_id: m.channel_id,
        thread_id: m.thread_id ?? null,
        card_id: m.card_id ?? null,
      });
    };
    sw.addEventListener('message', onMessage);
    return () => sw.removeEventListener('message', onMessage);
  }, [space.id]);
  // The Mission board shows once the server says it is on (the default); Settings tells us when it changes.
  useEffect(() => {
    let live = true;
    fetch('/api/settings')
      .then((r) => (r.ok ? r.json() : null))
      .then((d) => {
        const on = parseMissionBoardSetting(d);
        if (live && on !== null) setMissionBoard(on);
      })
      .catch(() => {});
    const onSetting = (e: Event) => setMissionBoard(Boolean((e as CustomEvent<boolean>).detail));
    window.addEventListener(MISSION_BOARD_SETTING_EVENT, onSetting);
    return () => {
      live = false;
      window.removeEventListener(MISSION_BOARD_SETTING_EVENT, onSetting);
    };
  }, []);
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
    if (ctx) {
      delete streamsRef.current[ctx];
      delete liveKwantRef.current[ctx];
    }
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
  // 0.8.1 F4: which bot runs in each live chat, so a name that wasn't known yet can be filled in. A page
  // opened mid-turn reads the running turns before the bot list arrives; without this the strip kept the
  // chat's own title ("@QATeam is getting started") for the whole turn.
  const liveKwantRef = useRef<Record<string, string>>({});
  const markLiveKwant = (ctx: string | null | undefined, kwantId: unknown) => {
    if (ctx && typeof kwantId === 'string' && kwantId) liveKwantRef.current[ctx] = kwantId;
    markLive(ctx, kwantName(kwantId));
  };
  // Once the bot list arrives (or changes), name the live chats still waiting for one.
  useEffect(() => {
    for (const [ctx, name] of namesToFill(liveRef.current, liveKwantRef.current, kwantName)) markLive(ctx, name);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [kwants]);

  const chatLoadsRef = useRef(new ChatLoads());
  /** Loads a chat's latest messages into the cache, and on screen if it's open. */
  const refreshContext = async (ctx: string | null | undefined): Promise<void> => {
    if (!ctx) return;
    const n = chatLoadsRef.current.start(ctx);
    try {
      let data: ChatMessage[];
      if (knownThreadsRef.current.has(ctx)) {
        data = await threadsApi.messages<ChatMessage>(ctx);
      } else {
        const res = await fetch(`/api/channels/${encodeURIComponent(ctx)}/messages`);
        if (!res.ok) throw new Error(`HTTP ${res.status}`);
        data = await res.json();
      }
      // A newer load of this chat started meanwhile: it has the newer messages
      if (!chatLoadsRef.current.isLatest(ctx, n)) return;
      messagesCacheRef.current[ctx] = data;
      if (openCtxRef.current === ctx) {
        setMessages(data);
        setMessagesError(null);
      }
    } catch {
      if (openCtxRef.current === ctx && chatLoadsRef.current.isLatest(ctx, n)) setMessagesError('Could not load this chat.');
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
    awayApi.approvals('allowed_once').then((rows) => setSettledApprovals((prev) => addSettled(prev, settledFromRecords(rows))), () => undefined);
    questionsApi.listOpen().then((list) => {
      // Closed ones seen live stay; open ones come from the server.
      setQuestions((prev) => {
        const next = hydrateQuestions(list);
        for (const q of Object.values(prev)) if (q.status !== 'open' && !next[q.id]) next[q.id] = q;
        return next;
      });
    }, () => undefined);
    suggestionsApi.list().then((list) => setSuggestions((prev) => list.reduce(applySuggestion, prev)), () => undefined);
    humanStepsApi.listOpen().then((list) => {
      setHumanSteps((prev) => {
        const next: Record<string, HumanStep> = {};
        for (const st of list) next[st.id] = st;
        for (const st of Object.values(prev)) if (st.status !== 'open' && !next[st.id]) next[st.id] = st;
        return next;
      });
    }, () => undefined);
    try {
      const { turns: list } = await threadsApi.turns();
      const map = hydrateTurns(list);
      setDeciding(hydrateDeciding(list));
      for (const t of Object.values(map)) if (t.thread_id) knownThreadsRef.current.add(t.thread_id);
      turnsRef.current = map;
      setTurns(map);
      resetTurns(Object.values(map).filter((t) => t.state === 'running'));
      // A page opened (or back) mid-turn: what each running bot is doing now, not "getting started".
      let seeded = activityRef.current;
      for (const t of Object.values(map)) {
        const next = t.state === 'running' ? seedActivity(seeded[t.turn_id], t.turn_id, t.activity) : null;
        if (next) seeded = { ...seeded, [t.turn_id]: { ...next, ctx: t.context_id } };
      }
      if (seeded !== activityRef.current) setActivityMap(seeded);
      const busy = new Set(Object.values(map).map((t) => t.context_id));
      for (const ctx of Object.keys(liveRef.current)) {
        if (!busy.has(ctx) && !discussionInProgressRef.current) clearLive(ctx);
      }
      // The strip names the bot that is replying: queued turns first, so a running one in the same chat wins.
      for (const t of liveNamingOrder(Object.values(map))) markLiveKwant(t.context_id, t.kwant_id);
      // Titles for the "working on" badges
      for (const t of Object.values(map)) {
        if (t.thread_id && !threadsByKwantRef.current[t.kwant_id]) loadThreads(t.kwant_id);
      }
    } catch {
      clearAllTurns();
    }
    setTurnsHydrated((n) => n + 1);
  };

  const [isNewAgentOpen, setIsNewAgentOpen] = useState(false);
  // Team that a newly created bot should join (set by a team's "Add member")
  const [newAgentTeam, setNewAgentTeam] = useState<Team | null>(null);
  const [isNewTeamOpen, setIsNewTeamOpen] = useState(false);
  const [isNewChannelOpen, setIsNewChannelOpen] = useState(false);
  const [editingTeam, setEditingTeam] = useState<Team | null>(null);
  const [editingTeamFocusWs, setEditingTeamFocusWs] = useState<boolean | 'folder'>(false);
  useEffect(() => {
    if (!editingTeam) setEditingTeamFocusWs(false);
  }, [editingTeam]);
  const [editingKwant, setEditingKwant] = useState<Kwant | null>(null);
  const [deletingKwant, setDeletingKwant] = useState<Kwant | null>(null);
  const [editingTab, setEditingTab] = useState<EditTab>('profile');
  // Skills waiting for review, for the counts on the bot's header and Skills tab.
  const [pendingSkills, setPendingSkills] = useState<Skill[]>([]);
  // Bots' questions by id: open ones from the server, plus every question_updated since.
  const [questions, setQuestions] = useState<Record<string, Question>>({});
  const questionsRef = useRef(questions);
  questionsRef.current = questions;
  // Steps bots asked the person to do in the browser: open ones from the server, plus every human_step_updated.
  const [humanSteps, setHumanSteps] = useState<Record<string, HumanStep>>({});
  const humanStepsRef = useRef(humanSteps);
  humanStepsRef.current = humanSteps;
  // Bots' tool suggestions by id (from the list and mcp_suggestion_updated), and installed
  // servers that still need the person to sign in.
  const [suggestions, setSuggestions] = useState<Record<string, Suggestion>>({});
  const [needsSignIn, setNeedsSignIn] = useState<Record<string, boolean>>({});
  // Cards for changes bots made (bot_action_updated) and hire proposals (hire_updated), by id.
  const [botActions, setBotActions] = useState<Record<string, BotActionMeta>>({});
  const [hires, setHires] = useState<Record<string, HireMeta>>({});
  // Laya's "Who should answer this?" cards, by route_choice_id (route_choice_updated).
  const [routeChoices, setRouteChoices] = useState<Record<string, RouteChoiceMeta>>({});

  // Virtual Machine & Live Browser state
  // A `/live` address (W2) opens the chat with its browser showing, cold opens included, once the
  // running turns are in and the chat's browser is active. Otherwise (an unknown, malformed or idle
  // link) it shows the plain "this bot isn't browsing now" page (W-L4).
  const [isVmPaneOpen, setIsVmPaneOpen] = useState(false);
  const [liveGate, setLiveGate] = useState<null | 'pending' | 'miss'>(() => (initialTarget?.live ? 'pending' : null));
  // W1: chats whose browser is active right now, from the browser events every stream gets
  const [browsing, setBrowsing] = useState<BrowsingMap>({});
  // Bumped each time the running turns come in from the server (first connect and every reconnect)
  const [turnsHydrated, setTurnsHydrated] = useState(0);
  const [browsingSeeded, setBrowsingSeeded] = useState(0);
  const [solveRequest, setSolveRequest] = useState(0);
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
          // 0.8.1 F6: a notification about one of the bot's conversations opens that one
          if (initialTarget?.threadId && initialTarget.botId === first.bot.id) openThreadChecked(first.bot.id, initialTarget.threadId);
        } else if (first?.kind === 'channel') {
          const init = first.channel;
          setSelectedId(init.id);
          selectedIdRef.current = init.id;
          setIsChannel(true);
          isChannelRef.current = true;
          const matchedTeam = teamOfChannel(normalizedTeams, init);
          setSelectedTeamId(matchedTeam ? matchedTeam.id : null);
        }
        // 0.8.1 F6: and shows the approval's card it names, when that chat opened
        const landed = first?.kind === 'bot' ? initialTarget?.botId === first.bot.id : first?.kind === 'channel' && initialTarget?.channelId === first.channel.id;
        if (landed && initialTarget?.cardId) setHighlightMessageId(initialTarget.cardId);
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
      setMessagesLoadedFor(openCtx);
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
    /** Turns whose TurnComplete came: their live steps can go once the reply is on screen. */
    const completed = new Set<string>();

    /** After a reply: drop the chat's stream once nothing else runs or waits there. */
    /** Live steps of turns in `ctx` that have finished: their saved reply carries them now. */
    const dropActivity = (ctx: string) => {
      const cur = activityRef.current;
      const running = new Set(turnsIn(turnsRef.current, ctx).map((t) => t.turn_id));
      const gone = Object.keys(cur).filter((id) => cur[id].ctx === ctx && (completed.has(id) || !running.has(id)));
      if (!gone.length) return;
      const next = { ...cur };
      for (const id of gone) delete next[id];
      setActivityMap(next);
    };
    const settle = (ctx: string | null) => {
      if (!ctx) return;
      dropActivity(ctx);
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
        if (event.type === 'bot_status') setH4Status((prev) => applyBotStatusEvent(prev ?? {}, event.data));
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
              moodTurnEnded(d.kwant_id, d.outcome);
              starAskStore().turnFinished();
              if (d.turn_id) completed.delete(d.turn_id);
              // The reply is saved by now; TurnComplete's load may have come back without it
              if (reloadOnTurnEnd(d.turn_id)) refreshContext(ctx).then(() => settle(ctx));
              else settle(ctx);
            } else {
              if (event.type === 'turn_started' || event.type === 'TurnStarted') {
                turnStarted(d.kwant_id, key);
                moodTurnStarted(d.kwant_id);
              }
              if (!discussionInProgressRef.current || ctx !== streamingContextIdRef.current) markLiveKwant(ctx, d.kwant_id);
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
            // The same message again is an update (e.g. a decision the person changed): replace it.
            if (list.some((x) => x.id === m.id)) {
              const replaced = list.map((x) => (x.id === m.id ? m : x));
              messagesCacheRef.current[ctx] = replaced;
              if (ctx === openCtxRef.current) setMessages(replaced);
              break;
            }
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
          case 'decide_updated':
          case 'DecideUpdated': {
            const d = event.data ?? {};
            const ctx: string | null = d.context_id ?? d.thread_id ?? d.kwant_id ?? null;
            setDeciding((prev) => applyDecideEvent(prev, event));
            if (!ctx) break;
            // The held draft is never saved: drop it, the line stands in for it.
            if (d.state === 'deciding') {
              if (streamsRef.current[ctx]) streamsRef.current[ctx] = { text: '', thinking: '' };
              if (ctx === openCtxRef.current) {
                setStreamingText('');
                setStreamingThinking('');
              }
            } else {
              refreshContext(ctx);
            }
            break;
          }
          case 'profile_switched':
          case 'ProfileSwitched': {
            // A quiet line under the bot's name, once per chat per outage; no toast and no sound.
            const sw = event.data as FailoverEvent | undefined;
            const botName = kwantName(sw?.kwant_id);
            if (sw && botName && switchLogRef.current.first(outageKey(sw, sw.context_id))) {
              const note: SwitchNote = { name: botName, text: failoverNotice(sw), at: Date.now() };
              setSwitchNotes((prev) => [...prev.filter((n) => Date.now() - n.at < 24 * 3600_000).slice(-19), note]);
            }
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
          case 'skill_recording_updated':
          case 'SkillRecordingUpdated': {
            // Show it once (api.md §5): state changes only, never steps
            const rec = event.data?.recording;
            if (rec?.id) emitContinuity({ type: 'recording_updated', data: rec });
            break;
          }
          case 'show_once_trial_ended':
          case 'ShowOnceTrialEnded': {
            // Show it once (api.md §10): a trial finished, so the chat can show the "still waits" card
            const t = event.data;
            if (t && typeof t.skill_id === 'string' && t.skill_id)
              emitContinuity({
                type: 'trial_ended',
                data: { skill_id: t.skill_id, recording_id: typeof t.recording_id === 'string' ? t.recording_id : null, bot: String(t.bot ?? ''), outcome: String(t.outcome ?? '') },
              });
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
          case 'route_choice_updated':
          case 'RouteChoiceUpdated': {
            const c = event.data?.choice;
            if (c) setRouteChoices((prev) => applyRouteChoice(prev, c));
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
          case 'idea_updated':
          case 'IdeaUpdated':
            emitIdeaUpdated(event.data);
            break;
          case 'task_updated':
            if (event.data?.task) emitMission({ type: 'task_updated', data: { task: event.data.task } });
            break;
          case 'mission_updated':
            if (event.data?.mission) emitMission({ type: 'mission_updated', data: { mission: event.data.mission } });
            break;
          case 'capability_changed':
          case 'CapabilityChanged':
            if (event.data?.capability?.kwant_id) emitMission({ type: 'capability_changed', data: { capability: event.data.capability } });
            break;
          // Truth Ledger (C2): the board's Facts reload; the stream is already this space's.
          case 'fact_changed':
          case 'FactChanged':
            if (event.data?.fact?.id) emitMission({ type: 'fact_changed', data: { fact: event.data.fact } });
            break;
          case 'fact_dispute':
          case 'FactDispute':
            if (event.data?.dispute?.id) emitMission({ type: 'fact_dispute', data: { dispute: event.data.dispute } });
            break;
          // Dev mode (0.7): the switch, tiles, hats and grid follow `dev_mode`; the desk refetches on `dev_card`.
          case 'dev_mode':
          case 'DevMode': {
            const d = event.data ?? {};
            if (typeof d.space_id === 'string' && typeof d.on === 'boolean') {
              devStore.setOn(d.space_id, d.on);
              devApi.get(d.space_id).then((st) => devStore.set(d.space_id, st), () => undefined);
            }
            break;
          }
          case 'dev_card':
          case 'DevCard': {
            const d = event.data ?? {};
            if (typeof d.space_id === 'string' && typeof d.id === 'string') {
              window.dispatchEvent(new CustomEvent<DevCardEvent>(DEV_CARD_EVENT, { detail: { space_id: d.space_id, id: d.id, kind: String(d.kind ?? ''), status: String(d.status ?? '') } }));
            }
            break;
          }
          // 0.8.0 B3: a pre-push card changed; the space's list refetches that card.
          case 'dev_prepush':
          case 'DevPrepush': {
            const d = event.data ?? {};
            if (typeof d.space_id === 'string' && typeof d.id === 'string') {
              window.dispatchEvent(
                new CustomEvent<PrepushEventDetail>(PREPUSH_EVENT, { detail: { space_id: d.space_id, id: d.id, kwant_id: typeof d.kwant_id === 'string' ? d.kwant_id : undefined, status: typeof d.status === 'string' ? d.status : undefined } }),
              );
            }
            break;
          }
          // 0.7.6 Work from Jira: the Tickets tab refetches on `dev_ticket`; a reading card folds in `dev_ticket_progress`.
          case 'dev_ticket':
          case 'dev_ticket_progress': {
            const d = event.data ?? {};
            if (typeof d.space_id === 'string' && typeof d.key === 'string') {
              window.dispatchEvent(new CustomEvent(event.type === 'dev_ticket' ? DEV_TICKET_EVENT : DEV_TICKET_PROGRESS_EVENT, { detail: d }));
            }
            break;
          }
          // Goals (0.6.5): the board's Goals strip reads the goal again.
          case 'goal_updated':
          case 'GoalUpdated': {
            const id = event.data?.id ?? event.data?.goal?.id;
            if (typeof id === 'string' && id) emitMission({ type: 'goal_updated', data: { id, status: event.data?.status ?? event.data?.goal?.status ?? null } });
            break;
          }
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
            moodOutput(event.kwant_id);
            // Each chat streams into its own buffer; only the open chat's is drawn.
            const ctx = streamTarget(event, streamingContextIdRef.current);
            if (!ctx) break;
            const buf = (streamsRef.current[ctx] ??= { text: '', thinking: '' });
            if (event.type === 'ThinkingDelta' && typeof event.turn_id === 'string') {
              // Thinking under an open thinking step goes into it; anything else is a status line.
              const cur = activityRef.current[event.turn_id];
              const next = appendThinking(cur, event.turn_id, String(event.data ?? ''));
              if (cur && next) {
                setActivityMap({ ...activityRef.current, [event.turn_id]: { ...next, ctx: cur.ctx } });
                break;
              }
            }
            if (event.type === 'TextDelta') buf.text += event.data;
            else buf.thinking = event.data;
            if (!(ctx in liveRef.current)) markLiveKwant(ctx, event.kwant_id);
            if (shouldRenderDelta(event, openCtxRef.current, streamingContextIdRef.current)) {
              if (event.type === 'TextDelta') {
                setStreamingText(buf.text);
                playSfx('blip');
              } else setStreamingThinking(buf.thinking);
            }
            break;
          }
          case 'activity':
          case 'Activity': {
            moodOutput(event.kwant_id);
            const step = parseStep(event.data?.step);
            const ctx = streamTarget(event, streamingContextIdRef.current);
            const turnId: string | undefined = typeof event.turn_id === 'string' ? event.turn_id : undefined;
            if (!step || !ctx || !turnId) break;
            const cur = activityRef.current[turnId];
            setActivityMap({ ...activityRef.current, [turnId]: { ...applyActivity(cur, turnId, step), ctx } });
            if (!(ctx in liveRef.current)) markLiveKwant(ctx, event.kwant_id);
            break;
          }
          case 'human_step_updated':
          case 'HumanStepUpdated': {
            const st: HumanStep | undefined = event.data?.step;
            if (!st?.id) break;
            setHumanSteps((prev) => applyHumanStep(prev, st));
            if (st.status === 'open') playSfx('alert');
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
          case 'ApprovalWithdrawn': {
            // Answered in Telegram (when the server says so): kept in the chat as a resolved row (G4).
            // Allowed in Telegram: the card stays as one line. Old {id}-only events just close it.
            const { settled } = withdrawApproval(pendingRef.current, [], event.data);
            if (settled.length) setSettledApprovals((prev) => addSettled(prev, settled));
            dropApproval(event.data?.id);
            break;
          }
          case 'telegram.status':
          case 'telegram.paired':
          case 'telegram.expired':
            if (forThisSpace(event.data?.space_id, getCurrentSpace()))
              window.dispatchEvent(new CustomEvent<TelegramEventDetail>(TG_EVENT, { detail: { type: event.type, data: event.data ?? {} } }));
            break;
          case 'discussion_started':
          case 'DiscussionStarted': {
            discussionInProgressRef.current = true;
            const teamId = event.data?.team_id;
            const initialSpeakerId = event.data?.speakers?.[0];
            const initialSpeakerName =
              kwantsRef.current.find((k) => k.id === initialSpeakerId)?.name || initialSpeakerId;

            const matchedChannel = teamChannelOf(channelsRef.current, { id: teamId ?? '', name: event.data?.team_name ?? '' });
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
            messageHaptic();
            refreshContext(targetId).then(() => settle(targetId));
            break;
          }
          case 'TurnComplete': {
            const ctx = streamTarget(event, streamingContextIdRef.current);
            if (ctx === openCtxRef.current) {
              playSfx('coin');
              // Live events only (the stream has no replay; history loads via refreshContext).
              messageHaptic({ id: typeof event.turn_id === 'string' ? event.turn_id : undefined });
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
            setBrowsing((m) => noteBrowser(m, teamId, event.data?.url, Date.now()));
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
            setBrowsing((m) => noteBrowser(m, teamId, event.data?.url, Date.now()));
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
      clearMoods();
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

  const handleSendMessage = async (text: string, attachments: AttachmentRef[] = [], opts: SendOptions = {}): Promise<boolean | string> => {
    haptic('tap');
    setHistoryWindow(null);
    // O2 §3: the Get started card goes after the person's first message in any chat, once something can answer.
    if (canRun !== false && !getStartedDismissed(space.id)) dismissGetStarted(space.id);
    const targetId = openCtx;
    const threadId = openThreadId;
    const isTargetChannel = isChannel;
    const targetTeam = selectedTeamId;
    // `/idea …` (spec §2.3): the server files it and starts no turn, so no bubble and no "thinking".
    // A string back is the composer's error line; the text goes back into the box.
    if (isIdeaCommand(text) && !attachments.length) {
      try {
        const res = await fetch('/api/chat/send', {
          method: 'POST',
          headers: { 'Content-Type': 'application/json' },
          body: JSON.stringify(
            threadId
              ? { thread_id: threadId, prompt: text, sender_id: 'PlayerHero', attachment_ids: [] }
              : { channel_id: isTargetChannel ? targetId : null, kwant_id: !isTargetChannel ? targetId : null, team_id: targetTeam || null, prompt: text, sender_id: 'PlayerHero', attachment_ids: [] },
          ),
        });
        const out = ideaSendOutcome(res.status, await res.json().catch(() => null));
        if (out.kind === 'error') {
          haptic('warning');
          return out.line;
        }
        if (out.kind === 'dropped') {
          emitIdeaUpdated({ id: out.idea.id, status: out.idea.status, space_id: out.idea.space_id || space.id, idea: out.idea });
          setIdeaToast({ id: out.idea.id, state: 'dropped' });
          wobbleJar();
          haptic('success');
          return true;
        }
        // An older server ran it as a chat message: show it the usual way.
        refreshContext(targetId);
        return true;
      } catch {
        haptic('warning');
        return JAR_SEND_FAILED;
      }
    }
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
      reply_to: opts.replyTo ? optimisticReplyRef(opts.replyTo) : null,
    };
    const reply_to_message_id = opts.replyTo?.id;
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
            ? { thread_id: threadId, prompt: text, sender_id: 'PlayerHero', attachment_ids, ...(reply_to_message_id ? { reply_to_message_id } : {}) }
            : {
                channel_id: isTargetChannel ? targetId : null,
                kwant_id: !isTargetChannel ? targetId : null,
                team_id: targetTeam || null,
                prompt: text,
                sender_id: 'PlayerHero',
                attachment_ids,
                ...(reply_to_message_id ? { reply_to_message_id } : {}),
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
      const reply = await res.json().catch(() => null);
      const held = heldFromReply(reply, tempId);
      if (held) {
        // H3: saved, but no turn starts yet, so no "thinking" for it; the line under the message says why.
        if (turnsIn(turnsRef.current, targetId).length === 0) clearLive(targetId);
        if (streamingContextIdRef.current === targetId) streamingContextIdRef.current = null;
        setHeldSends((prev) => [...prev.filter((h) => h.messageId !== held.messageId), held]);
        h4Api.list(space.id).then((m) => m && setH4Status(m), () => {});
      }
      if (isAskingReply(reply)) {
        // A close call: Laya posted "Who should answer this?" and nothing runs until the person picks.
        if (turnsIn(turnsRef.current, targetId).length === 0) clearLive(targetId);
        if (streamingContextIdRef.current === targetId) {
          setRoutingState(null);
          streamingContextIdRef.current = null;
        }
      }
      return true;
    } catch (err) {
      console.error('Failed sending chat:', err);
      haptic('warning');
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

  /** Change on a decision chip: record the new pick, show it, then ask the bot to redo it in this chat. */
  const handleChangeDecision = async (msg: ChatMessage, index: number, picked: string): Promise<string | void> => {
    haptic('tap');
    let prompt: string;
    try {
      const res = await decisionsApi.change<ChatMessage>(msg.id, index, picked);
      prompt = res.prompt;
      const ctx = messageContextId(res.message ?? msg);
      if (ctx && res.message?.id) {
        const list = messagesCacheRef.current[ctx];
        if (list) {
          const replaced = list.map((x) => (x.id === res.message.id ? res.message : x));
          messagesCacheRef.current[ctx] = replaced;
          if (ctx === openCtxRef.current) setMessages(replaced);
        }
      }
    } catch (err) {
      haptic('warning');
      return err instanceof Error ? err.message : 'Couldn’t change it.';
    }
    if (prompt && !(await handleSendMessage(prompt))) return 'Your choice is saved, but the bot wasn’t told. Ask it in this chat to redo it with your choice.';
  };

  const handleResolveApproval = async (
    ticketId: string,
    decision: 'allow_once' | 'always_allow' | 'deny'
  ) => {
    haptic('tap');
    try {
      const res = await fetch(`/api/approvals/${ticketId}/resolve`, {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ decision }),
      });
      // Production floor (Dev mode): Allow without a fresh backup is refused and the card stays waiting.
      if (res.status === 409) {
        const body = await res.clone().json().catch(() => null);
        if (body && typeof body === 'object' && (body as { status?: unknown }).status === 'needs_backup') {
          haptic('warning');
          pushToast('Back up first', 'This step needs a backup from the last 10 minutes. Take one on the card, then Allow.');
          return;
        }
      }
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
        haptic('warning');
        pushToast("Couldn't send your answer", `Request failed (HTTP ${res.status}). Try again.`);
      }
    } catch (err) {
      console.error('Failed resolving approval:', err);
      haptic('warning');
      pushToast("Couldn't send your answer", 'Check the connection and try again.');
    }
  };

  const handleSelectTeam = (team: Team) => {
    setOpenThreadId(null);
    setPage('chat');
    setPhoneView('chat');
    setSelectedTeamId(team.id);
    const targetChannel = teamChannelOf(channels, team);
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
          const match = teamChannelOf(freshChannels, team);
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

  // 0.8.1 F3: where phone Back returns after a team chat's pointer line opened a 1:1. Any other
  // chat open clears it (openPrepushFromTeam sets it after its own open), so it never goes stale.
  const prepushReturnRef = useRef<{ channel: string; kwant: string } | null>(null);

  const handleSelectChannel = (channel: Channel) => {
    prepushReturnRef.current = null;
    setOpenThreadId(null);
    setPage('chat');
    setPhoneView('chat');
    setSelectedId(channel.id);
    selectedIdRef.current = channel.id;
    setIsChannel(true);
    isChannelRef.current = true;
    const matchedTeam = teamOfChannel(teams, channel);
    setSelectedTeamId(matchedTeam ? matchedTeam.id : null);
    setMessages(messagesCacheRef.current[channel.id] || []);
    playSfx('click');
  };

  const handleSelectKwant = (kwant: Kwant) => {
    prepushReturnRef.current = null;
    setOpenThreadId(null);
    setPage('chat');
    setPhoneView('chat');
    setSelectedId(kwant.id);
    selectedIdRef.current = kwant.id;
    setIsChannel(false);
    isChannelRef.current = false;
    setSelectedTeamId(null);
    setMessages(messagesCacheRef.current[kwant.id] || []);
    playSfx('click');
  };

  /** Closes the page on top of the chat (board, Loose ends, Away…) and shows the chat pane. */
  const revealChat = () => {
    setPage('chat');
    setPhoneView('chat');
  };

  /** Opens a channel, or the direct chat with a bot when there's no channel. False if it's gone. */
  const openTarget = (t: ChatTarget): boolean => {
    const plan = planOpenTarget(
      t,
      { selectedId: selectedIdRef.current, isChannel: isChannelRef.current, openThreadId },
      { channel: (id) => channels.some((c) => c.id === id), kwant: (id) => kwants.some((k) => k.id === id) },
    );
    if (plan === 'gone') return false;
    // Already the selected chat: the board or Loose ends may still cover it.
    if (plan === 'reveal') revealChat();
    else if (t.channel_id) handleSelectChannel(channels.find((c) => c.id === t.channel_id)!);
    else handleSelectKwant(kwants.find((k) => k.id === t.kwant_id)!);
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
  openHumanStepRef.current = (kwantId: string) => {
    const st = latestOpenStep(humanStepsRef.current, kwantId);
    if (!st) return false;
    if (st.thread_id) openThread(st.kwant_id, st.thread_id);
    else openTarget({ channel_id: st.channel_id, kwant_id: st.channel_id ? null : st.kwant_id });
    if (st.message_id) setHighlightMessageId(st.message_id);
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

  /** Sets one message's reaction wherever it is shown: every cached chat and the open history window. */
  const applyReaction = (messageId: string, reaction: Reaction | null) => {
    const patch = (list: ChatMessage[]) =>
      list.some((m) => m.id === messageId) ? list.map((m) => (m.id === messageId ? { ...m, reaction } : m)) : list;
    for (const ctx of Object.keys(messagesCacheRef.current)) {
      const list = messagesCacheRef.current[ctx];
      const next = patch(list);
      if (next !== list) {
        messagesCacheRef.current[ctx] = next;
        if (ctx === openCtxRef.current) setMessages(next);
      }
    }
    setHistoryWindow((w) => {
      if (!w) return w;
      const next = patch(w.messages);
      return next === w.messages ? w : { ...w, messages: next };
    });
  };
  applyReactionRef.current = applyReaction;

  /**
   * Shows the new reaction at once; if the save fails, puts the old one back (ChatView says so).
   * A thumb on the bot's newest question is the person's yes or no: the server starts a turn, and the
   * chat shows the bot working exactly as after a send.
   */
  const handleReact = async (msg: ChatMessage, reaction: Reaction | null): Promise<ReactResult> => {
    const before = reactionOf(msg.reaction);
    applyReaction(msg.id, reaction);
    try {
      const saved = await saveReaction(fetch, msg.id, reaction);
      if (saved.reaction !== reaction) applyReaction(msg.id, saved.reaction);
      if (saved.answered) {
        const ctx = messageContextId(msg) ?? openCtxRef.current;
        if (ctx) {
          haptic('tap');
          streamingContextIdRef.current = ctx;
          if (!(ctx in liveRef.current)) {
            streamsRef.current[ctx] = { text: '', thinking: '' };
            if (ctx === openCtxRef.current) {
              setStreamingText('');
              setStreamingThinking('');
            }
          }
          markLive(ctx, senderLabel(msg.sender_role, msg.sender_id, liveRef.current[ctx] || 'Bot'));
        }
      }
      return { ok: true, answered: saved.answered };
    } catch (err) {
      applyReaction(msg.id, before);
      haptic('warning');
      return { ok: false, answered: false, error: err instanceof Error ? err.message : undefined };
    }
  };

  /** Opens one of a bot's threads (from the thread list, a delegation card or a badge). */
  const openThread = (kwantId: string, threadId: string) => {
    const kwant = kwants.find((k) => k.id === kwantId);
    if (!kwant) {
      pushToast('That bot no longer exists.');
      return;
    }
    knownThreadsRef.current.add(threadId);
    setPage('chat');
    if (selectedIdRef.current !== kwant.id || isChannelRef.current) handleSelectKwant(kwant);
    else playSfx('click');
    setOpenThreadId(threadId);
    if (threadsByKwantRef.current[kwantId] === undefined) loadThreads(kwantId);
  };

  /**
   * 0.8.1 F6: shows one of a bot's conversations a link or notification names. When kwant doesn't
   * know it yet, asks the server (archived ones included); if it is gone, the bot's direct chat
   * shows instead, with a line saying so, never an empty pane.
   */
  function openThreadChecked(kwantId: string, threadId: string) {
    const known = knownThreadsRef.current.has(threadId);
    knownThreadsRef.current.add(threadId);
    setOpenThreadId(threadId);
    if (known) return;
    threadsApi.list(kwantId, true).then((list) => {
      rememberThreads(list);
      if (list.some((t) => t.id === threadId)) return;
      knownThreadsRef.current.delete(threadId);
      setOpenThreadId((cur) => (cur === threadId ? null : cur));
      pushToast('That conversation no longer exists, so here is the direct chat.');
    }, () => undefined);
  }

  /** 0.8.1 F6: opens the chat a notification names, at its card; false when neither it nor the bot exists. */
  const openPlace = (n: NotifiedChat): boolean => {
    const place = notifiedPlace(n, {
      channel: (id) => channels.some((c) => c.id === id),
      kwant: (id) => kwants.some((k) => k.id === id),
    });
    if (place.kind === 'gone') {
      pushToast('That chat no longer exists.');
      return false;
    }
    if (place.kind === 'thread') {
      const kwant = kwants.find((k) => k.id === place.kwant_id)!;
      setPage('chat');
      setPhoneView('chat');
      if (selectedIdRef.current !== kwant.id || isChannelRef.current) handleSelectKwant(kwant);
      openThreadChecked(place.kwant_id, place.thread_id);
    } else {
      // A gone channel lands in the bot's direct chat: say so
      if (n.channel_id && !place.target.channel_id) pushToast('That chat no longer exists, so here is the direct chat.');
      openTarget(place.target);
      setPage('chat');
    }
    if (n.card_id) setHighlightMessageId(n.card_id);
    return true;
  };
  openPlaceRef.current = openPlace;

  /** Opens the thread, channel or direct chat an away item links to. */
  const openAwayItem = (item: AwayItem) => {
    const t = awayTarget(item);
    // 0.8.1 F6: the same opener as a notification's tap, so a gone chat lands in the bot's 1:1 and
    // an approval row shows its card
    openPlace({
      channel_id: 'channel' in t ? t.channel : null,
      kwant_id: 'thread' in t ? t.kwant : 'direct' in t ? t.direct : item.kwant_id,
      thread_id: 'thread' in t ? t.thread : null,
      card_id: item.kind === 'approval' ? item.id : null,
    });
  };

  /** 0.8.0 B3: a pre-push Inbox item opens the bot's chat at its card. */
  const openPrepush = (c: PrepushCardData) => {
    if (!openTarget({ channel_id: null, kwant_id: c.kwant_id })) return pushToast('That bot no longer exists.');
    setPage('chat');
    window.setTimeout(() => {
      const el = document.getElementById(`prepush-${c.id}`);
      el?.scrollIntoView?.({ block: 'center' });
      // 0.8.1 F3: a short ring so the eye finds the card it landed on (none under reduced motion, in CSS).
      if (el) {
        el.classList.remove('kw-prepush-flash');
        void el.offsetWidth;
        el.classList.add('kw-prepush-flash');
        window.setTimeout(() => el.classList.remove('kw-prepush-flash'), 1300);
      }
    }, 250);
  };
  // 0.8.1 F3: a team chat's pointer line opens the bot's 1:1; on a phone, back returns to that team chat.
  const openPrepushFromTeam = (c: PrepushCardData) => {
    const from = isChannelRef.current ? selectedIdRef.current : null;
    openPrepush(c);
    prepushReturnRef.current = from ? { channel: from, kwant: c.kwant_id } : null;
  };
  const phoneBackFromChat = () => {
    const r = prepushReturnRef.current;
    prepushReturnRef.current = null;
    if (r && !isChannelRef.current && selectedIdRef.current === r.kwant && !openThreadId && openTarget({ channel_id: r.channel, kwant_id: null })) return;
    setPhoneView('list');
  };
  // The card call first; a 409 means the card moved on, so fetch it again (Rigel QA 1). Then the
  // chat message: if only that fails, say the change applied (Ada Later 7).
  const prepushAct = async (c: PrepushCardData, act: () => Promise<PrepushCardData>, message: string) => {
    try {
      prepush.put(await act());
    } catch (e) {
      if (e instanceof DevModeError && e.status === 409) void prepush.refresh(c.id);
      throw e;
    }
    let sent: boolean | string = false;
    try {
      sent = await handleSendMessage(message);
    } catch {
      sent = false;
    }
    if (sent !== true) throw new DevModeError(prepushMessageNotSent(kwantNames[c.kwant_id] || 'the bot'), 0, 'message_not_sent');
  };
  const prepushRetry = (c: PrepushCardData) => prepushAct(c, () => prepushApi.retry(c.id), retryMessage(c));
  const prepushOverride = (c: PrepushCardData, reason: string) => prepushAct(c, () => prepushApi.override(c.id, reason), overrideMessage(c));

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
    haptic('tap');
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
      haptic('warning');
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

  /** Done or Skip on a browser step. A 409 means it was answered elsewhere: show how it ended. */
  const answerHumanStep = async (id: string, which: 'done' | 'decline'): Promise<string | void> => {
    haptic('tap');
    try {
      const { step } = await (which === 'done' ? humanStepsApi.done(id) : humanStepsApi.decline(id));
      setHumanSteps((prev) => applyHumanStep(prev, step));
      if (which === 'done') haptic('success');
    } catch (err) {
      try {
        const { step } = await humanStepsApi.get(id);
        setHumanSteps((prev) => applyHumanStep(prev, step));
        if (step.status === 'done') return 'This was already marked done.';
        if (step.status === 'declined') return 'This was already skipped.';
        if (step.status === 'cancelled') return 'This was closed: the reply was stopped.';
      } catch {
        // keep the first error
      }
      haptic('warning');
      return `Couldn't send that: ${errText(err)}`;
    }
  };
  const renderHumanStep = (msg: ChatMessage, meta: HumanStepMeta, flash: boolean) => {
    const st = humanSteps[meta.human_step_id];
    const turnId = st?.turn_id;
    const name = st ? kwantName(st.kwant_id) : undefined;
    return (
      <HumanStepCard
        meta={meta}
        messageId={msg.id}
        kwantName={name ?? (msg.sender_id && msg.sender_id.length <= 20 ? msg.sender_id : chatTitle)}
        kwantId={st?.kwant_id}
        live={st}
        waiting={!!turnId && !!turns[turnId]}
        onOpenBrowser={(_target, solve) => {
          setIsVmPaneOpen(true);
          if (solve) {
            setSolveRequest((v) => v + 1);
            haptic('tap');
          }
          playSfx('click');
        }}
        onDone={(id) => answerHumanStep(id, 'done')}
        onSkip={(id) => answerHumanStep(id, 'decline')}
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
  // In the Mac app the sign-in opens in the default browser and the server is watched instead.
  const handleSuggestionSignIn = async (serverId: string): Promise<string | void> => {
    const macApp = inDesktopApp();
    const popup = macApp ? null : window.open('', 'kwnt-mcp-oauth', 'popup,width=520,height=720');
    const before = macApp ? await mcpSignIn(serverId).catch(() => undefined) : undefined;
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
      leaveForSignIn(url);
      if (!macApp) return;
      const finished = await new Promise<boolean>((resolve) => {
        pollUntil(async () => signInChanged(before, await mcpSignIn(serverId)), () => resolve(true), {
          onTimeout: () => resolve(false),
        });
      });
      if (!finished) return 'The sign-in didn’t finish in your browser. Try again.';
    } else {
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
    }
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

  const renderConnectionSuggestion = (msg: ChatMessage, meta: ConnectionSuggestionMeta, flash: boolean) => {
    const name = kwantName(meta.kwant_id ?? undefined);
    return (
      <ConnectionSuggestionCard
        meta={meta}
        messageId={msg.id}
        kwantName={name ?? (msg.sender_id && msg.sender_id.length <= 20 ? msg.sender_id : chatTitle)}
        kwantId={meta.kwant_id ?? undefined}
        onOpenConnections={openConnections}
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
      // U-1: an Undo from the history can remove a team and its chat; pick up the lists without a reload.
      if (d === 'undo') {
        refreshRoster();
        void syncChannelsAfterUndo();
      }
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

  const renderRouteChoice = (msg: ChatMessage, meta: RouteChoiceMeta, flash: boolean, question?: string) => (
    <RouteChoiceCard
      meta={meta}
      messageId={msg.id}
      live={routeChoices[meta.route_choice_id]}
      question={question}
      flash={flash}
      onResolve={async (id, pick) => {
        haptic('tap');
        const r = await resolveRouteChoice(id, pick);
        if (r.kind === 'error') haptic('warning');
        return r;
      }}
    />
  );

  /** "Wrong bot?": teach routing that this message is the other bot's, then ask that bot now. */
  const handleSendToOtherBot = async (question: ChatMessage, kwantId: string): Promise<string | void> => {
    haptic('tap');
    const err = await correctRoute(question.id, kwantId);
    if (err) {
      haptic('warning');
      return err;
    }
    const name = kwants.find((k) => k.id === kwantId)?.name;
    if (!name) return 'That bot is gone.';
    if (!(await handleSendMessage(`@${name} ${question.text}`))) return `Routing learned, but ${name} wasn’t asked. Send it again with @${name}.`;
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
          ...(agentData.covers?.length ? { covers: agentData.covers } : {}),
          ...(agentData.avoids?.length ? { avoids: agentData.avoids } : {}),
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

  /** Creates a channel and opens it; resolves to what went wrong, or null. */
  const handleCreateChannel = async (body: NewChannelBody): Promise<string | null> => {
    try {
      const res = await fetch('/api/channels', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify(body),
      });
      if (!res.ok) {
        const data = await res.json().catch(() => null);
        return typeof data?.error === 'string' ? data.error : `Could not create the channel (HTTP ${res.status}).`;
      }
      const created: Channel = await res.json();
      setChannels((prev) => (prev.some((c) => c.id === created.id) ? prev : [...prev, created]));
      messagesCacheRef.current[created.id] = [];
      setIsNewChannelOpen(false);
      handleSelectChannel(created);
      playSfx('fanfare');
      onSpacesChanged();
      return null;
    } catch {
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
          const teamChan = teamChannelOf(freshChannels, created);
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

  // 0.7.7 T: a template team was made; open its chat (the server answers with its channel id).
  // U-1: where the person was before a new team's chat opened, so Undo can take them back there.
  const beforeNewTeamRef = useRef<{ id: string; isChannel: boolean } | null>(null);
  const openNewTeamChat = async (created: { id: string; name: string }, channelId: string) => {
    if (selectedIdRef.current) beforeNewTeamRef.current = { id: selectedIdRef.current, isChannel: isChannelRef.current };
    // U-2: the team is known at once (the roster reload brings the full row), so its chat is a team
    // chat from the first frame: the Needs card, never the first-run welcome.
    setTeams((prev) => (prev.some((t) => t.id === created.id) ? prev : [...prev, { id: created.id, name: created.name, icon: '', lead_kwant_id: '', member_kwants: [], created_at: new Date().toISOString() }]));
    setSelectedTeamId(created.id);
    let chan: Channel | undefined;
    try {
      const cRes = await fetch('/api/channels');
      if (cRes.ok) {
        const freshChannels: Channel[] = await cRes.json();
        setChannels(freshChannels);
        chan = freshChannels.find((c) => c.id === channelId) ?? teamChannelOf(freshChannels, created as Team);
      }
    } catch {
      /* the channel event selects nothing; the id alone is enough below */
    }
    const id = chan?.id ?? channelId;
    if (!id) return;
    setSelectedId(id);
    selectedIdRef.current = id;
    setIsChannel(true);
    isChannelRef.current = true;
    messagesCacheRef.current[id] = [];
    setMessages([]);
    setPage('chat');
    setPhoneView('chat');
  };
  // U-1: Undo removed a team. Its chat leaves the lists at once; if it was open, go back to where the
  // person was before (or the space's home). Then the server's channel list confirms it.
  const leaveRemovedChat = (gone: Set<string>) => {
    if (!isChannelRef.current || !selectedIdRef.current || !gone.has(selectedIdRef.current)) return;
    const prev = beforeNewTeamRef.current;
    beforeNewTeamRef.current = null;
    const left = channelsRef.current.filter((c) => !gone.has(c.id));
    if (prev && !prev.isChannel && kwants.some((k) => k.id === prev.id)) {
      handleSelectKwant(kwants.find((k) => k.id === prev.id)!);
      return;
    }
    const back =
      (prev?.isChannel && !gone.has(prev.id) && left.find((c) => c.id === prev.id)) ||
      left.find((c) => c.id === space.general_channel_id) ||
      left[0];
    if (back) handleSelectChannel(back);
    else {
      setSelectedId('');
      selectedIdRef.current = '';
      setSelectedTeamId(null);
      setMessages([]);
    }
  };
  const syncChannelsAfterUndo = async () => {
    try {
      const cRes = await fetch('/api/channels');
      if (!cRes.ok) return;
      const fresh: Channel[] = await cRes.json();
      const ids = new Set(fresh.map((c) => c.id));
      const gone = new Set(channelsRef.current.filter((c) => !ids.has(c.id)).map((c) => c.id));
      setChannels(fresh);
      channelsRef.current = fresh;
      for (const id of gone) delete messagesCacheRef.current[id];
      leaveRemovedChat(gone);
    } catch {
      /* the next load brings it */
    }
  };
  const handleTeamUndone = (team: { id: string; name: string }) => {
    const chan = teamChannelOf(channelsRef.current, team);
    setTeams((prev) => prev.filter((t) => t.id !== team.id));
    if (chan) {
      const gone = new Set([chan.id]);
      leaveRemovedChat(gone);
      const left = channelsRef.current.filter((c) => c.id !== chan.id);
      setChannels(left);
      channelsRef.current = left;
      delete messagesCacheRef.current[chan.id];
    } else if (selectedTeamId === team.id) setSelectedTeamId(null);
    void syncChannelsAfterUndo();
  };
  const [looseAddImap, setLooseAddImap] = useState(0);
  const openTeamNeed = (opens: string, team: ReadyTeam) => {
    if (opens === 'loose_ends_imap') {
      setLooseAddImap((n) => n + 1);
      showPage('inbox', 'loose');
    } else if (opens === 'connections') openConnections();
    else if (opens === 'dev_mode_setup') {
      // The team's repo is set in Manage team (Team settings > Git Repo), where Dev mode's line shows.
      const t = teams.find((x) => x.id === team.id);
      if (t) {
        setEditingTeamFocusWs(true);
        setEditingTeam(t);
      }
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
    ? teamOfChannel(teams, currentChannel)
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
  // 0.8.1 F3: whose pre-push lines a team chat or channel shows (membership, like approvalsForChat).
  const pointerMemberIds: string[] = currentTeam ? currentTeamMemberIds : openChat?.kind === 'channel' ? openChat.memberIds.filter((id): id is string => !!id) : [];

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

  // "Suggested:" prompts from who is in this chat, so a kitchen team isn't offered developer prompts
  const idsOf = (v: string[] | string | undefined): string[] => {
    if (Array.isArray(v)) return v;
    try {
      const parsed = JSON.parse(v || '[]');
      return Array.isArray(parsed) ? parsed : [];
    } catch {
      return [];
    }
  };
  const channelMembers = currentChannel
    ? kwants.filter((k) => k.id === currentChannel.lead_kwant_id || idsOf(currentChannel.member_kwants).includes(k.id))
    : [];
  const promptContext: PromptContext | null = currentKwant
    ? { kind: 'bot', bot: currentKwant }
    : isGeneral
      ? { kind: 'general', bots: kwants }
      : currentTeam
        ? {
            kind: 'team',
            members: currentTeamMembers,
            lead: kwants.find((k) => k.id === currentTeam.lead_kwant_id)?.name,
            purpose: currentTeam.description,
          }
        : currentChannel
          ? {
              kind: 'team',
              members: channelMembers,
              lead: kwants.find((k) => k.id === currentChannel.lead_kwant_id)?.name,
              purpose: currentChannel.description,
            }
          : null;
  const chatSuggestions = promptContext ? suggestedPrompts(promptContext) : [];

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



  // The address bar and "reopen where I was" follow the open chat.
  const chatChangeRef = useRef(onChatChange);
  chatChangeRef.current = onChatChange;
  // W-L4: the chat a `/live` link names is one of this space's
  const liveTargetKnown = !!(initialTarget?.botId ? kwants.some((k) => k.id === initialTarget.botId) : initialTarget?.channelId && channels.some((c) => c.id === initialTarget.channelId));
  useEffect(() => {
    if (!loaded || !selectedId) return;
    // The address says `/live` while the panel is open; closing it rewrites it back to the chat in
    // place (the Shell replaces, never pushes), so Back doesn't reopen the panel.
    // A `/live` link that is still being checked, or showing "isn't browsing now", keeps its address.
    const live = isVmPaneOpen || (liveGate && liveTargetKnown) ? { live: true } : {};
    chatChangeRef.current(isChannel ? { channelId: selectedId, ...live } : { botId: selectedId, ...live });
  }, [loaded, selectedId, isChannel, isVmPaneOpen, liveGate, liveTargetKnown]);

  const isCurrentContextStreaming = openCtx in live;
  const decidingHere = deciding[openCtx] ?? null;
  const openTurns = turnsIn(turns, openCtx);
  // Turns paused on a question wait for the person; they don't make the bot "busy".
  const browserTurns = turnsAwaitingStep(humanSteps);
  const awaitingTurns = new Set([...turnsAwaitingAnswer(questions), ...browserTurns]);
  const awaitingHere = openTurns.some((t) => t.state === 'running' && awaitingTurns.has(t.turn_id)) && !openTurns.some((t) => t.state === 'running' && !awaitingTurns.has(t.turn_id));
  const awaitingBrowserHere = awaitingHere && openTurns.some((t) => t.state === 'running' && browserTurns.has(t.turn_id));
  // The open chat's running turn's live steps (the newest, when several reply at once).
  const activityHere = Object.values(activity).filter((a) => a.ctx === openCtx && turns[a.turnId]).pop() ?? null;
  const needsYouByKwant = openStepsByKwant(humanSteps);
  const chatQueue = queuedInChat(turns, openCtx);
  const asQueued = (t: TrackedTurn): QueuedState => ({
    position: queuePosition(turns, t.turn_id),
    reason: t.reason ?? 'agent_busy',
    slots: t.slots,
    speaker: kwantName(t.kwant_id) ?? live[openCtx] ?? 'The bot',
    turnId: t.turn_id,
  });
  const queuedHere: QueuedState | null = chatQueue.alone ? asQueued(chatQueue.alone) : null;
  // P6: a turn waiting behind the reply that runs here shows under it, not hidden until it starts.
  const queuedBeside: QueuedState[] = chatQueue.beside.map((t) => ({ ...asQueued(t), named: waitsForOtherBot(turns, openCtx, t) }));

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
          {elsewhere[0].state === 'queued' ? 'Waiting to work on ' : browserTurns.has(elsewhere[0].turn_id) ? 'Needs you in the browser in ' : awaitingTurns.has(elsewhere[0].turn_id) ? 'Asked you a question in ' : 'Working on '}
          {describeContext(elsewhere[0].context_id, elsewhere[0].kwant_id)}
        </span>
        {elsewhere.length > 1 && <span className="shrink-0 text-sky-300/80">+{elsewhere.length - 1} more</span>}
      </button>
    ) : null;
  const pendingSkillCounts = pendingByKwant(pendingSkills);
  const pendingHere = currentKwant ? pendingSkillCounts[currentKwant.id] ?? 0 : 0;
  // "Approvals off" is said once in the sidebar for the whole space; here only a bot that differs says it.
  const approvalsBadge = approvalsOff(currentKwant) && approvalsDiffers(currentKwant!, space.approvals) ? (
    <ApprovalsOffBadge onClick={() => currentKwant && openAgentSettings(currentKwant)} title={`${currentKwant?.name} acts without asking, including deletions, publishing and sign-ins. Payments still ask you. Open its settings to change it.`} />
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
  // W1–W3 Watch live. An owner stays while a running turn drives its browser, and drops after a minute
  // with no browser event.
  const teamOfCh = (c: { id: string; name: string }) => teamOfChannel(teams, c);
  const runningTurns = Object.values(turns);
  const liveBrowsing: BrowsingMap = Object.fromEntries(
    Object.entries(browsing).filter(([owner]) => ownerStillRunning(owner, runningTurns, channels, teamOfCh)),
  );
  const browsingKey = Object.keys(liveBrowsing).sort().join(',');
  useEffect(() => {
    // Forget owners whose last turn ended, so an old page never comes back with the next turn. The
    // check reads the newest turns inside the updater: a key from this render could be older than a
    // browser event queued meanwhile, and dropped it (QA F6).
    if (!loaded) return;
    setBrowsing((m) => pruneBrowsing(m, Object.values(turnsRef.current), channelsRef.current, (c) => teamOfChannel(teamsRef.current, c)));
  }, [turns, loaded]);
  useEffect(() => {
    // A page opened mid-turn: a running turn whose newest step is a browser tool is browsing already
    if (!loaded || !turnsHydrated) return;
    const withSteps = Object.values(turnsRef.current).map((t) => ({ ...t, steps: activityRef.current[t.turn_id]?.steps }));
    setBrowsing((m) => seedBrowsing(m, withSteps, channelsRef.current, (c) => teamOfChannel(teamsRef.current, c), Date.now()));
    setBrowsingSeeded(turnsHydrated);
  }, [turnsHydrated, loaded]);
  useEffect(() => {
    if (!browsingKey) return;
    const id = window.setInterval(() => setBrowsing((m) => expireBrowsing(m, Date.now())), 5_000);
    return () => window.clearInterval(id);
  }, [browsingKey]);
  const globes = globeRows(liveBrowsing, runningTurns, channels, teamOfCh);
  const browserNeedsYou = new Set(Object.values(humanSteps).filter((st) => st.status === 'open' && st.sandbox_target).map((st) => st.sandbox_target!));
  const describeOwner = (owner: string) => {
    const t = teams.find((x) => x.id === owner);
    if (t) return { kind: 'team' as const, label: t.name };
    const c = channels.find((x) => x.id === owner);
    if (c) {
      const ct = teamOfCh(c);
      return ct ? { kind: 'team' as const, label: ct.name } : { kind: 'channel' as const, label: c.name };
    }
    const k = kwants.find((x) => x.id === owner);
    return k ? { kind: 'bot' as const, label: k.name, sprite: k.avatar_sprite } : null;
  };
  /** Opens the chat a browser belongs to, with the panel open (the W2 `/live` address follows). */
  const watchOwner = (owner: string) => {
    const t = teams.find((x) => x.id === owner);
    const c = !t ? channels.find((x) => x.id === owner) : undefined;
    const k = !t && !c ? kwants.find((x) => x.id === owner) : undefined;
    if (t) handleSelectTeam(t);
    else if (c) handleSelectChannel(c);
    else if (k) handleSelectKwant(k);
    else return;
    setIsVmPaneOpen(true);
  };
  // W-L4: decide a `/live` link once the chats, the running turns and the seeded browsers are in.
  const liveHere = !!liveBrowsing[activeSandboxId];
  useEffect(() => {
    if (!liveGate || !loaded || !browsingSeeded) return;
    if (liveTargetKnown && liveHere) {
      setIsVmPaneOpen(true);
      setLiveGate(null);
    } else if (liveGate === 'pending') setLiveGate('miss');
  }, [liveGate, loaded, browsingSeeded, liveTargetKnown, liveHere]);
  // Opening any other chat (or a page) leaves the "isn't browsing now" page
  const missChatRef = useRef<string | null>(null);
  useEffect(() => {
    if (liveGate !== 'miss') return;
    if (missChatRef.current === null) missChatRef.current = selectedId;
    else if (missChatRef.current !== selectedId) setLiveGate(null);
  }, [liveGate, selectedId]);
  const switcherEntries = switchEntries(liveBrowsing, describeOwner, (o) => browserNeedsYou.has(o));
  const vmSwitcher = switcherEntries.length >= 2 ? { entries: switcherEntries, current: activeSandboxId, onPick: watchOwner } : null;

  // Sidebar: what busy bots are doing, "here" for the open chat.
  const busyKwants = busyLines({ turns: Object.values(turns), openCtx, awaiting: awaitingTurns, browser: browserTurns, describe: describeContext });
  const botStatus = botStatuses({ turns: Object.values(turns), awaiting: awaitingTurns, needsYou: needsYouByKwant });

  // Avatar moods and the background-tab signal come from the same state as the sidebar's dots.
  const needsYouNow = Object.entries(botStatus).filter(([, st]) => st === 'needs_you').map(([id]) => id);
  const needsYouKey = needsYouNow.join(',');
  useEffect(() => setMoodNeedsYou(needsYouKey ? needsYouKey.split(',') : []), [needsYouKey]);
  useEffect(() => {
    const apply = () => {
      const now = Date.now();
      setMoodResting(Object.fromEntries(kwants.map((k) => [k.id, restingUntil(k.preferred_model, profiles, now)])));
    };
    apply();
    const t = setInterval(apply, 30_000);
    return () => clearInterval(t);
  }, [kwants, profiles]);
  const tabSignal = useMemo(() => createTabSignal(browserTabDoc()), []);
  // The title counts what /api/away says waits (questions + approvals) or Pixel's bot count, whichever is
  // higher (browser steps are only in the latter); "Show nudges" off keeps the plain title.
  const tabNeedsYou = nudgesOn ? Math.max(needsYouCount(botStatus, pendingApprovals.length), awayWaiting) : 0;
  const anyRunning = Object.values(turns).some((t) => t.state === 'running');
  useEffect(() => {
    tabSignal.setNeedsYou(tabNeedsYou);
    // Something else (the sandbox viewer) may set the title later: put the count back in front.
    const el = document.querySelector('title');
    if (!el || typeof MutationObserver === 'undefined') return;
    const mo = new MutationObserver(() => tabSignal.setNeedsYou(tabNeedsYou));
    mo.observe(el, { childList: true, characterData: true, subtree: true });
    return () => mo.disconnect();
  }, [tabSignal, tabNeedsYou]);
  // While something waits the "!" favicon wins over the working dot; the app icon badge always shows the count.
  const nudgeIcon = useMemo(() => createNudgeIcon(browserIconDoc()), []);
  const markIcon = nudgesOn && awayWaiting > 0;
  useEffect(() => {
    void nudgeIcon.set(markIcon);
    void tabSignal.setWorking(anyRunning && !markIcon);
  }, [nudgeIcon, tabSignal, anyRunning, markIcon]);
  useEffect(() => {
    setAppBadgeCount(awayWaiting);
  }, [awayWaiting]);
  useEffect(() => {
    const on = (e: Event) => setNudgesOn(Boolean((e as CustomEvent<boolean>).detail));
    window.addEventListener(NUDGES_SETTING_EVENT, on);
    return () => window.removeEventListener(NUDGES_SETTING_EVENT, on);
  }, []);
  useEffect(() => {
    if (!nudgesOn) setPill(EMPTY_PILL);
  }, [nudgesOn]);
  const dismissPill = useCallback((rev: number) => setPill((p) => (p.rev === rev ? EMPTY_PILL : p)), []);
  const openPill = (items: Nudge[]) => {
    setPill(EMPTY_PILL);
    const n = items[0];
    if (items.length > 1 || !n) return showPage('inbox');
    if (n.away) return openAwayItem(n.away);
    openNotified({ kind: 'human_step', kwant_id: n.kwantId, channel_id: n.channelId ?? null });
  };
  const spriteOf = useCallback((id: string) => kwants.find((k) => k.id === id)?.avatar_sprite, [kwants]);
  // Avatars that only know a bot's title or name look its saved style up here.
  useEffect(() => setKnownBots(kwants), [kwants]);

  // What the empty chat shows: a bot's home, #general's, or nothing special for a team or other channel.
  const chatHome: ChatHome = currentKwant
    ? {
        kind: 'bot',
        bot: currentKwant,
        recent: recentWork(threadsByKwant[currentKwant.id]),
        onOpenRecent: (id: string) => openThread(currentKwant.id, id),
      }
    : { kind: isGeneral ? 'general' : currentTeam ? 'team' : 'channel' };

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
  const threadBarProps: ThreadBarProps | null = currentKwant
    ? {
        kwantName: currentKwant.name,
        threads: agentThreads,
        error: threadErrors[currentKwant.id] ?? null,
        openThreadId,
        turns,
        awaiting: awaitingTurns,
        directBadge: threadBadge({ id: currentKwant.id, delegation: null }, turns, awaitingTurns),
        showArchived,
        onToggleArchived: () => setShowArchived((v) => !v),
        onOpenDirect: openDirectChat,
        onOpenThread: (t) => openThread(t.kwant_id, t.id),
        onNewThread: handleNewThread,
        onRetry: () => loadThreads(currentKwant.id),
      }
    : null;
  // Phone: the conversations live in the header's second line (ThreadPicker), not in a bar of their own
  const subHeader = currentKwant ? (
    <>
      {!isPhone && threadBarProps && <ThreadBar {...threadBarProps} />}
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
  // §7.1: outside #general the banner is the card's one-liner; Open goes to #general and the card.
  const openGetStarted = () => {
    const general = channels.find((c) => c.id === space.general_channel_id) ?? channels.find((c) => c.name === 'general');
    if (general) handleSelectChannel(general);
    window.setTimeout(() => document.querySelector('[data-get-started]')?.scrollIntoView({ block: 'start', behavior: 'auto' }), 60);
  };
  const setupBanner =
    loaded && kwants.length > 0 && canRun === false && !gsDismissed ? (
      <GetStartedBanner space={space.id} engines={engines.engines} onOpen={openGetStarted} />
    ) : null;
  const getStartedEligible = showGetStarted({
    dismissed: gsDismissed,
    canRun,
    generalMessages: messages.length,
    teams: teams.length,
    started: getStartedWasStarted(space.id),
  });
  // New bots, teams and their team chats after an import (Space settings or Get started).
  const reloadAfterImport = () => {
    refreshRoster();
    fetch('/api/channels')
      .then((r) => (r.ok ? r.json() : null))
      .then((c) => Array.isArray(c) && setChannels(c))
      .catch(() => undefined);
  };
  const getStartedCard =
    loaded && (!openCtx || messagesLoadedFor === openCtx) && kwants.length > 0 && isGeneral && getStartedEligible ? (
      <GetStarted
        space={space.id}
        bots={kwants}
        engines={engines.engines}
        reloadEngines={engines.reload}
        onDismissed={() => undefined}
        spaceName={space.name}
        onImported={reloadAfterImport}
      />
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

  // H5: every bot in the one attention order. Not computed until the Inbox's first fetch lands, so the
  // pinned block never flashes empty-then-full. The open chat counts as read while it is open.
  const attention = awayDone
    ? attentionRows({
        kwants,
        needs: needsYouRows(awayItems, needsYouByKwant, new Set(kwants.map((k) => k.id))),
        failed: awayDone.failed,
        finished: awayDone.finished,
        busy: busyKwants,
        status: botStatus,
        h4: h4Status ?? undefined,
        openedAt: page === 'chat' && !isChannel && selectedId ? { ...openedAt, [selectedId]: Date.now() } : openedAt,
      })
    : undefined;

  // H3 + N3: the line under each held message of this space, keyed by the bubble's temp and saved ids.
  const tryAgainFor = (kwantId: string, name: string) => async (): Promise<string | void> => {
    haptic('tap');
    try {
      const out = await h4Api.retry(kwantId);
      if (out.status) setH4Status((prev) => ({ ...(prev ?? {}), [kwantId]: out.status! }));
      setHeldSends((prev) => prev.filter((h) => h.kwantId !== kwantId));
      pushToast(out.released > 0 ? `Sent to ${name} again.` : `${name} can take messages again.`);
    } catch {
      haptic('warning');
      return 'Couldn’t try again. Check your connection, then press it again.';
    }
  };
  const heldNotes: Record<string, React.ReactNode> = {};
  for (const h of heldSends) {
    const { text, tryAgain } = heldLine(h, h4Status?.[h.kwantId]);
    const node = <HeldLine key={h.messageId} text={text} onTryAgain={tryAgain ? tryAgainFor(h.kwantId, h.name) : undefined} />;
    heldNotes[h.tempId] = node;
    heldNotes[h.messageId] = node;
  }

  const sidebar = (section: 'all' | 'chats' | 'bots') => (
    <Sidebar
      space={space}
      spaces={spaces}
      onSwitchSpace={onSwitchSpace}
      onCreateSpace={onCreateSpace}
      onOpenSpaceSettings={() => openSpaceSettings('general')}
      onOpenArchived={onOpenArchived}
      onOpenSearch={() => setIsSearchOpen(true)}
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
      browsingOwners={globes.owners}
      browsingBots={globes.bots}
      browserNeedsYou={browserNeedsYou}
      onWatchBrowser={watchOwner}
      botStatus={botStatus}
      needsYou={needsYouByKwant}
      needsYouRows={isPhone && section === 'chats' ? needsYouRows(awayItems, needsYouByKwant, new Set(kwants.map((k) => k.id))) : undefined}
      attention={attention}
      onOpenNeedsYou={(row) => {
        // A question or approval opens its chat (or thread) on the card; a browser step, the bot's chat.
        if (row.item) openAwayItem(row.item);
        else {
          const k = kwants.find((x) => x.id === row.kwant_id);
          if (k) handleSelectKwant(k);
        }
        setPhoneView('chat');
      }}
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
      onCreateChannel={() => {
        setIsNewChannelOpen(true);
        playSfx('click');
      }}
      inboxCount={awayCount + prepushWaiting}
      inboxActive={page === 'inbox'}
      onOpenInbox={() => {
        showPage('inbox');
        playSfx('click');
      }}
      parallelActive={page === 'parallel'}
      onOpenParallel={() => {
        showPage('parallel');
        playSfx('click');
      }}
      missionBoard={missionBoard}
      missionsActive={page === 'board'}
      onOpenMissions={() => {
        showPage('board');
        playSfx('click');
      }}
      devDeskActive={page === 'dev'}
      onOpenDevDesk={() => {
        showPage('dev');
        playSfx('click');
      }}
      onConnectDevGitHub={connectDevGitHub}
      ideaJarCount={ideaJarCount}
      ideaJarActive={page === 'jar'}
      onOpenIdeaJar={() => {
        showPage('jar');
        playSfx('click');
      }}
      onDropIdea={async (text) => {
        await dropIdea(space.id, text);
        wobbleJar();
        announceJar(JAR_DROPPED);
      }}
    />
  );

  const away = (
    <AwayPage since={awaySinceRef.current} onSince={(t) => (awaySinceRef.current = t)} onOpen={openAwayItem} onCounts={setAwayCount} onApprovalResolved={dropApproval}
      top={
        <>
          <PrepushNeedsYou cards={prepush.cards} names={kwantNames} onOpen={openPrepush} />
          <SpendStopNeedsYou refreshKey={spendStopSeq} onOpenSettings={openUsageSettings} />
        </>
      }
    />
  );
  const backToList = isPhone ? () => { setPage('chat'); setPhoneView('list'); } : undefined;
  const parallel = <ParallelPage onBack={backToList} />;
  const connectionsPage = (
    <ConnectionsPage
      embedded
      onOpenTools={() => setIsMcpOpen(true)}
      onAddTool={() => { setMcpStartAdding(true); setIsMcpOpen(true); }}
      onOpenAccounts={openSettings}
      onOpenLooseEnds={() => showPage('inbox', 'loose')}
    />
  );
  const missionNames = Object.fromEntries(kwants.map((k) => [k.id, k.name]));
  const missionAvatars = Object.fromEntries(kwants.map((k) => [k.id, k.avatar_sprite ?? '']));
  const missionTurns = useMemo(() => Object.values(turns), [turns]);
  const missionRooms = useMemo(() => Object.fromEntries(channels.map((c) => [c.id, c.name])), [channels]);
  const missionThreadTitles = useMemo(
    () => Object.fromEntries(Object.values(threadsByKwant).flatMap((list) => (list ?? []).map((t) => [t.id, t.title]))),
    [threadsByKwant],
  );
  const missionsPage = (
    <MissionBoardPage
      key={space.id}
      spaceId={space.id}
      names={missionNames}
      avatars={missionAvatars}
      onBack={backToList}
      onOpenChat={(t, place) => {
        const c = place ?? sourceChat(t);
        if (!c) return;
        if (c.kwantId && c.threadId) return openThread(c.kwantId, c.threadId);
        if (!openTarget({ channel_id: c.channelId, kwant_id: c.kwantId })) pushToast('That chat no longer exists.');
      }}
      turns={missionTurns}
      teams={teams}
      rooms={missionRooms}
      threadTitles={missionThreadTitles}
      onOpenPlace={(c) => {
        if (c.kwantId && c.threadId) return openThread(c.kwantId, c.threadId);
        if (!openTarget({ channel_id: c.channelId, kwant_id: c.kwantId })) pushToast('That chat no longer exists.');
      }}
      openTaskId={pendingTaskId}
      onOpenedTaskId={() => setPendingTaskId(null)}
      openMissionId={pendingMissionId}
      onOpenedMissionId={() => setPendingMissionId(null)}
    />
  );
  const loose = (
    <LooseEndsPage
      key={`loose-${looseAddImap}`}
      onOpenChat={openThread}
      addImap={looseAddImap > 0}
    />
  );
  const inbox = <InboxPage tab={inboxTab} onTab={setInboxTab} count={awayCount + prepushWaiting} onBack={backToList} newPage={away} loosePage={loose} />;
  const chat = (
        <ChatView
          heldNotes={heldNotes}
          dropOff={!!spaceSettingsTab}
          title={chatTitle}
          subtitle={chatSubtitle}
          spaceName={space.name}
          onBack={isPhone ? phoneBackFromChat : undefined}
          messages={viewingHistory ? historyWindow.messages : messages}
          viewingHistory={viewingHistory}
          onJumpToLatest={() => setHistoryWindow(null)}
          streamingText={isCurrentContextStreaming ? streamingText : ''}
          streamingThinking={isCurrentContextStreaming ? streamingThinking : ''}
          pendingApprovals={approvalsHere}
          prepush={
            devOn && currentKwant && !viewingHistory ? (
              <PrepushThread cards={threadCards(prepush.cards, currentKwant.id)} bot={currentKwant.name} live={prepush.live} onRetry={prepushRetry} onOverride={prepushOverride} />
            ) : devOn && isChannel && !viewingHistory && pointerMemberIds.length > 0 ? (
              // 0.8.1 F3: team chats and channels get one pointer line per member bot; the card stays in the 1:1.
              <PrepushPointers
                cards={prepush.cards}
                memberIds={pointerMemberIds}
                names={kwantNames}
                onOpen={openPrepushFromTeam}
                onOpenInbox={() => showPage('inbox')}
                avatar={(id, name) => <ModernAvatar role={name} name={name} size={20} kwantId={id} />}
              />
            ) : undefined
          }
          settledApprovals={approvalsForChat(settledApprovals, openChat)}
          home={chatHome}
          isStreaming={isCurrentContextStreaming}
          streamingSpeakerName={live[openCtx] || undefined}
          onSendMessage={handleSendMessage}
          onReact={handleReact}
          onResolveApproval={handleResolveApproval}
          kwants={kwants}
          teams={teams}
          suggestions={chatSuggestions}
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
          onOpenSkills={(kwantId) => openSkillsRef.current(kwantId)}
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
          vmCaptchaStep={captchaStepFor(humanSteps, activeSandboxId)}
          vmHumanStep={openStepFor(humanSteps, activeSandboxId)}
          onWatchBrowser={liveBrowsing[activeSandboxId] ? () => setIsVmPaneOpen(true) : undefined}
          vmSwitcher={vmSwitcher}
          onVmStepDone={(id) => answerHumanStep(id, 'done')}
          vmSolveRequest={solveRequest}
          vmShowOnceBot={currentKwant && !currentTeam ? { id: currentKwant.id, name: currentKwant.name } : undefined}
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
          threadLine={isPhone && threadBarProps ? <ThreadPicker {...threadBarProps} /> : undefined}
          notice={setupBanner}
          topCard={
            isChannel && currentTeam ? (
              <TeamNeedsTop
                team={currentTeam}
                canRun={canRun !== false}
                personPosts={messages.filter((m) => m.sender_role === 'user').length}
                refreshKey={settingsClosedCount}
                onAddAccount={setupAction}
                onOpenNeed={(opens) => openTeamNeed(opens, currentTeam)}
              />
            ) : getStartedCard && messages.length > 0 ? (
              getStartedCard
            ) : undefined
          }
          welcome={getStartedCard && messages.length === 0 ? getStartedCard : undefined}
          onOpenAccounts={openSettings}
          onOpenUsage={openUsageSettings}
          onOpenComputerSettings={openComputerSettings}
          onChangeTeamFolder={(teamId) => {
            const t = teams.find((x) => x.id === teamId);
            if (!t) return;
            setEditingTeamFocusWs('folder');
            setEditingTeam(t);
          }}
          onEditBot={(name) => {
            const k = kwants.find((x) => x.name === name) ?? kwants.find((x) => x.name.toLowerCase() === name.toLowerCase());
            if (k) openAgentSettings(k, 'profile');
          }}
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
          queuedBeside={queuedBeside}
          renderDelegation={renderDelegation}
          switchNotes={switchNotes}
          renderQuestion={renderQuestion}
          renderMcpSuggestion={renderMcpSuggestion}
          renderConnectionSuggestion={renderConnectionSuggestion}
          renderBotAction={renderBotAction}
          renderHire={renderHire}
          renderRouteChoice={renderRouteChoice}
          rerouteTargets={openThreadId ? [] : rerouteTargetsFor(currentChannel, kwants)}
          onSendToOtherBot={openThreadId ? undefined : handleSendToOtherBot}
          awaitingAnswer={awaitingHere}
          awaitingBrowser={awaitingBrowserHere}
          liveActivity={isCurrentContextStreaming ? activityHere : null}
          renderHumanStep={renderHumanStep}
          contextKey={openCtx}
          deciding={decidingHere ? { botName: kwantName(decidingHere.kwantId) ?? chatTitle, consulted: decidingHere.consulted } : null}
          onChangeDecision={handleChangeDecision}
        />
  );
  // The view on screen, for the address bar; and views asked for from outside (the rail's gear,
  // Back and Forward). The board stays the view while its setting loads; the pane shows a chat if it's off.
  const currentView: SpaceView | null = spaceSettingsTab
    ? settingsScope === 'app'
      ? { kind: 'settings', scope: 'app' }
      : { kind: 'settings', scope: 'space', tab: spaceSettingsTab }
    : page === 'inbox'
      ? { kind: 'inbox', tab: inboxTab }
      : page === 'chat'
        ? null
        : { kind: page };
  const viewKey = currentView ? viewSegment(currentView) : '';
  const viewChangeRef = useRef(onViewChange);
  viewChangeRef.current = onViewChange;
  const currentViewRef = useRef(currentView);
  currentViewRef.current = currentView;
  useEffect(() => {
    viewChangeRef.current?.(currentViewRef.current);
  }, [viewKey]);
  const applyViewRef = useRef<(v: SpaceView | null) => void>(() => undefined);
  applyViewRef.current = (v) => {
    if (!v) {
      if (spaceSettingsTab) closeSpaceSettings();
      setPage('chat');
      return;
    }
    if (v.kind === 'settings') return v.scope === 'app' ? openAppSettings() : openSpaceSettings(v.tab);
    showPage(v.kind, v.kind === 'inbox' ? v.tab : 'new');
  };
  useEffect(() => {
    if (viewRequest) applyViewRef.current(viewRequest.view);
  }, [viewRequest?.seq]);

  const jarBots = useMemo(() => kwants.map((k) => ({ id: k.id, name: k.name, role: k.title || null })), [kwants]);
  const jarPage = (
    <IdeaJarView
      key={space.id}
      spaceId={space.id}
      spaceName={space.name}
      bots={jarBots}
      onOpenMission={(id) => {
        setPendingMissionId(id);
        showPage('board');
      }}
      onOpenSettings={() => {
        setSettingsFocusRow('idea-jar');
        openAppSettings();
      }}
    />
  );
  const attentionRankOf = new Map((attention ?? []).map((a) => [a.kwant.id, a.rank]));
  const devDeskPage = <DevDeskView key={space.id} spaceId={space.id} spaceName={space.name} onBack={backToList} onOpenLooseEnds={connectDevGitHub} teams={devTeamRefs(teams, kwants)} bots={kwants.map((k) => ({ id: k.id, name: k.name, role: k.title, rank: attentionRankOf.get(k.id) }))} defaultBotId={!isChannel ? selectedId : null} onOpenChat={(id, thread) => { if (thread) { openThread(id, thread); return; } if (!openTarget({ channel_id: null, kwant_id: id })) pushToast('That bot no longer exists.'); }} />;
  const mainPane = needsSetup ? setupChecklist : page === 'parallel' ? parallel : page === 'board' && missionBoard ? missionsPage : page === 'dev' && devOn ? devDeskPage : page === 'jar' ? jarPage : page === 'inbox' ? inbox : !loaded ? (
    <main className="flex-1 flex items-center justify-center text-xs text-zinc-500">
      <Loader2 size={14} className="mr-2 animate-spin motion-reduce:animate-none" aria-hidden="true" /> Opening {space.name}…
    </main>
  ) : liveGate === 'miss' ? (
    <LiveMissPage
      chatLabel={isChannel ? (currentTeam?.name ? `@${currentTeam.name}` : currentChannel?.name ? `#${currentChannel.name}` : '') : currentKwant?.name ? `@${currentKwant.name}` : ''}
      kind={isChannel ? 'place' : 'bot'}
      onOpenChat={() => setLiveGate(null)}
      onBack={isPhone ? () => { setLiveGate(null); setPhoneView('list'); } : undefined}
    />
  ) : liveGate === 'pending' ? (
    <main className="flex-1 flex items-center justify-center text-xs text-zinc-500">
      <Loader2 size={14} className="mr-2 animate-spin motion-reduce:animate-none" aria-hidden="true" /> Opening the live page…
    </main>
  ) : chat;

  const phoneTabs: Array<PhoneTabItem<PhoneTab>> = [
    { id: 'chats', label: 'Chats', icon: <MessageSquare size={20} aria-hidden="true" /> },
    { id: 'bots', label: 'Bots', icon: <Bot size={20} aria-hidden="true" /> },
    {
      id: 'jar',
      label: 'Jar',
      icon: (
        <span key={jarWobble} className={`inline-flex ${jarWobble ? 'kw-jar-wobble' : ''}`}>
          <JarIcon size={20} />
        </span>
      ),
      badge: ideaJarCount,
      badgeTone: 'violet',
      badgeNoun: ideaJarCount === 1 ? 'take ready' : 'takes ready',
    },
    { id: 'inbox', label: 'Inbox', icon: <Inbox size={20} aria-hidden="true" />, badge: awayCount + prepushWaiting },
    { id: 'space', label: 'Space', icon: <Settings size={20} aria-hidden="true" /> },
  ];
  const showPhoneList = isPhone && phoneView === 'list' && !needsSetup;
  // What's new after an update: only for someone who has used kwant (they've written here, or an account
  // works and Get started isn't showing). A brand-new person gets Get started instead (F-1); unknown until
  // the space and the open chat's messages have loaded.
  const wroteHere = messages.some((m) => m.sender_role === 'user');
  const whatsNewEstablished = decideWhatsNewEstablished({
    loaded,
    messagesLoaded: !openCtx || messagesLoadedFor === openCtx,
    canRun,
    wroteHere,
    getStartedShowing: getStartedEligible,
  });
  const readyBots = useMemo(() => kwants.map((k) => ({ id: k.id, name: k.name, role: k.avatar_sprite })), [kwants]);

  return (
    <ReadyTeamsProvider
      space={space.id}
      bots={readyBots}
      teams={teams}
      onChanged={refreshRoster}
      onOpenTeam={(t, channelId) => void openNewTeamChat(t, channelId)}
      onUndone={handleTeamUndone}
    >
    {/* overflow: clip, not just hidden: a hidden box can still be scrolled from script (scrollIntoView), which
        once slid the whole chat under its header (P1-7). Browsers without clip keep hidden. */}
    <div className="relative flex-1 min-w-0 h-full flex flex-col bg-surface-main text-zinc-100 overflow-hidden [overflow:clip]" data-space={space.id}>
      <div ref={contentRef} className={`flex-1 min-h-0 flex ${isPhone ? 'flex-col' : 'overflow-hidden'}`}>
        {isPhone ? (
          <>
            {showPhoneList || needsSetup ? (
              <div className="flex-1 min-h-0 flex flex-col">
                {needsSetup ? (
                  <>
                    {/* U-4: the Bots tab has "+ Team", "+ Bot" and the ready-teams box with zero bots, as on desk.
                        It gets the full height: split with the checklist, the box was cut in half. */}
                    {phoneTab === 'bots' ? (
                      sidebar('bots')
                    ) : (
                      <>
                        {sidebar('chats')}
                        {setupChecklist}
                      </>
                    )}
                  </>
                ) : phoneTab === 'jar' ? (
                  <div className="flex-1 min-h-0 flex" data-main-pane>{jarPage}</div>
                ) : (
                  sidebar(phoneTab === 'bots' ? 'bots' : 'chats')
                )}
              </div>
            ) : (
              <div className={`relative flex-1 min-h-0 flex ${devOn ? 'kw-blueprint' : ''}`} data-main-pane>
                {mainPane}
                <DropPillHost />
              </div>
            )}
            {(showPhoneList || needsSetup) && (
              <PhoneTabBar
                label={`${space.name} sections`}
                tabs={phoneTabs}
                active={phoneTab}
                onSelect={(id) => {
                  playSfx('click');
                  if (id === 'inbox') {
                    showPage('inbox');
                    return;
                  }
                  if (id === 'space') {
                    // Nothing can run yet: the fix is in Accounts, so start there
                    openSpaceSettings(canRun === false ? 'accounts' : 'general');
                    return;
                  }
                  if (id === 'jar') {
                    showPage('jar');
                    return;
                  }
                  if (page === 'jar') setPage('chat');
                  setPhoneTab(id);
                }}
              />
            )}
          </>
        ) : (
          <>
            {sidebar('all')}
            <div className={`relative flex-1 min-w-0 min-h-0 flex ${devOn ? 'kw-blueprint' : ''}`} data-main-pane>
              {mainPane}
              <DropPillHost />
            </div>
          </>
        )}
      </div>

      {/* New bot */}
      <Suspense fallback={null}>
        {/* Show it once: the review sheet sits over the Take over stage, outside it (design §S) */}
        <StepReview />

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
          onOpenTask={
            missionBoard
              ? (id) => {
                  setEditingKwant(null);
                  setPendingTaskId(id);
                  showPage('board');
                }
              : undefined
          }
          onShowOnce={
            editingKwant
              ? () => {
                  const k = editingKwant;
                  setEditingKwant(null);
                  openTarget({ kwant_id: k.id, channel_id: null });
                  setIsVmPaneOpen(true);
                }
              : undefined
          }
          browserUrl={editingKwant && editingKwant.id === activeSandboxId ? activeUrl : undefined}
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

      <NewChannelModal
        isOpen={isNewChannelOpen}
        onClose={() => setIsNewChannelOpen(false)}
        kwants={kwants}
        channels={channels}
        onCreate={handleCreateChannel}
      />

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
        focusWorkspace={editingTeamFocusWs}
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
          scope={settingsScope}
          onScope={setSettingsScope}
          onOpenArchived={onOpenArchived}
          connections={connectionsPage}
          appSettings={
            <Suspense
              fallback={
                <p className="flex items-center gap-2 text-xs text-zinc-400">
                  <Loader2 size={13} className="animate-spin motion-reduce:animate-none" /> Loading…
                </p>
              }
            >
              <SettingsModal
                embedded
                isOpen
                onClose={closeSpaceSettings}
                tokensUsed={tokensUsed}
                onOpenApiKeys={() => openSpaceSettings('accounts')}
                focusRow={settingsFocusRow}
              />
            </Suspense>
          }
          routineFocus={routineFocus}
          isPhone={isPhone}
          onApprovalsChanged={refreshRoster}
          onImported={reloadAfterImport}
        />
      )}

      {/* MCP tool servers (opened last so it sits above the bot form) */}
      <Suspense fallback={null}>
        <McpServersModal
          isOpen={isMcpOpen}
          startAdding={mcpStartAdding}
          onClose={() => { setIsMcpOpen(false); setMcpStartAdding(false); }}
          kwants={kwants}
          onOpenConnections={() => {
            setIsMcpOpen(false);
            setMcpStartAdding(false);
            openConnections();
          }}
        />
        {isSearchOpen && <SearchModal isOpen onClose={() => setIsSearchOpen(false)} kwants={kwants} onJump={handleJump} />}
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

      <StarAsk needsYou={Object.keys(needsYouByKwant).length > 0} waiting={awaitingTurns.size > 0 || pendingApprovals.length > 0} />
      <Toasts toasts={toasts} onDismiss={dismissToast} />
      <JiraSetupHost onSaved={() => pushToast('Taking tickets from Jira', 'Tickets you hand over show up on the Dev desk.')} />
      <GitHubConnectHost
        onConnected={(_space, source, test) => {
          const t = connectedToast(source, test);
          pushToast(t.title, t.body);
        }}
      />
      {ideaToast && (
        <IdeaDropToast
          key={ideaToast.id}
          state={ideaToast.state}
          onClose={closeIdeaToast}
          onReturnFocus={() => document.querySelector<HTMLElement>('[data-composer]')?.focus()}
          onOpenJar={page === 'jar' ? undefined : () => {
            setIdeaToast(null);
            showPage('jar');
            focusJarQuickAdd();
          }}
          onUndo={() => {
            if (ideaToast.state !== 'dropped') return;
            const id = ideaToast.id;
            setIdeaToast({ id, state: 'undoing' });
            deleteIdea(id, space.id).then(
              () => setIdeaToast((t) => (t?.id === id ? { id, state: 'undone' } : t)),
              () => setIdeaToast((t) => (t?.id === id ? { id, state: 'undo_failed' } : t)),
            );
          }}
        />
      )}
      <p role="status" aria-live="polite" className="sr-only" data-jar-live>
        {jarAnnouncement}
      </p>
      <NeedsYouPill items={pill.items} rev={pill.rev} spriteOf={spriteOf} onOpen={openPill} onDismiss={dismissPill} />
      <WhatsNewHost established={whatsNewEstablished} onOpen={(v) => applyViewRef.current(v)} />
    </div>
    </ReadyTeamsProvider>
  );
};
