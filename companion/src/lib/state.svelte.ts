// Shared, reactive app state: the live snapshot, connection, the current route and toasts.
import { api, contact, explain, liveState, setToken, token, type State } from './api';

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
  /** Set once the TV has not answered for a while, so the page can say what to check. */
  away: false,
  /** Why the phone is on the Pair page when it did not choose to be. */
  pairNote: null as string | null,
});

export type Route = { page: 'home' | 'remote' | 'layout' | 'plugins' | 'more' | 'pair'; id?: string };

function parseRoute(hash: string): Route {
  const h = hash.replace(/^#\/?/, '');
  const [page, id] = h.split('/');
  const pages = ['home', 'remote', 'layout', 'plugins', 'more', 'pair'];
  return { page: (pages.includes(page) ? page : 'home') as Route['page'], id: id ? decodeId(id) : undefined };
}

/** A malformed escape such as `%` or `%GG` names nothing, so it leaves no id. */
function decodeId(id: string): string | undefined {
  try {
    return decodeURIComponent(id);
  } catch {
    return undefined;
  }
}

function hashOf(page: Route['page'], id?: string) {
  return id ? `#/${page}/${encodeURIComponent(id)}` : `#/${page}`;
}

/** Opens a page at its top. A tab takes the place of the page it leaves, so Back does not walk
 *  every tab visited; a page opened from another is one step that Back undoes. */
export function go(page: Route['page'], id?: string, { replace = false } = {}) {
  if (app.route.page !== page || app.route.id !== id) {
    if (replace) history.replaceState(null, '', hashOf(page, id));
    else history.pushState({ from: hashOf(app.route.page, app.route.id) }, '', hashOf(page, id));
    app.route = { page, id };
  }
  window.scrollTo(0, 0);
}

/** Back to `page`: one step back when this page was opened from it, else straight to it. */
export function goBack(page: Route['page']) {
  if (history.state?.from === hashOf(page)) history.back();
  else go(page, undefined, { replace: true });
}

/** The route follows the address when Back, Forward or a typed address changes it. */
function followAddress() {
  const next = parseRoute(location.hash);
  if (next.page !== app.route.page || next.id !== app.route.id) app.route = next;
}

/** How long a toast stays (DESIGN.md §7.4). */
const TOAST_MS = 4000;
let toastTimer: ReturnType<typeof setTimeout> | undefined;
export function toast(message: string) {
  app.toast = message;
  clearTimeout(toastTimer);
  toastTimer = setTimeout(() => (app.toast = null), TOAST_MS);
}

/** How long the TV may go unheard before the page says it is away. */
const AWAY_AFTER_MS = 20000;
let awayTimer: ReturnType<typeof setTimeout> | undefined;
contact.heard = () => {
  clearTimeout(awayTimer);
  awayTimer = undefined;
  if (app.away) app.away = false;
};
contact.missed = () => {
  if (awayTimer !== undefined || app.away) return;
  awayTimer = setTimeout(() => {
    awayTimer = undefined;
    if (app.paired) app.away = true;
  }, AWAY_AFTER_MS);
};

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
  window.addEventListener('hashchange', followAddress);
  window.addEventListener('popstate', followAddress);
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

/** Called once a code or link has given this phone its token. */
export function paired(device: string) {
  app.paired = true;
  app.pairNote = null;
  go('home', undefined, { replace: true });
  connect();
  toast(`Paired with ${device}`);
}

/** A phone that is already paired ignores the link and keeps its token, so a stray link cannot
 *  sign it out. The link is used only when the TV no longer takes that token. */
async function pairByLink(link: string) {
  if (app.paired) {
    if (await stillPaired()) {
      connect();
      return;
    }
    setToken(null);
    app.paired = false;
  }
  try {
    const r = await api.pairLink(link);
    setToken(r.token);
    paired(r.device);
  } catch (e) {
    toast(explain(e));
    history.replaceState(null, '', '#/pair');
    app.route = { page: 'pair' };
  }
}

/** Whether the TV still takes this phone's token. Only a refusal says no; a TV that does not
 *  answer may still know the phone. */
async function stillPaired(): Promise<boolean> {
  try {
    await api.session();
    return true;
  } catch (e: any) {
    return e?.status !== 401;
  }
}

export async function connect() {
  stopLive?.();
  stopLive = null;
  // Nothing has been heard yet, so a TV that never answers is reported like one that went away.
  contact.missed();
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
    (c) => {
      app.connected = c;
      if (c) { app.everConnected = true; contact.heard(); }
      else contact.missed();
    },
    unpair,
  );
  // The socket may already have delivered a newer snapshot than this first read.
  api.state().then((s) => { if (!app.state?.snapshot || (s.snapshot && s.snapshot.revision >= app.state.snapshot.revision)) app.state = s; }).catch((e) => {
    if (e?.status === 401) unpair();
    else app.error = explain(e);
  });
}

/** What the connection pill says: connected, or honest about which way it is heading. */
export function connectionLabel(): string {
  return app.connected ? 'Connected' : app.everConnected ? 'Reconnecting…' : 'Connecting…';
}

const FORGOTTEN = 'The TV has forgotten this phone. Pair it again to carry on.';

/** Lets go of this phone's token and shows the Pair page, saying why when `note` is given. */
export function unpair(note: string | null = FORGOTTEN) {
  stopLive?.();
  stopLive = null;
  setToken(null);
  clearTimeout(awayTimer);
  awayTimer = undefined;
  app.away = false;
  app.paired = false;
  app.state = null;
  app.pairNote = note;
  app.route = { page: 'pair' };
  history.replaceState(null, '', '#/pair');
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

/** Runs a request and toasts `label` when it works, or the reason when it does not. Undefined
 *  means it failed, unless the request itself answers nothing. */
export async function act<T>(label: string, f: () => Promise<T>): Promise<T | undefined> {
  try {
    const r = await f();
    if (label) toast(label);
    return r;
  } catch (e: any) {
    if (e?.status === 401) { unpair(); return undefined; }
    toast(explain(e));
    return undefined;
  }
}

/** Like `act`, for a request that answers nothing: true when it worked. */
export async function done(label: string, f: () => Promise<unknown>): Promise<boolean> {
  return (await act(label, () => f().then(() => true))) ?? false;
}
