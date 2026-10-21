// The TV's HTTP API. One bearer token, obtained by pairing, stored in localStorage and sent as a
// header. Pictures, the live socket and the backup load by URL, so they ride on a session cookie
// the TV sets once the token checks out; the token itself never goes into a URL.

export type Tile = {
  id: string; kind: 'POSTER' | 'LANDSCAPE' | 'APP_BANNER' | 'COMPACT' | string; title: string; subtitle?: string | null;
  artworkPath?: string | null; iconPath?: string | null; lightArgb: number; progress?: number | null;
  badge?: string | null; action: string; pinned: boolean; stagePinned: boolean;
};
export type Row = { id: string; title: string; kind: string; size: string; badge?: string | null; tiles: Tile[] };
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
export type Wallpaper = { artworkPath: string; title: string; source: string };
export type Snapshot = { revision: number; generatedAtMs: number; stage: { spotlights: Spotlight[]; strip: Tile[] }; rows: Row[]; nowPlaying?: NowPlaying | null; wallpapers?: Wallpaper[] };
export type State = { device: { name: string; model: string; version: string; tier: string }; snapshot: Snapshot };
export type Layout = { rowOrder: string[]; availableRows: { id: string; title: string }[]; hidden: string[]; pinnedApps: string[]; stagePins: string[]; enabledChannels: number[] };
export type PluginSetting = { key: string; label: string; kind: string; required: boolean; help?: string | null; defaultValue?: string | null; value?: string | null; hasValue: boolean };
export type Plugin = {
  id: string; name: string; version: string; description: string; publisher: string; publisherName: string; kinds: string[];
  fetchHosts: string[]; enabled: boolean; builtIn: boolean; configured: boolean; refreshMinutes: number;
  lastRefreshMs?: number | null; lastError?: string | null; items: number; settings: PluginSetting[];
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

export class ApiError extends Error {
  constructor(public status: number, message: string) { super(message); }
}

async function call<T>(method: string, path: string, body?: unknown, raw?: BodyInit): Promise<T> {
  const headers: Record<string, string> = {};
  const t = token();
  if (t) headers.Authorization = `Bearer ${t}`;
  if (body !== undefined) headers['Content-Type'] = 'application/json';
  const res = await fetch(path, { method, headers, body: raw ?? (body !== undefined ? JSON.stringify(body) : undefined) });
  if (!res.ok) {
    let msg = res.statusText;
    try { msg = (await res.json()).error ?? msg; } catch { /* plain */ }
    throw new ApiError(res.status, msg);
  }
  if (res.status === 204) return undefined as T;
  const text = await res.text();
  return (text ? JSON.parse(text) : undefined) as T;
}

export const api = {
  pair: (code: string) => call<{ token: string; device: string }>('POST', '/api/pair', { code }),
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
  installPlugin: (file: File) => call<Plugin>('POST', '/api/plugins/install', undefined, file),
  installPluginUrl: (url: string) => call<Plugin>('POST', '/api/plugins/install-url', { url }),
  diagnostics: () => call<Diagnostics>('GET', '/api/diagnostics'),
  devices: () => call<Device[]>('GET', '/api/devices'),
  revokeDevice: (id: string) => call<void>('DELETE', `/api/devices/${encodeURIComponent(id)}`),
  backupUrl: () => '/api/backup',
  restore: (json: string) => call<void>('POST', '/api/restore', undefined, json),
};

/** Artwork files live on the TV; the server exposes them by path under this route. */
export function art(path: string | null | undefined): string | undefined {
  if (!path) return undefined;
  return `/api/art?path=${encodeURIComponent(path)}`;
}

export function argb(v: number): string {
  const n = v >>> 0;
  return `#${(n & 0xffffff).toString(16).padStart(6, '0')}`;
}

/** Live snapshots over one WebSocket; reconnects with backoff. */
export function liveState(onState: (s: State) => void, onStatus: (connected: boolean) => void): () => void {
  let ws: WebSocket | null = null;
  let closed = false;
  let delay = 1000;
  const connect = () => {
    if (closed) return;
    const proto = location.protocol === 'https:' ? 'wss' : 'ws';
    ws = new WebSocket(`${proto}://${location.host}/api/ws`);
    ws.onopen = () => { delay = 1000; onStatus(true); };
    ws.onmessage = (e) => { try { onState(JSON.parse(e.data)); } catch { /* ignore */ } };
    ws.onclose = () => { onStatus(false); if (!closed) { setTimeout(connect, delay); delay = Math.min(delay * 2, 15000); } };
    ws.onerror = () => ws?.close();
  };
  connect();
  return () => { closed = true; ws?.close(); };
}
