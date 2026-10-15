import React from 'react';
import ReactDOM from 'react-dom/client';
import { App } from './App';
import { SandboxViewer } from './components/SandboxViewer';
import './styles/modern.css';

const sandboxMatch = window.location.pathname.match(/^\/sandbox\/([^/]+)\/?$/);

ReactDOM.createRoot(document.getElementById('root')!).render(
  <React.StrictMode>
    {sandboxMatch ? <SandboxViewer teamId={decodeURIComponent(sandboxMatch[1])} /> : <App />}
  </React.StrictMode>,
);
