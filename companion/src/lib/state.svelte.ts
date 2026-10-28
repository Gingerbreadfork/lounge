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
  // A pairing link carries a one-time key in the fragment, #token=…, which the TV swaps for this
  // phone's own token.
  const link = location.hash.match(/token=([A-Za-z0-9_-]+)/)?.[1];
  if (link) {
    history.replaceState(null, '', '#/home');
    app.route = { page: 'home' };
  }
  if (location.pathname === '/share' || sharedLink()) {
    handleShare();
  }
  if (link) {
    pairByLink(link);
    return;
  }
  if (!app.paired) {
    app.route = { page: 'pair' };
    return;
  }
  connect();
}

/** A phone that was already paired keeps its token when the link has expired. */
async function pairByLink(link: string) {
  try {
    const r = await api.pairLink(link);
    setToken(r.token);
    app.paired = true;
    toast(`Paired with ${r.device}`);
  } catch (e: any) {
    toast(e?.status === 403 ? 'That QR code has expired. Open Settings › Phone companion on the TV for a new one.' : (e?.message ?? 'The TV did not answer.'));
  }
  if (app.paired) connect();
  else app.route = { page: 'pair' };
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
    unpair,
  );
  // The socket may already have delivered a newer snapshot than this first read.
  api.state().then((s) => { if (!app.state?.snapshot || (s.snapshot && s.snapshot.revision >= app.state.snapshot.revision)) app.state = s; }).catch((e) => {
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

/** A background read: a failure is not worth a toast every few seconds, and nothing is read
 *  while the page is out of sight. */
export async function quiet<T>(f: () => Promise<T>): Promise<T | undefined> {
  if (document.hidden) return undefined;
  try {
    return await f();
  } catch (e: any) {
    if (e?.status === 401) unpair();
    return undefined;
  }
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
