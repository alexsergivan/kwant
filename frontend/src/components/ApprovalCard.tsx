import React from 'react';
import { playSfx } from '../audio/chiptune';

export interface PendingApproval {
  id: string;
  tool_name: string;
  details: Record<string, unknown>;
}

interface ApprovalCardProps {
  approval: PendingApproval;
  onResolve: (id: string, decision: 'allow_once' | 'always_allow' | 'deny') => void;
}

export const ApprovalCard: React.FC<ApprovalCardProps> = ({ approval, onResolve }) => {
  React.useEffect(() => {
    playSfx('alert');
  }, [approval.id]);

  return (
    <div className="pixel-box-amber p-4 my-4 max-w-xl mx-auto animate-bounce-short">
      {/* Header */}
      <div className="flex items-center justify-between border-b-2 border-arcade-amber pb-2 mb-3">
        <div className="flex items-center gap-2">
          <span className="w-2.5 h-2.5 bg-arcade-magenta inline-block animate-ping" />
          <h4 className="font-pixel text-[10px] text-arcade-amber">
            ⚔️ BATTLE DECISION REQUIRED
          </h4>
        </div>
        <span className="font-pixel text-[8px] bg-black text-red-400 px-1.5 py-0.5 border border-red-900">
          GATE
        </span>
      </div>

      {/* Tool details */}
      <div className="font-retro text-xs text-zinc-300 space-y-2 mb-4 bg-black/60 p-3 border border-zinc-800">
        <div>
          <span className="text-zinc-500 font-pixel text-[9px]">TOOL: </span>
          <span className="text-arcade-cyan font-bold">{approval.tool_name}</span>
        </div>

        <div>
          <span className="text-zinc-500 font-pixel text-[9px]">PARAMETERS: </span>
          <pre className="font-terminal text-sm text-green-400 mt-1 whitespace-pre-wrap break-all max-h-32 overflow-y-auto">
            {JSON.stringify(approval.details, null, 2)}
          </pre>
        </div>
      </div>

      {/* Decision Buttons */}
      <div className="flex flex-wrap items-center gap-3 justify-end pt-1">
        <button
          onClick={() => {
            playSfx('click');
            onResolve(approval.id, 'deny');
          }}
          className="pixel-btn pixel-btn-magenta text-[9px]"
        >
          🛑 FLEE (DENY)
        </button>

        <button
          onClick={() => {
            playSfx('coin');
            onResolve(approval.id, 'always_allow');
          }}
          className="pixel-btn pixel-btn-cyan text-[9px]"
        >
          🛡️ ALWAYS ALLOW
        </button>

        <button
          onClick={() => {
            playSfx('coin');
            onResolve(approval.id, 'allow_once');
          }}
          className="pixel-btn pixel-btn-amber text-[9px]"
        >
          ⚔️ ALLOW ONCE
        </button>
      </div>
    </div>
  );
};
