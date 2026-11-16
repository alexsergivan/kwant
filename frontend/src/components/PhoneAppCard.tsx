import React, { useEffect, useState } from 'react';
import { AlertTriangle, Check, Copy, Download, EllipsisVertical, ExternalLink, Share, Smartphone, SquarePlus } from 'lucide-react';
import {
  HTTPS_GUIDE_URL,
  PHONE_GUIDE_URL,
  TELEGRAM_GUIDE_URL,
  type PhoneAppKind,
  isAndroid,
  isEmbeddedWebView,
  isIos,
  isStandalone,
  phoneAppKind,
  phoneCardShown,
  runInstallPrompt,
} from '../pwa';
import { useInstall } from '../useInstall';
import { useAuth } from '../authContext';
import { AddDevice, QrImage } from './DeviceSignIn';
import { InstallAppButton, useDesktopInstallKind } from './InstallAppButton';
import { desktopAppOs, inDesktopApp } from '../externalLinks';
import { PHONE_APP_ANCHOR, PHONE_APP_TITLE } from './PhoneAppTip';

// "Use kwant bots on your phone": Settings › All spaces, first card. On a computer it shows a QR code
// (a one-time sign-in link when the server asks for sign-in) and three steps; on a phone it shows how
// to install this phone. On 127.0.0.1 or plain http it says why a phone can't use this address.

/** On a phone, what installing this phone takes: the browser's Install button, the iPhone Share
 *  steps, the Android menu steps, or https first. */
export type PhoneInstallCase = 'prompt' | 'ios' | 'android' | 'insecure';


const link =
  'inline-flex items-center gap-1 text-[11px] text-sky-300 hover:text-sky-200 rounded focus-visible:outline focus-visible:outline-2 focus-visible:outline-sky-500 max-md:min-h-[44px]';
const rowBtn =
  'flex items-center gap-1.5 px-2.5 py-1.5 rounded-lg text-xs font-medium text-zinc-300 border border-zinc-700/60 bg-zinc-800/60 hover:bg-zinc-700/70 hover:text-zinc-100 transition-colors focus-visible:outline focus-visible:outline-2 focus-visible:outline-sky-500';
const copyBtn =
  'shrink-0 flex items-center justify-center gap-1.5 min-h-[36px] max-md:min-h-[44px] px-3 rounded-lg text-xs font-medium text-zinc-200 bg-zinc-800 hover:bg-zinc-700 border border-zinc-700 transition-colors focus-visible:outline focus-visible:outline-2 focus-visible:outline-sky-500';
const inlineIcon = 'inline -mt-0.5 text-sky-300';

const Ext: React.FC<{ href: string; children: React.ReactNode }> = ({ href, children }) => (
  <a href={href} target="_blank" rel="noopener noreferrer" className={link}>
    {children} <ExternalLink size={11} aria-hidden="true" />
  </a>
);

const Mono: React.FC<{ children: React.ReactNode }> = ({ children }) => <span className="font-mono text-zinc-300">{children}</span>;

const Steps: React.FC<{ items: Array<{ title: React.ReactNode; detail?: React.ReactNode; body?: React.ReactNode }> }> = ({ items }) => (
  <ol className="space-y-4">
    {items.map((it, i) => (
      <li key={i} className="flex gap-3">
        <span
          aria-hidden="true"
          className="h-5 w-5 shrink-0 mt-px rounded-full bg-zinc-800 border border-zinc-700 text-[11px] font-semibold text-zinc-200 flex items-center justify-center"
        >
          {i + 1}
        </span>
        <div className="flex-1 min-w-0 space-y-2">
          <p className="text-xs font-medium text-zinc-200">{it.title}</p>
          {it.detail && <div className="text-[11px] text-zinc-400 leading-relaxed">{it.detail}</div>}
          {it.body}
        </div>
      </li>
    ))}
  </ol>
);

