import React from 'react';
import ReactDOM from 'react-dom/client';
import { App } from './Shell';
import { SandboxViewer } from './components/SandboxViewer';
import { AuthGate } from './components/AuthGate';
import { installFileDropGuard } from './dragPreview';
import { captureInstallPrompt, registerServiceWorker, trackAppHeight } from './pwa';
import './styles/fonts.css';
import './styles/modern.css';
import './styles/stream.css';
import './styles/needsYou.css';
import './styles/ideas.css';
import './styles/ideaShell.css';
import './styles/devStatus.css';
import './styles/tickets.css';
import './styles/telegram.css';
import './styles/showOnce.css';
import './styles/gettingStarted.css';

registerServiceWorker();
captureInstallPrompt();
trackAppHeight();
// A file dropped outside a chat never opens in place of kwant (browser, Mac and Windows apps).
installFileDropGuard(window);

const sandboxMatch = window.location.pathname.match(/^\/sandbox\/([^/]+)\/?$/);

ReactDOM.createRoot(document.getElementById('root')!).render(
  <React.StrictMode>
    <AuthGate>
      {sandboxMatch ? <SandboxViewer teamId={decodeURIComponent(sandboxMatch[1])} /> : <App />}
    </AuthGate>
  </React.StrictMode>,
);
