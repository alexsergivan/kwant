import React from 'react';
import { ShieldAlert, Check, CheckCheck, X } from 'lucide-react';
import { playSfx } from '../audio/chiptune';

export interface PendingApproval {
  id: string;
  tool_name: string;
  details: Record<string, unknown>;
}

interface ModernApprovalCardProps {
  approval: PendingApproval;
  onResolve: (id: string, decision: 'allow_once' | 'always_allow' | 'deny') => void;
}

export const ModernApprovalCard: React.FC<ModernApprovalCardProps> = ({
  approval,
  onResolve,
}) => {
  const handleDecision = (decision: 'allow_once' | 'always_allow' | 'deny') => {
    playSfx('click');
    onResolve(approval.id, decision);
  };

  return (
    <div className="my-3 border border-amber-500/30 bg-amber-950/20 rounded-xl p-4 text-zinc-100 max-w-2xl shadow-lg backdrop-blur-sm">
      <div className="flex items-center gap-2 mb-2 text-amber-400 font-semibold text-xs tracking-wide">
        <ShieldAlert size={16} />
        <span>Action Approval Required</span>
      </div>

      <div className="bg-zinc-900/90 rounded-lg p-3 border border-zinc-800 text-xs font-mono space-y-1 mb-3">
        <div className="text-zinc-400">
          <span className="text-sky-400 font-semibold">Tool:</span> {approval.tool_name}
        </div>
        <div className="text-zinc-300 break-all overflow-x-auto max-h-36 py-1">
          {JSON.stringify(approval.details, null, 2)}
        </div>
      </div>

      <div className="flex items-center justify-end gap-2">
        <button
          onClick={() => handleDecision('deny')}
          className="flex items-center gap-1.5 px-3 py-1.5 rounded-lg text-xs font-medium text-zinc-300 bg-zinc-800 hover:bg-zinc-700 transition-colors"
        >
          <X size={14} />
          <span>Deny</span>
        </button>

        <button
          onClick={() => handleDecision('allow_once')}
          className="flex items-center gap-1.5 px-3 py-1.5 rounded-lg text-xs font-medium text-white bg-emerald-600 hover:bg-emerald-500 transition-colors shadow-sm"
        >
          <Check size={14} />
          <span>Allow Once</span>
        </button>

        <button
          onClick={() => handleDecision('always_allow')}
          className="flex items-center gap-1.5 px-3 py-1.5 rounded-lg text-xs font-medium text-white bg-sky-600 hover:bg-sky-500 transition-colors shadow-sm"
        >
          <CheckCheck size={14} />
          <span>Always Allow</span>
        </button>
      </div>
    </div>
  );
};
