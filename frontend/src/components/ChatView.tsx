import React, { useState, useRef, useEffect, useLayoutEffect, useMemo } from 'react';
import { agentSenderLabel } from '../agentConnections';
import { ArrowDown, ArrowUp, AtSign, Paperclip, Sparkles, ChevronDown, ChevronRight, Copy, Check, ChevronLeft, Users, Square, Settings2, Folder, GitBranch, Pencil, Pin, Monitor } from 'lucide-react';
import { ModernAvatar } from './ModernAvatar';
import { BotHome } from './BotHome';
import { ChatHome, answerTarget, goodAt, recipientsOf } from '../botHome';
import { moodNoticed } from '../kwantMood';
import { NewTag } from './NewTag';
import { PHONE_QUERY, TOUCH_QUERY } from '../layout';
import { useMedia } from '../useMedia';
import { composerStacked, keyboardMode, measureText16, phonePlaceholder } from '../composer';
import { markNewSeen } from '../whatsNew';
import { ModernApprovalCard, PendingApproval } from './ModernApprovalCard';
import { MentionPopover, MentionItem } from './MentionPopover';
import { MarkdownRenderer } from './MarkdownRenderer';
import { ToolCallsList } from './ToolCallsList';
import { VirtualMachinePane, TerminalLogEntry } from './VirtualMachinePane';
import { playSfx } from '../audio/chiptune';
import { groupDiscussionThreads, stripDiscussionMarkers } from '../discussionThreads';
import { PinnedInstructions, PinnedItem } from './Instructions';
import { DelegationMeta, parseDelegationMeta } from '../threads';
import { QuestionMeta, parseMetaObject, parseQuestionMeta } from '../questions';
import { fromTelegram, sentToTelegram } from '../telegram';
import { ResolvedViaTelegram, ViaTelegram } from './TelegramMarks';
import { HumanStep, HumanStepMeta, parseHumanStepMeta } from '../humanSteps';
import { LiveActivity, hasThinkingText, parseActivityMeta } from '../activity';
import { LiveActivityList, SavedActivityList } from './ActivityList';
import { FailoverNote, ReplyMeta } from './ReplyMeta';
import { assignSwitchNotes, liveSwitchNote, type SwitchNote } from '../failoverNotes';
import { savedFailoverHandoff, savedFailoverNotice } from '../failover';
import { SuggestionMeta, parseSuggestionMeta } from '../mcpSuggestions';
import { BotActionMeta, HireMeta, parseBotActionMeta, parseHireMeta } from '../botActions';
import {
  AttachmentRef,
  MAX_ATTACHMENTS,
  PendingUpload,
  UploadError,
  composerReady,
  filesFrom,
  isSharedFileMessage,
  parseAttachments,
  preflightFile,
  readyAttachments,
  uploadWithRetry,
  uploadRetryable,
  uploadSlotsLeft,
} from '../attachments';
import { FileTiles, UploadChip } from './Attachments';
import { ranOnLabel } from '../engines';
import { replyTokens } from '../modelUsage';
import { parseRoutingMeta, ranOnAccount, routingLine } from '../spaces';
import { SetupNeededCard, parseSetupMeta } from './SetupNeededCard';
import { ConnectionSuggestionMeta, parseConnectionMeta } from '../connectionSuggestions';
import { routeChipLabel, routeReasonText } from '../routeReason';
import { BudgetPausedNotice, HandoffCappedNotice, InterruptedNotice, SystemNotice } from './SystemNotice';
import { chainBudgetApi, parseBudgetPaused, parseHandoffCapped } from '../chainBudget';
import { parseSpendStopped } from '../spendStop';
import { SpendStoppedNotice } from './SpendStop';
import { isSystemNotice, parseInterruption, retryTargets } from '../interruptions';
import { IDEA_MAX, isIdeaCommand } from '../ideas';
import { ideaBlockedLine, ideaLength } from '../ideaShell';
import { JarIcon } from './JarIcon';
import { consultedNames, hideStreamingDecisions, messageDecisions, parseWorkAhead } from '../decideFirst';
import { approvalScrollAction, fullyVisible, isNearBottom, nextFollowing } from '../approvalVisibility';
import { DecidingLine, DecisionChips, WorkAheadBody } from './Proactive';
import { RouteChoiceMeta, parseRouteChoiceMeta, questionsOfReplies } from '../routeChoice';
import { SmartActionsSlot, actionClass } from './results/SmartActions';
import { WorkingStrip } from './WorkingStrip';
import { ScrollRow } from './ScrollRow';
import { liveBubble, waitingOn } from '../workingStrip';
import { COMPOSE_PREFILL_EVENT, withPrefill } from './results/SmartResult';
import { FilesPreviewOnly } from './results/Files';
import { ReactionButtons, ReplyButton, ReplyChip, ReplyQuote } from './MessageReactions';
import { ReactResult, Reaction, ReplyRef, botNameOf, excerptOf, questionMessageId, reactionOf, replyRefOf } from '../reactReply';

export interface ChatMessage {
  id: string;
  sender_role: 'user' | 'kwant' | 'system';
  sender_id: string;
  text: string;
  reasoning_content?: string | null;
  tool_calls?: string | null;
  tokens_used?: number;
  channel_id?: string | null;
  direct_kwant_id?: string | null;
  thread_id?: string | null;
  /** JSON string kwnt sets on messages it writes itself, e.g. delegation notes. */
  meta?: string | null;
  created_at?: string | null;
  /** Smart Results blocks for a bot reply; null or absent while the setting is off. */
  results?: unknown;
  /** The person's thumbs on a bot reply; absent on servers before reactions (read as none). */
  reaction?: Reaction | null;
  /** The message the person replied to; absent or null when it isn't a reply. */
  reply_to?: ReplyRef | null;
}

/** Extra details of a message being sent. */
export interface SendOptions {
  /** The bot message this one replies to. */
  replyTo?: ChatMessage;
}

/** The open chat's turn is waiting for a free slot. */
export interface QueuedState {
  position: number | null;
  reason: 'agent_busy' | 'server_busy';
  speaker: string;
}

/** "Drafter: Write" for a resolved approval row; the card's own words, nothing computed. */
function approvalLead(a: PendingApproval): string {
  const d = (a.details ?? {}) as Record<string, unknown>;
  const who = typeof d.kwant_name === 'string' && d.kwant_name ? d.kwant_name : null;
  const what = typeof d.summary === 'string' && d.summary ? d.summary : a.tool_name;
  return who ? `${who}: ${what}` : what;
}

export interface DiscussionState {
  inProgress: boolean;
  teamId?: string;
  contextId?: string;
  teamName?: string;
  currentSpeaker?: string;
  currentTurn?: number;
  totalTurns?: number;
  /** 'handoff' when kwants pulled each other in with @mentions. */
  kind?: string;
  /** Who handed the current speaker the turn. */
  askedBy?: string;
  /** Team discussion phase of the current turn: 'open', 'reply' or 'summary'. */
  phase?: string;
}

export interface RoutingCandidate {
  id: string;
  name: string;
  title: string;
}

export interface RoutingState {
  phase: 'deciding' | 'decided';
  contextId?: string;
  candidates: RoutingCandidate[];
  kwantId?: string;
  kwantName?: string;
  kwantTitle?: string;
  reason?: string;
  mode?: 'direct' | 'discuss' | string;
  /** How many bots Laya chose from, and whether keyword matching stood in for her. */
  candidateCount?: number | null;
  fallback?: boolean;
  matchedTerms?: string[];
  /** How the bot was picked: by role, the team lead as fallback, keywords, or the only choice. */
  basis?: string;
}

export interface ChatViewProps {
  title: string;
  subtitle: string;
  /** The open space, for "Message #general in Work" and Laya's lines. */
  spaceName?: string;
  /** Phones: back to the list. */
  onBack?: () => void;
  messages: ChatMessage[];
  streamingText: string;
  streamingThinking: string;
  /** Tool approvals waiting in this chat, oldest first. */
  pendingApprovals: PendingApproval[];
  /** Approvals answered in Telegram while this chat was open (G4): kept as a resolved row. */
  settledApprovals?: { approval: PendingApproval; at: string }[];
  isStreaming: boolean;
  streamingSpeakerName?: string;
  /** `attachments` are files already uploaded from the composer. */
  /** Resolves to false when the message couldn't be sent. */
  /** A string back is a `/idea` that didn't go in: the composer gets the text back with that line. */
  onSendMessage: (text: string, attachments: AttachmentRef[], opts?: SendOptions) => void | Promise<boolean | string | void>;
  /** Saves the person's thumbs on a bot reply; absent hides the thumbs. */
  onReact?: (msg: ChatMessage, reaction: Reaction | null) => Promise<ReactResult> | void;
  onResolveApproval: (id: string, decision: 'allow_once' | 'always_allow' | 'deny') => void;
  kwants?: Array<{ id: string; name: string; title: string; avatar_sprite: string }>;
  teams?: Array<{ id: string; name: string; description?: string; member_kwants?: string[] }>;
  /** Prompts offered above an empty chat, made from who is in it; none hides the bar. */
  suggestions?: string[];
  /** What kind of chat this is, for the empty-chat home and the composer's "who answers" pill. */
  home?: ChatHome;
  discussionState?: DiscussionState;
  routingState?: RoutingState | null;
  onCancelDiscussion?: () => void;
  teamMembers?: Array<{ id: string; name: string; title: string; avatar_sprite: string }>;
  teamId?: string;
  teamName?: string;
  workspacePath?: string;
  repoUrl?: string;
  onManageTeam?: () => void;
  onEditKwant?: () => void;
  // Grok-style Sandbox VM Pane props
  isVmPaneOpen?: boolean;
  onToggleVmPane?: () => void;
  activeUrl?: string;
  pageTitle?: string;
  screenshotBase64?: string | null;
  isRunning?: boolean;
  browserError?: string | null;
  terminalLogs?: TerminalLogEntry[];
  onRefreshVm?: () => void;
  onRestartVm?: () => void;
  onNavigateVm?: (url: string) => void;
  /** Some bot in the space runs without asking (the saved-passwords note). */
  vmApprovalsOff?: boolean;
  /** The CAPTCHA the open bot's browser waits on, and a counter that opens it to solve. */
  vmCaptchaStep?: HumanStep | null;
  /** The newest open step (any kind) holding this browser. */
  vmHumanStep?: HumanStep | null;
  /** Answers that step as done from the browser; resolves to an error message on failure. */
  onVmStepDone?: (id: string) => Promise<string | void> | void;
  vmSolveRequest?: number;
  /** Pinned team/channel instructions, shown collapsed above the messages. */
  pinned?: PinnedItem[];
  /** Opens the channel's instructions editor; only for channels. */
  onEditInstructions?: () => void;
  /** A message to scroll to and flash (from search). */
  highlightMessageId?: string | null;
  onHighlightDone?: () => void;
  /** Set while `messages` is an older window (a search hit beyond the latest page). */
  viewingHistory?: boolean;
  /** Leaves that window for the latest messages. */
  onJumpToLatest?: () => void;
  /** Under the header: the bot's conversations and the open thread's controls. */
  subHeader?: React.ReactNode;
  /** Phone only: the header's second line in place of the subtitle (the thread picker in a bot's chat). */
  threadLine?: React.ReactNode;
  /** A note under the sub-header (the setup banner); hidden while `welcome` fills an empty chat. */
  notice?: React.ReactNode;
  /** Drawn in place of an empty chat (the first-run welcome). */
  welcome?: React.ReactNode;
  /** Next to the title, e.g. "Working on …" while the bot has background turns. */
  titleExtra?: React.ReactNode;
  /** Set while the open chat's turn waits in the queue. */
  queued?: QueuedState | null;
  /** Draws a delegation note or result (messages with delegation `meta`). */
  renderDelegation?: (msg: ChatMessage, meta: DelegationMeta, flash: boolean) => React.ReactNode;
  /** Draws a bot's question card (messages with `ask_user` meta). */
  renderQuestion?: (msg: ChatMessage, meta: QuestionMeta, flash: boolean) => React.ReactNode;
  /** Draws a bot's tool suggestion card (messages with `mcp_suggestion` meta). */
  renderMcpSuggestion?: (msg: ChatMessage, meta: SuggestionMeta, flash: boolean) => React.ReactNode;
  /** Draws a bot's offer to connect an app (message meta kind "connection_suggestion"). */
  renderConnectionSuggestion?: (msg: ChatMessage, meta: ConnectionSuggestionMeta, flash: boolean) => React.ReactNode;
  /** A change a bot made in the app (message meta kind "bot_action"). */
  renderBotAction?: (msg: ChatMessage, meta: BotActionMeta, flash: boolean) => React.ReactNode;
  /** A bot's proposal to hire a new bot (message meta kind "hire_proposal"). */
  renderHire?: (msg: ChatMessage, meta: HireMeta, flash: boolean) => React.ReactNode;
  /** Laya's "Who should answer this?" card (message meta kind "route_choice"); `question` is the
   *  person's message it is about, when it's loaded. */
  renderRouteChoice?: (msg: ChatMessage, meta: RouteChoiceMeta, flash: boolean, question?: string) => React.ReactNode;
  /** Bots a routed reply can be sent to instead ("Wrong bot? Send to…"); empty hides it. */
  rerouteTargets?: Array<{ id: string; name: string; title: string }>;
  /** Teaches routing that `question` should have gone to `kwantId` and asks that bot now.
   *  Resolves to an error message on failure. */
  onSendToOtherBot?: (question: ChatMessage, kwantId: string) => Promise<string | void>;
  /** Opens Space settings → Accounts; turns a "nothing can run" reply into a card with a button. */
  onOpenAccounts?: () => void;
  /** Opens Settings › All spaces › Usage at the daily spending stop. */
  onOpenUsage?: () => void;
  /** Whether anything can run a bot in this space now (null while unknown). */
  canRun?: boolean | null;
  /** The open chat's turn is paused on a question: it is waiting for the person, not working. */
  awaitingAnswer?: boolean;
  /** …and the pause is a browser step (sign-in, CAPTCHA) rather than a question. */
  awaitingBrowser?: boolean;
  /** The running reply's steps so far (thinking, tools, waits). */
  liveActivity?: LiveActivity | null;
  /** Draws a bot's browser-step card (message meta kind "human_step"). */
  renderHumanStep?: (msg: ChatMessage, meta: HumanStepMeta, flash: boolean) => React.ReactNode;
  /** Changes when another chat opens, so the view jumps to its bottom. */
  contextKey?: string;
  /** Set while a held reply in this chat is being decided ("Team is deciding…"). */
  deciding?: { botName: string; consulted: string[] } | null;
  /** Change on a decision chip; resolves to an error message on failure. */
  onChangeDecision?: (msg: ChatMessage, index: number, picked: string) => Promise<string | void>;
  /** Quiet "answered on another account" lines, one per outage; each shows under that bot's name. */
  switchNotes?: SwitchNote[];
}

