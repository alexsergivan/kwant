import React from 'react';
import ReactDOM from 'react-dom/client';
import { App } from './App';
import { SandboxViewer } from './components/SandboxViewer';
import { AuthGate } from './components/AuthGate';
import './styles/modern.css';

const sandboxMatch = window.location.pathname.match(/^\/sandbox\/([^/]+)\/?$/);

ReactDOM.createRoot(document.getElementById('root')!).render(
  <React.StrictMode>
    <AuthGate>
      {sandboxMatch ? <SandboxViewer teamId={decodeURIComponent(sandboxMatch[1])} /> : <App />}
    </AuthGate>
  </React.StrictMode>,
);
