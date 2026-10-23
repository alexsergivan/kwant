import React from 'react';
import ReactDOM from 'react-dom/client';
import { App } from './Shell';
import { SandboxViewer } from './components/SandboxViewer';
import { AuthGate } from './components/AuthGate';
import { registerServiceWorker, trackAppHeight } from './pwa';
import './styles/modern.css';

registerServiceWorker();
trackAppHeight();

const sandboxMatch = window.location.pathname.match(/^\/sandbox\/([^/]+)\/?$/);

ReactDOM.createRoot(document.getElementById('root')!).render(
  <React.StrictMode>
    <AuthGate>
      {sandboxMatch ? <SandboxViewer teamId={decodeURIComponent(sandboxMatch[1])} /> : <App />}
    </AuthGate>
  </React.StrictMode>,
);
