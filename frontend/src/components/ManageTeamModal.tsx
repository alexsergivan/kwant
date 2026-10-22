import React, { useState, useEffect, useRef } from 'react';
import { X, Users, Trash2, Check, AlertTriangle, ShieldCheck, Folder, GitBranch, FolderSearch } from 'lucide-react';
import { ModernAvatar } from './ModernAvatar';
import { Team, Kwant } from './Sidebar';
import { FolderBrowserModal } from './FolderBrowserModal';
import { playSfx } from '../audio/chiptune';
import { InstructionsField } from './Instructions';
import { MAX_INSTRUCTIONS } from '../continuity';
import { teamNameToSave } from '../teamChannel';

interface ManageTeamModalProps {
  isOpen: boolean;
  team: Team | null;
  kwants: Kwant[];
  onClose: () => void;
  onUpdateTeam: (
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
  ) => Promise<void>;
  onDeleteTeam: (id: string) => Promise<void>;
}

/**
 * Why a delete didn't happen, shown right below the Delete button. On a phone the modal is scrolled
 * down to that button, so a note at the top would be out of sight; this one scrolls itself into view.
 */
export const DeleteNote: React.FC<{ text: string | null }> = ({ text }) => {
  const ref = useRef<HTMLDivElement>(null);
  useEffect(() => {
    if (text) ref.current?.scrollIntoView?.({ block: 'nearest', behavior: 'smooth' });
  }, [text]);
  if (!text) return null;
  return (
    <div ref={ref} role="alert" data-testid="team-delete-note" className="p-2.5 bg-rose-500/10 border border-rose-500/20 rounded-xl text-xs text-rose-300 flex items-center gap-2">
      <AlertTriangle size={14} className="flex-shrink-0 text-rose-400" aria-hidden="true" />
      <span>{text}</span>
    </div>
  );
};