export const renderFormattedText = (text: string): React.ReactNode => {
  if (!text) return null;
  const parts = text.split(/(@[a-zA-Z0-9_-]+)/g);
  return parts.map((part, index) => {
    if (part.startsWith('@') && /^@[a-zA-Z0-9_-]+$/.test(part)) {
      return (
        <span
          key={index}
          className="inline-flex items-center px-1.5 py-0.5 rounded text-sky-400 bg-sky-500/10 font-medium text-[11px] border border-sky-500/20"
        >
          {part}
        </span>
      );
    }
    return part;
  });
};

const LayaRoutingCard: React.FC<{ routing: RoutingState; spaceName?: string }> = ({ routing, spaceName }) => {
  const deciding = routing.phase === 'deciding';
  const isDiscuss = routing.mode === 'discuss';
  const why = routeReasonText(routing);
  const pool = routing.candidateCount ?? (routing.candidates.length || null);
  const where = spaceName ? ` in ${spaceName}` : '';

  return (
    <div className="flex items-start gap-3.5 w-full max-w-5xl" aria-live="polite">
      <div className="w-[34px] h-[34px] flex-shrink-0 rounded-full bg-violet-500/15 border border-violet-500/40 flex items-center justify-center shadow-sm shadow-violet-500/10">
        <Sparkles size={16} className={`text-violet-300 ${deciding ? 'animate-pulse' : ''}`} />
      </div>
      <div className="flex flex-col items-start min-w-0 flex-1 max-w-4xl">
        <div className="flex items-center gap-2 mb-1.5 text-[11px] text-zinc-400 font-medium">
          <span className="text-zinc-200">Laya</span>
          <span className="text-zinc-500">· router</span>
        </div>
        <div className="px-4 py-3 rounded-2xl rounded-tl-sm bg-violet-950/20 border border-violet-500/20 text-sm text-zinc-200 shadow-sm w-full">
          {deciding ? (
            <div className="flex items-center gap-2">
              <span>
                {pool ? `Choosing from ${pool} ${pool === 1 ? 'bot' : 'bots'}${where}` : `Choosing who's best to answer${where}`}
              </span>
              <span className="inline-flex gap-0.5" aria-hidden="true">
                {[0, 1, 2].map((i) => (
                  <span
                    key={i}
                    className="w-1 h-1 rounded-full bg-violet-300 animate-bounce"
                    style={{ animationDelay: `${i * 150}ms` }}
                  />
                ))}
              </span>
            </div>
          ) : (
            <div className="flex flex-col gap-0.5">
              <div className="flex items-center gap-1.5 flex-wrap">
                <span>{isDiscuss ? 'Starting a team discussion led by' : 'Sending your question to'}</span>
                <span className="font-semibold text-violet-200">{routing.kwantName}</span>
                {routing.kwantTitle && <span className="text-zinc-400 text-xs">({routing.kwantTitle})</span>}
              </div>
              {why && <span className="text-xs text-zinc-400">{why}</span>}
              {routing.fallback && <span className="text-xs text-amber-300/90">Picked by keywords (Laya is offline)</span>}
            </div>
          )}

          {routing.candidates.length > 0 && (
            <div className="mt-2.5 flex items-center gap-1.5 flex-wrap">
              {routing.candidates.map((c, i) => {
                const chosen = !deciding && c.id === routing.kwantId;
                return (
                  <div
                    key={c.id}
                    title={`${c.name} — ${c.title}`}
                    className={`flex items-center gap-1.5 pl-0.5 pr-2.5 py-0.5 rounded-full border text-[11px] transition-all duration-300 ${
                      chosen
                        ? 'border-violet-400/60 bg-violet-500/20 text-violet-100 scale-105'
                        : deciding
                          ? 'border-zinc-700/60 bg-zinc-900/60 text-zinc-300 animate-pulse'
                          : 'border-zinc-800 bg-zinc-900/40 text-zinc-500 opacity-50'
                    }`}
                    style={deciding ? { animationDelay: `${i * 200}ms` } : undefined}
                  >
                    <ModernAvatar role={c.name} name={c.name} size={18} />
                    <span>{c.name}</span>
                    {chosen && <Check size={11} className="text-violet-300" />}
                  </div>
                );
              })}
            </div>
          )}
        </div>
      </div>
    </div>
  );
};

/** True below Tailwind's sm breakpoint, kept in sync with resizes. */
const useNarrow = () => {
  const query = '(max-width: 639px)';
  const [narrow, setNarrow] = useState(() => typeof window !== 'undefined' && !!window.matchMedia?.(query).matches);
  useEffect(() => {
    const mq = window.matchMedia?.(query);
    if (!mq) return;
    const on = () => setNarrow(mq.matches);
    mq.addEventListener('change', on);
    return () => mq.removeEventListener('change', on);
  }, []);
  return narrow;
};