const Note: React.FC<{ warn?: boolean; lead: React.ReactNode; children?: React.ReactNode }> = ({ warn, lead, children }) => (
  <div className="rounded-xl border border-zinc-800 bg-zinc-900/60 p-4 space-y-3">
    <p className="flex items-start gap-2 text-xs text-zinc-200 leading-relaxed">
      {warn && <AlertTriangle size={14} className="mt-0.5 shrink-0 text-amber-300" aria-hidden="true" />}
      <span>{lead}</span>
    </p>
    {children}
  </div>
);

export interface PhoneAppCardViewProps {
  kind: PhoneAppKind;
  /** `phone`: install this phone (phones and tablets). `computer`: get it onto a phone. */
  layout: 'computer' | 'phone';
  /** On a phone: how this phone installs. */
  phoneCase?: PhoneInstallCase;
  /** location.host and location.origin. */
  host: string;
  origin: string;
  /** "Mac" in the Mac app or a Mac browser, else "computer". */
  machine: 'Mac' | 'computer';
  /** The server asks for an access token but has no device login: say so under the QR. */
  tokenSignIn?: boolean;
  /** Step 1 of `signin`: AddDevice, minted on click. A slot so tests can render the card alone. */
  signIn?: React.ReactNode;
  /** The "Install on this computer" row, or null when this browser has nothing to install. */
  installFooter?: React.ReactNode;
  copied?: 'yes' | 'failed' | null;
  onCopy?: () => void;
  onInstall?: () => void;
}

const homeScreenStep = {
  title: 'Add it to the Home Screen',
  detail: (
    <div className="space-y-1">
      <p>
        <span className="text-zinc-200">iPhone or iPad:</span> in Safari, tap <Share size={12} className={inlineIcon} aria-label="Share" /> Share ›
        Add to Home Screen.
      </p>
      <p>
        <span className="text-zinc-200">Android:</span> in Chrome, tap <EllipsisVertical size={12} className={inlineIcon} aria-label="menu" /> › Add to
        Home screen (or Install app).
      </p>
    </div>
  ),
};

const notifyStep = {
  title: 'Turn on notifications',
  detail:
    'Open kwant bots from the Home Screen, then go to Settings › All spaces › Notifications › Notify me on this device. On an iPhone this only works in the Home Screen app.',
};

const notifyHere = 'Open kwant bots from the Home Screen and turn on Notify me on this device, under Notifications below.';

