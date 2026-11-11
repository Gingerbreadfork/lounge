// The TV's HTTP API. One bearer token, obtained by pairing, stored in localStorage and sent as a
// header. Pictures, the live socket and the backup load by URL, so they ride on a session cookie
// the TV sets once the token checks out; the token itself never goes into a URL.

export type Tile = {
  id: string; kind: 'POSTER' | 'LANDSCAPE' | 'APP_BANNER' | 'COMPACT' | string; title: string; subtitle?: string | null;
  artworkPath?: string | null; iconPath?: string | null; lightArgb: number; progress?: number | null;
  badge?: string | null; action: string; pinned: boolean; stagePinned: boolean;
  /** The picture is the owning app's banner, standing in for art the item did not bring. */
  borrowedArt?: boolean;
  /** What the tile says over a picture that already says some of the title; empty for nothing. */
  artTitle?: string | null;
  openedRank?: number | null; installedRank?: number | null;
  /** The app or plugin something to continue comes from, as the layout knows it, and its name. */
  provider?: string | null; providerName?: string | null;
};
export type Row = { id: string; title: string; kind: string; size: string; badge?: string | null; note?: string | null; tiles: Tile[] };
export type Spotlight = {
  id: string; kind: string; category: string; title: string; lines: string[]; progress?: number | null;
  artworkPath?: string | null; backdropPath?: string | null; lightArgb: number;
  primary: { label: string; action: string }; secondary?: { label: string; action: string } | null; explanation?: string | null;
  source?: string | null;
};
export type NowPlaying = {
  package: string; appLabel: string; title: string; subtitle?: string | null; artPath?: string | null; lightArgb: number;
  durationMs?: number | null; positionMs?: number | null; status: string; canPause: boolean; canPlay: boolean;
  canSkipNext: boolean; canSkipPrevious: boolean; updatedMs: number;
};
export type Wallpaper = { artworkPath: string; title: string; subtitle?: string | null; source: string; framed?: boolean };
export type Snapshot = { revision: number; generatedAtMs: number; stage: { spotlights: Spotlight[]; strip: Tile[] }; rows: Row[]; nowPlaying?: NowPlaying | null; wallpapers?: Wallpaper[] };
/** The snapshot is null while the TV is shutting down. */
export type State = { device: { name: string; model: string; version: string; tier: string }; snapshot: Snapshot | null };
/** One row the home could show: whether it is on, and one line on what it holds. */
export type RowRef = { id: string; title: string; shown: boolean; chosen: boolean; count: number; detail: string };
export type NamedRef = { id: string; title: string; iconPath?: string | null };
export type AppRef = { package: string; title: string; iconPath?: string | null; pinned: boolean };
/** An app or plugin Continue watching draws on: how many titles it brings and whether it is shown. */
export type SourceRef = { id: string; title: string; iconPath?: string | null; count: number; shown: boolean };
export type Layout = {
  rowOrder: string[]; availableRows: RowRef[]; hidden: string[]; pinnedApps: string[]; stagePins: string[]; enabledChannels: number[];
  /** Rows shown or hidden by hand; a row not listed follows its default. */
  rowVisibility: Record<string, boolean>;
  apps: AppRef[]; hiddenItems: NamedRef[]; stageItems: NamedRef[];
  /** How many recently opened apps follow the pins in Your apps; zero keeps the row to pins. */
  recentApps: number;
  /** Apps and plugins left out of Continue watching, each as a tile's `provider`, and everything it draws on. */
  mutedProviders: string[]; continueSources: SourceRef[];
  /** Notice tiles the person chose not to see again, by id. */
  dismissedNotices: string[];
};
export type PluginSetting = { key: string; label: string; kind: string; required: boolean; help?: string | null; defaultValue?: string | null; value?: string | null; hasValue: boolean };
/** A step a plugin waits on the person for, such as a sign-in. */
export type PluginAsk = { say: string; link?: string | null; linkText?: string | null; code?: string | null; after?: string | null };
export type Plugin = {
  id: string; name: string; version: string; description: string; publisher: string; publisherName: string; kinds: string[];
  fetchHosts: string[]; selfSignedServers: boolean; enabled: boolean; builtIn: boolean; configured: boolean; refreshMinutes: number;
  lastRefreshMs?: number | null; lastError?: string | null; ask?: PluginAsk | null; items: number; settings: PluginSetting[];
  /** Where an update that turned the plugin off also asks to reach; empty otherwise. */
  updateAsksToReach?: string[];
};
export type Diagnostics = Record<string, string | number>;
export type Device = { id: string; name: string; createdMs: number; lastSeenMs: number; current: boolean };

const TOKEN_KEY = 'lounge.token';

export function token(): string | null {
  try { return localStorage.getItem(TOKEN_KEY); } catch { return null; }
}
export function setToken(t: string | null) {
  try { if (t) localStorage.setItem(TOKEN_KEY, t); else localStorage.removeItem(TOKEN_KEY); } catch { /* private mode */ }
}

/** A request that failed, with a sentence to show for it. `status` is 0 when the TV did not answer. */
export class ApiError extends Error {
  constructor(public status: number, message: string) { super(message); }
}

