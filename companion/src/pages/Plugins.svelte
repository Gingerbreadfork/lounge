<script lang="ts">
  import { api, type Plugin } from '../lib/api';
  import Confirm from '../lib/Confirm.svelte';
  import { act, app, done, go, goBack, quiet, toast } from '../lib/state.svelte';
  import { ago, counted } from '../lib/words';

  let { id }: { id?: string } = $props();
  let plugins = $state<Plugin[]>([]);
  /** Whether the TV has listed its plugins yet, so an empty list is not shown while it has not. */
  let loaded = $state(false);
  let log = $state<string[]>([]);
  let showLog = $state(false);
  let url = $state('');
  let drafts = $state<Record<string, string>>({});
  let installing = $state(false);
  let picker = $state<HTMLInputElement>();
  let timer: ReturnType<typeof setInterval> | undefined;

  $effect(() => {
    load();
    timer = setInterval(load, 3000);
    return () => clearInterval(timer);
  });

  async function load() {
    const p = await quiet(() => api.plugins());
    if (p) {
      plugins = p;
      loaded = true;
    }
    if (id && showLog) log = (await quiet(() => api.pluginLog(id!))) ?? log;
  }
  let current = $derived(plugins.find((p) => p.id === id) ?? null);

  async function command(p: Plugin, cmd: 'enable' | 'disable' | 'refresh' | 'remove') {
    const labels = { enable: `${p.name} is on`, disable: `${p.name} is off`, refresh: 'Refreshing…', remove: `${p.name} removed` };
    const worked = await done(labels[cmd], () => api.pluginCommand(p.id, cmd));
    if (worked && cmd === 'remove') goBack('plugins');
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
  // One install at a time, from a file or a link, and both end the same way: on the plugin's page.
  async function installWith(f: () => Promise<Plugin>): Promise<boolean> {
    if (installing) return false;
    installing = true;
    const p = await act('', f);
    installing = false;
    if (!p) return false;
    toast(`${p.name} installed`);
    go('plugins', p.id);
    return true;
  }
  async function install(e: Event) {
    const input = e.target as HTMLInputElement;
    const file = input.files?.[0];
    input.value = '';
    if (file) await installWith(() => api.installPlugin(file));
  }
  async function installUrl() {
    const link = url.trim();
    if (link && (await installWith(() => api.installPluginUrl(link)))) url = '';
  }
  /** A plugin's text split into words and the links in it, so a sign-in address can be tapped. A
   *  bare name counts only with a path after it, so a file name such as guide.xml stays text. In
   *  `help`, an address that only shows what one looks like stays text too. */
  function linked(text: string, help = false): { text: string; href?: string }[] {
    const parts: { text: string; href?: string }[] = [];
    const link = /https?:\/\/[^\s]+|\b(?:[a-z0-9-]+\.)+[a-z]{2,}\/[^\s]*/gi;
    let last = 0;
    for (const m of text.matchAll(link)) {
      let found = m[0];
      // Punctuation that ends the sentence is not part of the address.
      while (/[.,;:)!?…]$/.test(found)) found = found.slice(0, -1);
      if (!found || (help && example(found))) continue;
      const start = m.index ?? 0;
      if (start > last) parts.push({ text: text.slice(last, start) });
      parts.push({ text: found, href: /^https?:\/\//i.test(found) ? found : `https://${found}` });
      last = start + found.length;
    }
    if (last < text.length) parts.push({ text: text.slice(last) });
    return parts;
  }
  /** Whether an address only shows what one looks like: a made-up home address or name. */
  function example(address: string) {
    const name = host(/^https?:\/\//i.test(address) ? address : `https://${address}`).replace(/:\d+$/, '');
    return /^(localhost|192\.168\.\d+\.\d+|10\.\d+\.\d+\.\d+|(.+\.)?example(\.[a-z]+)?)$/i.test(name);
  }
  /** Where a link leads, for its button, or the link itself when it cannot be read. */
  function host(link: string) {
    try { return new URL(link).host; } catch { return link; }
  }
  function status(p: Plugin) {
    // A plugin waiting on you is not a problem: a sign-in when there is a page or a code.
    if (p.enabled && p.ask) return { text: p.ask.link || p.ask.code ? 'Sign in' : 'Needs setup', cls: 'ask' };
    if (p.enabled && p.lastError) return { text: 'Problem', cls: 'bad' };
    if (p.enabled) return { text: 'On', cls: 'on' };
    // As on the TV, an update's ask comes before a setting still to fill in.
    if (p.updateAsksToReach?.length) return { text: 'Turned off by its update', cls: 'ask' };
    if (!p.configured) return { text: 'Needs setup', cls: '' };
    return { text: 'Off', cls: '' };
  }
  function joined(parts: string[]) {
    return parts.length < 2 ? (parts[0] ?? '') : `${parts.slice(0, -1).join(', ')} and ${parts[parts.length - 1]}`;
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
    const reach = parts.length === 1 ? parts[0] : `${parts.slice(0, -1).join(', ')} and ${parts[parts.length - 1]}`;
    return p.selfSignedServers ? `${reach}, accepting a home server's own certificate` : reach;
  }
</script>

<main class="page">
  {#if id && !current}
    <button class="btn ghost back" onclick={() => goBack('plugins')}>← Plugins</button>
    <p class="empty">{loaded ? "That plugin isn't installed on the TV any more." : app.away ? "The TV isn't answering, so this plugin can't be shown yet." : 'Reading the plugin from the TV…'}</p>
  {:else if current}
    {@const s = status(current)}
    <header class="page-head">
      <div>
        <button class="btn ghost back" onclick={() => goBack('plugins')}>← Plugins</button>
        <h1>{current.name}</h1>
        <p class="meta">Version {current.version}{current.publisherName ? `, by ${current.publisherName}` : ''}</p>
      </div>
      <span class="pill {s.cls}">{s.text}</span>
    </header>

    {#if current.description}<p class="dust">{current.description}</p>{/if}

    <section class="card status">
      {#if current.enabled && current.ask}
        {@const ask = current.ask}
        {#if ask.link || ask.code}
          <p class="houselight">Sign in</p>
          {#if ask.link}
            <a class="btn primary" href={ask.link} target="_blank" rel="noopener">Open {ask.linkText ?? host(ask.link)}</a>
            {#if ask.code}<p class="dust">and enter</p>{/if}
          {:else if ask.after}
            <p class="dust">{ask.after}</p>
          {/if}
          {#if ask.code}<p class="code">{ask.code}</p>{/if}
          {#if ask.link && ask.after}<p class="dust">{ask.after}</p>{/if}
        {:else}
          <p class="houselight">{#each linked(ask.say) as part, i (i)}{#if part.href}<a href={part.href} target="_blank" rel="noopener">{part.text}</a>{:else}{part.text}{/if}{/each}</p>
        {/if}
      {:else if current.enabled && current.lastError}
        <p class="ember">{#each linked(current.lastError) as part, i (i)}{#if part.href}<a href={part.href} target="_blank" rel="noopener">{part.text}</a>{:else}{part.text}{/if}{/each}</p>
      {:else if current.enabled && current.lastRefreshMs}
        <p class="signal">{current.items === 1 ? '1 item' : `${current.items} items`}, refreshed {ago(current.lastRefreshMs)}</p>
      {:else if current.enabled}
        <p class="dust">Fetching for the first time…</p>
      {:else if current.updateAsksToReach?.length}
        <p class="houselight">Turned off by its update, which also asks to reach {joined(current.updateAsksToReach)}.</p>
      {:else if !current.configured}
        <p class="houselight">Fill in the settings, then turn it on.</p>
      {:else}
        <p class="dust">Off</p>
      {/if}
      <div class="status-actions">
        <button class="btn {current.enabled ? '' : 'primary'}" disabled={!current.enabled && !current.configured} onclick={() => command(current!, current!.enabled ? 'disable' : 'enable')}>{current.enabled ? 'Turn off' : current.updateAsksToReach?.length ? 'Allow and turn on' : 'Turn on'}</button>
        {#if current.enabled}<button class="btn" onclick={() => command(current!, 'refresh')}>Refresh now</button>{/if}
      </div>
    </section>

    {#if current.settings.length}
      <section class="section">
        <div class="section-head"><h3>Settings</h3></div>
        {#each current.settings as st (st.key)}
          <div class="field">
            <label for={st.key}>{st.label}{st.required || st.kind === 'toggle' ? '' : ' (optional)'}</label>
            {#if st.kind === 'toggle'}
              {@const on = (st.value ?? st.defaultValue) === 'true'}
              <button id={st.key} class="list-row" role="switch" aria-checked={on} aria-label={st.label} onclick={() => act('Saved', () => api.pluginSetting(current!.id, st.key, on ? 'false' : 'true')).then(load)}>
                <span class="grow title" aria-hidden="true">{on ? 'On' : 'Off'}</span><span class="toggle" class:on></span>
              </button>
            {:else if st.kind === 'list'}
              <textarea
                id={st.key}
                rows="4"
                placeholder={st.defaultValue ?? ''}
                value={drafts[st.key] ?? st.value ?? ''}
                oninput={(e) => (drafts[st.key] = (e.target as HTMLTextAreaElement).value)}
                onblur={() => commit(current!, st.key)}
                autocapitalize="off" spellcheck="false"
              ></textarea>
            {:else}
              <input
                id={st.key}
                type={st.kind === 'password' ? 'password' : st.kind === 'url' || st.kind === 'secret_url' ? 'url' : st.kind === 'number' ? 'number' : 'text'}
                placeholder={(st.kind === 'password' || st.kind === 'secret_url') && st.hasValue ? '•••••••• (saved)' : (st.defaultValue ?? '')}
                value={drafts[st.key] ?? st.value ?? ''}
                oninput={(e) => (drafts[st.key] = (e.target as HTMLInputElement).value)}
                onblur={() => commit(current!, st.key)}
                onkeydown={(e) => { if (e.key === 'Enter') (e.target as HTMLInputElement).blur(); }}
                autocapitalize="off" autocorrect="off" spellcheck="false"
              />
            {/if}
            {#if st.help}<p class="caption">{#each linked(st.help, true) as part, i (i)}{#if part.href}<a href={part.href} target="_blank" rel="noopener">{part.text}</a>{:else}{part.text}{/if}{/each}</p>{/if}
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
      <p class="caption">It can reach {reachText(current)}. It refreshes every {counted(current.refreshMinutes, 'minute')} while the TV is on. Updates are taken only from its publisher, whose key starts {current.publisher.slice(0, 4)} {current.publisher.slice(4, 8)}.</p>
      {#if !current.builtIn}<Confirm label="Remove plugin" confirm="Yes, remove it and its settings" onconfirm={() => command(current!, 'remove')} />{/if}
    </section>
  {:else}
    <header class="page-head"><div><p class="meta">{app.state?.device.name ?? 'Your TV'}</p><h1>Plugins</h1></div></header>
    <p class="dust">They add rows, channels and guide data from your services. Each plugin can reach only the places it names, and sees only its own settings.</p>

    <section class="list">
      {#each plugins as p (p.id)}
        {@const s = status(p)}
        <button class="list-row" onclick={() => go('plugins', p.id)}>
          <div class="grow"><div class="title">{p.name}</div><div class="caption">{p.enabled && p.items ? `${p.items} items, ` : ''}version {p.version}</div></div>
          <span class="pill {s.cls}">{s.text}</span>
        </button>
      {/each}
      {#if !loaded}
        <p class="empty">{app.away ? "The TV isn't answering, so its plugins can't be listed yet." : 'Reading the plugins on the TV…'}</p>
      {:else if plugins.length === 0}
        <p class="empty">No plugins yet. Install one below.</p>
      {/if}
    </section>

    <section class="card install">
      <h3>Install a plugin</h3>
      <p class="caption">A signed <code>.lgp</code> bundle, from a file on this phone or a link.</p>
      <button class="btn" disabled={installing} onclick={() => picker?.click()}>{installing ? 'Installing…' : 'Choose file'}</button>
      <input type="file" accept=".lgp,application/zip" onchange={install} hidden bind:this={picker} />
      <div class="url-row">
        <input placeholder="https://…/plugin.lgp" bind:value={url} inputmode="url" autocapitalize="off" aria-label="Link to a plugin" />
        <button class="btn primary" onclick={installUrl} disabled={installing || !url.trim()}>{installing ? 'Installing…' : 'Install'}</button>
      </div>
    </section>
  {/if}
</main>

<style>
  .ember a, .houselight a, .caption a { text-decoration: underline; text-underline-offset: 3px; word-break: break-all; }
  .code { font: 600 32px/1.1 ui-monospace, SFMono-Regular, Menlo, monospace; letter-spacing: 0.08em; color: var(--limelight); user-select: all; }
  .status .btn.primary { align-self: flex-start; }
  .back { padding: 0 12px; margin: 0 0 4px -12px; min-height: 44px; align-self: flex-start; }
  .status { display: flex; flex-direction: column; gap: 12px; }
  .status-actions { display: flex; gap: 10px; }
  .log { margin: 0; padding: 12px; border-radius: var(--radius); background: var(--velvet); border: 1px solid var(--curtain-edge); font: 12px/1.5 ui-monospace, SFMono-Regular, Menlo, monospace; color: var(--dust); white-space: pre-wrap; word-break: break-word; max-height: 40vh; overflow: auto; }
  .install { display: flex; flex-direction: column; gap: 12px; }
  .url-row { display: flex; gap: 8px; }
  .url-row input { flex: 1; min-height: 48px; padding: 0 14px; border-radius: var(--radius); background: var(--velvet); border: 1px solid var(--curtain-edge); color: var(--limelight); outline: none; min-width: 0; }
  code { font-family: ui-monospace, monospace; color: var(--limelight); }
</style>
