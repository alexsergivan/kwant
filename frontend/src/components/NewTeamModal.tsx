import React, { useState, useEffect } from 'react';
import { X, Users, AlertCircle, Folder, GitBranch } from 'lucide-react';
import { ModernAvatar } from './ModernAvatar';
import { Kwant, Team } from './Sidebar';
import { playSfx } from '../audio/chiptune';

export interface NewTeamModalProps {
  isOpen: boolean;
  onClose: () => void;
  kwants: Kwant[];
  onCreateTeam?: (data: {
    name: string;
    description?: string;
    workspace_path?: string;
    repo_url?: string;
    lead_kwant_id: string;
    member_kwants: string[];
    icon?: string;
  }) => Promise<void> | void;
  onTeamCreated?: (team: Team) => void;
}

export const NewTeamModal: React.FC<NewTeamModalProps> = ({
  isOpen,
  onClose,
  kwants,
  onCreateTeam,
  onTeamCreated,
}) => {
  const [name, setName] = useState('');
  const [description, setDescription] = useState('');
  const [workspaceType, setWorkspaceType] = useState<'local' | 'repo' | 'default'>('local');
  const [workspacePath, setWorkspacePath] = useState('');
  const [repoUrl, setRepoUrl] = useState('');
  const [leadKwantId, setLeadKwantId] = useState('');
  const [selectedMembers, setSelectedMembers] = useState<string[]>([]);
  const [submitting, setSubmitting] = useState(false);
  const [error, setError] = useState<string | null>(null);

  // Initialize or reset defaults when modal opens or kwants change
  useEffect(() => {
    if (isOpen) {
      setName('');
      setDescription('');
      setWorkspaceType('local');
      setWorkspacePath('');
      setRepoUrl('');
      setError(null);
      if (kwants.length > 0) {
        const defaultLead = kwants[0].id;
        setLeadKwantId(defaultLead);
        setSelectedMembers([defaultLead]);
      } else {
        setLeadKwantId('');
        setSelectedMembers([]);
      }
    }
  }, [isOpen, kwants]);

  if (!isOpen) return null;

  // Auto-slugging: convert spaces/special characters to hyphens, force lowercase
  const handleNameChange = (e: React.ChangeEvent<HTMLInputElement>) => {
    const raw = e.target.value.toLowerCase().replace(/[\s_]+/g, '-').replace(/[^a-z0-9-]/g, '');
    setName(raw);
    if (error) setError(null);
  };

  const cleanSlug = name.replace(/^-+|-+$/g, '');

  const handleLeadChange = (e: React.ChangeEvent<HTMLSelectElement>) => {
    const newLeadId = e.target.value;
    setLeadKwantId(newLeadId);
    if (newLeadId && !selectedMembers.includes(newLeadId)) {
      setSelectedMembers((prev) => [...prev, newLeadId]);
    }
    if (error) setError(null);
  };

  const toggleMember = (kwantId: string) => {
    playSfx('click');
    if (kwantId === leadKwantId) {
      // Lead Kwant must remain in members list
      return;
    }
    setSelectedMembers((prev) =>
      prev.includes(kwantId) ? prev.filter((id) => id !== kwantId) : [...prev, kwantId]
    );
    if (error) setError(null);
  };

  const handleSubmit = async (e: React.FormEvent) => {
    e.preventDefault();
    if (!cleanSlug) {
      setError('Please provide a valid team name.');
      return;
    }
    if (!leadKwantId) {
      setError('Please select a team lead.');
      return;
    }
    if (selectedMembers.length === 0) {
      setError('At least one Kwant member must be selected.');
      return;
    }

    setSubmitting(true);
    setError(null);
    playSfx('click');

    const finalMembers = selectedMembers.includes(leadKwantId)
      ? selectedMembers
      : [leadKwantId, ...selectedMembers];

    const teamPayload = {
      name: cleanSlug,
      description: description.trim() || undefined,
      workspace_path: workspaceType === 'local' && workspacePath.trim() ? workspacePath.trim() : undefined,
      repo_url: workspaceType === 'repo' && repoUrl.trim() ? repoUrl.trim() : undefined,
      lead_kwant_id: leadKwantId,
      member_kwants: finalMembers,
      icon: 'users',
    };

    try {
      if (onCreateTeam) {
        await onCreateTeam(teamPayload);
      } else {
        const res = await fetch('/api/teams', {
          method: 'POST',
          headers: { 'Content-Type': 'application/json' },
          body: JSON.stringify(teamPayload),
        });

        if (!res.ok) {
          const errText = await res.text();
          throw new Error(errText || 'Failed to create team');
        }

        const createdRaw = await res.json();
        const created: Team = {
          ...createdRaw,
          member_kwants: Array.isArray(createdRaw.member_kwants)
            ? createdRaw.member_kwants
            : JSON.parse(createdRaw.member_kwants || '[]'),
        };
        onTeamCreated?.(created);
        playSfx('fanfare');
        onClose();
      }
    } catch (err: unknown) {
      console.error('Failed creating team:', err);
      setError(err instanceof Error ? err.message : 'Failed to create team');
    } finally {
      setSubmitting(false);
    }
  };

  return (
    <div className="fixed inset-0 z-50 bg-black/70 backdrop-blur-sm flex items-center justify-center p-4 select-none">
      <div className="bg-zinc-900 border border-zinc-800 rounded-2xl max-w-md w-full p-6 text-zinc-100 shadow-2xl space-y-5">
        {/* Header */}
        <div className="flex items-center justify-between border-b border-zinc-800 pb-3">
          <div className="flex items-center gap-2">
            <div className="w-7 h-7 rounded-lg bg-sky-500/10 border border-sky-500/20 flex items-center justify-center text-sky-400">
              <Users size={15} />
            </div>
            <h3 className="text-sm font-semibold text-zinc-100">Create New Team</h3>
          </div>
          <button
            onClick={() => {
              playSfx('click');
              onClose();
            }}
            className="p-1 rounded-lg text-zinc-400 hover:text-zinc-100 hover:bg-zinc-800 transition-colors"
          >
            <X size={17} />
          </button>
        </div>

        {error && (
          <div className="p-2.5 rounded-lg bg-rose-500/10 border border-rose-500/20 flex items-center gap-2 text-rose-400 text-xs">
            <AlertCircle size={15} className="flex-shrink-0" />
            <span>{error}</span>
          </div>
        )}

        <form onSubmit={handleSubmit} className="space-y-4">
          {/* Team Name */}
          <div>
            <label className="block text-xs font-medium text-zinc-400 mb-1">
              Team Name
            </label>
            <input
              type="text"
              required
              value={name}
              onChange={handleNameChange}
              placeholder="e.g. core-engineering, qa-squad"
              className="w-full bg-zinc-950 border border-zinc-800 rounded-lg px-3 py-2 text-xs text-zinc-100 placeholder-zinc-600 focus:border-sky-500 focus:outline-none transition-colors"
            />
            <p className="text-[11px] text-zinc-500 mt-1 flex items-center gap-1.5">
              <span>Mention handle:</span>
              <span className="font-mono text-sky-400 bg-sky-500/10 px-1.5 py-0.5 rounded border border-sky-500/20">
                @{cleanSlug || 'team-name'}
              </span>
            </p>
          </div>

          {/* Description */}
          <div>
            <label className="block text-xs font-medium text-zinc-400 mb-1">
              Description <span className="text-zinc-600 font-normal">(Optional)</span>
            </label>
            <input
              type="text"
              value={description}
              onChange={(e) => setDescription(e.target.value)}
              placeholder="e.g. Core architectural and platform systems"
              className="w-full bg-zinc-950 border border-zinc-800 rounded-lg px-3 py-2 text-xs text-zinc-100 placeholder-zinc-600 focus:border-sky-500 focus:outline-none transition-colors"
            />
          </div>

          {/* Team Workspace */}
          <div>
            <label className="block text-xs font-medium text-zinc-400 mb-1.5">
              Team Workspace
            </label>
            <div className="grid grid-cols-3 gap-1.5 p-1 bg-zinc-950 rounded-lg border border-zinc-800 text-xs mb-2">
              <button
                type="button"
                onClick={() => {
                  playSfx('click');
                  setWorkspaceType('local');
                }}
                className={`flex items-center justify-center gap-1.5 py-1.5 rounded-md font-medium transition-all ${
                  workspaceType === 'local'
                    ? 'bg-zinc-800 text-sky-400 border border-sky-500/30 shadow-sm'
                    : 'text-zinc-400 hover:text-zinc-200 hover:bg-zinc-900'
                }`}
              >
                <Folder size={13} />
                <span>Local Folder</span>
              </button>
              <button
                type="button"
                onClick={() => {
                  playSfx('click');
                  setWorkspaceType('repo');
                }}
                className={`flex items-center justify-center gap-1.5 py-1.5 rounded-md font-medium transition-all ${
                  workspaceType === 'repo'
                    ? 'bg-zinc-800 text-sky-400 border border-sky-500/30 shadow-sm'
                    : 'text-zinc-400 hover:text-zinc-200 hover:bg-zinc-900'
                }`}
              >
                <GitBranch size={13} />
                <span>Git Repo</span>
              </button>
              <button
                type="button"
                onClick={() => {
                  playSfx('click');
                  setWorkspaceType('default');
                }}
                className={`flex items-center justify-center gap-1.5 py-1.5 rounded-md font-medium transition-all ${
                  workspaceType === 'default'
                    ? 'bg-zinc-800 text-sky-400 border border-sky-500/30 shadow-sm'
                    : 'text-zinc-400 hover:text-zinc-200 hover:bg-zinc-900'
                }`}
              >
                <span>Default</span>
              </button>
            </div>

            {workspaceType === 'local' && (
              <div>
                <input
                  type="text"
                  value={workspacePath}
                  onChange={(e) => setWorkspacePath(e.target.value)}
                  placeholder="e.g. ~/projects/my-app or /path/to/folder"
                  className="w-full bg-zinc-950 border border-zinc-800 rounded-lg px-3 py-2 text-xs text-zinc-100 placeholder-zinc-600 focus:border-sky-500 focus:outline-none transition-colors font-mono"
                />
                <p className="text-[11px] text-zinc-500 mt-1">
                  Team members will execute commands and tools directly in this directory.
                </p>
              </div>
            )}

            {workspaceType === 'repo' && (
              <div>
                <input
                  type="text"
                  value={repoUrl}
                  onChange={(e) => setRepoUrl(e.target.value)}
                  placeholder="https://github.com/owner/repository.git"
                  className="w-full bg-zinc-950 border border-zinc-800 rounded-lg px-3 py-2 text-xs text-zinc-100 placeholder-zinc-600 focus:border-sky-500 focus:outline-none transition-colors font-mono"
                />
                <p className="text-[11px] text-zinc-500 mt-1">
                  Repository will be cloned and managed in team workspace directory on first run.
                </p>
              </div>
            )}

            {workspaceType === 'default' && (
              <p className="text-[11px] text-zinc-500">
                Uses the default shared workspace directory.
              </p>
            )}
          </div>

          {/* Lead Kwant */}
          <div>
            <label className="block text-xs font-medium text-zinc-400 mb-1">
              Team Lead Kwant
            </label>
            <select
              value={leadKwantId}
              onChange={handleLeadChange}
              className="w-full bg-zinc-950 border border-zinc-800 rounded-lg px-3 py-2 text-xs text-zinc-100 focus:border-sky-500 focus:outline-none transition-colors"
            >
              {kwants.map((k) => (
                <option key={k.id} value={k.id}>
                  {k.name} ({k.title})
                </option>
              ))}
            </select>
          </div>

          {/* Member Kwants Checklist */}
          <div>
            <div className="flex items-center justify-between mb-1.5">
              <label className="text-xs font-medium text-zinc-400">
                Team Members
              </label>
              <span className="text-[11px] font-mono text-zinc-500">
                {selectedMembers.length} selected
              </span>
            </div>

            <div className="space-y-1 max-h-44 overflow-y-auto pr-1">
              {kwants.map((k) => {
                const isChecked = selectedMembers.includes(k.id);
                const isLead = k.id === leadKwantId;

                return (
                  <div
                    key={k.id}
                    onClick={() => toggleMember(k.id)}
                    className={`flex items-center justify-between p-2 rounded-lg border text-xs transition-all ${
                      isChecked
                        ? 'border-sky-500/40 bg-sky-500/10 text-zinc-100'
                        : 'border-zinc-800 bg-zinc-950/60 text-zinc-400 hover:border-zinc-700'
                    } ${isLead ? 'cursor-default' : 'cursor-pointer'}`}
                  >
                    <div className="flex items-center gap-2.5 min-w-0">
                      <input
                        type="checkbox"
                        checked={isChecked}
                        disabled={isLead}
                        onChange={() => {}}
                        className="rounded border-zinc-700 bg-zinc-900 text-sky-500 focus:ring-0 focus:ring-offset-0 pointer-events-none"
                      />
                      <ModernAvatar role={k.avatar_sprite} name={k.name} size={26} />
                      <div className="truncate">
                        <span className="font-semibold text-zinc-200">{k.name}</span>
                        <span className="text-[10px] text-zinc-400 ml-1.5 truncate">
                          ({k.title})
                        </span>
                      </div>
                    </div>
                    {isLead && (
                      <span className="text-[10px] font-mono font-medium text-amber-400 bg-amber-400/10 px-1.5 py-0.5 rounded border border-amber-400/20">
                        Lead
                      </span>
                    )}
                  </div>
                );
              })}
              {kwants.length === 0 && (
                <div className="p-3 text-center text-xs text-zinc-500">
                  No agents available to assign.
                </div>
              )}
            </div>
          </div>

          {/* Action Buttons */}
          <div className="flex items-center justify-end gap-2.5 pt-2 border-t border-zinc-800">
            <button
              type="button"
              onClick={() => {
                playSfx('click');
                onClose();
              }}
              className="px-3.5 py-2 rounded-lg text-xs font-medium text-zinc-300 hover:text-zinc-100 hover:bg-zinc-800 transition-colors"
            >
              Cancel
            </button>
            <button
              type="submit"
              disabled={submitting || !cleanSlug || selectedMembers.length === 0 || !leadKwantId}
              className="px-4 py-2 rounded-lg text-xs font-medium text-white bg-sky-600 hover:bg-sky-500 transition-colors shadow-sm disabled:opacity-50 disabled:cursor-not-allowed flex items-center gap-1.5"
            >
              <Users size={14} />
              <span>{submitting ? 'Creating...' : 'Create Team'}</span>
            </button>
          </div>
        </form>
      </div>
    </div>
  );
};