/** What the phone says while the TV cannot be reached. */
export const AWAY = "Lounge on the TV isn't answering. Check the TV is on and this phone is on the same Wi-Fi.";
const SLOW = 'The TV took too long to answer. Check it is on, then try again.';
/** How long a request may take before the phone gives up on it; uploads and installs get longer. */
const WAIT_MS = 8000;
const UPLOAD_WAIT_MS = 120000;

/** Told whenever the TV answers, or fails to, so the page can say when it is away. */
export const contact = { heard: () => {}, missed: () => {} };

/** Whether the TV's text already reads as a sentence for a person rather than for a developer. */
function written(text: string): boolean {
  return /^[A-Z]/.test(text) && /[.?]$/.test(text) && !text.includes('Exception') && text.length <= 400;
}

/** The TV's own sentence for a failure, or one for its status when it sent none. */
function said(status: number, text: string): string {
  if (written(text)) return text;
  if (text) console.warn(`The TV answered ${status}: ${text}`);
  if (status === 401) return 'Pair this phone with the TV first.';
  if (status === 413) return 'That is too much to send to the TV at once.';
  if (status >= 500) return 'Lounge on the TV ran into a problem. Try again in a moment.';
  return "The TV couldn't do that. Try again.";
}

/** The sentence for any failure, ready for a toast. */
export function explain(e: unknown): string {
  if (e instanceof ApiError) return e.message;
  console.warn(e);
  return "That didn't work. Try again.";
}

async function call<T>(method: string, path: string, body?: unknown, raw?: BodyInit, wait = WAIT_MS): Promise<T> {
  const headers: Record<string, string> = {};
  const t = token();
  if (t) headers.Authorization = `Bearer ${t}`;
  if (body !== undefined) headers['Content-Type'] = 'application/json';
  const abort = new AbortController();
  const timer = setTimeout(() => abort.abort(), wait);
  try {
    const res = await fetch(path, { method, headers, body: raw ?? (body !== undefined ? JSON.stringify(body) : undefined), signal: abort.signal });
    const text = await res.text();
    contact.heard();
    if (!res.ok) {
      let message = text;
      try { message = JSON.parse(text).error ?? text; } catch { /* plain text */ }
      throw new ApiError(res.status, said(res.status, String(message ?? '').trim()));
    }
    return (text ? JSON.parse(text) : undefined) as T;
  } catch (e) {
    if (e instanceof ApiError) throw e;
    if (e instanceof SyntaxError) throw new ApiError(500, said(500, ''));
    // The request never got an answer: the TV is off, asleep or on another network.
    contact.missed();
    throw new ApiError(0, abort.signal.aborted ? SLOW : AWAY);
  } finally {
    clearTimeout(timer);
  }
}

export const api = {
  pair: (code: string) => call<{ token: string; device: string }>('POST', '/api/pair', { code }),
  pairLink: (link: string) => call<{ token: string; device: string }>('POST', '/api/pair', { link }),
  session: () => call<void>('POST', '/api/session'),
  state: () => call<State>('GET', '/api/state'),
  key: (key: string) => call<void>('POST', '/api/key', { key }),
  text: (text: string) => call<void>('POST', '/api/text', { text }),
  open: (action: string) => call<void>('POST', '/api/open', { action }),
  launch: (pkg: string) => call<void>('POST', '/api/launch', { package: pkg }),
  search: (query: string) => call<void>('POST', '/api/search', { query }),
  layout: () => call<Layout>('GET', '/api/layout'),
  setLayout: (l: Partial<Layout>) => call<Layout>('PUT', '/api/layout', l),
  plugins: () => call<Plugin[]>('GET', '/api/plugins'),
  pluginCommand: (id: string, cmd: 'enable' | 'disable' | 'refresh' | 'remove') => call<void>('POST', `/api/plugins/${encodeURIComponent(id)}/${cmd}`),
  pluginSetting: (id: string, key: string, value: string | null) => call<void>('PUT', `/api/plugins/${encodeURIComponent(id)}/settings`, { key, value }),
  pluginLog: (id: string) => call<string[]>('GET', `/api/plugins/${encodeURIComponent(id)}/log`),
  installPlugin: (file: File) => call<Plugin>('POST', '/api/plugins/install', undefined, file, UPLOAD_WAIT_MS),
  installPluginUrl: (url: string) => call<Plugin>('POST', '/api/plugins/install-url', { url }, undefined, UPLOAD_WAIT_MS),
  diagnostics: () => call<Diagnostics>('GET', '/api/diagnostics'),
  devices: () => call<Device[]>('GET', '/api/devices'),
  revokeDevice: (id: string) => call<void>('DELETE', `/api/devices/${encodeURIComponent(id)}`),
  backupUrl: () => '/api/backup',
  restore: (json: string) => call<void>('POST', '/api/restore', undefined, json, UPLOAD_WAIT_MS),
};

/** An enum name as the TV sends it ("AppBanner", "Live", "Playing") in the page's own form: "APP_BANNER". */
export function tag(name: string | null | undefined): string {
  return (name ?? '').replace(/([a-z0-9])([A-Z])/g, '$1_$2').toUpperCase();
}

