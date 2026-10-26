<script lang="ts">
  import { api, type Plugin } from '../lib/api';
  import { act, go, quiet, toast } from '../lib/state.svelte';

  let { id }: { id?: string } = $props();
  let plugins = $state<Plugin[]>([]);
  let log = $state<string[]>([]);
  let showLog = $state(false);
  let url = $state('');
  let drafts = $state<Record<string, string>>({});
  let timer: ReturnType<typeof setInterval> | undefined;

  $effect(() => {
    load();
    timer = setInterval(load, 3000);
    return () => clearInterval(timer);
  });

  async function load() {
    const p = await quiet(() => api.plugins());
    if (p) plugins = p;
    if (id && showLog) log = (await quiet(() => api.pluginLog(id!))) ?? log;
  }
  let current = $derived(plugins.find((p) => p.id === id) ?? null);

  async function command(p: Plugin, cmd: 'enable' | 'disable' | 'refresh' | 'remove') {
    const labels = { enable: `${p.name} is on`, disable: `${p.name} is off`, refresh: 'Refreshing…', remove: `${p.name} removed` };
    await act(labels[cmd], () => api.pluginCommand(p.id, cmd));
    if (cmd === 'remove') go('plugins');
    load();
  }
  async function commit(p: Plugin, key: string) {
    const value = drafts[key];
    if (value === undefined) return;
    // A password is sent as typed; spaces at its ends can be part of it.
    const typed = p.settings.find((s) => s.key === key)?.kind === 'password' ? value : value.trim();
    const saved = await act('Saved', () => api.pluginSetting(p.id, key, typed ? typed : null).then(() => true));
    // A save that failed keeps what was typed, so it can be tried again.
    if (saved) delete drafts[key];
    load();
  }
  async function install(e: Event) {
    const file = (e.target as HTMLInputElement).files?.[0];
    if (!file) return;
    const p = await act(`${file.name} installed`, () => api.installPlugin(file));
    if (p) go('plugins', p.id);
    (e.target as HTMLInputElement).value = '';
  }
  async function installUrl() {
    if (!url.trim()) return;
    const p = await act('Installed', () => api.installPluginUrl(url.trim()));
    if (p) { url = ''; go('plugins', p.id); }
  }
  function status(p: Plugin) {
    if (p.enabled && p.lastError) return { text: 'Problem', cls: 'bad' };
    if (p.enabled) return { text: 'On', cls: 'on' };
    if (!p.configured) return { text: 'Needs setup', cls: '' };
    return { text: 'Off', cls: '' };
  }
  /** Where a plugin may connect, in words; a host it names is shown as it is. */
  function reachText(p: Plugin) {
    const parts: string[] = [];
    for (const h of p.fetchHosts) {
      let part: string | undefined;
      if (h.startsWith('$')) {
        const setting = p.settings.find((s) => s.key === h.slice(1));
        part = setting ? `what you enter in ${setting.label}` : undefined;
      } else if (h === '*') part = 'any public site';
      else if (h === 'local-network') part = 'devices on your home network';
      else if (h === 'localhost' || h === '127.0.0.1' || h === '::1') part = 'this TV';
      else part = h;
      if (part && !parts.includes(part)) parts.push(part);
    }
    if (parts.length === 0) return 'nothing';
    return parts.length === 1 ? parts[0] : `${parts.slice(0, -1).join(', ')} and ${parts[parts.length - 1]}`;
  }
  function ago(ms: number) {
    const m = Math.floor((Date.now() - ms) / 60000);
    return m < 1 ? 'just now' : m < 60 ? `${m} min ago` : m < 1440 ? `${Math.floor(m / 60)} h ago` : `${Math.floor(m / 1440)} d ago`;
  }
</script>

