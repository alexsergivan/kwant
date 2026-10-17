import React, { useState, useRef, useEffect } from 'react';
import { ArrowUp, Sparkles, ChevronDown, ChevronRight, Copy, Check, Cpu, Users, Square, Settings2, Folder, GitBranch, Pencil, Pin } from 'lucide-react';
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
}

export interface ChatViewProps {
  title: string;
  subtitle: string;
  messages: ChatMessage[];
  streamingText: string;
  streamingThinking: string;
  pendingApproval: PendingApproval | null;
  isStreaming: boolean;
  streamingSpeakerName?: string;
  onSendMessage: (text: string) => void;
  onResolveApproval: (id: string, decision: 'allow_once' | 'always_allow' | 'deny') => void;
  kwants?: Array<{ id: string; name: string; title: string; avatar_sprite: string }>;
  teams?: Array<{ id: string; name: string; description?: string; member_kwants?: string[] }>;
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
  /** Pinned team/channel instructions, shown collapsed above the messages. */
  pinned?: PinnedItem[];
  /** Opens the channel's instructions editor; only for channels. */
  onEditInstructions?: () => void;
  /** A message to scroll to and flash (from search). */
  highlightMessageId?: string | null;
  onHighlightDone?: () => void;
  /** Under the header: the agent's conversations and the open thread's controls. */
  subHeader?: React.ReactNode;
  /** Next to the title, e.g. "Working on …" while the agent has background turns. */
  titleExtra?: React.ReactNode;
  /** Set while the open chat's turn waits in the queue. */
  queued?: QueuedState | null;
  /** Draws a delegation note or result (messages with delegation `meta`). */
  renderDelegation?: (msg: ChatMessage, meta: DelegationMeta, flash: boolean) => React.ReactNode;
  /** Draws an agent's question card (messages with `ask_user` meta). */
  renderQuestion?: (msg: ChatMessage, meta: QuestionMeta, flash: boolean) => React.ReactNode;
  /** The open chat's turn is paused on a question: it is waiting for the person, not working. */
  awaitingAnswer?: boolean;
  /** Changes when another chat opens, so the view jumps to its bottom. */
  contextKey?: string;
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

const LayaRoutingCard: React.FC<{ routing: RoutingState }> = ({ routing }) => {
  const deciding = routing.phase === 'deciding';
  const isDiscuss = routing.mode === 'discuss';

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
              <span>Choosing who's best to answer your question</span>
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
              {routing.reason && <span className="text-xs text-zinc-400">{routing.reason}</span>}
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

export const ChatView: React.FC<ChatViewProps> = ({
  title,
  subtitle,
  messages,
  streamingText,
  streamingThinking,
  pendingApproval,
  isStreaming,
  streamingSpeakerName,
  onSendMessage,
  onResolveApproval,
  kwants: kwantsProp,
  teams: teamsProp,
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
  pinned = [],
  onEditInstructions,
  highlightMessageId,
  onHighlightDone,
  subHeader,
  titleExtra,
  queued,
  renderDelegation,
  renderQuestion,
  awaitingAnswer,
  contextKey,
}) => {
  const [internalIsVmPaneOpen, setInternalIsVmPaneOpen] = useState(false);
  const vmOpen = isVmPaneOpen !== undefined ? isVmPaneOpen : internalIsVmPaneOpen;
  const toggleVm = onToggleVmPane || (() => setInternalIsVmPaneOpen((prev) => !prev));

  const [inputVal, setInputVal] = useState('');
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
  const onScroll = () => {
    const el = scrollRef.current;
    if (el) stickRef.current = el.scrollHeight - el.scrollTop - el.clientHeight < 80;
  };
  useEffect(() => {
    stickRef.current = true;
  }, [contextKey]);
  useEffect(() => {
    if (scrollRef.current && stickRef.current) {
      scrollRef.current.scrollTop = scrollRef.current.scrollHeight;
    }
  }, [messages, streamingText, streamingThinking, pendingApproval, routingState, queued, contextKey]);

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

  const handleSubmit = (e?: React.FormEvent) => {
    if (e) e.preventDefault();
    // A reply in progress doesn't block the next message (it waits its turn); a team discussion does.
    if (!inputVal.trim() || (isStreaming && discussionState?.inProgress)) return;
    playSfx('click');
    stickRef.current = true;
    onSendMessage(inputVal.trim());
    setInputVal('');
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

  const renderMessage = (msg: ChatMessage) => {
    const meta = msg.meta && renderDelegation ? parseDelegationMeta(msg.meta) : null;
    if (meta && renderDelegation) return <React.Fragment key={msg.id}>{renderDelegation(msg, meta, flashId === msg.id)}</React.Fragment>;
    const question = msg.meta && renderQuestion ? parseQuestionMeta(msg.meta) : null;
    if (question && renderQuestion) return <React.Fragment key={msg.id}>{renderQuestion(msg, question, flashId === msg.id)}</React.Fragment>;
    const isUser = msg.sender_role === 'user';
    const isThoughtOpen = !!openThoughts[msg.id];
    const displayName = isUser ? 'You' : (msg.sender_id && msg.sender_id.length > 20 ? title : msg.sender_id);

    return (
      <div
        key={msg.id}
        data-message-id={msg.id}
        className={`flex items-start gap-3.5 w-full max-w-5xl rounded-xl ${isUser ? 'ml-auto flex-row-reverse justify-start' : ''} ${
          flashId === msg.id ? 'kw-flash' : ''
        }`}
      >
        <ModernAvatar
          role={isUser ? 'user' : title}
          name={displayName}
          size={34}
        />

        <div className={`flex flex-col min-w-0 ${isUser ? 'max-w-2xl items-end' : 'flex-1 max-w-4xl items-start'}`}>
          {/* Header info */}
          <div className="flex items-center gap-2 mb-1.5 text-[11px] text-zinc-400 font-medium">
            <span className="text-zinc-200">{displayName}</span>
            {msg.tokens_used ? (
              <span className="text-zinc-500 font-mono text-[10px]">({msg.tokens_used} tokens)</span>
            ) : null}
          </div>

          {/* Collapsible Reasoning Disclosure */}
          {msg.reasoning_content && (
            <div className="mb-2 max-w-full">
              <button
                type="button"
                onClick={() => toggleThought(msg.id)}
                className="flex items-center gap-2 px-3 py-1.5 rounded-xl text-xs font-mono text-zinc-400 hover:text-zinc-200 bg-zinc-900/80 hover:bg-zinc-900 border border-zinc-800 hover:border-zinc-700/80 transition-all cursor-pointer shadow-sm group"
              >
                <Sparkles size={13} className="text-violet-400 group-hover:rotate-12 transition-transform flex-shrink-0" />
                <span className="truncate max-w-xs sm:max-w-md">
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
          <div
            className={`p-4 sm:p-5 rounded-2xl text-sm leading-relaxed shadow-sm w-full ${
              isUser
                ? 'bg-zinc-800 text-zinc-100 rounded-tr-sm border border-zinc-700/60 max-w-2xl'
                : 'bg-zinc-900/60 text-zinc-200 rounded-tl-sm border border-zinc-800/80'
            }`}
          >
            {isUser ? (
              <div className="whitespace-pre-wrap break-words">{renderFormattedText(msg.text)}</div>
            ) : (
              <MarkdownRenderer content={msg.text} />
            )}
          </div>

          {!isUser && msg.tool_calls && <ToolCallsList raw={msg.tool_calls} />}

          {/* Copy action */}
          {!isUser && (
            <div className="mt-1.5 flex items-center gap-2 text-zinc-500 text-xs">
              <button
                type="button"
                onClick={() => handleCopy(msg.text, msg.id)}
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
                    <span>Copy markdown</span>
                  </>
                )}
              </button>
            </div>
          )}
        </div>
      </div>
    );
  };

  return (
    <div className="flex-1 flex flex-col h-full bg-zinc-950 overflow-hidden">
      {/* Channel / Agent Header */}
      <div className="h-14 border-b border-zinc-800/80 px-6 flex items-center justify-between bg-zinc-900/30 backdrop-blur-sm select-none">
        <div>
          <h2 className="text-sm font-semibold text-zinc-100 flex items-center gap-2 min-w-0">
            <span className="truncate">{title}</span>
            {titleExtra}
          </h2>
          <p className="text-[11px] text-zinc-400 truncate max-w-lg mt-0.5">{subtitle}</p>
        </div>

        <div className="flex items-center gap-3">
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
            <div className="flex items-center gap-2 pl-3 border-l border-zinc-800">
              <div className="flex -space-x-2 items-center px-1 py-1 -my-0.5">
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
                  className="flex items-center gap-1.5 px-2.5 py-1 rounded-lg text-xs font-medium text-zinc-300 bg-zinc-800/80 hover:bg-zinc-700/80 hover:text-zinc-100 border border-zinc-700/60 transition-colors shadow-sm cursor-pointer"
                  title="Manage Team members and settings"
                >
                  <Settings2 size={12} className="text-zinc-400" />
                  <span>Manage</span>
                </button>
              )}
            </div>
          )}