export const ChatView: React.FC<ChatViewProps> = ({
  title,
  subtitle,
  spaceName,
  onBack,
  messages,
  streamingText,
  streamingThinking,
  pendingApprovals,
  settledApprovals = [],
  isStreaming,
  streamingSpeakerName,
  onSendMessage,
  onReact,
  onResolveApproval,
  kwants: kwantsProp,
  teams: teamsProp,
  suggestions = [],
  home,
  discussionState,
  routingState,
  onCancelDiscussion,
  teamMembers,
  teamId,
  teamName,
  workspacePath,
  repoUrl,
  onManageTeam,
  onEditKwant,
  isVmPaneOpen,
  onToggleVmPane,
  activeUrl,
  pageTitle,
  screenshotBase64,
  isRunning = true,
  browserError,
  terminalLogs = [],
  onRefreshVm,
  onRestartVm,
  onNavigateVm,
  vmApprovalsOff,
  vmCaptchaStep,
  vmHumanStep,
  onVmStepDone,
  vmSolveRequest,
  pinned = [],
  onEditInstructions,
  highlightMessageId,
  onHighlightDone,
  viewingHistory,
  onJumpToLatest,
  subHeader,
  threadLine,
  notice,
  welcome,
  titleExtra,
  queued,
  renderDelegation,
  renderQuestion,
  switchNotes,
  renderMcpSuggestion,
  renderConnectionSuggestion,
  renderBotAction,
  renderHire,
  renderRouteChoice,
  rerouteTargets = [],
  onSendToOtherBot,
  onOpenAccounts,
  onOpenUsage,
  canRun = null,
  awaitingAnswer,
  awaitingBrowser,
  liveActivity,
  renderHumanStep,
  contextKey,
  deciding,
  onChangeDecision,
}) => {
  const narrow = useNarrow();
  const [internalIsVmPaneOpen, setInternalIsVmPaneOpen] = useState(false);
  const vmOpen = isVmPaneOpen !== undefined ? isVmPaneOpen : internalIsVmPaneOpen;
  const toggleVm = onToggleVmPane || (() => setInternalIsVmPaneOpen((prev) => !prev));

  const [inputVal, setInputVal] = useState('');
  // Phone (spec A1): keyboard mode while the composer has focus on a touch screen; the composer is two
  // rows while in use. Desktop never changes.
  const phone = useMedia(PHONE_QUERY);
  const touch = useMedia(TOUCH_QUERY);
  const [composerFocused, setComposerFocused] = useState(false);
  const kbd = keyboardMode({ phone, touch, focused: composerFocused });
  const [fieldWidth, setFieldWidth] = useState(0);
  const [uploads, setUploads] = useState<PendingUpload[]>([]);
  const stacked = composerStacked({ phone, focused: composerFocused, hasContent: inputVal.length > 0 || uploads.length > 0 });
  const [uploadNote, setUploadNote] = useState<string | null>(null);
  const [dragOver, setDragOver] = useState(false);
  const uploadFiles = useRef(new Map<string, { file: File; abort: AbortController }>());
  const fileInputRef = useRef<HTMLInputElement>(null);
  const dragDepth = useRef(0);
  const [openThoughts, setOpenThoughts] = useState<Record<string, boolean>>({});
  const [copiedId, setCopiedId] = useState<string | null>(null);
  const scrollRef = useRef<HTMLDivElement>(null);
  const textareaRef = useRef<HTMLTextAreaElement>(null);

  const handleInsertTag = (tag: string) => {
    playSfx('click');
    setInputVal((prev) => {
      const trimmed = prev.trim();
      return trimmed ? `${trimmed} ${tag} ` : `${tag} `;
    });
    if (textareaRef.current) {
      textareaRef.current.focus();
    }
  };

  // Fallback to fetch kwants and teams if not provided as props
  const [internalKwants, setInternalKwants] = useState<Array<{ id: string; name: string; title: string; avatar_sprite: string }>>([]);
  const [internalTeams, setInternalTeams] = useState<Array<{ id: string; name: string; description?: string; member_kwants?: string[] }>>([]);

  useEffect(() => {
    if (!kwantsProp) {
      fetch('/api/kwants')
        .then((r) => (r.ok ? r.json() : []))
        .then((data) => setInternalKwants(data))
        .catch(() => {});
    }
  }, [kwantsProp]);

  useEffect(() => {
    if (!teamsProp) {
      fetch('/api/teams')
        .then((r) => (r.ok ? r.json() : []))
        .then((data) => setInternalTeams(data))
        .catch(() => {});
    }
  }, [teamsProp]);

  const activeKwants = kwantsProp || internalKwants;
  // Each routed reply's question: the person's latest saved message before it ("Wrong bot?").
  const questionOf = useMemo(() => questionsOfReplies(messages), [messages]);
  const activeTeams = teamsProp || internalTeams;

  const mentionItems: MentionItem[] = [
    ...activeKwants.map((k) => ({
      type: 'kwant' as const,
      id: k.id,
      name: k.name,
      title: k.title,
      goodAt: goodAt(k),
      avatar_sprite: k.avatar_sprite,
    })),
    ...activeTeams.map((t) => ({
      type: 'team' as const,
      id: t.id,
      name: t.name,
      title: t.description || 'Team',
      memberCount: t.member_kwants?.length || 0,
    })),
  ];

  const [mentionQuery, setMentionQuery] = useState<string | null>(null);
  const [mentionStartIndex, setMentionStartIndex] = useState<number>(-1);
  const [selectedIndex, setSelectedIndex] = useState(0);

  const checkMention = (text: string, cursorPos: number | null) => {
    if (cursorPos === null || cursorPos === undefined) {
      setMentionQuery(null);
      setMentionStartIndex(-1);
      return;
    }
    const textBeforeCursor = text.slice(0, cursorPos);
    const match = textBeforeCursor.match(/(?:^|\s)@([a-zA-Z0-9_-]*)$/);
    if (match) {
      const query = match[1];
      const atIndex = textBeforeCursor.lastIndexOf('@');
      setMentionQuery(query);
      setMentionStartIndex(atIndex);
    } else {
      setMentionQuery(null);
      setMentionStartIndex(-1);
    }
  };

  const matchingItems = mentionQuery !== null
    ? mentionItems.filter((item) => {
        const q = mentionQuery.toLowerCase();
        return (
          item.name.toLowerCase().includes(q) ||
          (item.title && item.title.toLowerCase().includes(q)) ||
          (item.goodAt?.some((g) => g.toLowerCase().includes(q)) ?? false)
        );
      })
    : [];

  const isMentionOpen = mentionQuery !== null && matchingItems.length > 0;
  const safeSelectedIndex = Math.min(
    selectedIndex,
    Math.max(0, matchingItems.length - 1)
  );

  useEffect(() => {
    setSelectedIndex(0);
  }, [mentionQuery]);
  // The picker opening by any route (typing "@" or the button) counts as using the new mention: "New" goes.
  useEffect(() => {
    if (isMentionOpen) markNewSeen('mention');
  }, [isMentionOpen]);

  const handleSelectMention = (item: MentionItem) => {
    if (mentionStartIndex < 0) return;
    const cursorPos = textareaRef.current?.selectionStart ?? inputVal.length;
    const textBefore = inputVal.slice(0, mentionStartIndex);
    const textAfter = inputVal.slice(cursorPos);
    markNewSeen('mention');
    const mentionText = `@${item.name} `;
    const newVal = `${textBefore}${mentionText}${textAfter}`;
    setInputVal(newVal);
    setMentionQuery(null);
    setMentionStartIndex(-1);

    const nextCursor = mentionStartIndex + mentionText.length;
    setTimeout(() => {
      if (textareaRef.current) {
        textareaRef.current.focus();
        textareaRef.current.setSelectionRange(nextCursor, nextCursor);
        textareaRef.current.style.height = 'auto';
        textareaRef.current.style.height = `${Math.min(textareaRef.current.scrollHeight, 160)}px`;
      }
    }, 0);
  };

  // Follow new tokens only while the reader is at the bottom; scrolling up to read stays put.
  const stickRef = useRef(true);
  const lastScrollTopRef = useRef(0);
  const pinToBottom = (el: HTMLDivElement) => {
    el.scrollTop = el.scrollHeight;
    lastScrollTopRef.current = el.scrollTop;
  };
  // A waiting card whose buttons are off screen while the person reads further up.
  const [approvalOffscreen, setApprovalOffscreen] = useState(false);
  const approvalKey = pendingApprovals.map((a) => a.id).join(',');
  const checkApprovalVisible = () => {
    const el = scrollRef.current;
    if (!el) return;
    const view = el.getBoundingClientRect();
    const rows = Array.from(el.querySelectorAll<HTMLElement>('[data-approval-actions]'));
    const action = approvalScrollAction({
      waiting: rows.length,
      following: stickRef.current,
      buttonsVisible: rows.some((r) => fullyVisible(r.getBoundingClientRect(), view)),
    });
    setApprovalOffscreen(action === 'notify');
  };
  const onScroll = () => {
    const el = scrollRef.current;
    if (el) {
      stickRef.current = nextFollowing(stickRef.current, { nearBottom: isNearBottom(el), scrollTop: el.scrollTop, lastScrollTop: lastScrollTopRef.current });
      lastScrollTopRef.current = el.scrollTop;
    }
    checkApprovalVisible();
  };
  // Phone (0.6.13 spec A3/C2): the working strip is the one live line, so the stream drops what repeats it.
  // (`phone` is the A1 PHONE_QUERY flag above).
  const stripWaitsOn = isStreaming ? waitingOn({ live: liveActivity, queued: !!queued, awaitingAnswer: !!awaitingAnswer, awaitingBrowser: !!awaitingBrowser }) : null;
  // What the live bubble holds before its first words: a pending approval reads as "waiting for your OK", not loading.
  const bubble = liveBubble({ live: liveActivity, queued: !!queued, awaitingAnswer: !!awaitingAnswer, awaitingBrowser: !!awaitingBrowser, writing: !!streamingText });
  // The strip's "Answer ↑": the open question card, else the approval card, fully in view.
  const showWaitingCard = (on: 'question' | 'approval') => {
    if (on === 'approval') return showApproval();
    const cards = scrollRef.current?.querySelectorAll<HTMLElement>('[data-question-card="open"]');
    const card = cards?.[cards.length - 1];
    if (!card) return;
    const reduce = window.matchMedia?.('(prefers-reduced-motion: reduce)').matches;
    card.scrollIntoView({ block: 'nearest', behavior: reduce ? 'auto' : 'smooth' });
  };
  const showApproval = () => {
    const cards = scrollRef.current?.querySelectorAll<HTMLElement>('[data-approval-card]');
    const card = cards?.[cards.length - 1];
    if (!card) return;
    const reduce = window.matchMedia?.('(prefers-reduced-motion: reduce)').matches;
    card.scrollIntoView({ block: 'end', behavior: reduce ? 'auto' : 'smooth' });
    setApprovalOffscreen(false);
  };
  // Cards are last in the stream. While the person follows the chat, keep the bottom pinned when a
  // card grows after it appears (its details arrive from the server) or the composer grows and
  // shrinks the stream, so Deny / Allow never sit below the composer.
  useEffect(() => {
    const el = scrollRef.current;
    if (!el || typeof ResizeObserver === 'undefined') return;
    const ro = new ResizeObserver(() => {
      if (stickRef.current) pinToBottom(el);
      checkApprovalVisible();
    });
    ro.observe(el);
    el.querySelectorAll('[data-approval-card]').forEach((c) => ro.observe(c));
    return () => ro.disconnect();
  }, [approvalKey]);
  useEffect(() => {
    stickRef.current = true;
  }, [contextKey]);
  // Phone: the field's width picks the placeholder (A1), and its height follows when it moves between
  // one row and two. Keyboard mode keeps the last message in view while the chat follows the bottom.
  useEffect(() => {
    const el = textareaRef.current;
    if (!el || !phone) return;
    const measure = () => setFieldWidth(el.clientWidth);
    measure();
    if (typeof ResizeObserver === 'undefined') return;
    const ro = new ResizeObserver(measure);
    ro.observe(el);
    return () => ro.disconnect();
  }, [phone]);
  useLayoutEffect(() => {
    const el = textareaRef.current;
    if (el && phone && el.value) {
      el.style.height = 'auto';
      el.style.height = `${Math.min(el.scrollHeight, 160)}px`;
    }
    if (scrollRef.current && stickRef.current) pinToBottom(scrollRef.current);
  }, [kbd, stacked]);
  // Back from an older window: land on the newest message
  useEffect(() => {
    if (!viewingHistory) stickRef.current = true;
  }, [viewingHistory]);
  useEffect(() => {
    if (scrollRef.current && stickRef.current) pinToBottom(scrollRef.current);
    checkApprovalVisible();
  }, [messages, streamingText, streamingThinking, approvalKey, routingState, queued, contextKey]);

  // Jump to a search hit: open its folded discussion if needed, then centre and flash it.
  // Runs after the auto-scroll above, so it wins.
  const [flashId, setFlashId] = useState<string | null>(null);
  // A search hit glows amber for 2.4 s; the original of a reply quote gets a violet ring for 1.6 s.
  const [flashKind, setFlashKind] = useState<'search' | 'quote'>('search');
  useEffect(() => {
    if (!highlightMessageId || !messages.some((m) => m.id === highlightMessageId)) return;
    const thread = groupDiscussionThreads(messages).find(
      (item) => item.kind === 'thread' && item.turns.some((t) => t.id === highlightMessageId),
    );
    if (thread && thread.kind === 'thread' && !openThoughts[`thread:${thread.id}`]) {
      setOpenThoughts((prev) => ({ ...prev, [`thread:${thread.id}`]: true }));
      return;
    }
    const el = scrollRef.current?.querySelector<HTMLElement>(`[data-message-id="${CSS.escape(highlightMessageId)}"]`);
    if (!el) return;
    const reduce = window.matchMedia?.('(prefers-reduced-motion: reduce)').matches;
    el.scrollIntoView({ block: 'center', behavior: reduce ? 'auto' : 'smooth' });
    setFlashKind('search');
    setFlashId(highlightMessageId);
    onHighlightDone?.();
  }, [highlightMessageId, messages, openThoughts, onHighlightDone]);
  useEffect(() => {
    if (!flashId) return;
    const t = setTimeout(() => setFlashId(null), flashKind === 'quote' ? 1600 : 2400);
    return () => clearTimeout(t);
  }, [flashId, flashKind]);

  // Reply: the bot message the next send answers. Cleared on send and when the chat changes.
  const [replyTo, setReplyTo] = useState<ChatMessage | null>(null);
  useEffect(() => {
    setReplyTo(null);
  }, [contextKey]);
  const focusComposer = () => {
    const ta = textareaRef.current;
    if (!ta) return;
    ta.focus();
    ta.setSelectionRange(ta.value.length, ta.value.length);
  };
  const startReply = (msg: ChatMessage) => {
    setReplyTo(msg);
    focusComposer();
  };
  /** The chip's name: the bot by id or name, else this chat's title. */
  const replyBotName = (msg: ChatMessage) => {
    const name = botNameOf(msg.sender_id, activeKwants);
    return name === 'a bot' ? title : name;
  };
  const cancelReply = () => {
    setReplyTo(null);
    focusComposer();
  };
  /**
   * Jumps to a quote's original: opens its folded discussion if needed, centres it, moves focus there
   * and rings it. False when it isn't loaded in this chat.
   */
  const [quoteJump, setQuoteJump] = useState<{ id: string; n: number } | null>(null);
  const showOriginal = (id: string): boolean => {
    if (!messages.some((m) => m.id === id)) return false;
    stickRef.current = false;
    setQuoteJump((p) => ({ id, n: (p?.n ?? 0) + 1 }));
    return true;
  };
  useEffect(() => {
    if (!quoteJump) return;
    const thread = groupDiscussionThreads(messages).find((item) => item.kind === 'thread' && item.turns.some((t) => t.id === quoteJump.id));
    if (thread && thread.kind === 'thread' && !openThoughts[`thread:${thread.id}`]) {
      setOpenThoughts((prev) => ({ ...prev, [`thread:${thread.id}`]: true }));
      return;
    }
    const el = scrollRef.current?.querySelector<HTMLElement>(`[data-message-id="${CSS.escape(quoteJump.id)}"]`);
    setQuoteJump(null);
    if (!el) return;
    const reduce = window.matchMedia?.('(prefers-reduced-motion: reduce)').matches;
    el.scrollIntoView({ block: 'center', behavior: reduce ? 'auto' : 'smooth' });
    if (el.tabIndex < 0 || el.hasAttribute('tabindex')) el.focus({ preventScroll: true });
    setFlashId(null);
    setFlashKind('quote');
    requestAnimationFrame(() => setFlashId(quoteJump.id));
  }, [quoteJump, messages, openThoughts]);

  // Thumbs on the bot's newest question are a yes or no; once one has answered it, they're feedback again.
  const [answered, setAnswered] = useState<ReadonlySet<string>>(() => new Set());
  const questionId = questionMessageId(messages, isStreaming);
  const react = async (msg: ChatMessage, reaction: Reaction | null): Promise<boolean> => {
    const result = await onReact?.(msg, reaction);
    if (result?.answered) setAnswered((prev) => new Set(prev).add(msg.id));
    return result ? result.ok : true;
  };

  const toggleThought = (id: string) => {
    setOpenThoughts((prev) => ({ ...prev, [id]: !prev[id] }));
  };

  const handleCopy = (text: string, id: string) => {
    navigator.clipboard.writeText(text);
    setCopiedId(id);
    playSfx('click');
    setTimeout(() => setCopiedId(null), 2000);
  };

  const patchUpload = (key: string, patch: Partial<PendingUpload>) =>
    setUploads((list) => list.map((u) => (u.key === key ? { ...u, ...patch } : u)));

  const startUpload = (key: string, file: File) => {
    const abort = new AbortController();
    uploadFiles.current.set(key, { file, abort });
    uploadWithRetry(file, (progress) => patchUpload(key, { progress, retrying: false }), abort.signal, {
      onRetrying: () => patchUpload(key, { retrying: true, progress: 0 }),
    }).then(
      (attachment) => patchUpload(key, { state: 'done', progress: 1, attachment, error: undefined, retrying: false }),
      (err: Error) => {
        if (err.name === 'AbortError') return;
        const retryable = !(err instanceof UploadError) || uploadRetryable(err.status, err.code);
        patchUpload(key, { state: 'error', error: err.message, retryable, retrying: false });
      },
    );
  };

  /** Files from the picker, a drop or a paste: checked here, then uploaded right away. */
  const addFiles = (files: File[]) => {
    if (!files.length) return;
    const room = uploadSlotsLeft(uploads);
    const notes: string[] = [];
    if (files.length > room) notes.push(`Up to ${MAX_ATTACHMENTS} files per message; ${files.length - room === 1 ? 'one was' : `${files.length - room} were`} left out.`);
    const added: PendingUpload[] = [];
    for (const file of files.slice(0, room)) {
      const problem = preflightFile(file);
      if (problem) {
        notes.push(problem);
        continue;
      }
      const key = `${Date.now()}-${Math.random().toString(36).slice(2)}`;
      added.push({ key, name: file.name || 'pasted file', size: file.size, mime: file.type || undefined, progress: 0, state: 'uploading' });
      startUpload(key, file);
    }
    setUploadNote(notes.length ? notes.join(' ') : null);
    if (added.length) setUploads((list) => [...list, ...added]);
  };

  const removeUpload = (key: string) => {
    uploadFiles.current.get(key)?.abort.abort();
    uploadFiles.current.delete(key);
    setUploads((list) => list.filter((u) => u.key !== key));
  };

  const retryUpload = (key: string) => {
    const f = uploadFiles.current.get(key)?.file;
    if (!f) return;
    patchUpload(key, { state: 'uploading', progress: 0, error: undefined });
    startUpload(key, f);
  };

  const hasFiles = (e: React.DragEvent) => Array.from(e.dataTransfer.types).includes('Files');
  const dropHandlers = {
    onDragEnter: (e: React.DragEvent) => {
      if (!hasFiles(e)) return;
      e.preventDefault();
      dragDepth.current += 1;
      setDragOver(true);
    },
    onDragOver: (e: React.DragEvent) => {
      if (!hasFiles(e)) return;
      e.preventDefault();
      e.dataTransfer.dropEffect = 'copy';
    },
    onDragLeave: (e: React.DragEvent) => {
      if (!hasFiles(e)) return;
      dragDepth.current = Math.max(0, dragDepth.current - 1);
      if (dragDepth.current === 0) setDragOver(false);
    },
    onDrop: (e: React.DragEvent) => {
      if (!hasFiles(e)) return;
      e.preventDefault();
      dragDepth.current = 0;
      setDragOver(false);
      addFiles(filesFrom(e.dataTransfer.files));
      textareaRef.current?.focus();
    },
  };

  // `/idea …` goes in the idea jar, not to a bot (spec §2.3): the field dresses violet and blocks a send it can't take.
  const ideaMode = isIdeaCommand(inputVal);
  const ideaBlocked = ideaMode ? ideaBlockedLine(inputVal, IDEA_MAX, uploads.length > 0) : null;
  // A failed drop's reason, shown while the field still holds the words it gave back.
  const [ideaError, setIdeaError] = useState<{ text: string; line: string } | null>(null);
  const [ideaTried, setIdeaTried] = useState(false);
  useEffect(() => {
    if (!ideaMode) setIdeaTried(false);
  }, [ideaMode]);
  // The one-item "/" popover: shows while the field holds a start of "/idea" and nothing else.
  const slashOpen = !ideaMode && inputVal.length > 0 && inputVal.length < 5 && '/idea'.startsWith(inputVal.toLowerCase()) && !uploads.length;
  const completeIdea = () => {
    setInputVal('/idea ');
    requestAnimationFrame(() => {
      const el = textareaRef.current;
      if (el) {
        el.focus();
        el.setSelectionRange(6, 6);
      }
    });
  };

  const canSend = ideaMode ? !ideaBlocked : composerReady(inputVal, uploads);
  // The line under the field: too long or a file shows at once; an empty idea only once Enter was tried.
  const ideaLine = (ideaError && ideaError.text === inputVal ? ideaError.line : null) ?? (ideaBlocked && (ideaTried || ideaLength(inputVal) > 0 || uploads.length > 0) ? ideaBlocked : null);
  const stillUploading = uploads.some((u) => u.state === 'uploading');

  const handleSubmit = (e?: React.FormEvent) => {
    if (e) e.preventDefault();
    // A reply in progress doesn't block the next message (it waits its turn); a team discussion does.
    if (ideaMode) {
      // Ideas never wait on a reply or a discussion: the jar is not a bot.
      if (ideaBlocked) {
        setIdeaTried(true);
        return;
      }
      const draft = inputVal;
      playSfx('click');
      const sent = onSendMessage(draft.trim(), [], undefined);
      setInputVal('');
      if (textareaRef.current) textareaRef.current.style.height = 'auto';
      if (sent && typeof (sent as Promise<unknown>).then === 'function')
        void (sent as Promise<boolean | string | void>).then((r) => {
          if (typeof r === 'string') {
            // A failed drop gives the words back, with the reason under the field.
            setInputVal((now) => (now ? now : draft));
            setIdeaError({ text: draft, line: r });
          }
        });
      return;
    }
    if (!canSend || (isStreaming && discussionState?.inProgress)) return;
    playSfx('click');
    stickRef.current = true;
    const replying = replyTo;
    const sent = onSendMessage(inputVal.trim(), readyAttachments(uploads), replying ? { replyTo: replying } : undefined);
    setReplyTo(null);
    // A send that fails keeps the reply, unless the person has moved on to another one.
    if (replying && sent && typeof (sent as Promise<unknown>).then === 'function')
      void (sent as Promise<boolean | void>).then((ok) => {
        if (ok === false) setReplyTo((now) => now ?? replying);
      });
    // The bots this goes to blink once, so you see who received it.
    recipientsOf(inputVal, activeKwants, home?.kind === 'bot' ? home.bot?.id : undefined).forEach(moodNoticed);
    setInputVal('');
    setUploads([]);
    setUploadNote(null);
    uploadFiles.current.clear();
    setMentionQuery(null);
    setMentionStartIndex(-1);
    if (textareaRef.current) {
      textareaRef.current.style.height = 'auto';
    }
  };

  const handleKeyDown = (e: React.KeyboardEvent<HTMLTextAreaElement>) => {
    if (slashOpen && (e.key === 'Tab' || (e.key === 'Enter' && !e.shiftKey))) {
      e.preventDefault();
      completeIdea();
      return;
    }
    if (isMentionOpen) {
      if (e.key === 'ArrowDown') {
        e.preventDefault();
        setSelectedIndex((prev) => (prev + 1) % matchingItems.length);
        return;
      }
      if (e.key === 'ArrowUp') {
        e.preventDefault();
        setSelectedIndex((prev) => (prev - 1 + matchingItems.length) % matchingItems.length);
        return;
      }
      if (e.key === 'Enter' || e.key === 'Tab') {
        e.preventDefault();
        if (matchingItems[safeSelectedIndex]) {
          handleSelectMention(matchingItems[safeSelectedIndex]);
        }
        return;
      }
      if (e.key === 'Escape') {
        e.preventDefault();
        setMentionQuery(null);
        return;
      }
    }

    if (e.key === 'Escape' && replyTo) {
      e.preventDefault();
      setReplyTo(null);
      return;
    }

    if (e.key === 'Enter' && !e.shiftKey) {
      e.preventDefault();
      handleSubmit();
    }
  };

  // "Ask about this" on a Smart Results row, card or finding: fill in the composer, never send.
  useEffect(() => {
    const prefill = (e: Event) => {
      const text = (e as CustomEvent<{ text?: unknown }>).detail?.text;
      if (typeof text !== 'string' || !text) return;
      setInputVal((prev) => withPrefill(prev, text));
      requestAnimationFrame(() => {
        const el = textareaRef.current;
        if (!el) return;
        el.focus();
        el.setSelectionRange(el.value.length, el.value.length);
        el.style.height = 'auto';
        el.style.height = `${Math.min(el.scrollHeight, 160)}px`;
      });
    };
    window.addEventListener(COMPOSE_PREFILL_EVENT, prefill);
    return () => window.removeEventListener(COMPOSE_PREFILL_EVENT, prefill);
  }, []);

  const handlePromptSuggestion = (prompt: string) => {
    setInputVal(prompt);
    playSfx('click');
    if (textareaRef.current) {
      textareaRef.current.focus();
    }
  };

  const retries = retryTargets(messages);
  const retrySend = async (target: { text: string; attachments: AttachmentRef[] }) => {
    stickRef.current = true;
    if ((await onSendMessage(target.text, target.attachments)) === false) throw new Error('not sent');
  };

  const switchNoteFor = useMemo(
    () => assignSwitchNotes(switchNotes ?? [], messages, (m) => (m.sender_id && m.sender_id.length > 20 ? title : m.sender_id)),
    [switchNotes, messages, title],
  );
  const liveNote = isStreaming ? liveSwitchNote(switchNotes ?? [], switchNoteFor, streamingSpeakerName || title) : null;

  // Replies that arrive while the chat is open slide in; the first load and a stream handing over to its saved copy do not
  const freshTrack = useRef<{ key: string | undefined; seen: Set<string> | null; fresh: Set<string>; streamEnded: number; wasStreaming: boolean }>({
    key: undefined,
    seen: null,
    fresh: new Set(),
    streamEnded: 0,
    wasStreaming: false,
  });
  {
    const t = freshTrack.current;
    if (isStreaming) t.wasStreaming = true;
    else if (t.wasStreaming) {
      t.wasStreaming = false;
      t.streamEnded = Date.now();
    }
    if (t.key !== contextKey || t.seen === null) {
      t.key = contextKey;
      t.seen = new Set(messages.map((m) => m.id));
      t.fresh = new Set();
    } else {
      for (const m of messages) {
        if (t.seen.has(m.id)) continue;
        t.seen.add(m.id);
        if (!isStreaming && Date.now() - t.streamEnded > 2500) t.fresh.add(m.id);
      }
    }
  }

  const renderMessage = (msg: ChatMessage) => {
    const interruption = parseInterruption(msg);
    if (interruption)
      return (
        <InterruptedNotice
          key={msg.id}
          messageId={msg.id}
          interruption={interruption}
          retry={retries.get(msg.id)}
          onRetry={retrySend}
          flash={flashId === msg.id}
        />
      );
    const meta = msg.meta && renderDelegation ? parseDelegationMeta(msg.meta) : null;
    if (meta && renderDelegation) return <React.Fragment key={msg.id}>{renderDelegation(msg, meta, flashId === msg.id)}</React.Fragment>;
    const question = msg.meta && renderQuestion ? parseQuestionMeta(msg.meta) : null;
    if (question && renderQuestion) return <React.Fragment key={msg.id}>{renderQuestion(msg, question, flashId === msg.id)}</React.Fragment>;
    const humanStep = msg.meta && renderHumanStep ? parseHumanStepMeta(msg.meta) : null;
    if (humanStep && renderHumanStep) return <React.Fragment key={msg.id}>{renderHumanStep(msg, humanStep, flashId === msg.id)}</React.Fragment>;
    const suggestion = msg.meta && renderMcpSuggestion ? parseSuggestionMeta(msg.meta) : null;
    if (suggestion && renderMcpSuggestion) return <React.Fragment key={msg.id}>{renderMcpSuggestion(msg, suggestion, flashId === msg.id)}</React.Fragment>;
    const connection = msg.meta && renderConnectionSuggestion ? parseConnectionMeta(msg.meta) : null;
    if (connection && renderConnectionSuggestion)
      return <React.Fragment key={msg.id}>{renderConnectionSuggestion(msg, connection, flashId === msg.id)}</React.Fragment>;
    const botAction = msg.meta && renderBotAction ? parseBotActionMeta(msg.meta) : null;
    if (botAction && renderBotAction) return <React.Fragment key={msg.id}>{renderBotAction(msg, botAction, flashId === msg.id)}</React.Fragment>;
    const hire = msg.meta && renderHire ? parseHireMeta(msg.meta) : null;
    if (hire && renderHire) return <React.Fragment key={msg.id}>{renderHire(msg, hire, flashId === msg.id)}</React.Fragment>;
    const routeChoice = msg.meta && renderRouteChoice ? parseRouteChoiceMeta(msg.meta) : null;
    if (routeChoice && renderRouteChoice) {
      const asked = messages.find((m) => m.id === routeChoice.user_message_id);
      return <React.Fragment key={msg.id}>{renderRouteChoice(msg, routeChoice, flashId === msg.id, asked?.text)}</React.Fragment>;
    }
    const setup = msg.meta && onOpenAccounts ? parseSetupMeta(msg.meta) : null;
    if (setup && onOpenAccounts)
      return (
        <div key={msg.id} data-message-id={msg.id} className="w-full max-w-5xl">
          <SetupNeededCard
            meta={setup}
            botName={msg.sender_id || title}
            spaceName={spaceName ?? 'this space'}
            canRun={canRun}
            onOpenAccounts={onOpenAccounts}
            flash={flashId === msg.id}
          />
        </div>
      );
    const spend = parseSpendStopped(msg.meta);
    if (spend)
      return (
        <SpendStoppedNotice
          key={msg.id}
          messageId={msg.id}
          text={msg.text}
          meta={spend}
          onOpenSettings={onOpenUsage}
          flash={flashId === msg.id}
        />
      );
    const paused = parseBudgetPaused(msg.meta);
    if (paused)
      return (
        <BudgetPausedNotice
          key={msg.id}
          messageId={msg.id}
          text={msg.text}
          meta={paused}
          onContinue={canRun === false ? undefined : chainBudgetApi.continue}
          flash={flashId === msg.id}
        />
      );
    const capped = parseHandoffCapped(msg.meta);
    if (capped)
      return (
        <HandoffCappedNotice
          key={msg.id}
          messageId={msg.id}
          meta={capped}
          onContinue={canRun === false ? undefined : chainBudgetApi.continue}
          flash={flashId === msg.id}
        />
      );
    if (isSystemNotice(msg) && !parseAttachments(msg.meta).length)
      return <SystemNotice key={msg.id} messageId={msg.id} text={msg.text} flash={flashId === msg.id} />;
    const isUser = msg.sender_role === 'user';
    const files = msg.meta ? parseAttachments(msg.meta) : [];
    // A bot's share_file note: the file tiles say it all.
    const filesOnly = files.length > 0 && isSharedFileMessage(msg.meta);
    const isThoughtOpen = !!openThoughts[msg.id];
    const savedActivity = !isUser && msg.meta ? parseActivityMeta(msg.meta) : null;
    const ranOn = isUser ? null : ranOnLabel(msg.meta);
    const account = isUser ? null : ranOnAccount(msg.meta);
    const tokens = isUser ? null : replyTokens(msg.tokens_used, msg.meta);
    // Any auto-routed channel, not only #general: the server attaches `routing` wherever it picked the bot.
    const routed = isUser ? null : parseRoutingMeta(msg.meta);
    // Older channel replies still hold a raw [DECISIONS] block: hidden here, its lines shown as chips
    const shownDecisions = isUser ? null : messageDecisions(msg.text, msg.meta);
    const decisions = shownDecisions?.decisions ?? [];
    const text = shownDecisions ? shownDecisions.text : msg.text;
    const workAhead = isUser ? null : parseWorkAhead(msg.meta);
    const displayName = isUser ? agentSenderLabel(msg.meta) ?? 'You' : (msg.sender_id && msg.sender_id.length > 20 ? title : msg.sender_id);
    const replyQuote = isUser ? replyRefOf(msg.reply_to) : null;
    // Whose mood the reply header shows (Pixel's AvatarMood, drawn by ModernAvatar given a kwantId):
    // Laya's pick, else this bot chat's bot, else the team member with that name. Unknown: no mood.
    const replyKwantId = isUser
      ? undefined
      : routed?.kwant_id ?? (home?.kind === 'bot' ? home.bot?.id : undefined) ?? activeKwants.find((k) => k.name === displayName)?.id;

    return (
      <div
        key={msg.id}
        data-message-id={msg.id}
        tabIndex={-1}
        className={`flex items-start gap-3.5 w-full max-w-5xl rounded-xl focus:outline-none ${isUser ? 'ml-auto flex-row-reverse justify-start' : ''} ${
          flashId === msg.id ? (flashKind === 'quote' ? 'kw-flash-quote' : 'kw-flash') : ''
        } ${freshTrack.current.fresh.has(msg.id) ? 'kws-in' : ''}`}
      >
        {/* Phones show the avatar inside the name line instead, so the bubble gets the full width */}
        <span className="hidden sm:contents">
          {/* Laya hand-off: a reply she just routed brings the chosen bot's avatar up into the header */}
          <span className={routed && freshTrack.current.fresh.has(msg.id) ? 'kws-handoff inline-flex' : 'contents'}>
            <ModernAvatar
              role={isUser ? 'user' : title}
              name={displayName}
              size={36}
              kwantId={replyKwantId}
            />
          </span>
        </span>

        <div className={`flex flex-col min-w-0 ${isUser ? 'max-w-[85%] sm:max-w-2xl items-end' : 'flex-1 max-w-4xl items-start'}`}>
          {isUser ? (
            <div className="flex items-center gap-2 mb-1.5 text-[11px] text-zinc-400 font-medium max-w-full min-w-0 whitespace-nowrap">
              <span className="text-zinc-200 shrink-0">{displayName}</span>
              {fromTelegram(parseMetaObject(msg.meta)) && <ViaTelegram />}
            </div>
          ) : (
            // One line, "Name · 3:42 PM"; the model, account, tokens, steps and reasoning wait behind Details
            <ReplyMeta
              name={displayName}
              at={msg.created_at}
              sentToTelegram={sentToTelegram(parseMetaObject(msg.meta))}
              avatar={<span className={routed && freshTrack.current.fresh.has(msg.id) ? 'kws-handoff inline-flex' : 'inline-flex'}><ModernAvatar role={title} name={displayName} size={20} kwantId={replyKwantId} /></span>}
              ranOn={ranOn}
              account={account}
              tokens={tokens}
              failed={!!savedActivity?.steps.some((s) => s.status === 'failed')}
              note={savedFailoverNotice(msg.meta) ?? switchNoteFor.get(msg.id)}
              handoff={savedFailoverHandoff(msg.meta)}
              route={
                routed
                  ? {
                      label: routeChipLabel({ basis: routed.basis, fallback: routed.fallback }),
                      text: routingLine(routed, spaceName ?? 'this space', (id) => activeKwants.find((k) => k.id === id)?.name),
                      fallback: !!routed.fallback,
                      reroute:
                        onSendToOtherBot && questionOf.get(msg.id)
                          ? {
                              key: msg.id,
                              targets: rerouteTargets.filter((k) => k.id !== routed.kwant_id && k.name !== displayName),
                              onPick: (kwantId) => onSendToOtherBot(questionOf.get(msg.id)!, kwantId),
                            }
                          : undefined,
                    }
                  : undefined
              }
            >
              {/* What the reply did: "12 steps · 41s", opening to the steps (tool calls go inside) */}
              {savedActivity && (
                <SavedActivityList activity={savedActivity} idBase={`act-${msg.id}`}>
                  {msg.tool_calls && (
                    <div className="border-t border-zinc-800/60 p-2">
                      <ToolCallsList raw={msg.tool_calls} />
                    </div>
                  )}
                </SavedActivityList>
              )}

              {/* Collapsible Reasoning Disclosure (the steps above already hold the thinking when they have it) */}
              {msg.reasoning_content && !hasThinkingText(savedActivity) && (
                <div className="mb-2 max-w-full">
                  <button
                    type="button"
                    onClick={() => toggleThought(msg.id)}
                    className="max-w-full min-w-0 flex items-center gap-2 px-3 py-1.5 rounded-xl text-xs text-zinc-400 hover:text-zinc-200 bg-zinc-900/80 hover:bg-zinc-900 border border-zinc-800 hover:border-zinc-700/80 transition-all cursor-pointer shadow-sm group"
                  >
                    <Sparkles size={13} className="text-violet-400 group-hover:rotate-12 transition-transform flex-shrink-0" />
                    <span className="truncate min-w-0 sm:max-w-md">
                      {msg.reasoning_content.split('\n')[0].replace(/^🔮\s*/, '')}
                    </span>
                    {isThoughtOpen ? <ChevronDown size={13} className="text-zinc-500 flex-shrink-0" /> : <ChevronRight size={13} className="text-zinc-500 flex-shrink-0" />}
                  </button>

                  {isThoughtOpen && (
                    <div className="mt-1.5 p-3.5 rounded-xl bg-zinc-950/90 border border-zinc-800 text-xs text-zinc-300 whitespace-pre-wrap leading-relaxed shadow-inner max-h-80 overflow-y-auto">
                      {msg.reasoning_content}
                    </div>
                  )}
                </div>
              )}
            </ReplyMeta>
          )}

          {/* Message Body Bubble */}
          {!filesOnly && (text.trim() || !files.length) && (
          <div
            className={`px-3.5 py-3 sm:p-5 rounded-2xl text-sm leading-relaxed shadow-sm w-full ${
              isUser
                ? 'bg-zinc-800 text-zinc-100 rounded-tr-sm border border-zinc-700/60 max-w-2xl'
                : workAhead
                  ? 'bg-indigo-950/15 text-zinc-200 rounded-tl-sm border border-dashed border-indigo-400/25'
                  : 'bg-zinc-900/60 text-zinc-200 rounded-tl-sm border border-zinc-800/80'
            }`}
            data-work-ahead={workAhead ? '' : undefined}
          >
            {isUser ? (
              <>
                {replyQuote && (
                  <ReplyQuote reply={replyQuote} botName={botNameOf(replyQuote.sender_id, activeKwants)} onOpen={() => showOriginal(replyQuote.id)} />
                )}
                <div className="whitespace-pre-wrap break-words">{renderFormattedText(msg.text)}</div>
              </>
            ) : workAhead ? (
              <WorkAheadBody messageId={msg.id} text={text} />
            ) : (
              <MarkdownRenderer content={text} results={msg.results} messageId={msg.id} />
            )}
          </div>
          )}
          {files.length > 0 && <FileTiles files={files} align={isUser ? 'end' : 'start'} />}
          {/* A shared-file card has no bubble, so the files block (the table or text preview) is mounted here, after the tiles. */}
          {filesOnly && !isUser && msg.results != null && (
            // The column is `items-start`, so without a width cap a long cell stretches the preview past the message.
            <div className="w-full min-w-0 max-w-full">
              <FilesPreviewOnly.Provider value={true}>
                <MarkdownRenderer content="" results={msg.results} messageId={msg.id} />
              </FilesPreviewOnly.Provider>
            </div>
          )}

          {decisions.length > 0 && (
            <DecisionChips
              messageId={msg.id}
              decisions={decisions}
              consulted={consultedNames(msg.meta)}
              onChange={onChangeDecision && !shownDecisions?.readOnly ? (index, picked) => onChangeDecision(msg, index, picked) : undefined}
            />
          )}

          {!isUser && msg.tool_calls && !savedActivity && <ToolCallsList raw={msg.tool_calls} />}

          {/* Action row: icon chips; the smart-result actions portal in after Copy */}
          {!isUser && !filesOnly && (
            <div className="mt-2 flex flex-wrap items-center gap-2 text-zinc-400 text-xs max-w-full min-w-0">
              <button
                type="button"
                onClick={() => handleCopy(text, msg.id)}
                className={actionClass}
                title="Copy response"
              >
                {copiedId === msg.id ? (
                  <>
                    <Check size={14} className="text-emerald-400" aria-hidden="true" />
                    <span className="text-emerald-400">Copied</span>
                  </>
                ) : (
                  <>
                    <Copy size={14} aria-hidden="true" />
                    <span className="sm:hidden">Copy</span>
                    <span className="hidden sm:inline">Copy markdown</span>
                  </>
                )}
              </button>
              {msg.sender_role === 'kwant' && !workAhead && !msg.id.startsWith('temp-') && (
                <>
                  {onReact && (
                    <ReactionButtons
                      messageId={msg.id}
                      reaction={reactionOf(msg.reaction)}
                      asks={msg.id === questionId && !answered.has(msg.id)}
                      onReact={(r) => react(msg, r)}
                    />
                  )}
                  <ReplyButton botName={displayName} active={replyTo?.id === msg.id} onReply={() => startReply(msg)} />
                </>
              )}
              {msg.results != null && <SmartActionsSlot messageId={msg.id} />}
            </div>
          )}
        </div>
      </div>
    );
  };

  // The empty chat shows the bot home (a bot's own chat, or #general) instead of a bare pane and a row of chips
  const showHome =
    !!home && (home.kind === 'bot' || home.kind === 'general') && !welcome && messages.length === 0 && !isStreaming && !queued && !deciding && pendingApprovals.length === 0;
  // Who answers what's typed: the chat decides, an @mention overrides
  const target = answerTarget({
    kind: home?.kind ?? 'channel',
    text: inputVal,
    botName: home?.bot?.name ?? title,
    teamName,
    names: activeKwants.map((k) => k.name).concat(activeTeams.map((t) => t.name)),
  });
  const toolPhone = 'w-11 h-11 text-[#a39dcc] hover:text-zinc-100 hover:bg-white/[0.06] active:bg-white/[0.06] focus-visible:outline-[#b8adff]';
  // Phone hint row (spec A2): no "@ mention · attach files" on touch, and "Answers:" only where it isn't
  // obvious (channels and team chats, or a direct chat with an @mention typed).
  const showAnswers = !ideaMode && (!phone || home?.kind !== 'bot' || target.kind === 'mention');
  const showHintText = !(phone && touch);
  const placeholder = phone
    ? phonePlaceholder(title, fieldWidth, (t) => measureText16(t, textareaRef.current ? getComputedStyle(textareaRef.current).fontFamily : ''))
    : spaceName && !narrow
      ? `Message ${title} in ${spaceName}`
      : `Message ${title}`;
  const insertAt = () => {
    playSfx('click');
    const el = textareaRef.current;
    const next = inputVal && !/\s$/.test(inputVal) ? `${inputVal} @` : `${inputVal}@`;
    setInputVal(next);
    el?.focus();
    requestAnimationFrame(() => checkMention(next, next.length));
  };

  return (
    <div className="flex-1 flex flex-col h-full bg-surface-main overflow-hidden" data-keyboard-mode={kbd ? 'on' : undefined}>
      {/* Channel / bot header: 56 px on a phone, 48 px in keyboard mode (instant, no animation) */}
      <div
        data-chat-header
        className={`${kbd ? 'h-12' : 'h-14'} shrink-0 border-b border-hairline px-2 sm:px-6 gap-2 flex items-center justify-between bg-surface-main/80 backdrop-blur-md select-none`}
      >
        {onBack && (
          <button
            type="button"
            onClick={onBack}
            aria-label="Back to chats"
            className="shrink-0 w-11 h-11 -ml-1 inline-flex items-center justify-center rounded-lg text-zinc-300 hover:text-zinc-100 hover:bg-zinc-800 focus-visible:outline focus-visible:outline-2 focus-visible:outline-[#b8adff]"
          >
            <ChevronLeft size={18} />
          </button>
        )}
        {home?.kind === 'bot' && home.bot && (
          <ModernAvatar role={home.bot.avatar_sprite} name={home.bot.name} size={phone ? (kbd ? 28 : 32) : 34} kwantId={home.bot.id} className="rounded-xl" />
        )}
        <div className="min-w-0 flex-1">
          <h2
            className={`${phone ? (kbd ? 'text-[15px] leading-5' : 'text-base leading-5') : 'text-body'} font-semibold text-zinc-50 flex items-center gap-2 min-w-0`}
          >
            <span className="truncate">{title}</span>
            {titleExtra}
          </h2>
          {kbd ? null : phone && threadLine ? (
            <div className="flex min-w-0 mt-0.5">{threadLine}</div>
          ) : (
            <p className={`${phone ? 'text-[13px] leading-4 mt-0.5' : 'text-meta'} text-zinc-400 truncate max-w-lg`}>{subtitle}</p>
          )}
        </div>

        <div className="flex items-center gap-0.5 sm:gap-3 shrink-0">
          {/* Active Team Workspace Badge */}
          {(workspacePath || repoUrl) && (
            <div
              className="hidden sm:flex items-center gap-1.5 px-2.5 py-1 rounded-lg text-xs font-mono text-zinc-300 bg-zinc-900 border border-zinc-700/60 max-w-[200px] truncate"
              title={workspacePath ? `Workspace directory: ${workspacePath}` : `Git Repository: ${repoUrl}`}
            >
              {workspacePath ? (
                <Folder size={12} className="text-amber-400 flex-shrink-0" />
              ) : (
                <GitBranch size={12} className="text-emerald-400 flex-shrink-0" />
              )}
              <span className="truncate">
                {workspacePath
                  ? workspacePath.replace(/^\/Users\/[^/]+/, '~')
                  : repoUrl?.replace(/^https?:\/\/github\.com\//, '').replace(/\.git$/, '')}
              </span>
            </div>
          )}

          {teamMembers && teamMembers.length > 0 && (
            <div className="flex items-center gap-0.5 sm:gap-2 sm:pl-3 sm:border-l border-zinc-800">
              <div className="hidden sm:flex -space-x-2 items-center px-1 py-1 -my-0.5">
                {teamMembers.map((member) => (
                  <div
                    key={member.id}
                    title={`${member.name} (${member.title})`}
                    className="inline-block ring-2 ring-zinc-950 rounded-full"
                  >
                    <ModernAvatar role={member.avatar_sprite} name={member.name} size={24} kwantId={member.id} />
                  </div>
                ))}
              </div>
              {onManageTeam && (
                <button
                  type="button"
                  onClick={onManageTeam}
                  className="kw-head-btn flex items-center justify-center gap-1.5 max-sm:min-w-11 px-2.5 py-1 rounded-lg text-xs font-medium text-zinc-200 bg-white/[0.04] hover:bg-white/[0.09] hover:text-zinc-50 border border-hairline-strong transition-colors cursor-pointer focus-visible:outline focus-visible:outline-2 focus-visible:outline-sky-400"
                  title="Team members and settings"
                  aria-label="Manage team"
                >
                  <Settings2 size={12} className="text-zinc-400" />
                  <span className="hidden sm:inline">Manage</span>
                </button>
              )}
            </div>
          )}

          {onEditInstructions && pinned.length === 0 && (
            <button
              type="button"
              onClick={onEditInstructions}
              className="kw-head-btn flex items-center justify-center gap-1.5 max-md:min-w-11 px-2.5 py-1 rounded-lg text-xs font-medium text-zinc-200 bg-white/[0.04] hover:bg-white/[0.09] hover:text-zinc-50 border border-hairline-strong transition-colors cursor-pointer focus-visible:outline focus-visible:outline-2 focus-visible:outline-sky-400"
              title="Pin instructions every bot reads in this channel"
              aria-label="Pin instructions"
            >
              <Pin size={12} className="text-amber-300" />
              <span className="hidden sm:inline">Pin</span>
            </button>
          )}

          {onEditKwant && (
            <button
              type="button"
              onClick={onEditKwant}
              className="kw-head-btn flex items-center gap-1.5 px-2.5 py-1 rounded-lg text-xs font-medium text-zinc-200 bg-white/[0.04] hover:bg-white/[0.09] hover:text-zinc-50 border border-hairline-strong transition-colors cursor-pointer focus-visible:outline focus-visible:outline-2 focus-visible:outline-sky-400"
              title={`Edit ${title}'s name, avatar, model and persona`}
              aria-label={`Edit ${title}`}
            >
              <Pencil size={12} className="text-zinc-400" />
              <span className="hidden sm:inline">Edit</span>
            </button>
          )}

          {/* No header Stop while the working strip shows (always, when a reply runs): the composer's stop square is the one Stop. */}
          {/* Sandbox VM & Live Browser Toggle Button */}
          <button
            type="button"
            onClick={() => {
              playSfx('click');
              toggleVm();
            }}
            className={`kw-head-btn flex items-center justify-center gap-1.5 max-md:min-w-11 px-2.5 py-1 rounded-full text-xs font-medium transition-colors border cursor-pointer focus-visible:outline focus-visible:outline-2 focus-visible:outline-sky-400 ${
              vmOpen
                ? 'text-sky-200 bg-sky-400/15 border-sky-300/40'
                : 'text-zinc-200 bg-white/[0.04] hover:bg-white/[0.09] border-hairline-strong hover:text-zinc-50'
            }`}
            title="Show or hide the bot's browser"
            aria-label="Bot browser"
            aria-pressed={vmOpen}
          >
            <Monitor size={12} aria-hidden="true" />
            <span className="hidden sm:inline">Bot browser</span>
          </button>
        </div>
      </div>

      {/* In keyboard mode the thread controls and pinned instructions step aside (display: none) */}
      <div data-thread-bar className={kbd ? 'hidden' : 'contents'}>
        {subHeader}
      </div>
      {!(welcome && messages.length === 0) && notice}

      <div className={kbd ? 'hidden' : 'contents'}>
        <PinnedInstructions items={pinned} onEdit={onEditInstructions} />
      </div>

      {/* Discussion In-Flight Banner */}
      {discussionState?.inProgress && (
        <div className="px-3 sm:px-6 pt-3 select-none flex-shrink-0">
          <div className="flex items-center justify-between gap-2 px-3 sm:px-4 py-2 bg-sky-950/40 border border-sky-800/40 rounded-xl mb-3 text-xs">
            <div className="flex items-center gap-2 text-sky-200 min-w-0">
              <Users size={14} className="text-sky-400 animate-pulse shrink-0" />
              <span className="min-w-0">
                {discussionState.kind === 'handoff' ? (
                  <>
                    Handoff: <strong>{discussionState.currentSpeaker || 'Teammate'}</strong> is replying
                    {discussionState.askedBy ? <> (asked by {discussionState.askedBy})</> : null}
                    {' '}• {discussionState.currentTurn || 1} of {discussionState.totalTurns || 1}
                  </>
                ) : discussionState.phase === 'summary' ? (
                  <>
                    <strong>{discussionState.currentSpeaker || 'The lead'}</strong> is summarizing the team's answer
                  </>
                ) : (
                  <>
                    Team discussion: <strong>{discussionState.currentSpeaker || 'Team member'}</strong>
                    {discussionState.phase === 'open'
                      ? ' is opening'
                      : discussionState.askedBy
                        ? <> is answering {discussionState.askedBy}</>
                        : ' is responding'}
                    {' '}• turn {discussionState.currentTurn || 1} of {discussionState.totalTurns || 1}
                  </>
                )}
              </span>
            </div>
            {onCancelDiscussion && (
              <button
                type="button"
                onClick={onCancelDiscussion}
                className="shrink-0 flex items-center gap-1 px-2.5 py-1 bg-red-500/20 hover:bg-red-500/30 text-red-300 rounded border border-red-500/30 transition-colors cursor-pointer"
              >
                <Square size={11} fill="currentColor" />
                <span>Stop<span className="hidden sm:inline"> Discussion</span></span>
              </button>
            )}
          </div>
        </div>
      )}

      {/* Main Split Layout: Messages/Input on Left + Virtual Machine Side Drawer on Right */}
      <div className="flex-1 flex overflow-hidden min-h-0">
        <div className="flex-1 flex flex-col h-full overflow-hidden min-w-0">
          {/* Messages Stream */}
          <div
            ref={scrollRef}
            onScroll={onScroll}
            className="relative flex-1 overflow-y-auto px-3 py-4 sm:p-6 space-y-5 sm:space-y-6"
            role="log"
            aria-label={`Messages in ${title}`}
            aria-busy={isStreaming}
          >
        {welcome && messages.length === 0 && welcome}
        {showHome && home && (
          <div className="flex min-h-full items-center justify-center">
            <BotHome home={home} spaceName={spaceName} team={activeKwants} prompts={suggestions} canRun={canRun} onPick={handlePromptSuggestion} />
          </div>
        )}
        {groupDiscussionThreads(messages).map((item) => {
          if (item.kind === 'message') return renderMessage(item.message);
          const threadKey = `thread:${item.id}`;
          const isThreadOpen = !!openThoughts[threadKey];
          return (
            <div key={threadKey} className="w-full max-w-5xl">
              <button
                type="button"
                onClick={() => toggleThought(threadKey)}
                className="sm:ml-[48px] max-w-full flex items-center gap-2.5 px-3 py-1.5 rounded-xl text-xs text-zinc-400 hover:text-zinc-200 bg-zinc-900/60 hover:bg-zinc-900 border border-zinc-800 hover:border-zinc-700/80 transition-all cursor-pointer"
              >
                {isThreadOpen ? <ChevronDown size={13} className="text-zinc-500" /> : <ChevronRight size={13} className="text-zinc-500" />}
                <span className="flex -space-x-1.5">
                  {item.speakers.map((name) => (
                    <ModernAvatar key={name} role={name} name={name} size={18} />
                  ))}
                </span>
                <span className="min-w-0 truncate">
                  Team discussed • {item.turns.length} {item.turns.length === 1 ? 'turn' : 'turns'} • {item.speakers.join(', ')}
                </span>
              </button>
              {isThreadOpen && (
                <div className="mt-4 ml-2 pl-3 sm:ml-[30px] sm:pl-4 border-l border-zinc-800 space-y-6">
                  {item.turns.map(renderMessage)}
                </div>
              )}
            </div>
          );
        })}

        {/* Laya routing: which agent will answer */}
        {isStreaming && routingState && !discussionState?.inProgress && (
          <LayaRoutingCard routing={routingState} spaceName={spaceName} />
        )}

        {/* A held reply is being decided: the line stands in for the streamed draft and the consults */}
        {deciding && <DecidingLine botName={deciding.botName} consulted={deciding.consulted} />}

        {/* Live Streaming Assistant Message (or its place in the queue, in the same frame) */}
        {isStreaming && !deciding && routingState?.phase !== 'deciding' && (
          <div className="flex items-start gap-3.5 w-full max-w-5xl">
            <ModernAvatar
              role={
                discussionState?.inProgress && discussionState.currentSpeaker
                  ? discussionState.currentSpeaker
                  : streamingSpeakerName || title
              }
              name={
                discussionState?.inProgress && discussionState.currentSpeaker
                  ? discussionState.currentSpeaker
                  : streamingSpeakerName || title
              }
              size={34}
            />
            <div className="flex flex-col items-start min-w-0 flex-1 max-w-4xl">
              <div className="flex items-center gap-2 mb-1.5 text-[11px] text-zinc-400 font-medium">
                <span className="text-zinc-200">
                  {discussionState?.inProgress && discussionState.currentSpeaker
                    ? discussionState.currentSpeaker
                    : streamingSpeakerName || title}
                </span>
                {queued ? (
                  <span className="inline-flex items-center gap-1 text-amber-300 text-[10px]">
                    <span className="w-1.5 h-1.5 rounded-full bg-amber-300" aria-hidden="true" />
                    Queued
                  </span>
                ) : (awaitingAnswer || bubble === 'approval') && !phone ? (
                  <span className="inline-flex items-center gap-1 text-amber-300 text-[10px]">
                    <span className="w-1.5 h-1.5 rounded-full bg-amber-300" aria-hidden="true" />
                    {bubble === 'approval' ? 'Waiting for your OK' : awaitingBrowser ? 'Waiting for you in the browser' : 'Waiting for your answer'}
                  </span>
                ) : discussionState?.inProgress ? (
                  <span className="inline-flex items-center gap-1 text-sky-400 text-[10px]">
                    <span className="w-1.5 h-1.5 rounded-full bg-sky-400 animate-ping motion-reduce:animate-none" aria-hidden="true" />
                    Speaking...
                  </span>
                ) : null /* "Generating" would repeat the working strip */}
              </div>

              {liveNote && <FailoverNote text={liveNote} />}

              {liveActivity && <LiveActivityList live={liveActivity} idBase={`live-${liveActivity.turnId}`} compact={phone} />}

              {streamingThinking && (
                <div className="mb-2 p-2.5 rounded-xl bg-violet-950/20 border border-violet-500/20 text-xs text-violet-300/90 flex items-center gap-2 animate-pulse motion-reduce:animate-none max-w-full">
                  <Sparkles size={13} className="text-violet-400 flex-shrink-0" />
                  <span className="truncate">{streamingThinking}</span>
                </div>
              )}

              {/* On a phone the strip says "working" and "waiting": no skeleton, no "Paused on…" bubble. */}
              {(!phone || queued || streamingText) && (
              <div className="p-4 sm:p-5 min-h-[3.5rem] rounded-2xl rounded-tl-sm bg-zinc-900/60 text-zinc-200 border border-zinc-800/80 text-sm leading-relaxed shadow-sm w-full">
                {queued ? (
                  <p className="text-zinc-300" role="status">
                    {queued.position ? <strong className="text-amber-200">#{queued.position} in line.</strong> : <strong className="text-amber-200">In line.</strong>}{' '}
                    {queued.reason === 'server_busy'
                      ? 'The server is running as many turns as it allows right now; this one starts as soon as a slot frees up.'
                      : `${queued.speaker} is busy with other work; this starts as soon as one of those finishes.`}
                  </p>
                ) : streamingText ? (
                  <MarkdownRenderer content={hideStreamingDecisions(stripDiscussionMarkers(streamingText))} isStreaming={true} />
                ) : awaitingAnswer ? (
                  <p className="text-zinc-400" role="status">
                    {awaitingBrowser
                      ? 'Paused on the browser step above. Do it in the browser panel and press Done; this reply carries on from it.'
                      : 'Paused on the question above. Answer it there; this reply carries on from it.'}
                  </p>
                ) : bubble === 'approval' ? (
                  <p className="text-zinc-400" role="status" data-live-bubble="approval">
                    Paused on the approval below. Allow or deny it there; this reply carries on from it.
                  </p>
                ) : (
                  <div className="space-y-2.5 py-1" role="status" aria-label="Waiting for the first words">
                    <div className="kws-skel h-3 w-11/12 rounded-full bg-zinc-700/60" />
                    <div className="kws-skel h-3 w-9/12 rounded-full bg-zinc-700/60" style={{ animationDelay: '120ms' }} />
                    <div className="kws-skel h-3 w-6/12 rounded-full bg-zinc-700/60" style={{ animationDelay: '240ms' }} />
                  </div>
                )}
              </div>
              )}
            </div>
          </div>
        )}

        {/* Approvals answered in Telegram (G4): "Allowed once · via Telegram" */}
        {settledApprovals.map(({ approval, at }) => (
          <ResolvedViaTelegram key={approval.id} kind="approval" lead={approvalLead(approval)} outcome="Allowed once" at={at} />
        ))}

        {/* Pending Security Approval Ticket */}
        {pendingApprovals.map((approval) => (
          <ModernApprovalCard key={approval.id} approval={approval} onResolve={onResolveApproval} />
        ))}
      </div>

      {/* Suggested prompts for who is in this chat: one swipeable row on phones */}
      {messages.length <= 1 && suggestions.length > 0 && !showHome && (
        <ScrollRow
          className="kw-scroll-row px-3 sm:px-6 py-2 flex flex-nowrap sm:flex-wrap items-center gap-2 overflow-x-auto sm:overflow-visible select-none"
          role="group"
          aria-label="Suggested prompts"
        >
          <span className="shrink-0 text-[11px] font-medium text-zinc-500">Suggested:</span>
          {suggestions.map((prompt) => (
            <button
              key={prompt}
              type="button"
              onClick={() => handlePromptSuggestion(prompt)}
              disabled={canRun === false}
              title={canRun === false ? 'Add an account first' : undefined}
              className="shrink-0 whitespace-nowrap px-2.5 py-1 rounded-full text-xs text-zinc-400 bg-zinc-900 border border-zinc-800 hover:border-zinc-700 hover:text-zinc-200 transition-all shadow-sm focus-visible:outline focus-visible:outline-2 focus-visible:outline-sky-500 disabled:opacity-50 disabled:cursor-not-allowed disabled:hover:border-zinc-800 disabled:hover:text-zinc-400"
            >
              {prompt}
            </button>
          ))}
        </ScrollRow>
      )}

      <WorkingStrip
        active={isStreaming}
        name={discussionState?.inProgress && discussionState.currentSpeaker ? discussionState.currentSpeaker : streamingSpeakerName || title}
        title={title}
        live={liveActivity}
        queued={!!queued}
        awaitingAnswer={!!awaitingAnswer}
        awaitingBrowser={!!awaitingBrowser}
        writing={!!streamingText}
        onAnswer={showWaitingCard}
      />

      {/* Modern Composer Input Bar */}
      <div
        className={`px-3 ${phone ? 'pt-2 pb-[max(0.5rem,env(safe-area-inset-bottom))]' : 'pt-3 pb-[max(0.75rem,env(safe-area-inset-bottom))]'} sm:p-4 border-t border-hairline bg-surface-side/60`}
      >
        {teamMembers && teamMembers.length > 0 && (
          <ScrollRow className="kw-scroll-row max-w-4xl mx-auto mb-2.5 flex items-center gap-1.5 overflow-x-auto text-xs select-none" role="group" aria-label="Ask a member">
            <span className="shrink-0 text-[11px] font-medium text-zinc-500 whitespace-nowrap">Ask:</span>
            {teamMembers.map((m) => (
              <button
                key={m.id}
                type="button"
                onClick={() => handleInsertTag(`@${m.name}`)}
                className="shrink-0 whitespace-nowrap flex items-center gap-1.5 px-2.5 py-1 rounded-full bg-zinc-800/90 hover:bg-zinc-700 text-zinc-300 hover:text-zinc-100 border border-zinc-700/60 transition-all text-xs cursor-pointer shadow-sm active:scale-95"
                title={`Ask ${m.name}`}
              >
                <ModernAvatar role={m.avatar_sprite} name={m.name} size={14} kwantId={m.id} />
                <span>@{m.name}</span>
              </button>
            ))}
            {teamName && (
              <button
                type="button"
                onClick={() => handleInsertTag(`@${teamName}`)}
                className="shrink-0 whitespace-nowrap flex items-center gap-1.5 px-2.5 py-1 rounded-full bg-sky-950/60 hover:bg-sky-900/60 text-sky-300 hover:text-sky-200 border border-sky-800/60 transition-all text-xs cursor-pointer shadow-sm active:scale-95"
                title={`Ask whole team @${teamName}`}
              >
                <Users size={12} className="text-sky-400" />
                <span>@{teamName} (Team)</span>
              </button>
            )}
          </ScrollRow>
        )}

        {/* On a phone the strip's "Answer ↑" stands in for this pill while the bot waits on the card. */}
        {approvalOffscreen && pendingApprovals.length > 0 && !(phone && stripWaitsOn === 'approval') && (
          <div role="status" className="max-w-4xl mx-auto mb-2 flex justify-center">
            <button
              type="button"
              onClick={() => {
                playSfx('click');
                showApproval();
              }}
              className="flex items-center gap-1.5 px-3 py-1.5 min-h-[36px] rounded-full border border-amber-500/40 bg-amber-950/70 text-xs font-medium text-amber-200 hover:bg-amber-900/70 shadow-lg focus-visible:outline focus-visible:outline-2 focus-visible:outline-sky-500"
            >
              <ArrowDown size={12} aria-hidden="true" />
              {pendingApprovals.length === 1 ? 'Approval waiting' : `${pendingApprovals.length} approvals waiting`}
            </button>
          </div>
        )}

        {viewingHistory && (
          <div role="status" className="max-w-4xl mx-auto mb-2 flex items-center justify-between gap-2 px-3 py-1.5 rounded-xl border border-sky-800/50 bg-sky-950/40 text-xs text-sky-200">
            <span className="min-w-0">Showing older messages around the search result.</span>
            <button
              type="button"
              onClick={() => {
                playSfx('click');
                onJumpToLatest?.();
              }}
              className="shrink-0 flex items-center gap-1 px-2 py-0.5 rounded-lg border border-sky-700/60 text-sky-100 hover:bg-sky-900/60 focus-visible:outline focus-visible:outline-2 focus-visible:outline-sky-500"
            >
              <ArrowDown size={12} aria-hidden="true" /> Jump to latest
            </button>
          </div>
        )}

        <form
          onSubmit={handleSubmit}
          {...dropHandlers}
          aria-label={`Message ${title}`}
          data-composer-form
          data-stacked={phone ? String(stacked) : undefined}
          className={`relative max-w-4xl mx-auto flex flex-wrap items-end ${stacked ? 'gap-x-1.5 gap-y-1' : 'gap-1.5'} border rounded-2xl p-2 transition-colors ${
            phone ? 'bg-[#1a1538]' : 'bg-surface-raised shadow-raised'
          } ${
            dragOver
              ? 'border-sky-500/70 bg-sky-950/30'
              : phone
                ? 'border-white/10 focus-within:border-[#6c58e0]/70'
                : 'border-hairline-strong focus-within:border-sky-400/50'
          }`}
        >
          {dragOver && (
            <div className="absolute inset-0 z-10 rounded-2xl border-2 border-dashed border-sky-400/70 bg-zinc-950/80 flex items-center justify-center gap-2 text-sm font-medium text-sky-200 pointer-events-none">
              <Paperclip size={15} aria-hidden="true" /> Drop files to attach
            </div>
          )}
          {replyTo && (
            <ReplyChip
              key={replyTo.id}
              botName={replyBotName(replyTo)}
              excerpt={excerptOf(replyTo.text, 200)}
              onOpen={() => showOriginal(replyTo.id)}
              onCancel={cancelReply}
            />
          )}
          {(uploads.length > 0 || uploadNote) && (
            <div className="basis-full min-w-0 px-1 pt-0.5 space-y-1.5">
              {uploads.length > 0 && (
                <ul className="flex flex-wrap gap-1.5 min-w-0" aria-label="Files to send">
                  {uploads.map((u) => (
                    <UploadChip key={u.key} item={u} onRemove={() => removeUpload(u.key)} onRetry={() => retryUpload(u.key)} />
                  ))}
                </ul>
              )}
              {uploadNote && (
                <p role="alert" className="flex items-start gap-2 text-[11px] text-amber-300">
                  <span className="min-w-0">{uploadNote}</span>
                  <button type="button" onClick={() => setUploadNote(null)} className="shrink-0 text-zinc-500 hover:text-zinc-300 underline-offset-2 hover:underline">
                    Dismiss
                  </button>
                </p>
              )}
            </div>
          )}
          {ideaMode && (
            <p data-idea-hint className="basis-full min-w-0 px-1 pt-0.5 flex items-center gap-1.5 text-[12px] leading-4 text-violet-200">
              <JarIcon size={13} className="shrink-0 text-violet-300" />
              <span className="min-w-0 line-clamp-2">
                Goes in the idea jar, not to{' '}
                <span className="font-medium text-violet-100">
                  {home?.kind === 'bot' ? home.bot?.name ?? title : title.startsWith('#') || home?.kind === 'team' ? title : `#${title}`}
                </span>
                . No bot reads it now.
              </span>
            </p>
          )}
          {slashOpen && (
            <div
              role="listbox"
              id="kw-slash-list"
              aria-label="Commands"
              data-slash-popover
              className="kw-fade-in absolute bottom-full left-0 mb-2 bg-surface-overlay border border-hairline-strong rounded-xl shadow-raised p-1.5 min-w-[260px] z-50"
            >
              <button
                type="button"
                role="option"
                id="kw-slash-idea"
                aria-selected="true"
                onMouseDown={(e) => e.preventDefault()}
                onClick={completeIdea}
                className="w-full flex items-center gap-2.5 px-2.5 min-h-11 sm:min-h-9 rounded-lg bg-white/[0.06] text-left text-sm text-zinc-100 hover:bg-white/[0.08]"
              >
                <JarIcon size={15} className="text-violet-300 shrink-0" />
                <span className="font-semibold text-violet-300">/idea</span>
                <span className="text-zinc-400">Drop an idea in the jar</span>
                <kbd className="ml-auto font-sans text-[11px] text-zinc-500">Tab</kbd>
              </button>
            </div>
          )}
          {isMentionOpen && (
            <MentionPopover
              items={matchingItems}
              selectedIndex={safeSelectedIndex}
              onSelect={handleSelectMention}
            />
          )}

          <textarea
            ref={textareaRef}
            data-composer
            role="combobox"
            aria-expanded={isMentionOpen || slashOpen}
            aria-controls={isMentionOpen ? 'kw-mention-list' : slashOpen ? 'kw-slash-list' : undefined}
            aria-activedescendant={isMentionOpen ? `kw-mention-${safeSelectedIndex}` : slashOpen ? 'kw-slash-idea' : undefined}
            aria-invalid={ideaLine ? true : undefined}
            aria-describedby={ideaLine ? 'kw-idea-line' : undefined}
            aria-autocomplete="list"
            rows={1}
            value={inputVal}
            onFocus={() => setComposerFocused(true)}
            onBlur={() => setComposerFocused(false)}
            onChange={(e) => {
              const val = e.target.value;
              setInputVal(val);
              e.target.style.height = 'auto';
              e.target.style.height = `${Math.min(e.target.scrollHeight, 160)}px`;
              checkMention(val, e.target.selectionStart);
            }}
            onKeyDown={handleKeyDown}
            onPaste={(e) => {
              const pasted = filesFrom(e.clipboardData.items);
              if (pasted.length) {
                e.preventDefault();
                addFiles(pasted);
              }
            }}
            onClick={(e) => {
              checkMention(inputVal, (e.target as HTMLTextAreaElement).selectionStart);
            }}
            onKeyUp={(e) => {
              if (['ArrowLeft', 'ArrowRight', 'Home', 'End'].includes(e.key)) {
                checkMention(inputVal, (e.target as HTMLTextAreaElement).selectionStart);
              }
            }}
            // Phones: "Message {name}" only when it fits on one line, else "Message" (the header names the chat)
            placeholder={placeholder}
            title="Enter to send, Shift+Enter for a new line, @ to mention"

            className={
              phone
                ? `${stacked ? 'basis-full w-full py-2' : 'flex-1 py-2.5'} min-w-0 bg-transparent px-3 text-base leading-6 text-zinc-50 placeholder:text-[#8f88bb] placeholder:whitespace-nowrap placeholder:overflow-hidden placeholder:text-ellipsis focus:outline-none resize-none max-h-40 font-sans`
                : 'flex-1 min-w-0 bg-transparent px-3 py-1.5 text-body text-zinc-50 placeholder-zinc-400 focus:outline-none resize-none max-h-40 font-sans'
            }
          />
          <input
            ref={fileInputRef}
            type="file"
            multiple
            hidden
            tabIndex={-1}
            onChange={(e) => {
              addFiles(filesFrom(e.target.files));
              e.target.value = '';
            }}
          />
          {/* Phone, in use: the tools get their own 44 px row under the text. Elsewhere this wrapper is
              display: contents, so the buttons sit beside the field as before. */}
          <div data-composer-tools className={stacked ? 'basis-full flex items-center h-11' : 'contents'}>
          <button
            type="button"
            onClick={() => fileInputRef.current?.click()}
            disabled={uploadSlotsLeft(uploads) === 0}
            title={uploadSlotsLeft(uploads) === 0 ? `Up to ${MAX_ATTACHMENTS} files per message` : 'Attach files (or drop / paste them here). Max 20 MB each, videos 100 MB.'}
            aria-label="Attach files"
            className={`${phone ? toolPhone : 'w-11 h-11 sm:w-9 sm:h-9 text-zinc-300 hover:text-zinc-50 hover:bg-white/[0.08] focus-visible:outline-sky-400'} rounded-xl flex items-center justify-center flex-shrink-0 disabled:opacity-40 disabled:cursor-not-allowed transition-colors focus-visible:outline focus-visible:outline-2`}
          >
            <Paperclip size={phone ? 18 : 17} aria-hidden="true" />
          </button>
          <button
            type="button"
            onClick={() => {
              markNewSeen('mention');
              insertAt();
            }}
            title="Mention a bot: it answers instead of the default"
            aria-label="Mention a bot"
            className={`relative ${phone ? toolPhone : 'w-11 h-11 sm:w-9 sm:h-9 text-zinc-300 hover:text-zinc-50 hover:bg-white/[0.08] focus-visible:outline-sky-400'} rounded-xl flex items-center justify-center flex-shrink-0 transition-colors focus-visible:outline focus-visible:outline-2`}
          >
            <AtSign size={phone ? 18 : 17} aria-hidden="true" />
            {/* Phone (spec A5): "New" anchors to the @ button's own box, never on the field's border or an Ask chip */}
            {phone && !isStreaming && (
              <NewTag id="mention" className={`absolute -top-1.5 -right-1 ${isMentionOpen ? 'hidden' : ''}`} />
            )}
          </button>
          {stacked && <span className="flex-1" aria-hidden="true" />}
          {/* "New" sits just above the field's top edge, over the "@" button: never on or inside the border.
              Only the send button follows "@" when it shows: 44px on phones / 36px from sm, plus a 6px gap;
              11px is the form's border + padding. Hidden (not marked as seen) while a reply streams, so it
              never sits on the working strip's edge and comes back after the turn; hidden while the mention
              picker is open, so it never peeks under it. */}
          {!phone && !isStreaming && (
            <NewTag
              id="mention"
              className={`absolute bottom-full mb-1.5 [--kw-cbtn-step:50px] sm:[--kw-cbtn-step:42px] ${isMentionOpen ? 'hidden' : ''}`}
              style={{ right: 'calc(11px + var(--kw-cbtn-step))' }}
            />
          )}

          {isStreaming && (
            <button
              type="button"
              onClick={onCancelDiscussion}
              className="w-11 h-11 sm:w-9 sm:h-9 rounded-xl flex items-center justify-center transition-colors flex-shrink-0 bg-red-500/20 hover:bg-red-500/30 text-red-300 border border-red-400/30 cursor-pointer focus-visible:outline focus-visible:outline-2 focus-visible:outline-rose-400"
              title={discussionState?.inProgress ? 'Stop Discussion' : 'Stop'}
              aria-label={discussionState?.inProgress ? 'Stop discussion' : 'Stop'}
            >
              <Square size={12} fill="currentColor" aria-hidden="true" />
            </button>
          )}
          {ideaMode && (
            <span
              data-idea-counter
              aria-hidden="true"
              className={`flex-shrink-0 px-1.5 text-[11px] tabular-nums ${ideaLength(inputVal) > IDEA_MAX ? 'text-rose-300' : 'text-zinc-500'}`}
            >
              {ideaLength(inputVal).toLocaleString('en-US')} / {IDEA_MAX.toLocaleString('en-US')}
            </span>
          )}
          {ideaMode ? (
            <button
              type="submit"
              aria-label="Drop in the jar"
              title="Drop in the jar"
              disabled={!canSend}
              data-send-ready={canSend ? 'true' : 'false'}
              data-idea-send
              className={`w-11 h-11 ${phone ? '' : 'sm:w-9 sm:h-9'} rounded-xl flex items-center justify-center flex-shrink-0 transition-[transform,background-color] duration-150 focus-visible:outline focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-violet-300 ${
                canSend
                  ? 'bg-violet-500 text-white hover:bg-violet-400 active:scale-95 motion-reduce:transition-none motion-reduce:active:scale-100'
                  : 'bg-violet-500/20 text-violet-300/60 cursor-not-allowed'
              }`}
            >
              <JarIcon size={phone ? 18 : 17} />
            </button>
          ) : !(isStreaming && (discussionState?.inProgress || !canSend)) && (
            <button
              type="submit"
              aria-label={stillUploading ? 'Send (waiting for uploads)' : isStreaming ? 'Send (waits for the current reply)' : 'Send'}
              title={stillUploading ? 'Waiting for uploads to finish' : undefined}
              disabled={!canSend}
              data-send-ready={canSend ? 'true' : 'false'}
              className={`${phone ? 'w-11 h-11 focus-visible:outline-[#b8adff]' : 'w-11 h-11 sm:w-9 sm:h-9 focus-visible:outline-sky-300'} rounded-xl flex items-center justify-center flex-shrink-0 transition-[transform,filter,box-shadow,background-color] duration-150 focus-visible:outline focus-visible:outline-2 focus-visible:outline-offset-2 ${
                canSend
                  ? phone
                    ? 'bg-[#6c58e0] text-white hover:bg-[#5845bd] active:scale-95 motion-reduce:transition-none motion-reduce:active:scale-100'
                    : 'bg-accent-gradient text-white shadow-glow hover:brightness-110 active:scale-95 motion-reduce:transition-none motion-reduce:active:scale-100'
                  : phone
                    ? 'bg-[#2c2552] text-[#5a5383] cursor-not-allowed'
                    : 'bg-surface-overlay text-zinc-400 cursor-not-allowed'
              }`}
            >
              <ArrowUp size={phone ? 18 : 17} strokeWidth={2.4} aria-hidden="true" />
            </button>
          )}
          </div>
        </form>
        {ideaLine && (
          <p id="kw-idea-line" role="alert" data-idea-line className="max-w-4xl mx-auto mt-1.5 px-1 text-meta text-rose-300">
            {ideaLine}
          </p>
        )}
        {/* Under the field: who will answer, and what the field understands */}
        <div
          className={`max-w-4xl mx-auto mt-1.5 px-1 flex items-center justify-between gap-3 text-meta text-zinc-400${kbd || (!showAnswers && !showHintText) ? ' hidden' : ''}`}
          data-composer-hints
        >
          {showAnswers && (
            <span
              className={`kw-chip shrink-0 ${target.kind === 'laya' ? 'kw-chip-accent' : target.kind === 'mention' ? 'kw-chip-info' : ''}`}
              title={target.detail}
              data-answer-target={target.kind}
            >
              {target.kind === 'laya' ? <Sparkles size={12} aria-hidden="true" /> : <AtSign size={12} aria-hidden="true" />}
              <span className="text-zinc-400 font-normal">Answers:</span> {target.kind === 'laya' ? target.label : target.label.replace(/^@/, '')}
            </span>
          )}
          {showHintText && (
            <span className="min-w-0 truncate text-right">
              <span className="sm:hidden">@ mention · attach files</span>
              <span className="hidden sm:inline">
                <kbd className="font-sans text-zinc-300">@</kbd> mention a bot · <kbd className="font-sans text-zinc-300">Enter</kbd> send · <kbd className="font-sans text-zinc-300">Shift+Enter</kbd> new line · drop or paste files
              </span>
            </span>
          )}
        </div>
      </div>
    </div>

    {/* Live Grok-style Virtual Machine / Browser Side Pane */}
    <VirtualMachinePane
      key={teamId}
      isOpen={vmOpen}
      onClose={() => {
        if (onToggleVmPane) {
          onToggleVmPane();
        } else {
          setInternalIsVmPaneOpen(false);
        }
      }}
      teamId={teamId}
      teamName={teamName}
      activeUrl={activeUrl}
      pageTitle={pageTitle}
      screenshotBase64={screenshotBase64}
      isRunning={isRunning}
      browserError={browserError}
      terminalLogs={terminalLogs}
      onRefresh={onRefreshVm}
      onRestartVm={onRestartVm}
      onNavigate={onNavigateVm}
      approvalsOff={vmApprovalsOff}
      captchaStep={vmCaptchaStep}
      humanStep={vmHumanStep}
      onStepDone={onVmStepDone}
      solveRequest={vmSolveRequest}
    />
  </div>
</div>
  );
};
