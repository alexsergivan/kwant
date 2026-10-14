import React, { useState, useEffect } from 'react';
import { AppHeader } from './components/AppHeader';
import { Sidebar, Kwant, Channel } from './components/Sidebar';
import { ChatView, ChatMessage } from './components/ChatView';
import { PendingApproval } from './components/ModernApprovalCard';
import { SettingsModal } from './components/SettingsModal';
import { NewAgentModal } from './components/NewAgentModal';
import { playSfx } from './audio/chiptune';

export const App: React.FC = () => {
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

  // Modals state
  const [isSettingsOpen, setIsSettingsOpen] = useState(false);
  const [isNewAgentOpen, setIsNewAgentOpen] = useState(false);

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
            playSfx('alert');
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

  const handleCreateAgent = async (agentData: {
    name: string;
    title: string;
    persona_prompt: string;
    avatar_sprite: string;
    preferred_model: string;
  }) => {
    try {
      const res = await fetch('/api/kwants', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({
          name: agentData.name,
          title: agentData.title,
          persona_prompt: agentData.persona_prompt,
          avatar_sprite: agentData.avatar_sprite,
          driver_type: 'api',
          preferred_model: agentData.preferred_model,
        }),
      });

      if (res.ok) {
        const created: Kwant = await res.json();
        setKwants((prev) => [...prev, created]);
        setSelectedId(created.id);
        setIsChannel(false);
        setIsNewAgentOpen(false);
        playSfx('fanfare');
      }
    } catch (err) {
      console.error('Failed creating Agent:', err);
    }
  };

  const currentChannel = isChannel ? channels.find((c) => c.id === selectedId) : null;
  const currentKwant = !isChannel ? kwants.find((k) => k.id === selectedId) : null;

  return (
    <div className="h-screen w-screen flex flex-col bg-zinc-950 text-zinc-100 overflow-hidden font-sans">
      {/* Modern App Header */}
      <AppHeader
        onOpenSettings={() => setIsSettingsOpen(true)}
        tokensUsed={tokensUsed}
        activeModel={currentKwant?.preferred_model || 'auto'}
      />

      {/* Main Workspace Shell */}
      <div className="flex-1 flex overflow-hidden">
        <Sidebar
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
            setIsNewAgentOpen(true);
            playSfx('click');
          }}
        />

        <ChatView
          title={isChannel ? `#${currentChannel?.name || 'general'}` : currentKwant?.name || 'Agent'}
          subtitle={
            isChannel
              ? currentChannel?.description || 'Team collaboration room'
              : `${currentKwant?.title || 'Specialist'} [Laya Auto-Fleet]`
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

      {/* New Agent Modal */}
      <NewAgentModal
        isOpen={isNewAgentOpen}
        onClose={() => setIsNewAgentOpen(false)}
        onCreateAgent={handleCreateAgent}
      />

      {/* Settings Modal */}
      <SettingsModal
        isOpen={isSettingsOpen}
        onClose={() => setIsSettingsOpen(false)}
      />
    </div>
  );
};
