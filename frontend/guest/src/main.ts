// Entry for /p/<code>. Reads the optional v1 state the server put in the page (api.ts fromState checks its shape),
// then joins the live game.
import './guest.css';
import { codeFrom } from './api';
import { GuestApp } from './app';

function boot(): unknown {
  const el = document.getElementById('pocket-boot');
  if (!el?.textContent) return null;
  try {
    return JSON.parse(el.textContent);
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
// A socket can die quietly while the phone sleeps or switches network: ask the box again on wake, online and restore.
document.addEventListener('visibilitychange', () => {
  if (document.visibilityState === 'visible') app.wake();
});
w.addEventListener('online', () => app.wake());
w.addEventListener('pageshow', (e) => {
  if (e.persisted) app.wake();
});
