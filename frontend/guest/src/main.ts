// Entry for /p/<code>. Reads the optional view the server put in the page, then joins the live game.
import './guest.css';
import { codeFrom } from './api';
import { GuestApp } from './app';
import type { GuestView } from './view';

function boot(): GuestView | null {
  const el = document.getElementById('pocket-boot');
  if (!el?.textContent) return null;
  try {
    const v = JSON.parse(el.textContent);
    return v && typeof v === 'object' && v.phase ? (v as GuestView) : null;
  } catch {
    return null;
  }
}

const root = document.getElementById('app')!;
const w = window;
const app = new GuestApp(
  root,
  {
    fetch: w.fetch.bind(w),
    WebSocket: w.WebSocket,
    setTimeout: (f, ms) => w.setTimeout(f, ms),
    clearTimeout: (id) => w.clearTimeout(id),
    setInterval: (f, ms) => w.setInterval(f, ms),
    clearInterval: (id) => w.clearInterval(id),
    now: () => Date.now(),
    location: w.location,
  },
  codeFrom(w.location.pathname),
);
void app.start(boot());
