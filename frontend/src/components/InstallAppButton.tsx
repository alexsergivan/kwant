import React, { useId, useState } from 'react';
import { Download, ExternalLink, Laptop, Share, SquarePlus } from 'lucide-react';
import { MAC_DOWNLOAD_URL, desktopInstallKind, isEmbeddedWebView, isIos, isMacSafari, isStandalone, runInstallPrompt } from '../pwa';
import { useInstall } from '../useInstall';

const onMac = () => /Macintosh/.test(navigator.userAgent) && !isIos(navigator);

/** "Install app" in the sidebar, on desktop: Chrome and Edge install in one click; other browsers
 *  get the steps that work there. Gone once kwant runs installed, or in the Mac app. */
export const InstallAppButton: React.FC<{ className: string }> = ({ className }) => {
  const { prompt, installed } = useInstall();
  const [open, setOpen] = useState(false);
  const panelId = useId();

  // Server rendering (tests) has no browser to install into.
  const kind = typeof window === 'undefined' ? null : desktopInstallKind({
    standalone: installed || isStandalone(window),
    embedded: isEmbeddedWebView(navigator),
    secure: window.isSecureContext,
    canPrompt: !!prompt,
    macSafari: isMacSafari(navigator),
    ios: isIos(navigator),
  });
  if (!kind) return null;

  const onClick = () => {
    if (kind === 'prompt') void runInstallPrompt();
    else setOpen((o) => !o);
  };

  return (
    <div className="hidden md:block" data-install-app={kind}>
      <button
        type="button"
        onClick={onClick}
        aria-expanded={kind === 'prompt' ? undefined : open}
        aria-controls={kind === 'prompt' ? undefined : panelId}
        className={className}
      >
        <Download size={15} className="text-zinc-500" aria-hidden="true" />
        <span className="truncate flex-1">Install app</span>
      </button>
      {open && kind !== 'prompt' && (
        <div id={panelId} className="mx-2.5 mt-1 mb-1.5 rounded-lg border border-zinc-800 bg-zinc-900/70 p-2.5 text-[11px] leading-relaxed text-zinc-400">
          {kind === 'safari' ? (
            <p>
              In Safari, choose <span className="text-zinc-200">File › Add to Dock</span>. kwant bots then opens in its own window.
            </p>
          ) : kind === 'ios' ? (
            <p>
              Tap <Share size={12} className="inline -mt-0.5 text-sky-300" aria-label="Share" /> Share, then{' '}
              <SquarePlus size={12} className="inline -mt-0.5 text-sky-300" aria-hidden="true" /> Add to Home Screen.
            </p>
          ) : kind === 'insecure' ? (
            <p>
              Installing needs <span className="text-zinc-200">https</span>. Browsers don't install pages served over plain http. Put kwant bots
              behind an https proxy, or open it at 127.0.0.1 on the machine it runs on.
            </p>
          ) : (
            <p>
              Use your browser's install option. In Chrome or Edge it's the install icon at the right of the address bar. Firefox can't install
              web apps.
            </p>
          )}
          {onMac() && (
            <a
              href={MAC_DOWNLOAD_URL}
              target="_blank"
              rel="noopener noreferrer"
              className="mt-2 flex items-center gap-1.5 text-sky-300 hover:text-sky-200 focus-visible:outline focus-visible:outline-2 focus-visible:outline-sky-500 rounded"
            >
              <Laptop size={12} aria-hidden="true" /> Download the Mac app <ExternalLink size={11} aria-hidden="true" />
            </a>
          )}
          {onMac() && <p className="mt-0.5 text-zinc-500">It runs its own kwant bots on this Mac.</p>}
        </div>
      )}
    </div>
  );
};