          {onEditInstructions && pinned.length === 0 && (
            <button
              type="button"
              onClick={onEditInstructions}
              className="flex items-center gap-1.5 px-2.5 py-1 rounded-lg text-xs font-medium text-zinc-300 bg-zinc-800/80 hover:bg-zinc-700/80 hover:text-zinc-100 border border-zinc-700/60 transition-colors shadow-sm cursor-pointer"
              title="Pin instructions every kwant reads in this channel"
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
              className="flex items-center gap-1.5 px-2.5 py-1 rounded-lg text-xs font-medium text-zinc-300 bg-zinc-800/80 hover:bg-zinc-700/80 hover:text-zinc-100 border border-zinc-700/60 transition-colors shadow-sm cursor-pointer"
              title={`Edit ${title}'s name, avatar, model and persona`}
            >
              <Pencil size={12} className="text-zinc-400" />
              <span>Edit</span>
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
            className={`flex items-center gap-1.5 px-2.5 py-1 rounded-full text-xs font-medium transition-colors border shadow-sm cursor-pointer ${
              vmOpen
                ? 'text-sky-300 bg-sky-500/20 border-sky-500/40'
                : 'text-zinc-300 bg-zinc-900 hover:bg-zinc-800 border-zinc-700/60 hover:text-zinc-100'
            }`}
            title="Toggle Bot Browser Pane"
          >
            <span className="text-xs">🖥️</span>
            <span>Bot Browser</span>
          </button>

          <div className="flex items-center gap-1.5 px-2.5 py-1 rounded-full text-xs font-medium text-sky-400 bg-sky-500/10 border border-sky-500/20 shadow-sm">
            <Cpu size={13} />
            <span>Laya Auto-Fleet</span>
          </div>
        </div>
      </div>

      {subHeader}

      <PinnedInstructions items={pinned} onEdit={onEditInstructions} />

      {/* Discussion In-Flight Banner */}
      {discussionState?.inProgress && (
        <div className="px-6 pt-3 select-none flex-shrink-0">
          <div className="flex items-center justify-between px-4 py-2 bg-sky-950/40 border border-sky-800/40 rounded-xl mb-3 text-xs">
            <div className="flex items-center gap-2 text-sky-200">
              <Users size={14} className="text-sky-400 animate-pulse" />
              <span>
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
                className="flex items-center gap-1 px-2.5 py-1 bg-red-500/20 hover:bg-red-500/30 text-red-300 rounded border border-red-500/30 transition-colors cursor-pointer"
              >
                <Square size={11} fill="currentColor" />
                <span>Stop Discussion</span>
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
            className="flex-1 overflow-y-auto p-4 sm:p-6 space-y-6"
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
                className="ml-[48px] flex items-center gap-2.5 px-3 py-1.5 rounded-xl text-xs text-zinc-400 hover:text-zinc-200 bg-zinc-900/60 hover:bg-zinc-900 border border-zinc-800 hover:border-zinc-700/80 transition-all cursor-pointer"
              >
                {isThreadOpen ? <ChevronDown size={13} className="text-zinc-500" /> : <ChevronRight size={13} className="text-zinc-500" />}
                <span className="flex -space-x-1.5">
                  {item.speakers.map((name) => (
                    <ModernAvatar key={name} role={name} name={name} size={18} />
                  ))}
                </span>
                <span>
                  Team discussed • {item.turns.length} {item.turns.length === 1 ? 'turn' : 'turns'} • {item.speakers.join(', ')}
                </span>
              </button>
              {isThreadOpen && (
                <div className="mt-4 ml-[30px] pl-4 border-l border-zinc-800 space-y-6">
                  {item.turns.map(renderMessage)}
                </div>
              )}
            </div>
          );
        })}

