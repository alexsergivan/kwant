import React, { useState, useEffect, useRef } from 'react';
import { AppHeader } from './components/AppHeader';
import { Sidebar, Kwant, Channel, Team } from './components/Sidebar';
import { ChatView, ChatMessage, DiscussionState } from './components/ChatView';
import { PendingApproval } from './components/ModernApprovalCard';
import { SettingsModal } from './components/SettingsModal';
import { NewAgentModal } from './components/NewAgentModal';
import { NewTeamModal } from './components/NewTeamModal';
import { ManageTeamModal } from './components/ManageTeamModal';
import { playSfx } from './audio/chiptune';

export const App: React.FC = () => {
  const [kwants, setKwants] = useState<Kwant[]>([]);
  const [channels, setChannels] = useState<Channel[]>([]);
  const [teams, setTeams] = useState<Team[]>([]);
  const [selectedId, setSelectedId] = useState<string>('');
  const [isChannel, setIsChannel] = useState(true);
  const [selectedTeamId, setSelectedTeamId] = useState<string | null>(null);
  const [messages, setMessages] = useState<ChatMessage[]>([]);
  const [streamingText, setStreamingText] = useState('');
  const [streamingThinking, setStreamingThinking] = useState('');
  const [isStreaming, setIsStreaming] = useState(false);
  const [discussionState, setDiscussionState] = useState<DiscussionState>({ inProgress: false });
  const discussionInProgressRef = useRef(false);
  const kwantsRef = useRef<Kwant[]>(kwants);
  kwantsRef.current = kwants;
  const [pendingApproval, setPendingApproval] = useState<PendingApproval | null>(null);
  const [tokensUsed, setTokensUsed] = useState(0);

  // Modals state
  const [isSettingsOpen, setIsSettingsOpen] = useState(false);
  const [isNewAgentOpen, setIsNewAgentOpen] = useState(false);
  const [isNewTeamOpen, setIsNewTeamOpen] = useState(false);
  const [editingTeam, setEditingTeam] = useState<Team | null>(null);

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
        setChannels(cRes);
        setTeams(normalizedTeams);

        if (cRes.length > 0) {
          setSelectedId(cRes[0].id);
          setIsChannel(true);
          const matchedTeam = normalizedTeams.find(
            (t) => `team-${t.name}` === cRes[0].name || t.id === cRes[0].id
          );
          setSelectedTeamId(matchedTeam ? matchedTeam.id : null);
        } else if (kRes.length > 0) {
          setSelectedId(kRes[0].id);
          setIsChannel(false);
          setSelectedTeamId(null);
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
          case 'discussion_started':
          case 'DiscussionStarted': {
            discussionInProgressRef.current = true;
            const initialSpeakerId = event.data?.speakers?.[0];
            const initialSpeakerName =
              kwantsRef.current.find((k) => k.id === initialSpeakerId)?.name || initialSpeakerId;
            setDiscussionState({
              inProgress: true,
              teamName: event.data?.team_name,
              totalTurns: event.data?.total_turns,
              currentSpeaker: initialSpeakerName,
              currentTurn: 1,
            });
            setIsStreaming(true);
            playSfx('alert');
            break;
          }
          case 'discussion_turn_started':
          case 'DiscussionTurnStarted':
            discussionInProgressRef.current = true;
            setDiscussionState((prev) => ({
              ...prev,
              inProgress: true,
              currentSpeaker: event.data?.speaker_name,
              currentTurn: event.data?.turn_index,
              totalTurns: event.data?.total_turns ?? prev.totalTurns,
            }));
            setIsStreaming(true);
            setStreamingText('');
            setStreamingThinking('');
            if (selectedId) {
              fetch(`/api/channels/${selectedId}/messages`)
                .then((r) => r.json())
                .then((msgs) => setMessages(msgs))
                .catch(() => {});
            }
            break;
          case 'discussion_completed':
          case 'DiscussionCompleted':
            discussionInProgressRef.current = false;
            setDiscussionState({ inProgress: false });
            setIsStreaming(false);
            setStreamingText('');
            setStreamingThinking('');
            playSfx('coin');
            if (selectedId) {
              fetch(`/api/channels/${selectedId}/messages`)
                .then((r) => r.json())
                .then((msgs) => setMessages(msgs))
                .catch(() => {});
            }
            break;
          case 'TurnComplete':
            playSfx('coin');
            setStreamingText('');
            setStreamingThinking('');
            setTokensUsed((prev) => prev + (event.data?.tokens || 0));
            // Refresh messages
            if (selectedId) {
              fetch(`/api/channels/${selectedId}/messages`)
                .then((r) => r.json())
                .then((msgs) => setMessages(msgs))
                .catch(() => {});
            }
            if (!discussionInProgressRef.current) {
              setIsStreaming(false);
            }
            break;
          case 'Error':
            discussionInProgressRef.current = false;
            setDiscussionState({ inProgress: false });
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

  const handleCancelDiscussion = async () => {
    try {
      await fetch('/api/chat/cancel', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({
          channel_id: isChannel ? selectedId : null,
          kwant_id: !isChannel ? selectedId : null,
          team_id: selectedTeamId || null,
        }),
      });
    } catch (err) {
      console.error('Failed cancelling chat/discussion:', err);
    }
    discussionInProgressRef.current = false;
    setDiscussionState({ inProgress: false });
    setIsStreaming(false);
    setStreamingText('');
    setStreamingThinking('');
    if (selectedId) {
      fetch(`/api/channels/${selectedId}/messages`)
        .then((r) => r.json())
        .then((msgs) => setMessages(msgs))
        .catch(() => {});
    }
  };

  const handleSendMessage = async (text: string) => {
    discussionInProgressRef.current = false;
    setDiscussionState({ inProgress: false });
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
          team_id: selectedTeamId || null,
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

  const handleSelectTeam = (team: Team) => {
    setSelectedTeamId(team.id);
    const targetChannel = channels.find(
      (c) => c.name === `team-${team.name}` || c.id === team.id
    );
    if (targetChannel) {
      setSelectedId(targetChannel.id);
      setIsChannel(true);
    } else {
      fetch('/api/channels')
        .then((r) => (r.ok ? r.json() : []))
        .then((freshChannels: Channel[]) => {
          setChannels(freshChannels);
          const match = freshChannels.find(
            (c) => c.name === `team-${team.name}` || c.id === team.id
          );
          if (match) {
            setSelectedId(match.id);
            setIsChannel(true);
          } else {
            setSelectedId(team.id);
            setIsChannel(true);
          }
        })
        .catch(() => {
          setSelectedId(team.id);
          setIsChannel(true);
        });
    }
    playSfx('click');
  };

  const handleSelectChannel = (channel: Channel) => {
    setSelectedId(channel.id);
    setIsChannel(true);
    const matchedTeam = teams.find(
      (t) => `team-${t.name}` === channel.name || t.id === channel.id
    );
    setSelectedTeamId(matchedTeam ? matchedTeam.id : null);
    playSfx('click');
  };

  const handleSelectKwant = (kwant: Kwant) => {
    setSelectedId(kwant.id);
    setIsChannel(false);
    setSelectedTeamId(null);
    playSfx('click');
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
        setSelectedTeamId(null);
        setIsNewAgentOpen(false);
        playSfx('fanfare');
      }
    } catch (err) {
      console.error('Failed creating Agent:', err);
    }
  };

  const handleCreateTeam = async (teamData: {
    name: string;
    description?: string;
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
            setIsChannel(true);
          }
        }

        setIsNewTeamOpen(false);
        playSfx('fanfare');
      }
    } catch (err) {
      console.error('Failed creating team:', err);
    }
  };

  const handleUpdateTeam = async (
    id: string,
    data: {
      name: string;
      description?: string;
      lead_kwant_id: string;
      member_kwants: string[];
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
      }
    } catch (err) {
      console.error('Failed updating team:', err);
    }
  };

  const handleDeleteTeam = async (id: string) => {
    try {
      const res = await fetch(`/api/teams/${id}`, {
        method: 'DELETE',
      });

      if (res.ok) {
        setTeams((prev) => prev.filter((t) => t.id !== id));
        if (selectedTeamId === id) {
          setSelectedTeamId(null);
          if (channels.length > 0) {
            setSelectedId(channels[0].id);
            setIsChannel(true);
          }
        }

        // Refresh channels
        const cRes = await fetch('/api/channels');
        if (cRes.ok) {
          const freshChannels: Channel[] = await cRes.json();
          setChannels(freshChannels);
        }

        setEditingTeam(null);
        playSfx('click');
      }
    } catch (err) {
      console.error('Failed deleting team:', err);
    }
  };

  const currentChannel = isChannel ? channels.find((c) => c.id === selectedId) : null;
  const currentKwant = !isChannel ? kwants.find((k) => k.id === selectedId) : null;
  const currentTeam = selectedTeamId
    ? teams.find((t) => t.id === selectedTeamId)
    : isChannel && currentChannel?.name.startsWith('team-')
    ? teams.find((t) => `team-${t.name}` === currentChannel.name)
    : null;

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
    ? currentKwant?.name || 'Agent'
    : currentTeam
    ? `@${currentTeam.name}`
    : currentChannel?.name.startsWith('team-')
    ? `@${currentChannel.name.replace(/^team-/, '')}`
    : `#${currentChannel?.name || 'general'}`;

  const chatSubtitle = !isChannel
    ? `${currentKwant?.title || 'Specialist'} [Laya Auto-Fleet]`
    : currentTeam?.description || currentChannel?.description || 'Team collaboration room';

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
          teams={teams}
          selectedId={selectedId}
          isChannel={isChannel}
          selectedTeamId={selectedTeamId}
          onSelectKwant={handleSelectKwant}
          onSelectChannel={handleSelectChannel}
          onSelectTeam={handleSelectTeam}
          onManageTeam={(team) => {
            setEditingTeam(team);
            playSfx('click');
          }}
          onSummonKwant={() => {
            setIsNewAgentOpen(true);
            playSfx('click');
          }}
          onCreateTeam={() => {
            setIsNewTeamOpen(true);
            playSfx('click');
          }}
        />

        <ChatView
          title={chatTitle}
          subtitle={chatSubtitle}
          messages={messages}
          streamingText={streamingText}
          streamingThinking={streamingThinking}
          pendingApproval={pendingApproval}
          isStreaming={isStreaming}
          onSendMessage={handleSendMessage}
          onResolveApproval={handleResolveApproval}
          kwants={kwants}
          teams={teams}
          discussionState={discussionState}
          onCancelDiscussion={handleCancelDiscussion}
          teamMembers={currentTeamMembers}
          teamName={currentTeam?.name}
          onManageTeam={
            currentTeam
              ? () => {
                  setEditingTeam(currentTeam);
                  playSfx('click');
                }
              : undefined
          }
        />
      </div>

      {/* New Agent Modal */}
      <NewAgentModal
        isOpen={isNewAgentOpen}
        onClose={() => setIsNewAgentOpen(false)}
        onCreateAgent={handleCreateAgent}
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
      />

      {/* Settings Modal */}
      <SettingsModal
        isOpen={isSettingsOpen}
        onClose={() => setIsSettingsOpen(false)}
      />
    </div>
  );
};
