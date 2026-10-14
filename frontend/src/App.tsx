import React, { useState, useEffect } from 'react';
import { HudBar } from './components/HudBar';
import { KwantSidebar, Kwant, Channel } from './components/KwantSidebar';
import { DialogueWindow, ChatMessage } from './components/DialogueWindow';
import { PendingApproval } from './components/ApprovalCard';
import { playSfx } from './audio/chiptune';

export const App: React.FC = () => {
  const [crtEnabled, setCrtEnabled] = useState(true);
  const [kwants, setKwants] = useState<Kwant[]>([]);
  const [channels, setChannels] = useState<Channel[]>([]);
  const [selectedId, setSelectedId] = useState<string>('');
  const [isChannel, setIsChannel] = useState(true);
  const [messages, setMessages] = useState<ChatMessage[]>([]);
  const [streamingText, setStreamingText] = useState('');
  const [streamingThinking, setStreamingThinking] = useState('');
  const [isStreaming, setIsStreaming] = useState(false);
  const [pendingApproval, setPendingApproval] = useState<PendingApproval | null>(null);
  const [tokensUsed, setTokensUsed] = useState(0);

  // Summon Modal state
  const [isSummonOpen, setIsSummonOpen] = useState(false);
  const [newKwantName, setNewKwantName] = useState('');
  const [newKwantTitle, setNewKwantTitle] = useState('');
  const [newKwantSprite, setNewKwantSprite] = useState('cyber_mage');
  const [newKwantPrompt, setNewKwantPrompt] = useState('');
  const [newKwantModel, setNewKwantModel] = useState('auto');

  // Load initial Kwants and Channels
  useEffect(() => {
    async function loadInitialData() {
      try {
        const [kRes, cRes] = await Promise.all([
          fetch('/api/kwants').then((r) => r.json()),
          fetch('/api/channels').then((r) => r.json()),
        ]);
        setKwants(kRes);
        setChannels(cRes);
        if (cRes.length > 0) {
          setSelectedId(cRes[0].id);
          setIsChannel(true);
        } else if (kRes.length > 0) {
          setSelectedId(kRes[0].id);
          setIsChannel(false);
        }
      } catch (err) {
        console.error('Failed to load initial data:', err);
      }
    }
    loadInitialData();
  }, []);

  // Load messages when selected channel/kwant changes
  useEffect(() => {
    if (!selectedId) return;
    async function loadMessages() {
      try {
        const res = await fetch(`/api/channels/${selectedId}/messages`);
        if (res.ok) {
          const data: ChatMessage[] = await res.json();
          setMessages(data);
          const totalTokens = data.reduce((acc, m) => acc + (m.tokens_used || 0), 0);
          setTokensUsed(totalTokens);
        }
      } catch (err) {
        console.error('Failed loading messages:', err);
      }
    }
    loadMessages();
  }, [selectedId]);

  // Connect to SSE stream
  useEffect(() => {
    const sse = new EventSource('/api/events');

    sse.onmessage = (e) => {
      try {
        const event = JSON.parse(e.data);
        switch (event.type) {
          case 'TextDelta':
            setStreamingText((prev) => prev + event.data);
            playSfx('blip');
            break;
          case 'ThinkingDelta':
            setStreamingThinking(event.data);
            break;
          case 'ApprovalRequired':
            setPendingApproval({
              id: event.data.id,
              tool_name: event.data.tool_name,
              details: event.data.details,
            });
            break;
          case 'TurnComplete':
            playSfx('coin');
            setIsStreaming(false);
            setStreamingText('');
            setStreamingThinking('');
            setTokensUsed((prev) => prev + (event.data.tokens || 0));
            // Refresh messages
            if (selectedId) {
              fetch(`/api/channels/${selectedId}/messages`)
                .then((r) => r.json())
                .then((msgs) => setMessages(msgs))
                .catch(() => {});
            }
            break;
          case 'Error':
            setIsStreaming(false);
            setStreamingText('');
            break;
        }
      } catch (err) {
        console.error('Failed parsing SSE event:', err);
      }
    };

    return () => {
      sse.close();
    };
  }, [selectedId]);

  const handleSendMessage = async (text: string) => {
    setIsStreaming(true);
    setStreamingText('');
    setStreamingThinking('');

    // Optimistically add user message
    const tempId = `temp-${Date.now()}`;
    const userMsg: ChatMessage = {
      id: tempId,
      sender_role: 'user',
      sender_id: 'PlayerHero',
      text,
    };
    setMessages((prev) => [...prev, userMsg]);

    try {
      await fetch('/api/chat/send', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({
          channel_id: isChannel ? selectedId : null,
          kwant_id: !isChannel ? selectedId : null,
          prompt: text,
          sender_id: 'PlayerHero',
        }),
      });
    } catch (err) {
      console.error('Failed sending chat:', err);
      setIsStreaming(false);
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

  const handleCreateKwant = async (e: React.FormEvent) => {
    e.preventDefault();
    if (!newKwantName.trim()) return;

    try {
      const res = await fetch('/api/kwants', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({
          name: newKwantName,
          title: newKwantTitle || 'Apprentice Kwant',
          persona_prompt: newKwantPrompt || 'You are an eager 8-bit AI agent ready to assist.',
          avatar_sprite: newKwantSprite,
          driver_type: 'api',
          preferred_model: newKwantModel,
        }),
      });

      if (res.ok) {
        const created: Kwant = await res.json();
        setKwants((prev) => [...prev, created]);
        setSelectedId(created.id);
        setIsChannel(false);
        setIsSummonOpen(false);
        playSfx('fanfare');
        // Reset form
        setNewKwantName('');
        setNewKwantTitle('');
        setNewKwantPrompt('');
      }
    } catch (err) {
      console.error('Failed creating Kwant:', err);
    }
  };

  const currentChannel = isChannel ? channels.find((c) => c.id === selectedId) : null;
  const currentKwant = !isChannel ? kwants.find((k) => k.id === selectedId) : null;

  return (
    <div className={`h-screen w-screen flex flex-col overflow-hidden ${crtEnabled ? 'crt-overlay' : ''}`}>
      {/* Top Arcade HUD */}
      <HudBar
        crtEnabled={crtEnabled}
        onToggleCrt={() => setCrtEnabled(!crtEnabled)}
        tokensUsed={tokensUsed}
        activeModel={currentKwant?.preferred_model || 'auto'}
        isLayaOnline={true}
      />

      {/* Main Game Shell */}
      <div className="flex-1 flex overflow-hidden">
        <KwantSidebar
          kwants={kwants}
          channels={channels}
          selectedId={selectedId}
          isChannel={isChannel}
          onSelectKwant={(k) => {
            setSelectedId(k.id);
            setIsChannel(false);
            playSfx('click');
          }}
          onSelectChannel={(c) => {
            setSelectedId(c.id);
            setIsChannel(true);
            playSfx('click');
          }}
          onSummonKwant={() => {
            setIsSummonOpen(true);
            playSfx('coin');
          }}
        />

        <DialogueWindow
          title={isChannel ? `#${currentChannel?.name || 'guild'}` : currentKwant?.name || 'Kwant'}
          subtitle={
            isChannel
              ? currentChannel?.description || 'Guild party room'
              : `${currentKwant?.title || 'Agent'} [Model: ${currentKwant?.preferred_model || 'auto'}]`
          }
          messages={messages}
          streamingText={streamingText}
          streamingThinking={streamingThinking}
          pendingApproval={pendingApproval}
          isStreaming={isStreaming}
          onSendMessage={handleSendMessage}
          onResolveApproval={handleResolveApproval}
        />
      </div>

      {/* Summon Kwant Modal */}
      {isSummonOpen && (
        <div className="fixed inset-0 bg-black/80 flex items-center justify-center p-4 z-50">
          <div className="pixel-box-amber p-5 max-w-md w-full bg-arcade-panel">
            <div className="flex items-center justify-between border-b-2 border-arcade-amber pb-2 mb-4">
              <h3 className="font-pixel text-xs text-arcade-amber">⚔️ SUMMON NEW KWANT</h3>
              <button
                onClick={() => setIsSummonOpen(false)}
                className="text-zinc-400 hover:text-white font-pixel text-xs"
              >
                [X]
              </button>
            </div>

            <form onSubmit={handleCreateKwant} className="space-y-3 font-retro text-xs">
              <div>
                <label className="block text-zinc-400 font-pixel text-[9px] mb-1">NAME</label>
                <input
                  type="text"
                  required
                  value={newKwantName}
                  onChange={(e) => setNewKwantName(e.target.value)}
                  placeholder="e.g. ChronoMage"
                  className="w-full bg-black border border-zinc-700 px-2 py-1.5 text-arcade-cyan focus:outline-none focus:border-arcade-cyan"
                />
              </div>

              <div>
                <label className="block text-zinc-400 font-pixel text-[9px] mb-1">TITLE</label>
                <input
                  type="text"
                  value={newKwantTitle}
                  onChange={(e) => setNewKwantTitle(e.target.value)}
                  placeholder="e.g. Master of Time and Space"
                  className="w-full bg-black border border-zinc-700 px-2 py-1.5 text-zinc-200 focus:outline-none focus:border-arcade-cyan"
                />
              </div>

              <div>
                <label className="block text-zinc-400 font-pixel text-[9px] mb-1">8-BIT SPRITE</label>
                <select
                  value={newKwantSprite}
                  onChange={(e) => setNewKwantSprite(e.target.value)}
                  className="w-full bg-black border border-zinc-700 px-2 py-1.5 text-arcade-amber focus:outline-none"
                >
                  <option value="cyber_mage">Cyber Mage (Sorcerer)</option>
                  <option value="pixel_ninja">Pixel Ninja (Assassin)</option>
                  <option value="terminal_knight">Terminal Knight (Paladin)</option>
                  <option value="glitch_rogue">Glitch Rogue (Scout)</option>
                </select>
              </div>

              <div>
                <label className="block text-zinc-400 font-pixel text-[9px] mb-1">MODEL ROUTE</label>
                <select
                  value={newKwantModel}
                  onChange={(e) => setNewKwantModel(e.target.value)}
                  className="w-full bg-black border border-zinc-700 px-2 py-1.5 text-arcade-amber focus:outline-none"
                >
                  <option value="auto">Auto (Laya 33ms Decision Router)</option>
                  <option value="grok">xAI Grok (Fast & Witty)</option>
                  <option value="claude">Anthropic Claude (Architecture)</option>
                  <option value="codex">OpenAI Codex / GPT (Coding)</option>
                  <option value="local_ollama">Local Ollama (Private)</option>
                </select>
              </div>

              <div>
                <label className="block text-zinc-400 font-pixel text-[9px] mb-1">PERSONA / SYSTEM PROMPT</label>
                <textarea
                  rows={3}
                  value={newKwantPrompt}
                  onChange={(e) => setNewKwantPrompt(e.target.value)}
                  placeholder="You are an 8-bit AI agent specialized in..."
                  className="w-full bg-black border border-zinc-700 px-2 py-1.5 text-zinc-200 focus:outline-none focus:border-arcade-cyan resize-none"
                />
              </div>

              <div className="flex justify-end gap-2 pt-2">
                <button
                  type="button"
                  onClick={() => setIsSummonOpen(false)}
                  className="pixel-btn pixel-btn-dark text-[9px]"
                >
                  CANCEL
                </button>
                <button type="submit" className="pixel-btn pixel-btn-amber text-[9px]">
                  ✨ SUMMON
                </button>
              </div>
            </form>
          </div>
        </div>
      )}
    </div>
  );
};