        {/* Laya routing: which agent will answer */}
        {isStreaming && routingState && !discussionState?.inProgress && (
          <LayaRoutingCard routing={routingState} />
        )}

        {/* Live Streaming Assistant Message (or its place in the queue, in the same frame) */}
        {isStreaming && routingState?.phase !== 'deciding' && (
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
                    Waiting for your answer
                  </span>
                ) : (
                  <span className="inline-flex items-center gap-1 text-sky-400 text-[10px]">
                    <span className="w-1.5 h-1.5 rounded-full bg-sky-400 animate-ping motion-reduce:animate-none" aria-hidden="true" />
                    {discussionState?.inProgress ? 'Speaking...' : 'Generating...'}
                  </span>
                )}
              </div>

              {streamingThinking && (
                <div className="mb-2 p-2.5 rounded-xl bg-violet-950/20 border border-violet-500/20 text-xs font-mono text-violet-300/90 flex items-center gap-2 animate-pulse max-w-full">
                  <Sparkles size={13} className="text-violet-400 flex-shrink-0" />
                  <span className="truncate">{streamingThinking}</span>
                </div>
              )}

              <div className="p-4 sm:p-5 min-h-[3.5rem] rounded-2xl rounded-tl-sm bg-zinc-900/60 text-zinc-200 border border-zinc-800/80 text-sm leading-relaxed shadow-sm w-full">
                {queued ? (
                  <p className="text-zinc-300" role="status">
                    {queued.position ? <strong className="text-amber-200">#{queued.position} in line.</strong> : <strong className="text-amber-200">In line.</strong>}{' '}
                    {queued.reason === 'server_busy'
                      ? 'kwnt is running as many turns as it allows right now; this one starts as soon as a slot frees up.'
                      : `${queued.speaker} is busy with other work; this starts as soon as one of those finishes.`}
                  </p>
                ) : streamingText ? (
                  <MarkdownRenderer content={stripDiscussionMarkers(streamingText)} isStreaming={true} />
                ) : awaitingAnswer ? (
                  <p className="text-zinc-400" role="status">
                    Paused on the question above. Answer it there; this reply carries on from it.
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
        {pendingApproval && (
          <ModernApprovalCard
            approval={pendingApproval}
            onResolve={onResolveApproval}
          />
        )}
      </div>

      {/* Suggestion Prompts Pill Bar */}
      {messages.length <= 1 && (
        <div className="px-6 py-2 flex flex-wrap items-center gap-2 select-none">
          <span className="text-[11px] font-medium text-zinc-500">Suggested:</span>
          {[
            "Analyze system architecture tradeoffs",
            "Write a fast Python script for data processing",
            "Review git status and security policies",
          ].map((prompt, idx) => (
            <button
              key={idx}
              onClick={() => handlePromptSuggestion(prompt)}
              className="px-2.5 py-1 rounded-full text-xs text-zinc-400 bg-zinc-900 border border-zinc-800 hover:border-zinc-700 hover:text-zinc-200 transition-all shadow-sm"
            >
              {prompt}
            </button>
          ))}
        </div>
      )}

      {/* Modern Composer Input Bar */}
      <div className="p-4 border-t border-zinc-800/80 bg-zinc-900/30">
        {teamMembers && teamMembers.length > 0 && (
          <div className="max-w-4xl mx-auto mb-2.5 flex items-center gap-1.5 overflow-x-auto text-xs select-none">
            <span className="text-[11px] font-medium text-zinc-500 whitespace-nowrap">Ask:</span>
            {teamMembers.map((m) => (
              <button
                key={m.id}
                type="button"
                onClick={() => handleInsertTag(`@${m.name}`)}
                className="flex items-center gap-1.5 px-2.5 py-1 rounded-full bg-zinc-800/90 hover:bg-zinc-700 text-zinc-300 hover:text-zinc-100 border border-zinc-700/60 transition-all text-xs cursor-pointer shadow-sm active:scale-95"
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
                className="flex items-center gap-1.5 px-2.5 py-1 rounded-full bg-sky-950/60 hover:bg-sky-900/60 text-sky-300 hover:text-sky-200 border border-sky-800/60 transition-all text-xs cursor-pointer shadow-sm active:scale-95"
                title={`Ask whole team @${teamName}`}
              >
                <Users size={12} className="text-sky-400" />
                <span>@{teamName} (Team)</span>
              </button>
            )}
          </div>
        )}

        <form
          onSubmit={handleSubmit}
          className="relative max-w-4xl mx-auto flex items-end gap-2 bg-zinc-900 border border-zinc-800 focus-within:border-zinc-700 rounded-2xl p-2 shadow-sm transition-all"
        >
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
            onClick={(e) => {
              checkMention(inputVal, (e.target as HTMLTextAreaElement).selectionStart);
            }}
            onKeyUp={(e) => {
              if (['ArrowLeft', 'ArrowRight', 'Home', 'End'].includes(e.key)) {
                checkMention(inputVal, (e.target as HTMLTextAreaElement).selectionStart);
              }
            }}
            placeholder={`Message ${title}... (Enter to send, Shift+Enter for newline, @ to mention)`}
            className="flex-1 bg-transparent px-3 py-1.5 text-sm text-zinc-100 placeholder-zinc-500 focus:outline-none resize-none max-h-40 leading-relaxed font-sans"
          />

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
          {!(isStreaming && (discussionState?.inProgress || !inputVal.trim())) && (
            <button
              type="submit"
              aria-label={isStreaming ? 'Send (waits for the current reply)' : 'Send'}
              disabled={!inputVal.trim()}
              className={`w-8 h-8 rounded-xl flex items-center justify-center transition-all flex-shrink-0 ${
                inputVal.trim()
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
    />
  </div>
</div>
  );
};
