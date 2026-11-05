import React from 'react';

/** A bot in the space, for names ("Lin is peeking") and the "Who should do it" card. */
export interface IdeaJarBot {
  id: string;
  name: string;
  role?: string | null;
}

export interface IdeaJarViewProps {
  spaceId: string;
  /** Open on board: the board with this (draft) mission selected. */
  onOpenMission: (missionId: string) => void;
  /** For copy like "Bots have had their 3 peeks for today in Trailhead". */
  spaceName?: string;
  /** The space's bots; without them the view says "A bot". */
  bots?: IdeaJarBot[];
  /** Missions App already has, for the Picked chips (by mission_id). */
  missions?: { id: string; status: string }[];
  /** Header Settings and "Turn on in Settings": open Settings focused on the idea-jar row. */
  onOpenSettings?: () => void;
}

export const IdeaJarView: React.FC<IdeaJarViewProps> = () => <div data-idea-jar>Idea jar</div>;
