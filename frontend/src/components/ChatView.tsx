import React, { useState, useRef, useEffect } from 'react';
import { ArrowUp, Sparkles, ChevronDown, ChevronRight, Copy, Check, Cpu, Users, Square } from 'lucide-react';
import { ModernAvatar } from './ModernAvatar';
import { ModernApprovalCard, PendingApproval } from './ModernApprovalCard';
import { MentionPopover, MentionItem } from './MentionPopover';
import { playSfx } from '../audio/chiptune';

export interface ChatMessage {
  id: string;
  sender_role: 'user' | 'kwant' | 'system';
  sender_id: string;
  text: string;
  reasoning_content?: string | null;
  tool_calls?: string | null;
  tokens_used?: number;
}

export interface DiscussionState {
  inProgress: boolean;
  teamName?: string;
  currentSpeaker?: string;
  currentTurn?: number;
  totalTurns?: number;
}

export interface ChatViewProps {
  title: string;
  subtitle: string;
  messages: ChatMessage[];
  streamingText: string;
  streamingThinking: string;
  pendingApproval: PendingApproval | null;
  isStreaming: boolean;
  onSendMessage: (text: string) => void;
  onResolveApproval: (id: string, decision: 'allow_once' | 'always_allow' | 'deny') => void;
  kwants?: Array<{ id: string; name: string; title: string; avatar_sprite: string }>;
  teams?: Array<{ id: string; name: string; description?: string; member_kwants?: string[] }>;
  discussionState?: DiscussionState;
  onCancelDiscussion?: () => void;
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

export const ChatView: React.FC<ChatViewProps> = ({
  title,
  subtitle,
  messages,
  streamingText,
  streamingThinking,
  pendingApproval,
  isStreaming,
  onSendMessage,
  onResolveApproval,
  kwants: kwantsProp,
  teams: teamsProp,
  discussionState,
  onCancelDiscussion,
}) => {
  const [inputVal, setInputVal] = useState('');
  const [openThoughts, setOpenThoughts] = useState<Record<string, boolean>>({});
  const [copiedId, setCopiedId] = useState<string | null>(null);
  const scrollRef = useRef<HTMLDivElement>(null);
  const textareaRef = useRef<HTMLTextAreaElement>(null);

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

  // Auto-scroll on new tokens
  useEffect(() => {
    if (scrollRef.current) {
      scrollRef.current.scrollTop = scrollRef.current.scrollHeight;
    }
  }, [messages, streamingText, streamingThinking, pendingApproval]);

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
    if (!inputVal.trim() || isStreaming) return;
    playSfx('click');
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

  return (
    <div className="flex-1 flex flex-col h-full bg-zinc-950 overflow-hidden">
      {/* Channel / Agent Header */}
      <div className="h-14 border-b border-zinc-800/80 px-6 flex items-center justify-between bg-zinc-900/30 backdrop-blur-sm select-none">
        <div>
          <h2 className="text-sm font-semibold text-zinc-100 flex items-center gap-2">
            <span>{title}</span>
          </h2>
          <p className="text-[11px] text-zinc-400 truncate max-w-lg mt-0.5">{subtitle}</p>
        </div>

        <div className="flex items-center gap-2">
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
          <div className="flex items-center gap-1.5 px-2.5 py-1 rounded-full text-xs font-medium text-sky-400 bg-sky-500/10 border border-sky-500/20 shadow-sm">
            <Cpu size={13} />
            <span>Laya Auto-Fleet</span>
          </div>
        </div>
      </div>

      {/* Discussion In-Flight Banner */}
      {discussionState?.inProgress && (
        <div className="px-6 pt-3 select-none flex-shrink-0">
          <div className="flex items-center justify-between px-4 py-2 bg-sky-950/40 border border-sky-800/40 rounded-xl mb-3 text-xs">
            <div className="flex items-center gap-2 text-sky-200">
              <Users size={14} className="text-sky-400 animate-pulse" />
              <span>
                Team Discussion: <strong>{discussionState.currentSpeaker || 'Team member'}</strong> speaking (Turn {discussionState.currentTurn || 1} of {discussionState.totalTurns || 1})
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

      {/* Messages Stream */}
      <div ref={scrollRef} className="flex-1 overflow-y-auto p-6 space-y-6">
        {messages.map((msg) => {
          const isUser = msg.sender_role === 'user';
          const isThoughtOpen = !!openThoughts[msg.id];
          const displayName = isUser ? 'You' : (msg.sender_id && msg.sender_id.length > 20 ? title : msg.sender_id);

          return (
            <div
              key={msg.id}
              className={`flex items-start gap-3.5 max-w-4xl ${isUser ? 'ml-auto flex-row-reverse' : ''}`}
            >
              <ModernAvatar
                role={isUser ? 'user' : title}
                name={displayName}
                size={34}
              />

              <div className={`flex flex-col min-w-0 max-w-2xl ${isUser ? 'items-end' : 'items-start'}`}>
                {/* Header info */}
                <div className="flex items-center gap-2 mb-1 text-[11px] text-zinc-400 font-medium">
                  <span className="text-zinc-200">{displayName}</span>
                  {msg.tokens_used ? (
                    <span className="text-zinc-500 font-mono text-[10px]">({msg.tokens_used} tokens)</span>
                  ) : null}
                </div>

                {/* Collapsible Reasoning Disclosure */}
                {msg.reasoning_content && (
                  <div className="mb-2 w-full">
                    <button
                      onClick={() => toggleThought(msg.id)}
                      className="flex items-center gap-1.5 px-2.5 py-1 rounded-lg text-xs font-mono text-zinc-400 hover:text-zinc-200 bg-zinc-900 border border-zinc-800 transition-colors"
                    >
                      <Sparkles size={12} className="text-violet-400" />
                      <span>{msg.reasoning_content.slice(0, 48)}...</span>
                      {isThoughtOpen ? <ChevronDown size={13} /> : <ChevronRight size={13} />}
                    </button>

                    {isThoughtOpen && (
                      <div className="mt-1.5 p-3 rounded-xl bg-zinc-900/90 border border-zinc-800 text-xs font-mono text-zinc-300 whitespace-pre-wrap leading-relaxed shadow-sm">
                        {msg.reasoning_content}
                      </div>
                    )}
                  </div>
                )}

                {/* Message Body Bubble */}
                <div
                  className={`p-4 rounded-2xl text-sm leading-relaxed whitespace-pre-wrap break-words ${
                    isUser
                      ? 'bg-zinc-800 text-zinc-100 rounded-tr-sm border border-zinc-700/60 shadow-sm'
                      : 'bg-zinc-900/90 text-zinc-200 rounded-tl-sm border border-zinc-800/90 shadow-sm'
                  }`}
                >
                  {renderFormattedText(msg.text)}
                </div>

                {/* Copy action */}
                {!isUser && (
                  <div className="mt-1 flex items-center gap-2 text-zinc-500 text-xs">
                    <button
                      onClick={() => handleCopy(msg.text, msg.id)}
                      className="p-1 hover:text-zinc-300 transition-colors flex items-center gap-1"
                      title="Copy response"
                    >
                      {copiedId === msg.id ? (
                        <>
                          <Check size={12} className="text-emerald-400" />
                          <span className="text-[10px] text-emerald-400">Copied</span>
                        </>
                      ) : (
                        <Copy size={12} />
                      )}
                    </button>
                  </div>
                )}
              </div>
            </div>
          );
        })}

        {/* Live Streaming Assistant Message */}
        {isStreaming && (
          <div className="flex items-start gap-3.5 max-w-4xl">
            <ModernAvatar
              role={discussionState?.inProgress && discussionState.currentSpeaker ? discussionState.currentSpeaker : title}
              name={discussionState?.inProgress && discussionState.currentSpeaker ? discussionState.currentSpeaker : title}
              size={34}
            />
            <div className="flex flex-col items-start min-w-0 max-w-2xl">
              <div className="flex items-center gap-2 mb-1 text-[11px] text-zinc-400 font-medium">
                <span className="text-zinc-200">
                  {discussionState?.inProgress && discussionState.currentSpeaker
                    ? discussionState.currentSpeaker
                    : title}
                </span>
                <span className="inline-flex items-center gap-1 text-sky-400 text-[10px]">
                  <span className="w-1.5 h-1.5 rounded-full bg-sky-400 animate-ping" />
                  {discussionState?.inProgress ? 'Speaking...' : 'Generating...'}
                </span>
              </div>

              {streamingThinking && (
                <div className="mb-2 p-2.5 rounded-xl bg-violet-950/20 border border-violet-500/20 text-xs font-mono text-violet-300/90 flex items-center gap-2 animate-pulse">
                  <Sparkles size={13} className="text-violet-400" />
                  <span>{streamingThinking}</span>
                </div>
              )}

              <div className="p-4 rounded-2xl rounded-tl-sm bg-zinc-900 text-zinc-200 border border-zinc-800 text-sm leading-relaxed whitespace-pre-wrap break-words shadow-sm">
                {renderFormattedText(streamingText)}
                <span className="inline-block w-1.5 h-4 bg-sky-400 ml-1 cursor-blink align-middle" />
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

          {isStreaming ? (
            <button
              type="button"
              onClick={onCancelDiscussion}
              className="w-8 h-8 rounded-xl flex items-center justify-center transition-all flex-shrink-0 bg-red-500/20 hover:bg-red-500/30 text-red-400 border border-red-500/30 shadow-sm cursor-pointer"
              title={discussionState?.inProgress ? 'Stop Discussion' : 'Stop generation'}
            >
              <Square size={12} fill="currentColor" />
            </button>
          ) : (
            <button
              type="submit"
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
  );
};
