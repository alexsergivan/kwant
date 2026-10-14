import React, { useState, useRef, useEffect } from 'react';
import { ArrowUp, Sparkles, ChevronDown, ChevronRight, Copy, Check, Cpu } from 'lucide-react';
import { ModernAvatar } from './ModernAvatar';
import { ModernApprovalCard, PendingApproval } from './ModernApprovalCard';
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

interface ChatViewProps {
  title: string;
  subtitle: string;
  messages: ChatMessage[];
  streamingText: string;
  streamingThinking: string;
  pendingApproval: PendingApproval | null;
  isStreaming: boolean;
  onSendMessage: (text: string) => void;
  onResolveApproval: (id: string, decision: 'allow_once' | 'always_allow' | 'deny') => void;
}

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
}) => {
  const [inputVal, setInputVal] = useState('');
  const [openThoughts, setOpenThoughts] = useState<Record<string, boolean>>({});
  const [copiedId, setCopiedId] = useState<string | null>(null);
  const scrollRef = useRef<HTMLDivElement>(null);
  const textareaRef = useRef<HTMLTextAreaElement>(null);

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
    if (textareaRef.current) {
      textareaRef.current.style.height = 'auto';
    }
  };

  const handleKeyDown = (e: React.KeyboardEvent<HTMLTextAreaElement>) => {
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
          <div className="flex items-center gap-1.5 px-2.5 py-1 rounded-full text-xs font-medium text-sky-400 bg-sky-500/10 border border-sky-500/20 shadow-sm">
            <Cpu size={13} />
            <span>Laya Auto-Fleet</span>
          </div>
        </div>
      </div>

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
                  {msg.text}
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
            <ModernAvatar role={title} name={title} size={34} />
            <div className="flex flex-col items-start min-w-0 max-w-2xl">
              <div className="flex items-center gap-2 mb-1 text-[11px] text-zinc-400 font-medium">
                <span className="text-zinc-200">{title}</span>
                <span className="inline-flex items-center gap-1 text-sky-400 text-[10px]">
                  <span className="w-1.5 h-1.5 rounded-full bg-sky-400 animate-ping" />
                  Generating...
                </span>
              </div>

              {streamingThinking && (
                <div className="mb-2 p-2.5 rounded-xl bg-violet-950/20 border border-violet-500/20 text-xs font-mono text-violet-300/90 flex items-center gap-2 animate-pulse">
                  <Sparkles size={13} className="text-violet-400" />
                  <span>{streamingThinking}</span>
                </div>
              )}

              <div className="p-4 rounded-2xl rounded-tl-sm bg-zinc-900 text-zinc-200 border border-zinc-800 text-sm leading-relaxed whitespace-pre-wrap break-words shadow-sm">
                {streamingText}
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
          className="max-w-4xl mx-auto flex items-end gap-2 bg-zinc-900 border border-zinc-800 focus-within:border-zinc-700 rounded-2xl p-2 shadow-sm transition-all"
        >
          <textarea
            ref={textareaRef}
            rows={1}
            value={inputVal}
            onChange={(e) => {
              setInputVal(e.target.value);
              e.target.style.height = 'auto';
              e.target.style.height = `${Math.min(e.target.scrollHeight, 160)}px`;
            }}
            onKeyDown={handleKeyDown}
            placeholder={`Message ${title}... (Enter to send, Shift+Enter for newline)`}
            className="flex-1 bg-transparent px-3 py-1.5 text-sm text-zinc-100 placeholder-zinc-500 focus:outline-none resize-none max-h-40 leading-relaxed font-sans"
          />

          <button
            type="submit"
            disabled={!inputVal.trim() || isStreaming}
            className={`w-8 h-8 rounded-xl flex items-center justify-center transition-all flex-shrink-0 ${
              inputVal.trim() && !isStreaming
                ? 'bg-sky-500 text-white hover:bg-sky-400 shadow-sm'
                : 'bg-zinc-800 text-zinc-500 cursor-not-allowed'
            }`}
          >
            <ArrowUp size={16} strokeWidth={2.4} />
          </button>
        </form>
      </div>
    </div>
  );
};