/** Artwork files live on the TV; the server exposes them by path under this route. */
export function art(path: string | null | undefined): string | undefined {
  if (!path) return undefined;
  return `/api/art?path=${encodeURIComponent(path)}`;
}

/** The light the core falls back on is House's lamp; the phone's room is Dusk, so it shows its own. */
const CORE_FALLBACK_LIGHT = 0xf2b263;
const HOUSELIGHT = '#ffa86b';
const VELVET = '#120e27';
const LIMELIGHT = '#f6efe7';

/** A tile's or the spotlight's light as a CSS colour. */
export function light(v: number): string {
  const rgb = (v >>> 0) & 0xffffff;
  return rgb === CORE_FALLBACK_LIGHT ? HOUSELIGHT : `#${rgb.toString(16).padStart(6, '0')}`;
}

function luminance(hex: string): number {
  const channel = (i: number) => {
    const c = parseInt(hex.slice(i, i + 2), 16) / 255;
    return c <= 0.04045 ? c / 12.92 : ((c + 0.055) / 1.055) ** 2.4;
  };
  return 0.2126 * channel(1) + 0.7152 * channel(3) + 0.0722 * channel(5);
}

function contrast(a: string, b: string): number {
  const [x, y] = [luminance(a), luminance(b)].sort((m, n) => n - m);
  return (x + 0.05) / (y + 0.05);
}

/** Velvet or limelight, whichever reads better on `hex`, for text that sits on a light. */
export function onLight(hex: string): string {
  return contrast(VELVET, hex) >= contrast(LIMELIGHT, hex) ? VELVET : LIMELIGHT;
}

/** The style for something lit by `v`: its light and the text colour that reads on it. */
export function lit(v: number): string {
  const l = light(v);
  return `--light: ${l}; --on-light: ${onLight(l)}`;
}

/** The live socket's close code for a phone the TV has forgotten. */
const UNPAIRED = 4401;
/** The TV sends this every few seconds between snapshots. */
const HEARTBEAT = 'ping';
/** A socket silent for this long counts as gone, though the browser may not have noticed. */
const SILENCE_MS = 15000;

/** Live snapshots over one WebSocket; reconnects with backoff until the TV has forgotten this
 *  phone, then calls `onUnpaired`. */
export function liveState(onState: (s: State) => void, onStatus: (connected: boolean) => void, onUnpaired: () => void): () => void {
  let ws: WebSocket | null = null;
  let closed = false;
  let delay = 1000;
  let timer: ReturnType<typeof setTimeout> | undefined;
  let watchdog: ReturnType<typeof setTimeout> | undefined;
  let lastHeard = 0;
  let drop: (() => void) | null = null;
  const unpaired = () => {
    if (closed) return;
    closed = true;
    onUnpaired();
  };
  const connect = () => {
    if (closed) return;
    const proto = location.protocol === 'https:' ? 'wss' : 'ws';
    let opened = false;
    const socket = new WebSocket(`${proto}://${location.host}/api/ws`);
    ws = socket;
    const ended = (code?: number) => {
      clearTimeout(watchdog);
      if (ws !== socket) return;
      ws = null;
      drop = null;
      onStatus(false);
      if (closed) return;
      if (code === UNPAIRED) { unpaired(); return; }
      // A refused socket looks the same as an unreachable TV. A new session tells a forgotten
      // phone apart, and renews the cookie if that was what failed.
      if (!opened) api.session().catch((err) => { if (err?.status === 401) unpaired(); });
      timer = setTimeout(connect, delay);
      delay = Math.min(delay * 2, 15000);
    };
    // A connection the TV stopped answering on can take minutes to close by itself, so it is let go at once.
    drop = () => {
      socket.onopen = socket.onmessage = socket.onclose = socket.onerror = null;
      socket.close();
      ended();
    };
    const listen = () => {
      lastHeard = Date.now();
      clearTimeout(watchdog);
      watchdog = setTimeout(() => drop?.(), SILENCE_MS);
    };
    listen();
    socket.onopen = () => { opened = true; delay = 1000; listen(); onStatus(true); };
    socket.onmessage = (e) => {
      listen();
      contact.heard();
      if (e.data === HEARTBEAT) return;
      try { onState(JSON.parse(e.data)); } catch { /* ignore */ }
    };
    socket.onclose = (e) => ended(e.code);
    socket.onerror = () => socket.close();
  };
  // Coming back to the app, or to the network, tries at once rather than waiting out the backoff.
  const retry = () => {
    if (closed || document.hidden) return;
    if (ws && Date.now() - lastHeard > SILENCE_MS) drop?.();
    if (ws) return;
    clearTimeout(timer);
    delay = 1000;
    connect();
  };
  document.addEventListener('visibilitychange', retry);
  window.addEventListener('online', retry);
  connect();
  return () => {
    closed = true;
    clearTimeout(timer);
    clearTimeout(watchdog);
    document.removeEventListener('visibilitychange', retry);
    window.removeEventListener('online', retry);
    ws?.close();
  };
}