/** The card, from plain values. Exported for tests. */
export const PhoneAppCardView: React.FC<PhoneAppCardViewProps> = (p) => {
  const phone = p.layout === 'phone';
  const address = `${p.origin}/`;

  let body: React.ReactNode;
  if (phone) {
    if (p.phoneCase === 'insecure' || p.kind === 'insecure' || p.kind === 'loopback') {
      body = (
        <Note warn lead={<>Phones only install apps from an https address. This one is <Mono>{p.origin}</Mono>.</>}>
          <Ext href={HTTPS_GUIDE_URL}>Set up https</Ext>
        </Note>
      );
    } else if (p.phoneCase === 'prompt') {
      body = (
        <div className="space-y-2">
          <button
            type="button"
            onClick={p.onInstall}
            className="w-full flex items-center justify-center gap-1.5 min-h-[44px] px-4 rounded-lg text-xs font-medium text-white bg-sky-600 hover:bg-sky-700 transition-colors focus-visible:outline focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-sky-500"
          >
            <Download size={14} aria-hidden="true" /> Install
          </button>
          <p className="text-[11px] text-zinc-400 leading-relaxed">Then open it from your Home Screen and turn on Notify me on this device, under Notifications below.</p>
        </div>
      );
    } else if (p.phoneCase === 'ios') {
      body = (
        <div className="space-y-3">
          <Steps
            items={[
              {
                title: (
                  <>
                    Tap <Share size={12} className={inlineIcon} aria-label="Share" /> Share
                  </>
                ),
                detail: 'In Safari it’s at the bottom of the screen; on iPad, at the top.',
              },
              {
                title: (
                  <>
                    Tap <SquarePlus size={12} className={inlineIcon} aria-hidden="true" /> Add to Home Screen, then Add
                  </>
                ),
              },
              { title: notifyHere },
            ]}
          />
          <p className="text-[11px] text-zinc-400">No Add to Home Screen? Open this page in Safari.</p>
        </div>
      );
    } else {
      body = (
        <Steps
          items={[
            {
              title: (
                <>
                  Tap <EllipsisVertical size={12} className={inlineIcon} aria-label="menu" /> in the browser’s menu
                </>
              ),
            },
            { title: 'Tap Add to Home screen (or Install app)' },
            { title: notifyHere },
          ]}
        />
      );
    }
  } else if (p.kind === 'loopback') {
    body = (
      <Note
        lead={
          <>
            Your phone can’t reach this one. kwant bots runs at <Mono>{p.host}</Mono>, an address only this {p.machine} can open.
          </>
        }
      >
        <p className="text-[11px] text-zinc-400">To have it on your phone:</p>
        <ul className="space-y-2">
          <li className="flex gap-2">
            <span className="h-1 w-1 shrink-0 rounded-full bg-zinc-500 mt-1.5" aria-hidden="true" />
            <div className="space-y-0.5">
              <p className="text-[11px] text-zinc-400 leading-relaxed">
                Run kwant bots on a server with an https address. Open that address on this computer, and this card shows a QR code for your phone.
              </p>
              <Ext href={PHONE_GUIDE_URL}>How to set it up</Ext>
            </div>
          </li>
          <li className="flex gap-2">
            <span className="h-1 w-1 shrink-0 rounded-full bg-zinc-500 mt-1.5" aria-hidden="true" />
            <div className="space-y-0.5">
              <p className="text-[11px] text-zinc-400 leading-relaxed">No server? Talk to your bots from your phone in Telegram.</p>
              <Ext href={TELEGRAM_GUIDE_URL}>Set up Telegram</Ext>
            </div>
          </li>
        </ul>
      </Note>
    );
  } else if (p.kind === 'insecure') {
    body = (
      <Note
        warn
        lead={
          <>
            Phones only install apps from an https address. This one is <Mono>{p.origin}</Mono>, so a phone can open it but can’t add it to the Home
            Screen or get notifications.
          </>
        }
      >
        <p className="text-[11px] text-zinc-400 leading-relaxed">
          Put kwant bots behind an https proxy, then open the https address on this computer. This card then shows a QR code for your phone.
        </p>
        <Ext href={HTTPS_GUIDE_URL}>Set up https</Ext>
      </Note>
    );
  } else {
    const step1 =
      p.kind === 'signin'
        ? { title: 'Scan with your phone’s camera', detail: 'It opens kwant bots on the phone and signs it in once.', body: p.signIn }
        : {
            title: 'Scan with your phone’s camera',
            detail: (
              <>
                It opens kwant bots at <Mono>{p.host}</Mono> on the phone.{p.tokenSignIn && ' Sign in there with your access token.'}
              </>
            ),
            body: (
              <div className="rounded-xl border border-sky-500/30 bg-sky-500/5 p-4 space-y-3" data-phone-qr>
                <div className="mx-auto w-full max-w-[176px]">
                  <QrImage text={address} label="QR code for this kwant bots address" />
                </div>
                <div className="flex gap-2">
                  <label className="sr-only" htmlFor="phone-app-address">
                    kwant bots address
                  </label>
                  <input
                    id="phone-app-address"
                    readOnly
                    value={address}
                    onFocus={(e) => e.currentTarget.select()}
                    className="flex-1 min-w-0 rounded-lg bg-zinc-950 border border-zinc-700 px-2.5 text-[11px] font-mono text-zinc-300"
                  />
                  <button type="button" onClick={p.onCopy} className={copyBtn}>
                    {p.copied === 'yes' ? <Check size={14} aria-hidden="true" /> : <Copy size={14} aria-hidden="true" />}
                    {p.copied === 'yes' ? 'Copied' : 'Copy link'}
                  </button>
                </div>
                {p.copied === 'failed' && <p className="text-[11px] text-rose-300">Couldn’t copy. Select the link and copy it yourself.</p>}
              </div>
            ),
          };
    body = (
      <div className="space-y-4">
        <Steps items={[step1, homeScreenStep, notifyStep]} />
        <Ext href={PHONE_GUIDE_URL}>Step-by-step guide</Ext>
      </div>
    );
  }

  return (
    <section
      id={PHONE_APP_ANCHOR}
      aria-labelledby={PHONE_APP_TITLE}
      data-phone-app={p.kind}
      data-phone-layout={p.layout}
      className="rounded-xl border border-zinc-800 bg-zinc-950/60 p-4 space-y-4 scroll-mt-4"
    >
      <div className="flex items-start gap-3">
        <div className="h-9 w-9 shrink-0 rounded-lg bg-sky-500/10 border border-sky-500/20 flex items-center justify-center">
          <Smartphone size={18} className="text-sky-300" aria-hidden="true" />
        </div>
        <div className="min-w-0">
          <h4 id={PHONE_APP_TITLE} tabIndex={-1} className="text-sm font-semibold text-zinc-100 focus:outline-none">
            {phone ? 'Add kwant bots to your Home Screen' : 'Use kwant bots on your phone'}
          </h4>
          <p className="text-[11px] text-zinc-400 leading-relaxed mt-0.5">
            {phone
              ? 'No app store: it opens full screen from your Home Screen, like an app, and can notify you when a bot needs you.'
              : 'No app store: open it in your phone’s browser and add it to the Home Screen. It opens like an app and can notify you when a bot needs you.'}
          </p>
        </div>
      </div>
      {body}
      {!phone && p.installFooter && <div className="pt-3 border-t border-zinc-800">{p.installFooter}</div>}
    </section>
  );
};

