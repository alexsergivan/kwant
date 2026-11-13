// 0.7.7 entry points bound to the ReadyTeams provider: the welcome cards (T7.1), the New team tiles
// (T7.2), the sidebar box (T7.3) and "Suggest members from the code" (D1). Each renders nothing on a
// server without the templates route.
import React, { useState } from 'react';
import { ScanSearch } from 'lucide-react';
import { useReadyTeams, type ReadyTeam } from './ReadyTeams';
import { TemplateSidebarBox, TemplateTiles, TemplateWelcome } from './TeamTemplatePicks';
import { useMedia } from '../useMedia';
import { PHONE_QUERY } from '../layout';
import { hideReadyTeams, readyTeamsHidden } from '../teams/teamTemplates';

const FOCUS = 'focus-visible:outline focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-sky-400';

export const ReadyTeamsWelcome: React.FC<{ onBlank: () => void }> = ({ onBlank }) => {
  const rt = useReadyTeams();
  if (!rt?.available || !rt.templates?.length) return null;
  return <TemplateWelcome templates={rt.templates} onPick={(id) => rt.open({ kind: 'template', templateId: id })} onBlank={onBlank} />;
};

/**
 * T7.2 at the top of New team. Picking a tile hides the dialog (`onHide`) and opens the preview; Esc
 * there brings the dialog back with its fields (`onShow`). The Coding tile with a folder or repo typed
 * reads the code first (T1).
 */
export const ReadyTeamsTiles: React.FC<{
  source: { workspace_path?: string; repo_url?: string; name?: string };
  onHide: () => void;
  onShow: () => void;
  onDone: () => void;
}> = ({ source, onHide, onShow, onDone }) => {
  const rt = useReadyTeams();
  if (!rt?.available || !rt.templates?.length) return null;
  return (
    <TemplateTiles
      templates={rt.templates}
      onPick={(id) => {
        onHide();
        const hasSource = !!(source.workspace_path || source.repo_url);
        rt.open(
          id === 'coding' && hasSource
            ? { kind: 'suggest', workspace_path: source.workspace_path || null, repo_url: source.repo_url || null, handleSeed: source.name, viaCoding: true }
            : { kind: 'template', templateId: id },
          { onCancel: onShow, onDone },
        );
      }}
    />
  );
};

/** T7.3: while the space has no teams and the box wasn't hidden here. */
export const ReadyTeamsSidebarBox: React.FC<{ teamCount: number }> = ({ teamCount }) => {
  const rt = useReadyTeams();
  const [hidden, setHidden] = useState(() => (rt ? readyTeamsHidden(rt.space) : false));
  if (!rt?.available || !rt.templates?.length || teamCount > 0 || hidden || readyTeamsHidden(rt.space)) return null;
  return (
    <TemplateSidebarBox
      templates={rt.templates}
      onPick={(id) => rt.open({ kind: 'template', templateId: id })}
      onDescribe={rt.describe}
      onHide={() => {
        hideReadyTeams(rt.space);
        setHidden(true);
      }}
    />
  );
};

/**
 * D1. New team: under the workspace field once Local Folder or Git Repo has a value. Manage team: with
 * `team`, labelled "Suggest members", only when the team has a folder or repo.
 */
export const SuggestMembersButton: React.FC<{
  workspace_path?: string;
  repo_url?: string;
  /** The New team name, the sheet's handle. */
  name?: string;
  team?: ReadyTeam;
  onHide?: () => void;
  onShow?: () => void;
  onDone?: () => void;
}> = ({ workspace_path, repo_url, name, team, onHide, onShow, onDone }) => {
  const rt = useReadyTeams();
  const phone = useMedia(PHONE_QUERY);
  const path = workspace_path?.trim();
  const url = repo_url?.trim();
  if (!rt?.available || (!path && !url)) return null;
  return (
    <div className="mt-2" data-suggest-members>
      <button
        type="button"
        onClick={() => {
          onHide?.();
          rt.open(
            team
              ? { kind: 'suggest', team: { id: team.id, name: team.name }, workspace_path: path || null, repo_url: url || null }
              : { kind: 'suggest', workspace_path: path || null, repo_url: path ? null : url || null, handleSeed: name },
            { onCancel: onShow, onDone },
          );
        }}
        className={`inline-flex items-center gap-1.5 ${phone ? 'min-h-11' : 'min-h-8'} px-3 rounded-lg border border-sky-300/35 bg-sky-300/[.08] text-[12.5px] font-medium text-sky-300 hover:bg-sky-300/[.14] ${FOCUS}`}
      >
        <ScanSearch size={14} aria-hidden="true" />
        {team ? 'Suggest members' : 'Suggest members from the code'}
      </button>
      <p className="mt-1 text-[11px] text-zinc-400">
        {team ? `Finds roles @${team.name} doesn’t have yet, from file names and manifests.` : 'Reads file names and manifests only. No model, about 10 seconds.'}
      </p>
    </div>
  );
};
