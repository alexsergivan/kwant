import React, { useState, useRef, useEffect } from 'react';
import { ArrowDown, ArrowUp, Paperclip, Sparkles, ChevronDown, ChevronRight, Copy, Check, ChevronLeft, Users, Square, Settings2, Folder, GitBranch, Pencil, Pin } from 'lucide-react';
import { ModernAvatar } from './ModernAvatar';
import { ModernApprovalCard, PendingApproval } from './ModernApprovalCard';
import { MentionPopover, MentionItem } from './MentionPopover';
import { MarkdownRenderer } from './MarkdownRenderer';
import { ToolCallsList } from './ToolCallsList';
import { VirtualMachinePane, TerminalLogEntry } from './VirtualMachinePane';
import { playSfx } from '../audio/chiptune';
import { groupDiscussionThreads, stripDiscussionMarkers } from '../discussionThreads';
import { PinnedInstructions, PinnedItem } from './Instructions';
import { DelegationMeta, parseDelegationMeta } from '../threads';
import { QuestionMeta, parseQuestionMeta } from '../questions';
import { HumanStep, HumanStepMeta, parseHumanStepMeta } from '../humanSteps';
import { LiveActivity, hasThinkingText, parseActivityMeta } from '../activity';
import { LiveActivityList, SavedActivityList } from './ActivityList';
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
  uploadAttachment,
  uploadRetryable,
  uploadSlotsLeft,
} from '../attachments';
import { FileTiles, UploadChip } from './Attachments';
import { ranOnLabel } from '../engines';
import { replyTokens } from '../modelUsage';
import { parseRoutingMeta, ranOnAccount, routingLine } from '../spaces';
import { SetupNeededCard, parseSetupMeta } from './SetupNeededCard';
import { ConnectionSuggestionMeta, parseConnectionMeta } from '../connectionSuggestions';
import { routeReasonText } from '../routeReason';
import { BudgetPausedNotice, HandoffCappedNotice, InterruptedNotice, SystemNotice } from './SystemNotice';
import { chainBudgetApi, parseBudgetPaused, parseHandoffCapped } from '../chainBudget';
import { isSystemNotice, parseInterruption, retryTargets } from '../interruptions';
import { consultedNames, hideStreamingDecisions, messageDecisions, parseWorkAhead } from '../decideFirst';
import { approvalScrollAction, fullyVisible, isNearBottom, nextFollowing } from '../approvalVisibility';
import { DecidingLine, DecisionChips, WorkAheadBody } from './Proactive';

/** A swipeable chip row whose right fade edge (phones only, see .kw-scroll-row) goes away once
 *  it is scrolled to the end, or when everything already fits. */
function ScrollRow({ children, ...rest }: React.HTMLAttributes<HTMLDivElement>) {
  const ref = useRef<HTMLDivElement>(null);
  const update = () => {
    const el = ref.current;
    if (el) el.dataset.atEnd = String(el.scrollLeft + el.clientWidth >= el.scrollWidth - 1);
  };
  useEffect(() => {
    const el = ref.current;
    if (!el) return;
    el.addEventListener('scroll', update, { passive: true });
    const resize = typeof ResizeObserver === 'undefined' ? null : new ResizeObserver(update);
    resize?.observe(el);
    return () => {
      el.removeEventListener('scroll', update);
      resize?.disconnect();
    };
  }, []);
  // The chips themselves can change without the row resizing.
  useEffect(update);
  return (
    <div ref={ref} {...rest}>
      {children}
    </div>
  );
}

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
}

/** The open chat's turn is waiting for a free slot. */
export interface QueuedState {
  position: number | null;
  reason: 'agent_busy' | 'server_busy';
  speaker: string;
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
  isStreaming: boolean;
  streamingSpeakerName?: string;
  /** `attachments` are files already uploaded from the composer. */
  /** Resolves to false when the message couldn't be sent. */
  onSendMessage: (text: string, attachments: AttachmentRef[]) => void | Promise<boolean | void>;
  onResolveApproval: (id: string, decision: 'allow_once' | 'always_allow' | 'deny') => void;
  kwants?: Array<{ id: string; name: string; title: string; avatar_sprite: string }>;
  teams?: Array<{ id: string; name: string; description?: string; member_kwants?: string[] }>;
  /** Prompts offered above an empty chat, made from who is in it; none hides the bar. */
  suggestions?: string[];
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
  /** Opens Space settings → Accounts; turns a "nothing can run" reply into a card with a button. */
  onOpenAccounts?: () => void;
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
}

