import React, { Suspense, lazy, useState, useEffect, useRef, useCallback } from 'react';
import { AppHeader } from './components/AppHeader';
import { Sidebar, Kwant, Channel, Team, memberIdsOf } from './components/Sidebar';
import { ChatView, ChatMessage, DiscussionState, RoutingState } from './components/ChatView';
import { PendingApproval } from './components/ModernApprovalCard';
import type { AgentFormData } from './components/NewAgentModal';
import { NewTeamModal } from './components/NewTeamModal';
import { ManageTeamModal } from './components/ManageTeamModal';
import { playSfx } from './audio/chiptune';
import { Toasts, Toast } from './components/Toasts';
import { clearAllTurns, turnEnded, turnStarted } from './kwantActivity';
import { Profile, describeProfileSwitch, profilesApi } from './profiles';
import {
  NotifyEvent,
  SearchHit,
  browserNotifyEnabled,
  channelLabel,
  continuityApi,
  emitContinuity,
  notificationPermission,
  shouldShowOsNotification,
} from './continuity';
import type { PinnedItem } from './components/Instructions';

// Kwant and Tools editors are their own chunks. They stay mounted (rendering nothing while closed),
// so both load right after start-up and opening them never waits.
const NewAgentModal = lazy(() => import('./components/NewAgentModal').then((m) => ({ default: m.NewAgentModal })));
const McpServersModal = lazy(() => import('./components/McpServersModal').then((m) => ({ default: m.McpServersModal })));
const ProfilesModal = lazy(() => import('./components/ProfilesModal').then((m) => ({ default: m.ProfilesModal })));
const SettingsModal = lazy(() => import('./components/SettingsModal').then((m) => ({ default: m.SettingsModal })));
const ChannelInstructionsModal = lazy(() =>
  import('./components/ChannelInstructionsModal').then((m) => ({ default: m.ChannelInstructionsModal })),
);
const SearchModal = lazy(() => import('./components/SearchModal').then((m) => ({ default: m.SearchModal })));
const RoutinesModal = lazy(() => import('./components/RoutinesModal').then((m) => ({ default: m.RoutinesModal })));

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
  const [routingState, setRoutingState] = useState<RoutingState | null>(null);
  const routingStartedAtRef = useRef<number>(0);
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
  const [isMcpOpen, setIsMcpOpen] = useState(false);
  const [isProfilesOpen, setIsProfilesOpen] = useState(false);
  const [isSearchOpen, setIsSearchOpen] = useState(false);
  const [isRoutinesOpen, setIsRoutinesOpen] = useState(false);
  const [editingChannelInstructions, setEditingChannelInstructions] = useState<Channel | null>(null);
  const [highlightMessageId, setHighlightMessageId] = useState<string | null>(null);
  const [profiles, setProfiles] = useState<Profile[]>([]);
  const [toasts, setToasts] = useState<Toast[]>([]);
  const toastIdRef = useRef(0);
  const dismissToast = useCallback((id: number) => setToasts((prev) => prev.filter((t) => t.id !== id)), []);
  // Set each render once the select handlers exist; the SSE handler and notification clicks go through it.
  const openTargetRef = useRef<(t: { channel_id: string | null; kwant_id: string | null }) => boolean>(() => false);
  const showNotification = (n: NotifyEvent) => {
    if (!document.hidden && n.kind === 'routine_failed') {
      const id = ++toastIdRef.current;
      setToasts((prev) => [...prev.slice(-2), { id, title: n.title, detail: n.body }]);
      return;
    }
    if (!shouldShowOsNotification({ enabled: browserNotifyEnabled(), permission: notificationPermission(), hidden: document.hidden, kind: n.kind })) return;
    try {
      const note = new Notification(n.title, { body: n.body, tag: `kwnt-${n.kind}-${n.channel_id ?? n.kwant_id}` });
      note.onclick = () => {
        window.focus();
        openTargetRef.current({ channel_id: n.channel_id, kwant_id: n.kwant_id });
        note.close();
      };
    } catch {
      // Some mobile browsers only allow notifications from a service worker.
    }
  };
  const refreshProfiles = useCallback(() => {
    profilesApi.list().then(setProfiles, () => undefined);
  }, []);
  const [isNewAgentOpen, setIsNewAgentOpen] = useState(false);
  // Team that a newly created agent should join (set by a team's "Add member")
  const [newAgentTeam, setNewAgentTeam] = useState<Team | null>(null);
  const [isNewTeamOpen, setIsNewTeamOpen] = useState(false);
  const [editingTeam, setEditingTeam] = useState<Team | null>(null);
  const [editingKwant, setEditingKwant] = useState<Kwant | null>(null);

  // Virtual Machine & Live Browser state
  const [isVmPaneOpen, setIsVmPaneOpen] = useState(false);
  const [activeUrl, setActiveUrl] = useState('');
  const [pageTitle, setPageTitle] = useState('');
  const [screenshotBase64, setScreenshotBase64] = useState<string | null>(null);
  const [isVmRunning, setIsVmRunning] = useState(true);
  const [browserError, setBrowserError] = useState<string | null>(null);
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
    refreshProfiles();
  }, [refreshProfiles]);

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
    // After a reconnect a `turn_ended` may have been missed; drop working states rather than spin forever.
    let dropped = false;
    sse.onerror = () => {
      dropped = true;
    };
    sse.onopen = () => {
      if (dropped) clearAllTurns();
      dropped = false;
    };

    sse.onmessage = (e) => {
      try {
        const event = JSON.parse(e.data);
        switch (event.type) {
          case 'turn_started':
          case 'TurnStarted':
            turnStarted(event.data?.kwant_id, event.data?.context_id);
            break;
          case 'turn_ended':
          case 'TurnEnded':
            turnEnded(event.data?.kwant_id, event.data?.context_id);
            break;
          case 'profile_switched':
          case 'ProfileSwitched': {
            const kwantName = kwantsRef.current.find((k) => k.id === event.data?.kwant_id)?.name;
            const { title, detail } = describeProfileSwitch(event.data, kwantName);
            const id = ++toastIdRef.current;
            setToasts((prev) => [...prev.slice(-2), { id, title, detail }]);
            playSfx('alert');
            refreshProfiles();
            break;
          }
          case 'memory_changed':
          case 'MemoryChanged':
            emitContinuity({ type: 'memory_changed', data: event.data });
            break;
          case 'routine_run':
          case 'RoutineRun':
            emitContinuity({ type: 'routine_run', data: event.data });
            break;
          case 'notify':
          case 'Notify':
            showNotification(event.data as NotifyEvent);
            break;
          case 'TextDelta':
            setStreamingText((prev) => prev + event.data);
            playSfx('blip');
            break;
          case 'ThinkingDelta':
            setStreamingThinking(event.data);
            break;
          case 'routing_started':
          case 'RoutingStarted': {
            const ctx = event.data?.context_id;
            const targetId = streamingContextIdRef.current;
            if (ctx && targetId && ctx !== targetId) break;
            routingStartedAtRef.current = Date.now();
            setRoutingState({
              phase: 'deciding',
              contextId: ctx || targetId || undefined,
              candidates: event.data?.candidates || [],
            });
            setStreamingSpeakerName('Laya');
            streamingSpeakerNameRef.current = 'Laya';
            break;
          }
          case 'routing_decided':
          case 'RoutingDecided': {
            const ctx = event.data?.context_id;
            const targetId = streamingContextIdRef.current;
            if (ctx && targetId && ctx !== targetId) break;
            const name: string = event.data?.kwant_name;
            const startedAt = routingStartedAtRef.current;
            const applyDecision = () => {
            if (routingStartedAtRef.current !== startedAt) return;
            setRoutingState((prev) => (startedAt && !prev) ? prev : ({
              phase: 'decided',
              contextId: ctx || targetId || undefined,
              candidates: prev?.candidates || [],
              kwantId: event.data?.kwant_id,
              kwantName: name,
              kwantTitle: event.data?.kwant_title,
              reason: event.data?.reason,
              mode: event.data?.mode,
            }));
            if (name) {
              setStreamingSpeakerName(name);
              streamingSpeakerNameRef.current = name;
            }
            playSfx('blip');
            };
            // Keep Laya's "deciding" visible briefly so instant heuristic routing doesn't just flash.
            const remaining = 700 - (Date.now() - startedAt);
            if (startedAt && remaining > 0) setTimeout(applyDecision, remaining);
            else applyDecision();
            break;
          }
          case 'ApprovalRequired':
            setPendingApproval({
              id: event.data.id,
              tool_name: event.data.tool_name,
              details: event.data.details,
            });
            playSfx('alert');
            break;
          case 'ApprovalWithdrawn':
            setPendingApproval((current) => (current?.id === event.data.id ? null : current));
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
              kind: event.data?.kind,
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
              askedBy: event.data?.asked_by,
              phase: event.data?.phase,
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
            setRoutingState(null);
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
              setRoutingState(null);
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
              teamId === selectedIdRef.current ||
              teamId === streamingContextIdRef.current ||
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
              teamId === selectedIdRef.current ||
              teamId === streamingContextIdRef.current ||
              (k && (k.id === teamId || k.name.toLowerCase() === teamId.toLowerCase())) ||
              (t && (t.id === teamId || t.name.toLowerCase() === teamId.toLowerCase()));
            if (isMatch) {
              setBrowserError(null);
              setIsVmRunning(true);
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
              teamId === selectedIdRef.current ||
              teamId === streamingContextIdRef.current ||
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
            setRoutingState(null);
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
      clearAllTurns();
    };
  }, [refreshProfiles]);

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
    setRoutingState(null);
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
    setRoutingState(null);
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
      setRoutingState(null);
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

  /** Opens a channel, or the direct chat with a kwant when there's no channel. False if it's gone. */
  const openTarget = (t: { channel_id: string | null; kwant_id: string | null }): boolean => {
    if (t.channel_id) {
      const channel = channels.find((c) => c.id === t.channel_id);
      if (!channel) return false;
      if (selectedIdRef.current !== channel.id) handleSelectChannel(channel);
      return true;
    }
    const kwant = kwants.find((k) => k.id === t.kwant_id);
    if (!kwant) return false;
    if (selectedIdRef.current !== kwant.id) handleSelectKwant(kwant);
    return true;
  };
  openTargetRef.current = openTarget;

  const pushToast = (title: string, detail?: string) => {
    const id = ++toastIdRef.current;
    setToasts((prev) => [...prev.slice(-2), { id, title, detail }]);
  };

  // Search result picked: open its chat, then scroll to the message if it's among the ones loaded.
  const handleJump = async (hit: SearchHit) => {
    const targetId = hit.channel_id ?? hit.kwant_id;
    if (!targetId || !openTarget(hit)) {
      pushToast('That chat no longer exists.');
      return;
    }
    try {
      const res = await fetch(`/api/channels/${encodeURIComponent(targetId)}/messages`);
      if (!res.ok) throw new Error(`HTTP ${res.status}`);
      const data: ChatMessage[] = await res.json();
      messagesCacheRef.current[targetId] = data;
      if (selectedIdRef.current === targetId) setMessages(data);
      if (data.some((m) => m.id === hit.message_id)) setHighlightMessageId(hit.message_id);
      else pushToast('Opened the chat', 'That message is older than the latest 100 shown here.');
    } catch {
      pushToast('Could not load that chat.');
    }
  };

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if ((e.metaKey || e.ctrlKey) && !e.altKey && !e.shiftKey && e.key.toLowerCase() === 'k') {
        e.preventDefault();
        setIsSearchOpen((v) => !v);
      }
    };
    window.addEventListener('keydown', onKey);
    return () => window.removeEventListener('keydown', onKey);
  }, []);

  const saveChannelInstructions = async (text: string): Promise<string | void> => {
    const channel = editingChannelInstructions;
    if (!channel) return;
    try {
      const updated = await continuityApi.setChannelInstructions(channel.id, text);
      setChannels((prev) => prev.map((c) => (c.id === channel.id ? { ...c, instructions: updated.instructions } : c)));
    } catch (err) {
      return err instanceof Error ? err.message : 'Could not save.';
    }
  };

  const saveKwantMcpServers = async (kwantId: string, serverIds: string[] | undefined): Promise<string | void> => {
    if (serverIds === undefined) return;
    const res = await fetch(`/api/kwants/${kwantId}/mcp-servers`, {
      method: 'PUT',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ server_ids: serverIds }),
    });
    if (!res.ok) return `Saved, but the tool servers couldn't be updated (HTTP ${res.status}).`;
  };

  const handleUpdateAgent = async (kwant: Kwant, agentData: AgentFormData): Promise<string | void> => {
    try {
      const { mcp_server_ids, ...kwantData } = agentData;
      const res = await fetch(`/api/kwants/${kwant.id}`, {
        method: 'PUT',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify(kwantData),
      });
      if (res.status === 409) return `Another agent is already named "${agentData.name}".`;
      if (res.status === 400) return 'Check the name (no spaces or @) and that the profile belongs to the chosen engine.';
      if (!res.ok) return `Saving failed (HTTP ${res.status}).`;

      const updated: Kwant = await res.json();
      setKwants((prev) => prev.map((k) => (k.id === updated.id ? updated : k)));
      if (updated.name !== kwant.name) {
        const rename = (list: ChatMessage[]) =>
          list.map((m) =>
            m.sender_role === 'kwant' && m.sender_id === kwant.name ? { ...m, sender_id: updated.name } : m,
          );
        for (const key of Object.keys(messagesCacheRef.current)) {
          messagesCacheRef.current[key] = rename(messagesCacheRef.current[key]);
        }
        setMessages((prev) => rename(prev));
      }
      const mcpError = await saveKwantMcpServers(updated.id, mcp_server_ids);
      if (mcpError) return mcpError;
      playSfx('click');
    } catch (err) {
      console.error('Failed updating Agent:', err);
      return 'Could not reach the server.';
    }
  };

  const handleCreateAgent = async (agentData: AgentFormData): Promise<string | void> => {
    try {
      const res = await fetch('/api/kwants', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({
          name: agentData.name,
          title: agentData.title,
          persona_prompt: agentData.persona_prompt,
          soul: agentData.soul ?? '',
          avatar_sprite: agentData.avatar_sprite,
          driver_type: 'api',
          preferred_model: agentData.preferred_model,
          ...(agentData.profile_id !== undefined ? { profile_id: agentData.profile_id } : {}),
          ...(agentData.profile_failover !== undefined ? { profile_failover: agentData.profile_failover } : {}),
        }),
      });

      if (res.ok) {
        const created: Kwant = await res.json();
        setKwants((prev) => [...prev, created]);
        // Saved even when empty: switching every server off means none.
        const mcpError = await saveKwantMcpServers(created.id, agentData.mcp_server_ids);
        if (mcpError) console.error(mcpError);
        setIsNewAgentOpen(false);
        playSfx('fanfare');
        const team = newAgentTeam && (teams.find((t) => t.id === newAgentTeam.id) ?? newAgentTeam);
        setNewAgentTeam(null);
        if (team) {
          const memberIds = memberIdsOf(team);
          await handleUpdateTeamMembers(team, [...memberIds.filter((id) => id !== created.id), created.id]);
          handleSelectTeam(team);
          return;
        }
        setSelectedId(created.id);
        selectedIdRef.current = created.id;
        setIsChannel(false);
        isChannelRef.current = false;
        setSelectedTeamId(null);
        messagesCacheRef.current[created.id] = [];
        setMessages([]);
        return;
      }
      if (res.status === 409) return `Another agent is already named "${agentData.name}".`;
      if (res.status === 400) return 'Check the name (no spaces or @) and that the profile belongs to the chosen engine.';
      return `Creating failed (HTTP ${res.status}).`;
    } catch (err) {
      console.error('Failed creating Agent:', err);
      return 'Could not reach the server.';
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
      instructions?: string;
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

  const handleUpdateTeamMembers = async (team: Team, memberIds: string[]) => {
    const previous = teams;
    // Optimistic update so the sidebar reacts instantly
    setTeams((prev) => prev.map((t) => (t.id === team.id ? { ...t, member_kwants: memberIds } : t)));
    try {
      const res = await fetch(`/api/teams/${team.id}`, {
        method: 'PUT',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({
          name: team.name,
          description: team.description,
          icon: team.icon,
          workspace_path: team.workspace_path,
          repo_url: team.repo_url,
          lead_kwant_id: team.lead_kwant_id,
          member_kwants: memberIds,
        }),
      });
      if (!res.ok) throw new Error(`HTTP ${res.status}`);
      const cRes = await fetch('/api/channels');
      if (cRes.ok) setChannels(await cRes.json());
      playSfx('click');
    } catch (err) {
      console.error('Failed updating team members:', err);
      setTeams(previous);
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

  const boundProfile = currentKwant?.profile_id ? profiles.find((p) => p.id === currentKwant.profile_id) : undefined;
  const chatSubtitle = !isChannel
    ? `${currentKwant?.title || 'Specialist'} [Laya Auto-Fleet]${boundProfile ? ` · profile: ${boundProfile.name}` : ''}`
    : currentTeam?.description || currentChannel?.description || 'Team collaboration room';

  // Load container status and screenshot for active team or bot
  useEffect(() => {
    const sid = activeSandboxId;
    let cancelled = false;
    setScreenshotBase64(null);
    setActiveUrl('');
    setPageTitle('');
    setBrowserError(null);
    if (!sid || sid === 'default') return;

    fetch(`/api/sandbox/${sid}/status`)
      .then((r) => (r.ok ? r.json() : null))
      .then((data) => {
        if (!cancelled && data && typeof data.is_running === 'boolean') {
          setIsVmRunning(data.is_running);
        }
      })
      .catch(() => {});

    fetch(`/api/sandbox/${sid}/screenshot`)
      .then(async (r) => {
        const data = await r.json();
        if (!r.ok) throw new Error(data.error || 'Could not load the sandbox browser.');
        return data;
      })
      .then((data) => {
        if (!cancelled && data) {
          setIsVmRunning(true);
          setBrowserError(null);
          if (data.image_base64 || data.screenshot) {
            setScreenshotBase64(data.image_base64 || data.screenshot);
          }
          if (data.url) {
            setActiveUrl(data.url);
          }
        }
      })
      .catch((err) => {
        if (!cancelled) {
          setIsVmRunning(false);
          setBrowserError(err instanceof Error ? err.message : 'Could not reach the sandbox browser.');
        }
      });
    return () => { cancelled = true; };
  }, [activeSandboxId]);

  const handleRefreshVm = async () => {
    const sid = activeSandboxId;
    if (!sid || sid === 'default') return;
    try {
      const res = await fetch(`/api/sandbox/${sid}/screenshot`);
      const data = await res.json();
      if (activeSandboxIdRef.current !== sid) return;
      if (!res.ok) throw new Error(data.error || 'Could not load the sandbox browser.');
      setBrowserError(null);
      setIsVmRunning(true);
      if (data.image_base64 || data.screenshot) {
        setScreenshotBase64(data.image_base64 || data.screenshot);
      }
      if (data.url) {
        setActiveUrl(data.url);
      }
    } catch (err) {
      if (activeSandboxIdRef.current !== sid) return;
      setScreenshotBase64(null);
      setIsVmRunning(false);
      setBrowserError(err instanceof Error ? err.message : 'Could not reach the sandbox browser.');
    }
  };

  const handleRestartVm = async () => {
    const sid = activeSandboxId;
    if (!sid || sid === 'default') return;
    setBrowserError(null);
    setScreenshotBase64(null);
    try {
      const res = await fetch(`/api/sandbox/${sid}/restart`, {
        method: 'POST',
      });
      const data = await res.json();
      if (activeSandboxIdRef.current !== sid) return;
      if (!res.ok || !data.is_running) throw new Error(data.error || 'Sandbox could not start. Check Docker and try again.');
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
    } catch (err) {
      if (activeSandboxIdRef.current !== sid) return;
      setIsVmRunning(false);
      setBrowserError(err instanceof Error ? err.message : 'Could not restart the sandbox browser.');
    }
  };

  const handleNavigateVm = async (url: string) => {
    const sid = activeSandboxId;
    if (!sid || sid === 'default') return;
    setBrowserError(null);
    setIsVmPaneOpen(true);
    try {
      const res = await fetch(`/api/sandbox/${sid}/navigate`, {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ url }),
      });
      const data = await res.json();
      if (activeSandboxIdRef.current !== sid) return;
      if (!res.ok) throw new Error(data.error || 'Browser navigation failed.');
      if (data.url) setActiveUrl(data.url);
      if (data.title) setPageTitle(data.title);
      setIsVmPaneOpen(true);
      handleRefreshVm();
    } catch (err) {
      if (activeSandboxIdRef.current !== sid) return;
      setBrowserError(err instanceof Error ? err.message : 'Browser navigation failed.');
    }
  };

  const closeProfiles = useCallback(() => setIsProfilesOpen(false), []);

  const isCurrentContextStreaming = isStreaming && streamingContextId === selectedId;
  const pinned: PinnedItem[] = [];
  if (currentTeam?.instructions?.trim()) pinned.push({ label: `Team @${currentTeam.name}`, text: currentTeam.instructions });
  if (isChannel && currentChannel?.instructions?.trim())
    pinned.push({ label: `Channel ${channelLabel(currentChannel.name)}`, text: currentChannel.instructions });

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
        onOpenTools={() => setIsMcpOpen(true)}
        onOpenSearch={() => setIsSearchOpen(true)}
        onOpenRoutines={() => setIsRoutinesOpen(true)}
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
          onUpdateTeamMembers={handleUpdateTeamMembers}
          onManageTeam={(team) => {
            setEditingTeam(team);
            playSfx('click');
          }}
          onEditKwant={(kwant) => {
            setEditingKwant(kwant);
            playSfx('click');
          }}
          onSummonKwant={(team) => {
            setNewAgentTeam(team ?? null);
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
          routingState={isCurrentContextStreaming ? routingState : null}
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
          onEditKwant={
            currentKwant
              ? () => {
                  setEditingKwant(currentKwant);
                  playSfx('click');
                }
              : undefined
          }
          isVmPaneOpen={isVmPaneOpen}
          onToggleVmPane={() => setIsVmPaneOpen((prev) => !prev)}
          activeUrl={activeUrl}
          pageTitle={pageTitle}
          screenshotBase64={screenshotBase64}
          browserError={browserError}
          isRunning={isVmRunning}
          terminalLogs={terminalLogs}
          onRefreshVm={handleRefreshVm}
          onRestartVm={handleRestartVm}
          onNavigateVm={handleNavigateVm}
          pinned={pinned}
          onEditInstructions={
            isChannel && currentChannel
              ? () => {
                  setEditingChannelInstructions(currentChannel);
                  playSfx('click');
                }
              : undefined
          }
          highlightMessageId={highlightMessageId}
          onHighlightDone={() => setHighlightMessageId(null)}
        />
      </div>

      {/* New Agent Modal */}
      <Suspense fallback={null}>
        <NewAgentModal
          isOpen={isNewAgentOpen}
          onClose={() => {
            setIsNewAgentOpen(false);
            setNewAgentTeam(null);
          }}
          joinTeamName={newAgentTeam?.name}
          takenNames={kwants.map((k) => k.name)}
          profiles={profiles}
          onManageProfiles={() => setIsProfilesOpen(true)}
          onManageServers={() => setIsMcpOpen(true)}
          onSubmit={handleCreateAgent}
        />

        {/* Edit Agent Modal */}
        <NewAgentModal
          isOpen={!!editingKwant}
          initial={editingKwant}
          kwantId={editingKwant?.id}
          kwants={kwants}
          channels={channels}
          profiles={profiles}
          onManageProfiles={() => setIsProfilesOpen(true)}
          onManageServers={() => setIsMcpOpen(true)}
          onClose={() => setEditingKwant(null)}
          onSubmit={(data) => (editingKwant ? handleUpdateAgent(editingKwant, data) : undefined)}
        />
      </Suspense>

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
      <Suspense fallback={null}>
        <SettingsModal
          isOpen={isSettingsOpen}
          onClose={() => setIsSettingsOpen(false)}
          onOpenTools={() => {
            setIsSettingsOpen(false);
            setIsMcpOpen(true);
          }}
          onOpenProfiles={() => {
            setIsSettingsOpen(false);
            setIsProfilesOpen(true);
          }}
        />
      </Suspense>

      {/* MCP tool servers (opened last so it sits above the agent modal) */}
      <Suspense fallback={null}>
        <McpServersModal isOpen={isMcpOpen} onClose={() => setIsMcpOpen(false)} kwants={kwants} />
        <ProfilesModal isOpen={isProfilesOpen} onClose={closeProfiles} onProfilesChanged={setProfiles} />
        {isSearchOpen && <SearchModal isOpen onClose={() => setIsSearchOpen(false)} kwants={kwants} onJump={handleJump} />}
        {isRoutinesOpen && <RoutinesModal isOpen onClose={() => setIsRoutinesOpen(false)} kwants={kwants} channels={channels} />}
      </Suspense>

      {editingChannelInstructions && (
        <Suspense fallback={null}>
          <ChannelInstructionsModal
            channelLabel={channelLabel(editingChannelInstructions.name)}
            initial={editingChannelInstructions.instructions ?? ''}
            onClose={() => setEditingChannelInstructions(null)}
            onSave={saveChannelInstructions}
          />
        </Suspense>
      )}

      <Toasts toasts={toasts} onDismiss={dismissToast} />
    </div>
  );
};