export const ManageTeamModal: React.FC<ManageTeamModalProps> = ({
  isOpen,
  team,
  kwants,
  onClose,
  onUpdateTeam,
  onDeleteTeam,
}) => {
  const [name, setName] = useState('');
  const [description, setDescription] = useState('');
  const [instructions, setInstructions] = useState('');
  const [workspaceType, setWorkspaceType] = useState<'local' | 'repo' | 'default'>('default');
  const [workspacePath, setWorkspacePath] = useState('');
  const [repoUrl, setRepoUrl] = useState('');
  const [isBrowsing, setIsBrowsing] = useState(false);
  const [isFolderBrowserOpen, setIsFolderBrowserOpen] = useState(false);
  const [leadKwantId, setLeadKwantId] = useState('');
  const [memberIds, setMemberIds] = useState<string[]>([]);
  const [isDeleting, setIsDeleting] = useState(false);
  const [confirmDelete, setConfirmDelete] = useState(false);
  const [isSaving, setIsSaving] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [deleteError, setDeleteError] = useState<string | null>(null);

  const handleBrowseFolder = async () => {
    playSfx('click');
    setIsBrowsing(true);
    try {
      const res = await fetch('/api/system/select-folder', { method: 'POST' });
      if (res.ok) {
        const data = await res.json();
        if (data.path) {
          setWorkspacePath(data.path);
          playSfx('coin');
          return;
        } else if (data.canceled) {
          return;
        }
      }
      setIsFolderBrowserOpen(true);
    } catch {
      setIsFolderBrowserOpen(true);
    } finally {
      setIsBrowsing(false);
    }
  };

  useEffect(() => {
    if (team) {
      setName(team.name);
      setDescription(team.description || '');
      setInstructions(team.instructions || '');
      setLeadKwantId(team.lead_kwant_id || (kwants[0]?.id ?? ''));

      if (team.workspace_path) {
        setWorkspaceType('local');
        setWorkspacePath(team.workspace_path);
        setRepoUrl('');
      } else if (team.repo_url) {
        setWorkspaceType('repo');
        setWorkspacePath('');
        setRepoUrl(team.repo_url);
      } else {
        setWorkspaceType('default');
        setWorkspacePath('');
        setRepoUrl('');
      }

      const parsedMembers: string[] = Array.isArray(team.member_kwants)
        ? team.member_kwants
        : typeof team.member_kwants === 'string'
        ? JSON.parse(team.member_kwants || '[]')
        : [];
      setMemberIds(parsedMembers);
      setConfirmDelete(false);
      setError(null);
      setDeleteError(null);
    }
  }, [team, kwants, isOpen]);

  if (!isOpen || !team) return null;

  const handleToggleMember = (kwantId: string) => {
    playSfx('click');
    if (memberIds.includes(kwantId)) {
      if (memberIds.length <= 1) {
        setError('A team needs at least one bot.');
        return;
      }
      setError(null);
      const next = memberIds.filter((id) => id !== kwantId);
      setMemberIds(next);
      if (leadKwantId === kwantId && next.length > 0) {
        setLeadKwantId(next[0]);
      }
    } else {
      setError(null);
      setMemberIds([...memberIds, kwantId]);
    }
  };

  const handleSave = async (e: React.FormEvent) => {
    e.preventDefault();
    const cleanName = teamNameToSave(name, team.name);
    if (!cleanName) {
      setError('Please provide a valid team name.');
      return;
    }
    if (memberIds.length === 0) {
      setError('Pick at least one bot for this team.');
      return;
    }
    if (instructions.length > MAX_INSTRUCTIONS) {
      setError(`Team instructions can be at most ${MAX_INSTRUCTIONS} characters.`);
      return;
    }

    try {
      setIsSaving(true);
      setError(null);
      await onUpdateTeam(team.id, {
        name: cleanName,
        description: description.trim() || undefined,
        workspace_path: workspaceType === 'local' && workspacePath.trim() ? workspacePath.trim() : undefined,
        repo_url: workspaceType === 'repo' && repoUrl.trim() ? repoUrl.trim() : undefined,
        lead_kwant_id: leadKwantId || memberIds[0],
        member_kwants: memberIds,
        instructions: instructions.trim(),
      });
      playSfx('fanfare');
      setIsSaving(false);
      onClose();
    } catch (err) {
      setIsSaving(false);
      setError(err instanceof Error && err.message ? err.message : 'Couldn’t save the team. Try again.');
    }
  };

  const handleDelete = async () => {
    if (!confirmDelete) {
      setConfirmDelete(true);
      playSfx('click');
      return;
    }

    try {
      setIsDeleting(true);
      setDeleteError(null);
      playSfx('click');
      await onDeleteTeam(team.id);
      setIsDeleting(false);
      onClose();
    } catch (err) {
      setIsDeleting(false);
      setDeleteError(err instanceof Error && err.message ? err.message : 'Couldn’t delete the team.');
    }
  };

  return (
    <div className="fixed inset-0 z-50 bg-black/75 backdrop-blur-sm flex items-center justify-center p-4 select-none animate-in fade-in duration-150">
      <div role="dialog" aria-modal="true" aria-labelledby="manage-team-title" className="bg-zinc-900 border border-zinc-800 rounded-2xl max-w-lg w-full max-h-[94vh] overflow-y-auto p-4 sm:p-6 text-zinc-100 shadow-2xl space-y-5">
        {/* Header */}
        <div className="flex items-center justify-between border-b border-zinc-800 pb-3">
          <div className="flex items-center gap-2.5">
            <div className="w-8 h-8 rounded-lg bg-sky-500/10 border border-sky-500/20 flex items-center justify-center text-sky-400">
              <Users size={16} />
            </div>
            <div>
              <h3 id="manage-team-title" className="text-sm font-semibold text-zinc-100">Manage Team: @{team.name}</h3>
              <p className="text-[11px] text-zinc-400">Members, lead and instructions</p>
            </div>
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
          <div role="alert" className="p-2.5 bg-rose-500/10 border border-rose-500/20 rounded-xl text-xs text-rose-300 flex items-center gap-2">
            <AlertTriangle size={14} className="flex-shrink-0 text-rose-400" />
            <span>{error}</span>
          </div>
        )}

        <form onSubmit={handleSave} className="space-y-4">
          {/* Team Name */}
          <div>
            <label className="block text-xs font-medium text-zinc-400 mb-1">Team Handle</label>
            <div className="relative flex items-center">
              <span className="absolute left-3 text-zinc-500 text-xs font-mono">@</span>
              <input
                type="text"
                required
                value={name}
                onChange={(e) => setName(e.target.value.toLowerCase().replace(/[^a-z0-9_-]/g, '-'))}
                className="w-full bg-zinc-950 border border-zinc-800 rounded-lg pl-7 pr-3 py-2 text-xs text-zinc-100 font-mono focus:border-sky-500 focus:outline-none transition-colors"
              />
            </div>
          </div>

          {/* Description */}
          <div>
            <label className="block text-xs font-medium text-zinc-400 mb-1">Mission / Purpose</label>
            <input
              type="text"
              value={description}
              onChange={(e) => setDescription(e.target.value)}
              placeholder="e.g. Systems architecture, performance & reliability"
              className="w-full bg-zinc-950 border border-zinc-800 rounded-lg px-3 py-2 text-xs text-zinc-100 placeholder-zinc-600 focus:border-sky-500 focus:outline-none transition-colors"
            />
          </div>

          <InstructionsField
            value={instructions}
            onChange={setInstructions}
            label="Team instructions"
            hint="Pinned for every bot in this team, before each turn. Leave empty to unpin."
            rows={4}
          />

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
              <div className="space-y-2">
                <div className="flex gap-2 items-center">
                  <input
                    type="text"
                    value={workspacePath}
                    onChange={(e) => setWorkspacePath(e.target.value)}
                    placeholder="e.g. ~/projects/my-app or /path/to/folder"
                    className="flex-1 bg-zinc-950 border border-zinc-800 rounded-lg px-3 py-2 text-xs text-zinc-100 placeholder-zinc-600 focus:border-sky-500 focus:outline-none transition-colors font-mono"
                  />
                  <button
                    type="button"
                    onClick={handleBrowseFolder}
                    disabled={isBrowsing}
                    className="flex items-center gap-1.5 px-3 py-2 bg-zinc-800 hover:bg-zinc-700 active:bg-zinc-600 text-zinc-200 border border-zinc-700 rounded-lg text-xs font-medium transition-colors shadow-sm cursor-pointer whitespace-nowrap disabled:opacity-50"
                    title="Open native folder chooser"
                  >
                    <Folder size={14} className="text-amber-400" />
                    <span>{isBrowsing ? 'Selecting...' : 'Browse...'}</span>
                  </button>
                  <button
                    type="button"
                    onClick={() => {
                      playSfx('click');
                      setIsFolderBrowserOpen(true);
                    }}
                    className="p-2 bg-zinc-800 hover:bg-zinc-700 text-zinc-300 border border-zinc-700 rounded-lg transition-colors cursor-pointer"
                    title="Explore local directories in-app"
                  >
                    <FolderSearch size={14} className="text-sky-400" />
                  </button>
                </div>

                {/* Quick Directory Chips */}
                <div className="flex flex-wrap items-center gap-1.5 text-[11px]">
                  <span className="text-zinc-500">Quick:</span>
                  <button
                    type="button"
                    onClick={() => {
                      playSfx('click');
                      setWorkspacePath('~/dev/my-app');
                    }}
                    className="px-2 py-0.5 rounded bg-zinc-950 border border-zinc-800 text-zinc-400 hover:text-zinc-200 hover:border-zinc-700 font-mono transition-colors"
                  >
                    ~/dev/my-app
                  </button>
                  <button
                    type="button"
                    onClick={() => {
                      playSfx('click');
                      setWorkspacePath('~');
                    }}
                    className="px-2 py-0.5 rounded bg-zinc-950 border border-zinc-800 text-zinc-400 hover:text-zinc-200 hover:border-zinc-700 font-mono transition-colors"
                  >
                    ~
                  </button>
                  <button
                    type="button"
                    onClick={() => {
                      playSfx('click');
                      setWorkspacePath('~/dev');
                    }}
                    className="px-2 py-0.5 rounded bg-zinc-950 border border-zinc-800 text-zinc-400 hover:text-zinc-200 hover:border-zinc-700 font-mono transition-colors"
                  >
                    ~/dev
                  </button>
                </div>

                <p className="text-[11px] text-zinc-500">
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

          {/* Lead Kwant Selector */}
          <div>
            <label className="block text-xs font-medium text-zinc-400 mb-1">Team Lead</label>
            <select
              value={leadKwantId}
              onChange={(e) => setLeadKwantId(e.target.value)}
              className="w-full bg-zinc-950 border border-zinc-800 rounded-lg px-3 py-2 text-xs text-zinc-100 focus:border-sky-500 focus:outline-none transition-colors"
            >
              {kwants
                .filter((k) => memberIds.includes(k.id))
                .map((k) => (
                  <option key={k.id} value={k.id}>
                    {k.name} — {k.title}
                  </option>
                ))}
            </select>
          </div>

          {/* Members Roster Checklist */}
          <div>
            <div className="flex items-center justify-between mb-1.5">
              <label className="block text-xs font-medium text-zinc-400">
                Team Roster ({memberIds.length}/{kwants.length} Assigned)
              </label>
              <span className="text-[11px] text-zinc-500">Click to add/remove</span>
            </div>
            <div className="space-y-1.5 max-h-48 overflow-y-auto p-1 bg-zinc-950/60 border border-zinc-800/80 rounded-xl">
              {kwants.map((k) => {
                const isMember = memberIds.includes(k.id);
                const isLead = leadKwantId === k.id;
                return (
                  <div
                    key={k.id}
                    onClick={() => handleToggleMember(k.id)}
                    className={`flex items-center justify-between p-2 rounded-lg cursor-pointer transition-all border ${
                      isMember
                        ? 'bg-zinc-800/70 border-sky-500/30 text-zinc-100'
                        : 'bg-zinc-900/30 border-zinc-800/40 text-zinc-400 hover:bg-zinc-800/30 hover:text-zinc-200'
                    }`}
                  >
                    <div className="flex items-center gap-2.5 min-w-0">
                      <ModernAvatar role={k.avatar_sprite} name={k.name} size={28} />
                      <div className="min-w-0">
                        <div className="flex items-center gap-1.5">
                          <span className="text-xs font-medium text-zinc-200 truncate">{k.name}</span>
                          {isLead && (
                            <span className="text-[9px] font-semibold uppercase px-1 py-0.2 rounded bg-sky-500/20 text-sky-300 border border-sky-500/30 flex items-center gap-0.5">
                              <ShieldCheck size={9} />
                              Lead
                            </span>
                          )}
                        </div>
                        <p className="text-[10px] text-zinc-400 truncate">{k.title}</p>
                      </div>
                    </div>

                    <div
                      className={`w-5 h-5 rounded flex items-center justify-center border transition-all ${
                        isMember
                          ? 'bg-sky-500 text-white border-sky-400'
                          : 'border-zinc-700 bg-zinc-900 text-transparent'
                      }`}
                    >
                      <Check size={13} strokeWidth={2.5} />
                    </div>
                  </div>
                );
              })}
            </div>
          </div>

          {/* Action Buttons & Danger Zone */}
          <div className="pt-2 border-t border-zinc-800 flex items-center justify-between gap-3">
            <button
              type="button"
              onClick={handleDelete}
              disabled={isDeleting}
              className={`flex items-center gap-1.5 px-3 py-1.5 rounded-lg text-xs font-medium border transition-all ${
                confirmDelete
                  ? 'bg-red-600 text-white border-red-500 animate-pulse'
                  : 'text-red-400 hover:text-red-300 bg-red-500/10 hover:bg-red-500/20 border-red-500/20'
              }`}
            >
              <Trash2 size={13} />
              <span>{confirmDelete ? 'Confirm Delete?' : 'Delete Team'}</span>
            </button>

            <div className="flex items-center gap-2">
              <button
                type="button"
                onClick={() => {
                  playSfx('click');
                  onClose();
                }}
                className="px-3 py-1.5 rounded-lg text-xs font-medium text-zinc-400 hover:text-zinc-200 hover:bg-zinc-800 transition-colors"
              >
                Cancel
              </button>
              <button
                type="submit"
                disabled={isSaving}
                className="flex items-center gap-1.5 px-4 py-1.5 rounded-lg text-xs font-medium bg-sky-500 hover:bg-sky-400 text-white shadow-sm transition-all"
              >
                <Check size={14} />
                <span>{isSaving ? 'Saving...' : 'Save Changes'}</span>
              </button>
            </div>
          </div>

          <DeleteNote text={deleteError} />
        </form>
      </div>

      <FolderBrowserModal
        isOpen={isFolderBrowserOpen}
        initialPath={workspacePath}
        onClose={() => setIsFolderBrowserOpen(false)}
        onSelectPath={(p) => {
          setWorkspacePath(p);
          playSfx('coin');
        }}
      />
    </div>
  );
};