/** Phones and tablets install themselves, so they get the phone layout. */
export function usesPhoneLayout(nav: Pick<Navigator, 'userAgent' | 'maxTouchPoints'>, narrow: boolean): boolean {
  return isIos(nav) || isAndroid(nav) || narrow;
}
/** "Mac" in the Mac app or a Mac browser; the Linux app and everything else is "computer". */
const machineName = (): 'Mac' | 'computer' => {
  if (inDesktopApp()) return desktopAppOs() === 'Mac' ? 'Mac' : 'computer';
  return isEmbeddedWebView(navigator) || (/Macintosh/.test(navigator.userAgent) && !isIos(navigator)) ? 'Mac' : 'computer';
};

/** The card, wired: reads this page's address, sign-in and install state. */
export const PhoneAppCard: React.FC<{ phone?: boolean }> = ({ phone: narrow = false }) => {
  const { authRequired, deviceLogin } = useAuth();
  const { prompt, installed } = useInstall();
  const desktopInstall = useDesktopInstallKind();
  const [copied, setCopied] = useState<'yes' | 'failed' | null>(null);
  // While this card is up, the floating InstallHint steps aside: the card already has its steps.
  useEffect(() => phoneCardShown(), []);
  if (typeof window === 'undefined') return null;

  const { hostname, protocol, host, origin } = window.location;
  const kind = phoneAppKind({ hostname, protocol, authRequired, deviceLogin });
  const layout = usesPhoneLayout(navigator, narrow) ? 'phone' : 'computer';
  // The installed app on a phone: nothing left to do here
  if (layout === 'phone' && (installed || isStandalone(window))) return null;

  const phoneCase: PhoneInstallCase = !window.isSecureContext ? 'insecure' : prompt ? 'prompt' : isIos(navigator) ? 'ios' : 'android';
  const copy = async () => {
    try {
      await navigator.clipboard.writeText(`${origin}/`);
      setCopied('yes');
    } catch {
      setCopied('failed');
    }
  };

  return (
    <PhoneAppCardView
      kind={kind}
      layout={layout}
      phoneCase={phoneCase}
      host={host}
      origin={origin}
      machine={machineName()}
      tokenSignIn={authRequired && !deviceLogin}
      signIn={kind === 'signin' ? <AddDevice variant="phone" onSignedIn={() => undefined} /> : null}
      installFooter={desktopInstall ? <InstallAppButton className={rowBtn} label="Install on this computer" /> : null}
      copied={copied}
      onCopy={copy}
      onInstall={() => void runInstallPrompt()}
    />
  );
};
