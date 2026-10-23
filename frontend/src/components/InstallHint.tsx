import React, { useEffect, useState } from 'react';
import { Download, Share, SquarePlus, X } from 'lucide-react';
import { INSTALL_HINT_DISMISSED_KEY, installHintKind, isIos, isStandalone } from '../pwa';

/** Chrome's install event: not in the DOM types yet. */
interface InstallPromptEvent extends Event {
  prompt: () => Promise<void>;
  userChoice: Promise<{ outcome: 'accepted' | 'dismissed' }>;
}

const PHONE_QUERY = '(max-width: 767px)';

const readDismissed = () => {
  try {
    return localStorage.getItem(INSTALL_HINT_DISMISSED_KEY) === '1';
  } catch {
    return false;
  }
};

/** "Install on your phone": the browser's install button on Android, the Share steps on iPhone.
 *  Sits above the phone tab bar until it's installed or dismissed. */
export const InstallHint: React.FC = () => {
  const [prompt, setPrompt] = useState<InstallPromptEvent | null>(null);
  const [dismissed, setDismissed] = useState(readDismissed);
  const [installed, setInstalled] = useState(() => isStandalone(window));
  const [phone, setPhone] = useState(() => !!window.matchMedia?.(PHONE_QUERY).matches);

  useEffect(() => {
    const onPrompt = (e: Event) => {
      // Keep Chrome's own mini bar away; the hint offers the same install.
      e.preventDefault();
      setPrompt(e as InstallPromptEvent);
    };
    const onInstalled = () => setInstalled(true);
    const mq = window.matchMedia?.(PHONE_QUERY);
    const onMq = () => setPhone(!!mq?.matches);
    window.addEventListener('beforeinstallprompt', onPrompt);
    window.addEventListener('appinstalled', onInstalled);
    mq?.addEventListener('change', onMq);
    return () => {
      window.removeEventListener('beforeinstallprompt', onPrompt);
      window.removeEventListener('appinstalled', onInstalled);
      mq?.removeEventListener('change', onMq);
    };
  }, []);

  const kind = installHintKind({
    standalone: installed,
    dismissed,
    phone,
    secure: window.isSecureContext,
    ios: isIos(navigator),
    canPrompt: !!prompt,
  });
  if (!kind) return null;

  const dismiss = () => {
    setDismissed(true);
    try {
      localStorage.setItem(INSTALL_HINT_DISMISSED_KEY, '1');
    } catch {
      // Private browsing: it comes back next time, which is fine.
    }
  };

  const install = async () => {
    if (!prompt) return;
    await prompt.prompt();
    const { outcome } = await prompt.userChoice.catch(() => ({ outcome: 'dismissed' as const }));
    // A prompt works once; Chrome sends a new event if it can be offered again.
    setPrompt(null);
    if (outcome === 'accepted') setInstalled(true);
  };

  return (
    <aside
      aria-label="Install on your phone"
      data-install-hint={kind}
      className="fixed inset-x-3 z-[55] bottom-[calc(env(safe-area-inset-bottom)+4.25rem)] flex items-start gap-3 rounded-xl border border-sky-500/30 bg-zinc-900/95 p-3 shadow-xl text-xs backdrop-blur"
    >
      <img src="/icons/icon-192.png" alt="" width={36} height={36} className="shrink-0 rounded-lg [image-rendering:pixelated]" />
      <div className="flex-1 min-w-0">
        <p className="font-medium text-zinc-100">Install kwant bots on your phone</p>
        {kind === 'prompt' ? (
          <>
            <p className="mt-0.5 text-zinc-400">It opens full screen from your home screen, like an app.</p>
            <button
              type="button"
              onClick={install}
              className="mt-2 inline-flex min-h-[44px] items-center gap-1.5 rounded-lg bg-sky-600 px-4 font-medium text-white hover:bg-sky-500 focus-visible:outline focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-sky-400"
            >
              <Download size={14} aria-hidden="true" /> Install
            </button>
          </>
        ) : (
          <p className="mt-0.5 text-zinc-400 leading-relaxed">
            Tap <Share size={13} className="inline -mt-0.5 text-sky-300" aria-label="Share" /> Share, then{' '}
            <SquarePlus size={13} className="inline -mt-0.5 text-sky-300" aria-hidden="true" /> Add to Home Screen.
          </p>
        )}
      </div>
      <button
        type="button"
        onClick={dismiss}
        aria-label="Not now"
        className="-m-1 flex h-11 w-11 shrink-0 items-center justify-center rounded-lg text-zinc-500 hover:bg-zinc-800 hover:text-zinc-200 focus-visible:outline focus-visible:outline-2 focus-visible:outline-sky-500"
      >
        <X size={16} />
      </button>
    </aside>
  );
};
