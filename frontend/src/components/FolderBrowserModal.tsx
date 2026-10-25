import React, { useState, useEffect } from 'react';
import { X, Folder, ChevronRight, ArrowUp, Check, RefreshCw, FolderSearch, Home, Laptop } from 'lucide-react';
import { playSfx } from '../audio/chiptune';

export interface FolderBrowserModalProps {
  isOpen: boolean;
  initialPath?: string;
  onClose: () => void;
  onSelectPath: (path: string) => void;
}

interface QuickPath {
  name: string;
  path: string;
}

interface DirectoriesResponse {
  current_path: string;
  parent_path?: string | null;
  directories: string[];
  quick_paths: QuickPath[];
}

export const FolderBrowserModal: React.FC<FolderBrowserModalProps> = ({
  isOpen,
  initialPath,
  onClose,
  onSelectPath,
}) => {
  const [currentPath, setCurrentPath] = useState(initialPath || '');
  const [parentPath, setParentPath] = useState<string | null>(null);
  const [directories, setDirectories] = useState<string[]>([]);
  const [quickPaths, setQuickPaths] = useState<QuickPath[]>([]);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const fetchDirectories = async (path?: string) => {
    setLoading(true);
    setError(null);
    try {
      const url = path ? `/api/system/directories?path=${encodeURIComponent(path)}` : '/api/system/directories';
      const res = await fetch(url);
      if (!res.ok) {
        throw new Error('Failed to load folder directory');
      }
      const data: DirectoriesResponse = await res.json();
      setCurrentPath(data.current_path);
      setParentPath(data.parent_path || null);
      setDirectories(data.directories);
      setQuickPaths(data.quick_paths);
    } catch (err) {
      setError(err instanceof Error ? err.message : 'Error reading folder');
    } finally {
      setLoading(false);
    }
  };

  useEffect(() => {
    if (isOpen) {
      fetchDirectories(initialPath);
    }
  }, [isOpen, initialPath]);

  if (!isOpen) return null;

  const handleNavigate = (subDir: string) => {
    playSfx('click');
    const separator = currentPath.endsWith('/') ? '' : '/';
    const target = `${currentPath}${separator}${subDir}`;
    fetchDirectories(target);
  };

  const handleNavigateParent = () => {
    if (parentPath) {
      playSfx('click');
      fetchDirectories(parentPath);
    }
  };

  const handleSelectCurrent = () => {
    playSfx('fanfare');
    onSelectPath(currentPath);
    onClose();
  };

  // Split path for breadcrumb navigation
  const pathParts = currentPath.split('/').filter(Boolean);

  return (
    <div className="kw-dialog fixed inset-0 z-50 bg-black/80 backdrop-blur-sm flex items-center justify-center p-4 select-none animate-in fade-in duration-150">
      <div className="kw-dialog-panel bg-zinc-900 border border-zinc-800 rounded-2xl max-w-xl w-full p-5 text-zinc-100 shadow-2xl flex flex-col max-h-[85vh]">
        {/* Header */}
        <div className="flex items-center justify-between border-b border-zinc-800 pb-3 mb-3">
          <div className="flex items-center gap-2">
            <div className="w-7 h-7 rounded-lg bg-amber-500/10 border border-amber-500/20 flex items-center justify-center text-amber-400">
              <FolderSearch size={15} />
            </div>
            <div>
              <h3 className="text-sm font-semibold text-zinc-100">Select Folder</h3>
              <p className="text-[11px] text-zinc-400">Choose a local directory for your team workspace</p>
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

        {/* Quick Jump Locations */}
        {quickPaths.length > 0 && (
          <div className="mb-3">
            <div className="text-[11px] font-medium text-zinc-400 mb-1.5 flex items-center gap-1">
              <span>Quick Jumps:</span>
            </div>
            <div className="flex flex-wrap gap-1.5">
              {quickPaths.map((qp, idx) => (
                <button
                  key={idx}
                  type="button"
                  onClick={() => {
                    playSfx('click');
                    fetchDirectories(qp.path);
                  }}
                  className={`px-2 py-1 rounded-md text-[11px] font-mono border transition-all flex items-center gap-1.5 ${
                    currentPath === qp.path
                      ? 'bg-amber-500/20 text-amber-300 border-amber-500/40 shadow-sm'
                      : 'bg-zinc-950 text-zinc-400 border-zinc-800 hover:border-zinc-700 hover:text-zinc-200'
                  }`}
                >
                  {qp.name.includes('Home') ? (
                    <Home size={11} className="text-sky-400" />
                  ) : (
                    <Laptop size={11} className="text-zinc-400" />
                  )}
                  <span>{qp.name}</span>
                </button>
              ))}
            </div>
          </div>
        )}

        {/* Current Path Bar & Breadcrumbs */}
        <div className="bg-zinc-950 border border-zinc-800 rounded-lg p-2 mb-3 flex items-center justify-between gap-2 overflow-x-auto text-xs font-mono">
          <div className="flex items-center gap-1 min-w-0 flex-1 overflow-x-auto">
            <button
              type="button"
              onClick={() => fetchDirectories('/')}
              className="hover:text-amber-400 text-zinc-500 px-1 py-0.5 rounded transition-colors"
            >
              /
            </button>
            {pathParts.map((part, index) => {
              const fullSegmentPath = '/' + pathParts.slice(0, index + 1).join('/');
              const isLast = index === pathParts.length - 1;

              return (
                <React.Fragment key={index}>
                  <ChevronRight size={12} className="text-zinc-600 flex-shrink-0" />
                  <button
                    type="button"
                    onClick={() => fetchDirectories(fullSegmentPath)}
                    className={`px-1 py-0.5 rounded transition-colors truncate max-w-[140px] ${
                      isLast ? 'text-amber-400 font-semibold bg-amber-500/10' : 'text-zinc-400 hover:text-zinc-200'
                    }`}
                    title={fullSegmentPath}
                  >
                    {part}
                  </button>
                </React.Fragment>
              );
            })}
          </div>

          <div className="flex items-center gap-1 flex-shrink-0">
            {parentPath && (
              <button
                type="button"
                onClick={handleNavigateParent}
                className="p-1 rounded bg-zinc-900 border border-zinc-800 hover:bg-zinc-800 text-zinc-300 text-[11px] flex items-center gap-1 px-2 transition-colors"
                title="Go to parent directory"
              >
                <ArrowUp size={12} />
                <span>Up</span>
              </button>
            )}
            <button
              type="button"
              onClick={() => fetchDirectories(currentPath)}
              className="p-1 rounded bg-zinc-900 border border-zinc-800 hover:bg-zinc-800 text-zinc-400 hover:text-zinc-200 transition-colors"
              title="Refresh"
            >
              <RefreshCw size={12} className={loading ? 'animate-spin' : ''} />
            </button>
          </div>
        </div>

        {error && (
          <div className="p-2 mb-3 bg-rose-500/10 border border-rose-500/20 rounded-lg text-rose-300 text-xs">
            {error}
          </div>
        )}

        {/* Directory List */}
        <div className="flex-1 overflow-y-auto border border-zinc-800 rounded-lg bg-zinc-950/60 p-1 min-h-[200px] max-h-[300px]">
          {loading ? (
            <div className="flex items-center justify-center h-36 text-xs text-zinc-500 gap-2">
              <RefreshCw size={14} className="animate-spin text-amber-400" />
              <span>Scanning directories...</span>
            </div>
          ) : directories.length === 0 ? (
            <div className="flex flex-col items-center justify-center h-36 text-xs text-zinc-500">
              <Folder size={24} className="text-zinc-600 mb-1 opacity-60" />
              <span>No subdirectories in this folder</span>
            </div>
          ) : (
            <div className="grid grid-cols-1 sm:grid-cols-2 gap-1 p-1">
              {directories.map((dir) => (
                <button
                  key={dir}
                  type="button"
                  onClick={() => handleNavigate(dir)}
                  className="flex items-center gap-2 p-2 rounded-lg text-left text-xs text-zinc-300 hover:bg-zinc-800/80 hover:text-zinc-100 border border-transparent hover:border-zinc-700/60 transition-all group"
                >
                  <Folder size={15} className="text-amber-400/80 group-hover:text-amber-400 flex-shrink-0" />
                  <span className="truncate flex-1 font-mono">{dir}</span>
                  <ChevronRight size={12} className="text-zinc-600 group-hover:text-zinc-400 flex-shrink-0" />
                </button>
              ))}
            </div>
          )}
        </div>

        {/* Footer Actions */}
        <div className="flex items-center justify-between pt-3 mt-3 border-t border-zinc-800">
          <div className="text-[11px] text-zinc-400 truncate max-w-xs font-mono">
            Selected: <span className="text-zinc-200">{currentPath || 'None'}</span>
          </div>
          <div className="flex items-center gap-2">
            <button
              type="button"
              onClick={() => {
                playSfx('click');
                onClose();
              }}
              className="px-3 py-1.5 rounded-lg text-xs font-medium text-zinc-300 hover:text-zinc-100 hover:bg-zinc-800 transition-colors"
            >
              Cancel
            </button>
            <button
              type="button"
              onClick={handleSelectCurrent}
              disabled={!currentPath}
              className="flex items-center gap-1.5 px-3.5 py-1.5 bg-sky-600 hover:bg-sky-500 text-white rounded-lg text-xs font-medium transition-colors shadow-sm disabled:opacity-50 disabled:cursor-not-allowed cursor-pointer"
            >
              <Check size={13} />
              <span>Select Current Folder</span>
            </button>
          </div>
        </div>
      </div>
    </div>
  );
};
