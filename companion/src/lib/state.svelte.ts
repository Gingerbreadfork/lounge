// Shared, reactive app state: the live snapshot, connection, the current route and toasts.
import { api, liveState, setToken, token, type State } from './api';

export const app = $state({
  route: parseRoute(location.hash),
  connected: false,
  everConnected: false,
  paired: !!token(),
  state: null as State | null,
  toast: null as string | null,
  error: null as string | null,
  /** A link shared to the app, waiting for the person to say it should open on the TV. */
  share: null as string | null,
});

export type Route = { page: 'home' | 'remote' | 'layout' | 'plugins' | 'more' | 'pair'; id?: string };

function parseRoute(hash: string): Route {
  const h = hash.replace(/^#\/?/, '');
  const [page, id] = h.split('/');
  const pages = ['home', 'remote', 'layout', 'plugins', 'more', 'pair'];
  return { page: (pages.includes(page) ? page : 'home') as Route['page'], id: id ? decodeURIComponent(id) : undefined };
}

export function go(page: Route['page'], id?: string) {
  location.hash = id ? `#/${page}/${encodeURIComponent(id)}` : `#/${page}`;
}

let toastTimer: ReturnType<typeof setTimeout> | undefined;
export function toast(message: string) {
  app.toast = message;
  clearTimeout(toastTimer);
  toastTimer = setTimeout(() => (app.toast = null), 3200);
}

let stopLive: (() => void) | null = null;

/** The first http(s) link in text shared to the app, if any. */
export function sharedLink(): string | null {
  const q = new URLSearchParams(location.search);
  const candidates = [q.get('url'), q.get('text'), q.get('title')].filter(Boolean) as string[];
  for (const c of candidates) {
    const m = c.match(/https?:\/\/[^\s]+/);
    if (m) return m[0];
  }
  return null;
}

/** Holds a shared link until the person confirms it; any page can put a link in our URL, so
 *  nothing opens on the TV without a tap. The URL is cleaned up either way. */
function handleShare() {
  const link = sharedLink();
  if (!link) return;
  history.replaceState(null, '', '/#/home');
  app.route = { page: 'home' };
  if (!app.paired) {
    try { sessionStorage.setItem('lounge.pendingShare', link); } catch { /* ignore */ }
    return;
  }
  app.share = link;
}

export async function openShare() {
  const link = app.share;
  app.share = null;
  if (link) await act('Opened on the TV', () => api.open(`url:${link}`));
}

export function dismissShare() {
  app.share = null;
}

export function start() {
  window.addEventListener('hashchange', () => (app.route = parseRoute(location.hash)));
  // A pairing link carries the token in the fragment: #token=…
  const m = location.hash.match(/token=([A-Za-z0-9_-]+)/);
  if (m) {
    setToken(m[1]);
    app.paired = true;
    history.replaceState(null, '', '#/home');
    app.route = { page: 'home' };
  }
  if (location.pathname === '/share' || sharedLink()) {
    handleShare();
  }
  if (!app.paired) {
    app.route = { page: 'pair' };
    return;
  }
  connect();
}

export async function connect() {
  stopLive?.();
  stopLive = null;
  // The cookie for pictures and the live socket comes first; without it they would be refused.
  try {
    await api.session();
  } catch (e: any) {
    if (e?.status === 401) { unpair(); return; }
  }
  // A link shared before the phone was paired waits here until it is, by link or by code.
  let pending: string | null = null;
  try { pending = sessionStorage.getItem('lounge.pendingShare'); sessionStorage.removeItem('lounge.pendingShare'); } catch { /* ignore */ }
  if (pending) app.share = pending;
  stopLive = liveState(
    (s) => { app.state = s; app.error = null; },
    (c) => { app.connected = c; if (c) app.everConnected = true; },
  );
  api.state().then((s) => (app.state = s)).catch((e) => {
    if (e?.status === 401) unpair();
    else app.error = e?.message ?? 'The TV did not answer.';
  });
}

/** What the connection pill says: live, or honest about which way it is heading. */
export function connectionLabel(): string {
  return app.connected ? 'Live' : app.everConnected ? 'Reconnecting…' : 'Connecting…';
}

export function unpair() {
  stopLive?.();
  stopLive = null;
  setToken(null);
  app.paired = false;
  app.state = null;
  app.route = { page: 'pair' };
  location.hash = '#/pair';
}

export async function act<T>(label: string, f: () => Promise<T>): Promise<T | undefined> {
  try {
    const r = await f();
    if (label) toast(label);
    return r;
  } catch (e: any) {
    if (e?.status === 401) { unpair(); return undefined; }
    toast(e?.message ?? 'That did not work.');
    return undefined;
  }
}