export const renderFormattedText = (text: string): React.ReactNode => {
  if (!text) return null;
  const parts = text.split(/(@[a-zA-Z0-9_-]+)/g);
  return parts.map((part, index) => {
    if (part.startsWith('@') && /^@[a-zA-Z0-9_-]+$/.test(part)) {
      return (
        <span
          key={index}
          className="inline-flex items-center px-1.5 py-0.5 rounded text-sky-400 bg-sky-500/10 font-medium font-mono text-[11px] border border-sky-500/20"
        >
          {part}
        </span>
      );
    }
    return part;
  });
};

/** Why Laya picked this bot. Phones show one line and open the rest on a tap. */
const RoutingLine: React.FC<{ text: string; fallback: boolean }> = ({ text, fallback }) => {
  const [open, setOpen] = useState(false);
  const tone = fallback ? 'text-amber-300/80' : 'text-violet-300/80';
  const icon = <Sparkles size={10} className="inline -mt-0.5 mr-1" aria-hidden="true" />;
  return (
    <>
      <p className={`hidden sm:block -mt-1 mb-1.5 text-[11px] ${tone} break-words`} data-routing-line>
        {icon}
        {text}
      </p>
      <button
        type="button"
        data-compact
        onClick={() => setOpen((v) => !v)}
        aria-expanded={open}
        title={open ? undefined : text}
        className={`sm:hidden -mt-3.5 -mb-1.5 py-3.5 max-w-full text-left text-[11px] ${tone} ${open ? 'break-words' : 'truncate'} rounded focus-visible:outline focus-visible:outline-2 focus-visible:outline-sky-500`}
      >
        {icon}
        {text}
      </button>
    </>
  );
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
  isStreaming,
  streamingSpeakerName,
  onSendMessage,
  onResolveApproval,
  kwants: kwantsProp,
  teams: teamsProp,
  suggestions = [],
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
  vmSolveRequest,
  pinned = [],
  onEditInstructions,
  highlightMessageId,
  onHighlightDone,
  viewingHistory,
  onJumpToLatest,
  subHeader,
  titleExtra,
  queued,
  renderDelegation,
  renderQuestion,
  renderMcpSuggestion,
  renderConnectionSuggestion,
  renderBotAction,
  renderHire,
  onOpenAccounts,
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
  const [uploads, setUploads] = useState<PendingUpload[]>([]);
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
  const activeTeams = teamsProp || internalTeams;

  const mentionItems: MentionItem[] = [
    ...activeKwants.map((k) => ({
      type: 'kwant' as const,
      id: k.id,
      name: k.name,
      title: k.title,
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
          (item.title && item.title.toLowerCase().includes(q))
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

  const handleSelectMention = (item: MentionItem) => {
    if (mentionStartIndex < 0) return;
    const cursorPos = textareaRef.current?.selectionStart ?? inputVal.length;
    const textBefore = inputVal.slice(0, mentionStartIndex);
    const textAfter = inputVal.slice(cursorPos);
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
    setFlashId(highlightMessageId);
    onHighlightDone?.();
  }, [highlightMessageId, messages, openThoughts, onHighlightDone]);
  useEffect(() => {
    if (!flashId) return;
    const t = setTimeout(() => setFlashId(null), 2400);
    return () => clearTimeout(t);
  }, [flashId]);

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
    uploadAttachment(file, (progress) => patchUpload(key, { progress }), abort.signal).then(
      (attachment) => patchUpload(key, { state: 'done', progress: 1, attachment, error: undefined }),
      (err: Error) => {
        if (err.name === 'AbortError') return;
        const retryable = !(err instanceof UploadError) || uploadRetryable(err.status);
        patchUpload(key, { state: 'error', error: err.message, retryable });
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
      added.push({ key, name: file.name || 'pasted file', size: file.size, progress: 0, state: 'uploading' });
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

  const canSend = composerReady(inputVal, uploads);
  const stillUploading = uploads.some((u) => u.state === 'uploading');

  const handleSubmit = (e?: React.FormEvent) => {
    if (e) e.preventDefault();
    // A reply in progress doesn't block the next message (it waits its turn); a team discussion does.
    if (!canSend || (isStreaming && discussionState?.inProgress)) return;
    playSfx('click');
    stickRef.current = true;
    onSendMessage(inputVal.trim(), readyAttachments(uploads));
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

    if (e.key === 'Enter' && !e.shiftKey) {
      e.preventDefault();
      handleSubmit();
    }
  };

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
    const displayName = isUser ? 'You' : (msg.sender_id && msg.sender_id.length > 20 ? title : msg.sender_id);

    return (
      <div
        key={msg.id}
        data-message-id={msg.id}
        className={`flex items-start gap-3.5 w-full max-w-5xl rounded-xl ${isUser ? 'ml-auto flex-row-reverse justify-start' : ''} ${
          flashId === msg.id ? 'kw-flash' : ''
        }`}
      >
        {/* Phones show the avatar inside the name line instead, so the bubble gets the full width */}
        <span className="hidden sm:contents">
          <ModernAvatar
            role={isUser ? 'user' : title}
            name={displayName}
            size={34}
          />
        </span>

        <div className={`flex flex-col min-w-0 ${isUser ? 'max-w-[85%] sm:max-w-2xl items-end' : 'flex-1 max-w-4xl items-start'}`}>
          {/* Header info: one line on phones; the account and token count move under the reply there */}
          <div className="flex items-center gap-2 mb-1.5 text-[11px] text-zinc-400 font-medium max-w-full min-w-0 whitespace-nowrap">
            {!isUser && (
              <span className="sm:hidden shrink-0 inline-flex">
                <ModernAvatar role={title} name={displayName} size={20} />
              </span>
            )}
            <span className="text-zinc-200 shrink-0">{displayName}</span>
            {ranOn && (
              <span className="text-zinc-500 font-mono text-[10px] truncate min-w-0" title={`Answered with ${ranOn}`}>
                {ranOn}
              </span>
            )}
            {account && <span className="hidden sm:inline text-zinc-500 text-[10px] truncate max-w-[12rem]">ran on {account}</span>}
            {tokens && (
              <span className="hidden sm:inline text-zinc-500 font-mono text-[10px] cursor-help" title={tokens.detail}>
                ({tokens.label})<span className="sr-only">: {tokens.detail}</span>
              </span>
            )}
          </div>

          {routed && <RoutingLine text={routingLine(routed, spaceName ?? 'this space')} fallback={!!routed.fallback} />}

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
                className="max-w-full min-w-0 flex items-center gap-2 px-3 py-1.5 rounded-xl text-xs font-mono text-zinc-400 hover:text-zinc-200 bg-zinc-900/80 hover:bg-zinc-900 border border-zinc-800 hover:border-zinc-700/80 transition-all cursor-pointer shadow-sm group"
              >
                <Sparkles size={13} className="text-violet-400 group-hover:rotate-12 transition-transform flex-shrink-0" />
                <span className="truncate min-w-0 sm:max-w-md">
                  {msg.reasoning_content.split('\n')[0].replace(/^🔮\s*/, '')}
                </span>
                {isThoughtOpen ? <ChevronDown size={13} className="text-zinc-500 flex-shrink-0" /> : <ChevronRight size={13} className="text-zinc-500 flex-shrink-0" />}
              </button>

              {isThoughtOpen && (
                <div className="mt-1.5 p-3.5 rounded-xl bg-zinc-950/90 border border-zinc-800 text-xs font-mono text-zinc-300 whitespace-pre-wrap leading-relaxed shadow-inner max-h-80 overflow-y-auto">
                  {msg.reasoning_content}
                </div>
              )}
            </div>
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
              <div className="whitespace-pre-wrap break-words">{renderFormattedText(msg.text)}</div>
            ) : workAhead ? (
              <WorkAheadBody messageId={msg.id} text={text} />
            ) : (
              <MarkdownRenderer content={text} />
            )}
          </div>
          )}
          {files.length > 0 && <FileTiles files={files} align={isUser ? 'end' : 'start'} />}

          {decisions.length > 0 && (
            <DecisionChips
              messageId={msg.id}
              decisions={decisions}
              consulted={consultedNames(msg.meta)}
              onChange={onChangeDecision && !shownDecisions?.readOnly ? (index, picked) => onChangeDecision(msg, index, picked) : undefined}
            />
          )}

          {!isUser && msg.tool_calls && !savedActivity && <ToolCallsList raw={msg.tool_calls} />}

          {/* Copy action */}
          {!isUser && !filesOnly && (
            <div className="mt-1.5 flex items-center gap-2 text-zinc-500 text-xs max-w-full min-w-0">
              <button
                type="button"
                onClick={() => handleCopy(text, msg.id)}
                className="px-2 py-1 rounded-md hover:bg-zinc-900 hover:text-zinc-300 transition-colors flex items-center gap-1.5 text-[11px] cursor-pointer"
                title="Copy response"
              >
                {copiedId === msg.id ? (
                  <>
                    <Check size={12} className="text-emerald-400" />
                    <span className="text-emerald-400 font-medium">Copied</span>
                  </>
                ) : (
                  <>
                    <Copy size={12} />
                    <span className="sm:hidden">Copy</span>
                    <span className="hidden sm:inline">Copy markdown</span>
                  </>
                )}
              </button>
              {(account || tokens) && (
                <span className="sm:hidden min-w-0 truncate text-[10px]" title={tokens?.detail}>
                  {[account && `ran on ${account}`, tokens?.label].filter(Boolean).join(' · ')}
                </span>
              )}
            </div>
          )}
        </div>
      </div>
    );
  };

  return (
    <div className="flex-1 flex flex-col h-full bg-zinc-950 overflow-hidden">
      {/* Channel / bot header */}
      <div className="h-14 shrink-0 border-b border-zinc-800/80 px-2 sm:px-6 gap-2 flex items-center justify-between bg-zinc-900/30 backdrop-blur-sm select-none">
        {onBack && (
          <button
            type="button"
            onClick={onBack}
            aria-label="Back to chats"
            className="shrink-0 p-2 -ml-0.5 rounded-lg text-zinc-300 hover:text-zinc-100 hover:bg-zinc-800 focus-visible:outline focus-visible:outline-2 focus-visible:outline-sky-500"
          >
            <ChevronLeft size={18} />
          </button>
        )}
        <div className="min-w-0 flex-1">
          <h2 className="text-sm font-semibold text-zinc-100 flex items-center gap-2 min-w-0">
            <span className="truncate">{title}</span>
            {titleExtra}
          </h2>
          <p className="text-[11px] text-zinc-400 truncate max-w-lg mt-0.5">{subtitle}</p>
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
                  className="kw-head-btn flex items-center justify-center gap-1.5 max-sm:min-w-11 px-2.5 py-1 rounded-lg text-xs font-medium text-zinc-300 bg-zinc-800/80 hover:bg-zinc-700/80 hover:text-zinc-100 border border-zinc-700/60 transition-colors shadow-sm cursor-pointer"
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
              className="kw-head-btn flex items-center justify-center gap-1.5 max-md:min-w-11 px-2.5 py-1 rounded-lg text-xs font-medium text-zinc-300 bg-zinc-800/80 hover:bg-zinc-700/80 hover:text-zinc-100 border border-zinc-700/60 transition-colors shadow-sm cursor-pointer"
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
              className="kw-head-btn flex items-center gap-1.5 px-2.5 py-1 rounded-lg text-xs font-medium text-zinc-300 bg-zinc-800/80 hover:bg-zinc-700/80 hover:text-zinc-100 border border-zinc-700/60 transition-colors shadow-sm cursor-pointer"
              title={`Edit ${title}'s name, avatar, model and persona`}
              aria-label={`Edit ${title}`}
            >
              <Pencil size={12} className="text-zinc-400" />
              <span className="hidden sm:inline">Edit</span>
            </button>
          )}

          {isStreaming && !discussionState?.inProgress && onCancelDiscussion && (
            <button
              type="button"
              onClick={onCancelDiscussion}
              className="flex items-center gap-1.5 px-2.5 py-1 rounded-full text-xs font-medium text-red-300 bg-red-500/10 hover:bg-red-500/20 border border-red-500/30 transition-colors shadow-sm cursor-pointer"
              title="Stop generation"
            >
              <Square size={10} fill="currentColor" />
              <span>Stop</span>
            </button>
          )}
          {/* Sandbox VM & Live Browser Toggle Button */}
          <button
            type="button"
            onClick={() => {
              playSfx('click');
              toggleVm();
            }}
            className={`kw-head-btn flex items-center justify-center gap-1.5 max-md:min-w-11 px-2.5 py-1 rounded-full text-xs font-medium transition-colors border shadow-sm cursor-pointer ${
              vmOpen
                ? 'text-sky-300 bg-sky-500/20 border-sky-500/40'
                : 'text-zinc-300 bg-zinc-900 hover:bg-zinc-800 border-zinc-700/60 hover:text-zinc-100'
            }`}
            title="Show or hide the bot's browser"
            aria-label="Bot browser"
            aria-pressed={vmOpen}
          >
            <span className="text-xs" aria-hidden="true">🖥️</span>
            <span className="hidden sm:inline">Bot browser</span>
          </button>
        </div>
      </div>

      {subHeader}

      <PinnedInstructions items={pinned} onEdit={onEditInstructions} />

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
            className="flex-1 overflow-y-auto px-3 py-4 sm:p-6 space-y-5 sm:space-y-6"
            role="log"
            aria-label={`Messages in ${title}`}
            aria-busy={isStreaming}
          >
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
                ) : awaitingAnswer ? (
                  <span className="inline-flex items-center gap-1 text-amber-300 text-[10px]">
                    <span className="w-1.5 h-1.5 rounded-full bg-amber-300" aria-hidden="true" />
                    {awaitingBrowser ? 'Waiting for you in the browser' : 'Waiting for your answer'}
                  </span>
                ) : (
                  <span className="inline-flex items-center gap-1 text-sky-400 text-[10px]">
                    <span className="w-1.5 h-1.5 rounded-full bg-sky-400 animate-ping motion-reduce:animate-none" aria-hidden="true" />
                    {discussionState?.inProgress ? 'Speaking...' : 'Generating...'}
                  </span>
                )}
              </div>

              {liveActivity && <LiveActivityList live={liveActivity} idBase={`live-${liveActivity.turnId}`} />}

              {streamingThinking && (
                <div className="mb-2 p-2.5 rounded-xl bg-violet-950/20 border border-violet-500/20 text-xs font-mono text-violet-300/90 flex items-center gap-2 animate-pulse motion-reduce:animate-none max-w-full">
                  <Sparkles size={13} className="text-violet-400 flex-shrink-0" />
                  <span className="truncate">{streamingThinking}</span>
                </div>
              )}

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
                ) : (
                  <span className="inline-flex gap-1 py-1.5" aria-label="Waiting for the first words">
                    {[0, 1, 2].map((i) => (
                      <span
                        key={i}
                        className="w-1.5 h-1.5 rounded-full bg-zinc-500 animate-bounce motion-reduce:animate-none"
                        style={{ animationDelay: `${i * 150}ms` }}
                      />
                    ))}
                  </span>
                )}
              </div>
            </div>
          </div>
        )}

        {/* Pending Security Approval Ticket */}
        {pendingApprovals.map((approval) => (
          <ModernApprovalCard key={approval.id} approval={approval} onResolve={onResolveApproval} />
        ))}
      </div>

      {/* Suggested prompts for who is in this chat: one swipeable row on phones */}
      {messages.length <= 1 && suggestions.length > 0 && (
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
              className="shrink-0 whitespace-nowrap px-2.5 py-1 rounded-full text-xs text-zinc-400 bg-zinc-900 border border-zinc-800 hover:border-zinc-700 hover:text-zinc-200 transition-all shadow-sm focus-visible:outline focus-visible:outline-2 focus-visible:outline-sky-500"
            >
              {prompt}
            </button>
          ))}
        </ScrollRow>
      )}

      {/* Modern Composer Input Bar */}
      <div className="px-3 pt-3 pb-[max(0.75rem,env(safe-area-inset-bottom))] sm:p-4 border-t border-zinc-800/80 bg-zinc-900/30">
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

        {approvalOffscreen && pendingApprovals.length > 0 && (
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
          className={`relative max-w-4xl mx-auto flex flex-wrap items-end gap-2 bg-zinc-900 border rounded-2xl p-2 shadow-sm transition-all ${
            dragOver ? 'border-sky-500/70 bg-sky-950/30' : 'border-zinc-800 focus-within:border-zinc-700'
          }`}
        >
          {dragOver && (
            <div className="absolute inset-0 z-10 rounded-2xl border-2 border-dashed border-sky-400/70 bg-zinc-950/80 flex items-center justify-center gap-2 text-sm font-medium text-sky-200 pointer-events-none">
              <Paperclip size={15} aria-hidden="true" /> Drop files to attach
            </div>
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
          {isMentionOpen && (
            <MentionPopover
              items={matchingItems}
              selectedIndex={safeSelectedIndex}
              onSelect={handleSelectMention}
            />
          )}

          <textarea
            ref={textareaRef}
            rows={1}
            value={inputVal}
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
            // Phones drop the space name so the 16px placeholder fits on one line
            placeholder={spaceName && !narrow ? `Message ${title} in ${spaceName}` : `Message ${title}`}
            title="Enter to send, Shift+Enter for a new line, @ to mention"

            className="flex-1 min-w-0 bg-transparent px-3 py-1.5 text-sm text-zinc-100 placeholder-zinc-500 focus:outline-none resize-none max-h-40 leading-relaxed font-sans"
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
          <button
            type="button"
            onClick={() => fileInputRef.current?.click()}
            disabled={uploadSlotsLeft(uploads) === 0}
            title={uploadSlotsLeft(uploads) === 0 ? `Up to ${MAX_ATTACHMENTS} files per message` : 'Attach files (or drop / paste them here). Max 20 MB each.'}
            aria-label="Attach files"
            className="w-8 h-8 rounded-xl flex items-center justify-center flex-shrink-0 text-zinc-400 hover:text-zinc-100 hover:bg-zinc-800 disabled:opacity-40 disabled:cursor-not-allowed transition-colors focus-visible:outline focus-visible:outline-2 focus-visible:outline-sky-500"
          >
            <Paperclip size={16} aria-hidden="true" />
          </button>

          {isStreaming && (
            <button
              type="button"
              onClick={onCancelDiscussion}
              className="w-8 h-8 rounded-xl flex items-center justify-center transition-all flex-shrink-0 bg-red-500/20 hover:bg-red-500/30 text-red-400 border border-red-500/30 shadow-sm cursor-pointer focus-visible:outline focus-visible:outline-2 focus-visible:outline-rose-500"
              title={discussionState?.inProgress ? 'Stop Discussion' : 'Stop generation'}
              aria-label={discussionState?.inProgress ? 'Stop discussion' : 'Stop the reply'}
            >
              <Square size={12} fill="currentColor" aria-hidden="true" />
            </button>
          )}
          {!(isStreaming && (discussionState?.inProgress || !canSend)) && (
            <button
              type="submit"
              aria-label={stillUploading ? 'Send (waiting for uploads)' : isStreaming ? 'Send (waits for the current reply)' : 'Send'}
              title={stillUploading ? 'Waiting for uploads to finish' : undefined}
              disabled={!canSend}
              className={`w-8 h-8 rounded-xl flex items-center justify-center transition-all flex-shrink-0 ${
                canSend
                  ? 'bg-sky-500 text-white hover:bg-sky-400 shadow-sm'
                  : 'bg-zinc-800 text-zinc-500 cursor-not-allowed'
              }`}
            >
              <ArrowUp size={16} strokeWidth={2.4} />
            </button>
          )}
        </form>
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
      solveRequest={vmSolveRequest}
    />
  </div>
</div>
  );
};
