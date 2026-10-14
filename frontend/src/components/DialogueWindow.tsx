import React from 'react';
import { SpriteAvatar } from './SpriteAvatar';
import { ApprovalCard, PendingApproval } from './ApprovalCard';
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

interface DialogueWindowProps {
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

export const DialogueWindow: React.FC<DialogueWindowProps> = ({
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
  const [inputVal, setInputVal] = React.useState('');
  const scrollRef = React.useRef<HTMLDivElement>(null);

  // Auto-scroll on new tokens or messages
  React.useEffect(() => {
    if (scrollRef.current) {
      scrollRef.current.scrollTop = scrollRef.current.scrollHeight;
    }
  }, [messages, streamingText, streamingThinking, pendingApproval]);

  const handleSubmit = (e: React.FormEvent) => {
    e.preventDefault();
    if (!inputVal.trim() || isStreaming) return;
    playSfx('click');
    onSendMessage(inputVal);
    setInputVal('');
  };

  const handleRollPrompt = () => {
    const funPrompts = [
      "Write a fast Rust macro to deserialize retro 8-bit packets",
      "Check the server status and report memory usage",
      "Refactor our SQLite schema with foreign key cascades",
      "Write a bash script to backup ~/.kwnt/ to a tarball",
      "What is the mana cost of training an agent with RLCD?"
    ];
    const picked = funPrompts[Math.floor(Math.random() * funPrompts.length)];
    setInputVal(picked);
    playSfx('coin');
  };

  return (
    <div className="flex-1 flex flex-col h-full bg-[#100e17] overflow-hidden">
      {/* Dialogue Header */}
      <div className="pixel-box bg-arcade-panel px-4 py-2 border-b-2 border-black flex items-center justify-between">
        <div>
          <h2 className="font-pixel text-xs text-arcade-cyan tracking-wide flex items-center gap-2">
            <span>⚔️</span>
            <span>{title}</span>
          </h2>
          <p className="font-retro text-[10px] text-zinc-400 mt-0.5">{subtitle}</p>
        </div>
      </div>

      {/* Messages Stream Area */}
      <div ref={scrollRef} className="flex-1 overflow-y-auto p-4 space-y-4">
        {messages.map((msg) => {
          const isUser = msg.sender_role === 'user';
          return (
            <div
              key={msg.id}
              className={`flex items-start gap-3 max-w-3xl ${
                isUser ? 'ml-auto flex-row-reverse' : ''
              }`}
            >
              <SpriteAvatar
                sprite={isUser ? 'user' : 'cyber_mage'}
                size={34}
              />
              <div
                className={`p-3 pixel-box ${
                  isUser
                    ? 'border-arcade-cyan bg-[#131b24]'
                    : 'border-arcade-border bg-arcade-panel'
                }`}
              >
                <div className="flex items-center justify-between gap-4 mb-1 border-b border-zinc-800 pb-1">
                  <span className="font-pixel text-[8px] text-zinc-400 uppercase">
                    {isUser ? 'PLAYER HERO' : msg.sender_id}
                  </span>
                  {msg.tokens_used ? (
                    <span className="font-pixel text-[7px] text-arcade-amber">
                      [{msg.tokens_used} MP]
                    </span>
                  ) : null}
                </div>

                {/* Reasoning / Thinking accordion if present */}
                {msg.reasoning_content && (
                  <div className="mb-2 p-2 bg-black/60 border-l-2 border-arcade-magenta font-retro text-[11px] text-zinc-400">
                    <span className="text-arcade-magenta font-bold">🧠 THOUGHT PROCESS: </span>
                    {msg.reasoning_content}
                  </div>
                )}

                {/* Message Body */}
                <div className="font-retro text-xs sm:text-sm text-zinc-200 leading-relaxed whitespace-pre-wrap break-words">
                  {msg.text}
                </div>
              </div>
            </div>
          );
        })}

        {/* Live Streaming Message */}
        {isStreaming && (
          <div className="flex items-start gap-3 max-w-3xl">
            <SpriteAvatar sprite="cyber_mage" size={34} />
            <div className="p-3 pixel-box border-arcade-amber bg-arcade-panel">
              <div className="flex items-center gap-2 mb-1 border-b border-zinc-800 pb-1">
                <span className="w-2 h-2 bg-arcade-amber inline-block animate-ping" />
                <span className="font-pixel text-[8px] text-arcade-amber">
                  KWANT CASTING...
                </span>
              </div>

              {streamingThinking && (
                <div className="mb-2 p-2 bg-black/60 border-l-2 border-arcade-magenta font-retro text-[11px] text-arcade-magenta/90 animate-pulse">
                  🔮 {streamingThinking}
                </div>
              )}

              <div className="font-retro text-xs sm:text-sm text-zinc-200 leading-relaxed whitespace-pre-wrap break-words">
                {streamingText}
                <span className="inline-block w-2 h-4 bg-arcade-cyan ml-1 cursor-blink align-middle" />
              </div>
            </div>
          </div>
        )}

        {/* Pending Approval Ticket */}
        {pendingApproval && (
          <ApprovalCard
            approval={pendingApproval}
            onResolve={onResolveApproval}
          />
        )}
      </div>

      {/* Input Bar */}
      <form onSubmit={handleSubmit} className="p-3 border-t-4 border-black bg-arcade-panel flex items-center gap-2">
        <button
          type="button"
          onClick={handleRollPrompt}
          className="pixel-btn pixel-btn-dark text-[9px] hidden sm:flex"
          title="Roll random prompt"
        >
          🎲 ROLL
        </button>

        <div className="flex-1 relative">
          <input
            type="text"
            value={inputVal}
            onChange={(e) => setInputVal(e.target.value)}
            placeholder="Type your quest command..."
            className="w-full bg-black border-2 border-zinc-700 px-3 py-2 text-xs sm:text-sm font-retro text-arcade-cyan placeholder-zinc-600 focus:outline-none focus:border-arcade-cyan"
            disabled={isStreaming}
          />
        </div>

        <button
          type="submit"
          disabled={!inputVal.trim() || isStreaming}
          className={`pixel-btn ${
            !inputVal.trim() || isStreaming ? 'pixel-btn-dark opacity-50 cursor-not-allowed' : 'pixel-btn-amber'
          }`}
        >
          ⚔️ SEND
        </button>
      </form>
    </div>
  );
};
