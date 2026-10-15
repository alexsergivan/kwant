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
  const [streamingContextId, setStreamingContextId] = useState<string | null>(null);
  const [streamingSpeakerName, setStreamingSpeakerName] = useState<string>('');
  const streamingContextIdRef = useRef<string | null>(null);
  const streamingSpeakerNameRef = useRef<string>('');
  const selectedIdRef = useRef<string>(selectedId);
  selectedIdRef.current = selectedId;
  const isChannelRef = useRef<boolean>(isChannel);
  isChannelRef.current = isChannel;
  const messagesCacheRef = useRef<Record<string, ChatMessage[]>>({});
  const [discussionState, setDiscussionState] = useState<DiscussionState>({ inProgress: false });
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
  const [pendingApproval, setPendingApproval] = useState<PendingApproval | null>(null);
  const [tokensUsed, setTokensUsed] = useState(0);

  // Modals state
  const [isSettingsOpen, setIsSettingsOpen] = useState(false);
  const [isNewAgentOpen, setIsNewAgentOpen] = useState(false);
  const [isNewTeamOpen, setIsNewTeamOpen] = useState(false);
  const [editingTeam, setEditingTeam] = useState<Team | null>(null);

  // Virtual Machine & Live Browser state
  const [isVmPaneOpen, setIsVmPaneOpen] = useState(false);
  const [activeUrl, setActiveUrl] = useState('');
  const [pageTitle, setPageTitle] = useState('');
  const [screenshotBase64, setScreenshotBase64] = useState<string | null>(null);
  const [isVmRunning, setIsVmRunning] = useState(true);
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
        setChannels(cRes);
        setTeams(normalizedTeams);

        if (cRes.length > 0) {
          const initId = cRes[0].id;
          setSelectedId(initId);
          selectedIdRef.current = initId;
          setIsChannel(true);
          isChannelRef.current = true;
          const matchedTeam = normalizedTeams.find(
            (t) => `team-${t.name}` === cRes[0].name || t.id === cRes[0].id
          );
          setSelectedTeamId(matchedTeam ? matchedTeam.id : null);
        } else if (kRes.length > 0) {
          const initId = kRes[0].id;
          setSelectedId(initId);
          selectedIdRef.current = initId;
          setIsChannel(false);
          isChannelRef.current = false;
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
    const currentTargetId = selectedId;
    async function loadMessages() {
      try {
        const res = await fetch(`/api/channels/${currentTargetId}/messages`);
        if (res.ok) {
          const data: ChatMessage[] = await res.json();
          messagesCacheRef.current[currentTargetId] = data;
          if (selectedIdRef.current === currentTargetId) {
            setMessages(data);
            const totalTokens = data.reduce((acc, m) => acc + (m.tokens_used || 0), 0);
            setTokensUsed(totalTokens);
          }
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
            const teamId = event.data?.team_id;
            const initialSpeakerId = event.data?.speakers?.[0];
            const initialSpeakerName =
              kwantsRef.current.find((k) => k.id === initialSpeakerId)?.name || initialSpeakerId;

            const matchedChannel = channelsRef.current.find(
              (c) => c.name === `team-${event.data?.team_name}` || c.id === teamId
            );
            const contextId = matchedChannel ? matchedChannel.id : (teamId || selectedIdRef.current);

            setStreamingContextId(contextId);
            streamingContextIdRef.current = contextId;
            setStreamingSpeakerName(initialSpeakerName);
            streamingSpeakerNameRef.current = initialSpeakerName;

            setDiscussionState({
              inProgress: true,
              teamId,
              contextId,
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
          case 'DiscussionTurnStarted': {
            discussionInProgressRef.current = true;
            const spkName = event.data?.speaker_name;
            setStreamingSpeakerName(spkName);
            streamingSpeakerNameRef.current = spkName;
            setDiscussionState((prev) => ({
              ...prev,
              inProgress: true,
              currentSpeaker: spkName,
              currentTurn: event.data?.turn_index,
              totalTurns: event.data?.total_turns ?? prev.totalTurns,
            }));
            setIsStreaming(true);
            setStreamingText('');
            setStreamingThinking('');
            const targetId = streamingContextIdRef.current || selectedIdRef.current;
            if (targetId) {
              fetch(`/api/channels/${targetId}/messages`)
                .then((r) => r.json())
                .then((msgs) => {
                  messagesCacheRef.current[targetId] = msgs;
                  if (selectedIdRef.current === targetId) {
                    setMessages(msgs);
                  }
                })
                .catch(() => {});
            }
            break;
          }
          case 'discussion_completed':
          case 'DiscussionCompleted': {
            discussionInProgressRef.current = false;
            setDiscussionState({ inProgress: false });
            setIsStreaming(false);
            setStreamingText('');
            setStreamingThinking('');
            const targetId = streamingContextIdRef.current || selectedIdRef.current;
            setStreamingContextId(null);
            streamingContextIdRef.current = null;
            setStreamingSpeakerName('');
            streamingSpeakerNameRef.current = '';
            playSfx('coin');
            if (targetId) {
              fetch(`/api/channels/${targetId}/messages`)
                .then((r) => r.json())
                .then((msgs) => {
                  messagesCacheRef.current[targetId] = msgs;
                  if (selectedIdRef.current === targetId) {
                    setMessages(msgs);
                  }
                })
                .catch(() => {});
            }
            break;
          }
          case 'TurnComplete': {
            playSfx('coin');
            setStreamingText('');
            setStreamingThinking('');
            setTokensUsed((prev) => prev + (event.data?.tokens || 0));
            const targetId = streamingContextIdRef.current || selectedIdRef.current;
            if (targetId) {
              fetch(`/api/channels/${targetId}/messages`)
                .then((r) => r.json())
                .then((msgs) => {
                  messagesCacheRef.current[targetId] = msgs;
                  if (selectedIdRef.current === targetId) {
                    setMessages(msgs);
                  }
                })
                .catch(() => {});
            }
            if (!discussionInProgressRef.current) {
              setIsStreaming(false);
              setStreamingContextId(null);
              streamingContextIdRef.current = null;
              setStreamingSpeakerName('');
              streamingSpeakerNameRef.current = '';
            }
            break;
          }
          case 'browser_navigated':
          case 'BrowserNavigated': {
            const teamId = event.data?.team_id;
            const currentTid = activeSandboxIdRef.current;
            const k = currentKwantRef.current;
            const t = currentTeamRef.current;
            const isMatch =
              !currentTid ||
              !teamId ||
              currentTid === teamId ||
              teamId === 'default' ||
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
            const currentTid = activeSandboxIdRef.current;
            const k = currentKwantRef.current;
            const t = currentTeamRef.current;
            const isMatch =
              !currentTid ||
              !teamId ||
              currentTid === teamId ||
              teamId === 'default' ||
              (k && (k.id === teamId || k.name.toLowerCase() === teamId.toLowerCase())) ||
              (t && (t.id === teamId || t.name.toLowerCase() === teamId.toLowerCase()));
            if (isMatch) {
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
            discussionInProgressRef.current = false;
            setDiscussionState({ inProgress: false });
            setIsStreaming(false);
            setStreamingContextId(null);
            streamingContextIdRef.current = null;
            setStreamingSpeakerName('');
            streamingSpeakerNameRef.current = '';
            setStreamingText('');
            setStreamingThinking('');
            break;
          }
        }
      } catch (err) {
        console.error('Failed parsing SSE event:', err);
      }
    };

    return () => {
      sse.close();
    };
  }, []);

  const handleCancelDiscussion = async () => {
    const targetId = selectedIdRef.current;
    try {
      await fetch('/api/chat/cancel', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({
          channel_id: isChannelRef.current ? targetId : null,
          kwant_id: !isChannelRef.current ? targetId : null,
          team_id: activeTeamIdRef.current || null,
        }),
      });
    } catch (err) {
      console.error('Failed cancelling chat/discussion:', err);
    }
    discussionInProgressRef.current = false;
    setDiscussionState({ inProgress: false });
    setIsStreaming(false);
    setStreamingContextId(null);
    streamingContextIdRef.current = null;
    setStreamingSpeakerName('');
    streamingSpeakerNameRef.current = '';
    setStreamingText('');
    setStreamingThinking('');
    if (targetId) {
      fetch(`/api/channels/${targetId}/messages`)
        .then((r) => r.json())
        .then((msgs) => {
          messagesCacheRef.current[targetId] = msgs;
          if (selectedIdRef.current === targetId) {
            setMessages(msgs);
          }
        })
        .catch(() => {});
    }
  };

  const handleSendMessage = async (text: string) => {
    const targetId = selectedId;
    const isTargetChannel = isChannel;
    const targetTeam = selectedTeamId;
    const speakerName = !isTargetChannel ? (currentKwant?.name || 'Agent') : chatTitle;

    discussionInProgressRef.current = false;
    setDiscussionState({ inProgress: false });
    setIsStreaming(true);
    setStreamingContextId(targetId);
    streamingContextIdRef.current = targetId;
    setStreamingSpeakerName(speakerName);
    streamingSpeakerNameRef.current = speakerName;
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
    const cached = messagesCacheRef.current[targetId] || messages;
    const updated = [...cached, userMsg];
    messagesCacheRef.current[targetId] = updated;
    if (selectedIdRef.current === targetId) {
      setMessages(updated);
    }

    try {
      await fetch('/api/chat/send', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({
          channel_id: isTargetChannel ? targetId : null,
          kwant_id: !isTargetChannel ? targetId : null,
          team_id: targetTeam || null,
          prompt: text,
          sender_id: 'PlayerHero',
        }),
      });
    } catch (err) {
      console.error('Failed sending chat:', err);
      setIsStreaming(false);
      setStreamingContextId(null);
      streamingContextIdRef.current = null;
      setStreamingSpeakerName('');
      streamingSpeakerNameRef.current = '';
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
          const match = freshChannels.find(
            (c) => c.name === `team-${team.name}` || c.id === team.id
          );
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

  const handleSelectChannel = (channel: Channel) => {
    setSelectedId(channel.id);
    selectedIdRef.current = channel.id;
    setIsChannel(true);
    isChannelRef.current = true;
    const matchedTeam = teams.find(
      (t) => `team-${t.name}` === channel.name || t.id === channel.id
    );
    setSelectedTeamId(matchedTeam ? matchedTeam.id : null);
    setMessages(messagesCacheRef.current[channel.id] || []);
    playSfx('click');
  };

  const handleSelectKwant = (kwant: Kwant) => {
    setSelectedId(kwant.id);
    selectedIdRef.current = kwant.id;
    setIsChannel(false);
    isChannelRef.current = false;
    setSelectedTeamId(null);
    setMessages(messagesCacheRef.current[kwant.id] || []);
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
        selectedIdRef.current = created.id;
        setIsChannel(false);
        isChannelRef.current = false;
        setSelectedTeamId(null);
        messagesCacheRef.current[created.id] = [];
        setMessages([]);
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
          const teamChan = freshChannels.find(
            (c) => c.name === `team-${created.name}` || c.id === created.id
          );
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
      workspace_path?: string;
      repo_url?: string;
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

  // Load container status and screenshot for active team or bot
  useEffect(() => {
    const sid = activeSandboxId;
    if (!sid || sid === 'default') return;

    fetch(`/api/sandbox/${sid}/status`)
      .then((r) => (r.ok ? r.json() : null))
      .then((data) => {
        if (data && typeof data.is_running === 'boolean') {
          setIsVmRunning(data.is_running);
        }
      })
      .catch(() => {});

    fetch(`/api/sandbox/${sid}/screenshot`)
      .then((r) => (r.ok ? r.json() : null))
      .then((data) => {
        if (data) {
          if (data.image_base64 || data.screenshot) {
            setScreenshotBase64(data.image_base64 || data.screenshot);
          }
          if (data.url) {
            setActiveUrl(data.url);
          }
        }
      })
      .catch(() => {});
  }, [activeSandboxId]);

  const handleRefreshVm = async () => {
    const sid = activeSandboxId;
    if (!sid || sid === 'default') return;
    try {
      const res = await fetch(`/api/sandbox/${sid}/screenshot`);
      if (res.ok) {
        const data = await res.json();
        if (data.image_base64 || data.screenshot) {
          setScreenshotBase64(data.image_base64 || data.screenshot);
        }
        if (data.url) {
          setActiveUrl(data.url);
        }
      }
    } catch (err) {
      console.error('Failed refreshing VM screenshot:', err);
    }
  };

  const handleRestartVm = async () => {
    const sid = activeSandboxId;
    if (!sid || sid === 'default') return;
    try {
      const res = await fetch(`/api/sandbox/${sid}/restart`, {
        method: 'POST',
      });
      if (res.ok) {
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
      }
    } catch (err) {
      console.error('Failed restarting VM:', err);
    }
  };

  const handleNavigateVm = async (url: string) => {
    const sid = activeSandboxId;
    if (!sid || sid === 'default') return;
    try {
      const res = await fetch(`/api/sandbox/${sid}/navigate`, {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ url }),
      });
      if (res.ok) {
        const data = await res.json();
        if (data.url) setActiveUrl(data.url);
        if (data.title) setPageTitle(data.title);
        setIsVmPaneOpen(true);
        handleRefreshVm();
      }
    } catch (err) {
      console.error('Failed navigating VM:', err);
    }
  };

  const isCurrentContextStreaming = isStreaming && streamingContextId === selectedId;
  const isDiscussionActiveHere =
    discussionState.inProgress &&
    (discussionState.contextId === selectedId ||
      discussionState.teamId === selectedTeamId ||
      (isChannel && currentTeam && discussionState.teamId === currentTeam.id));

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
          streamingText={isCurrentContextStreaming ? streamingText : ''}
          streamingThinking={isCurrentContextStreaming ? streamingThinking : ''}
          pendingApproval={pendingApproval}
          isStreaming={isCurrentContextStreaming}
          streamingSpeakerName={streamingSpeakerName}
          onSendMessage={handleSendMessage}
          onResolveApproval={handleResolveApproval}
          kwants={kwants}
          teams={teams}
          discussionState={isDiscussionActiveHere ? discussionState : { inProgress: false }}
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
          isVmPaneOpen={isVmPaneOpen}
          onToggleVmPane={() => setIsVmPaneOpen((prev) => !prev)}
          activeUrl={activeUrl}
          pageTitle={pageTitle}
          screenshotBase64={screenshotBase64}
          isRunning={isVmRunning}
          terminalLogs={terminalLogs}
          onRefreshVm={handleRefreshVm}
          onRestartVm={handleRestartVm}
          onNavigateVm={handleNavigateVm}
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