<main class="page">
  {#if current}
    {@const s = status(current)}
    <header class="page-head">
      <div>
        <button class="btn ghost back" onclick={() => go('plugins')}>← Plugins</button>
        <h1>{current.name}</h1>
        <p class="meta">v{current.version}{current.publisherName ? ` · by ${current.publisherName}` : ''}</p>
      </div>
      <span class="pill {s.cls}">{s.text}</span>
    </header>

    {#if current.description}<p class="dust">{current.description}</p>{/if}

    <section class="card status">
      {#if current.enabled && current.lastError}
        <p class="ember">{current.lastError}</p>
      {:else if current.enabled && current.lastRefreshMs}
        <p class="signal">{current.items} items · refreshed {ago(current.lastRefreshMs)}</p>
      {:else if current.enabled}
        <p class="dust">Fetching for the first time…</p>
      {:else if !current.configured}
        <p class="houselight">Fill in the settings, then turn it on.</p>
      {:else}
        <p class="dust">Off</p>
      {/if}
      <div class="status-actions">
        <button class="btn {current.enabled ? '' : 'primary'}" disabled={!current.enabled && !current.configured} onclick={() => command(current!, current!.enabled ? 'disable' : 'enable')}>{current.enabled ? 'Turn off' : 'Turn on'}</button>
        {#if current.enabled}<button class="btn" onclick={() => command(current!, 'refresh')}>Refresh now</button>{/if}
      </div>
    </section>

    {#if current.settings.length}
      <section class="section">
        <div class="section-head"><h3>Settings</h3></div>
        {#each current.settings as st (st.key)}
          <div class="field">
            <label for={st.key}>{st.label}{st.required ? '' : ' (optional)'}</label>
            {#if st.kind === 'toggle'}
              {@const on = (st.value ?? st.defaultValue) === 'true'}
              <button class="list-row" onclick={() => act('Saved', () => api.pluginSetting(current!.id, st.key, on ? 'false' : 'true')).then(load)}>
                <span class="grow title">{on ? 'On' : 'Off'}</span><span class="toggle" class:on></span>
              </button>
            {:else}
              <input
                id={st.key}
                type={st.kind === 'password' ? 'password' : st.kind === 'url' || st.kind === 'secret_url' ? 'url' : st.kind === 'number' ? 'number' : 'text'}
                placeholder={(st.kind === 'password' || st.kind === 'secret_url') && st.hasValue ? '•••••••• (saved)' : (st.defaultValue ?? st.help ?? '')}
                value={drafts[st.key] ?? st.value ?? ''}
                oninput={(e) => (drafts[st.key] = (e.target as HTMLInputElement).value)}
                onblur={() => commit(current!, st.key)}
                onkeydown={(e) => { if (e.key === 'Enter') (e.target as HTMLInputElement).blur(); }}
                autocapitalize="off" autocorrect="off" spellcheck="false"
              />
              {#if st.help}<p class="caption">{st.help}</p>{/if}
            {/if}
          </div>
        {/each}
      </section>
    {/if}

    <section class="section">
      <div class="section-head"><h3>Log</h3><button class="btn ghost" onclick={() => { showLog = !showLog; load(); }}>{showLog ? 'Hide' : 'Show'}</button></div>
      {#if showLog}
        <pre class="log">{log.length ? log.map((l) => l.replace(/^\d+ /, '')).join('\n') : 'Nothing logged yet.'}</pre>
      {/if}
    </section>

    <section class="section">
      <p class="caption">Can connect to: {reachText(current)}. Signed by {current.publisher.slice(0, 16)}… Refreshes every {current.refreshMinutes} min while the TV is on.</p>
      {#if !current.builtIn}<button class="btn danger" onclick={() => command(current!, 'remove')}>Remove plugin</button>{/if}
    </section>
  {:else}
    <header class="page-head"><div><p class="meta">Sources</p><h1>Plugins</h1></div></header>
    <p class="dust">Each plugin runs in its own sandbox on the TV and adds rows, channels or guide data.</p>

    <section class="list">
      {#each plugins as p (p.id)}
        {@const s = status(p)}
        <button class="list-row" onclick={() => go('plugins', p.id)}>
          <div class="grow"><div class="title">{p.name}</div><div class="caption">{p.enabled && p.items ? `${p.items} items · ` : ''}v{p.version}</div></div>
          <span class="pill {s.cls}">{s.text}</span>
        </button>
      {/each}
      {#if plugins.length === 0}<p class="empty">No plugins installed.</p>{/if}
    </section>

    <section class="card install">
      <h3>Install a plugin</h3>
      <p class="caption">A signed <code>.lgp</code> bundle, from a file on this phone or a link.</p>
      <label class="btn">Choose file<input type="file" accept=".lgp,application/zip" onchange={install} hidden /></label>
      <div class="url-row">
        <input placeholder="https://…/plugin.lgp" bind:value={url} inputmode="url" autocapitalize="off" />
        <button class="btn primary" onclick={installUrl} disabled={!url.trim()}>Install</button>
      </div>
    </section>
  {/if}
</main>

<style>
  .back { padding: 0; min-height: 0; margin-bottom: 8px; }
  .status { display: flex; flex-direction: column; gap: 12px; }
  .status-actions { display: flex; gap: 10px; }
  .log { margin: 0; padding: 12px; border-radius: var(--radius); background: var(--velvet); border: 1px solid var(--curtain-edge); font: 12px/1.5 ui-monospace, SFMono-Regular, Menlo, monospace; color: var(--dust); white-space: pre-wrap; word-break: break-word; max-height: 40vh; overflow: auto; }
  .install { display: flex; flex-direction: column; gap: 12px; }
  .url-row { display: flex; gap: 8px; }
  .url-row input { flex: 1; min-height: 48px; padding: 0 14px; border-radius: var(--radius); background: var(--velvet); border: 1px solid var(--curtain-edge); color: var(--limelight); outline: none; min-width: 0; }
  code { font-family: ui-monospace, monospace; color: var(--limelight); }
</style>
