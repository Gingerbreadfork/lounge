<script lang="ts">
  import { api, type Plugin } from '../lib/api';
  import Confirm from '../lib/Confirm.svelte';
  import { t, tc, tn } from '../lib/lang.svelte';
  import { act, app, done, go, goBack, quiet, toast } from '../lib/state.svelte';
  import { ago, around, list } from '../lib/words';

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
  // Translators: {ending} is where the file ending .lgp goes
  const bundle = $derived(around(t('A signed {ending} bundle, from a file on this phone or a link.'), 'ending'));

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
    const labels = { enable: t('{plugin} is on', { plugin: p.name }), disable: t('{plugin} is off', { plugin: p.name }), refresh: t('Refreshing…'), remove: t('{plugin} removed', { plugin: p.name }) };
    const worked = await done(labels[cmd], () => api.pluginCommand(p.id, cmd));
    if (worked && cmd === 'remove') goBack('plugins');
    load();
  }
  async function commit(p: Plugin, key: string) {
    const value = drafts[key];
    if (value === undefined) return;
    // A password is sent as typed; spaces at its ends can be part of it.
    const typed = p.settings.find((s) => s.key === key)?.kind === 'password' ? value : value.trim();
    const saved = await act(t('Saved'), () => api.pluginSetting(p.id, key, typed ? typed : null).then(() => true));
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
    toast(t('{plugin} installed', { plugin: p.name }));
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
  /** What stands in a field whose saved value is kept out of sight. */
  function savedHint() {
    // Translators: stands in a password field that already has a value saved
    return t('•••••••• (saved)');
  }
  /** Where a link leads, for its button, or the link itself when it cannot be read. */
  function host(link: string) {
    try { return new URL(link).host; } catch { return link; }
  }
  function status(p: Plugin) {
    // A plugin waiting on you is not a problem: a sign-in when there is a page or a code.
    // Translators: a plugin's state, in a small label beside its name: it waits for a sign-in, or for its settings
    if (p.enabled && p.ask) return { text: p.ask.link || p.ask.code ? t('Sign in') : t('Needs setup'), cls: 'ask' };
    // Translators: a plugin's state, in a small label beside its name: its last try failed
    if (p.enabled && p.lastError) return { text: t('Problem'), cls: 'bad' };
    // Translators: the state of a plugin or of one of its settings: turned on
    // Translators: a plugin's state in the list: it is turned on.
    if (p.enabled) return { text: tc('a plugin', 'On'), cls: 'on' };
    // As on the TV, an update's ask comes before a setting still to fill in.
    if (p.updateAsksToReach?.length) return { text: t('Turned off by its update'), cls: 'ask' };
    if (!p.configured) return { text: t('Needs setup'), cls: '' };
    // Translators: the state of a plugin or of one of its settings: turned off
    // Translators: a plugin's state in the list: it is turned off.
    return { text: tc('a plugin', 'Off'), cls: '' };
  }
  /** Where a plugin may connect, as a sentence; a host it names is shown as it is. */
  function reachText(p: Plugin) {
    const parts: string[] = [];
    for (const h of p.fetchHosts) {
      let part: string | undefined;
      if (h.startsWith('$')) {
        const setting = p.settings.find((s) => s.key === h.slice(1));
        // Translators: a place a plugin may connect to, listed in "It can reach {places}."; {setting} is the label of one of its settings
        part = setting ? t('what you enter in {setting}', { setting: setting.label }) : undefined;
      }
      // Translators: a place a plugin may connect to, listed in "It can reach {places}."
      else if (h === '*') part = t('any public site');
      // Translators: a place a plugin may connect to, listed in "It can reach {places}."
      else if (h === 'local-network') part = t('devices on your home network');
      // Translators: a place a plugin may connect to, listed in "It can reach {places}."
      else if (h === 'localhost' || h === '127.0.0.1' || h === '::1') part = t('this TV');
      else part = h;
      if (part && !parts.includes(part)) parts.push(part);
    }
    if (parts.length === 0) return t('It can reach nothing.');
    const places = list(parts);
    // Translators: {places} lists where a plugin may connect, such as "example.com and devices on your home network"
    return p.selfSignedServers ? t("It can reach {places}, accepting a home server's own certificate.", { places }) : t('It can reach {places}.', { places });
  }
  /** How often a plugin fetches again, as a sentence. */
  function refreshText(p: Plugin) {
    if (p.refreshSeconds) return tn(p.refreshSeconds, "It refreshes every {n} second while Lounge is on the TV's screen.", "It refreshes every {n} seconds while Lounge is on the TV's screen.");
    return tn(p.refreshMinutes, 'It refreshes every {n} minute while the TV is on.', 'It refreshes every {n} minutes while the TV is on.');
  }
</script>

<main class="page">
  {#if id && !current}
    <button class="btn ghost back" onclick={() => goBack('plugins')}>← {t('Plugins')}</button>
    <p class="empty">{loaded ? t("That plugin isn't installed on the TV any more.") : app.away ? t("The TV isn't answering, so this plugin can't be shown yet.") : t('Reading the plugin from the TV…')}</p>
  {:else if current}
    {@const s = status(current)}
    <header class="page-head">
      <div>
        <button class="btn ghost back" onclick={() => goBack('plugins')}>← {t('Plugins')}</button>
        <h1>{current.name}</h1>
        <!-- Translators: {version} is a plugin's version number and {publisher} is who made it -->
        <p class="meta">{current.publisherName ? t('Version {version}, by {publisher}', { version: current.version, publisher: current.publisherName }) : t('Version {version}', { version: current.version })}</p>
      </div>
      <span class="pill {s.cls}">{s.text}</span>
    </header>

    {#if current.description}<p class="dust">{current.description}</p>{/if}

    <section class="card status">
      {#if current.enabled && current.ask}
        {@const ask = current.ask}
        {#if ask.link || ask.code}
          <p class="houselight">{t('Sign in')}</p>
          {#if ask.link}
            <!-- Translators: a button; {page} is the name or address of a sign-in page -->
            <a class="btn primary" href={ask.link} target="_blank" rel="noopener">{t('Open {page}', { page: ask.linkText ?? host(ask.link) })}</a>
            <!-- Translators: between the button "Open {page}" and a sign-in code set large below: open the page and enter the code -->
            {#if ask.code}<p class="dust">{t('and enter')}</p>{/if}
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
        <!-- Translators: how many items a plugin brought; {when} is how long ago, such as "3 minutes ago" or "just now" -->
        <p class="signal">{tn(current.items, '{n} item, refreshed {when}', '{n} items, refreshed {when}', { when: ago(current.lastRefreshMs) })}</p>
      {:else if current.enabled}
        <p class="dust">{t('Fetching for the first time…')}</p>
      {:else if current.updateAsksToReach?.length}
        <!-- Translators: {places} lists the sites a plugin's update wants to connect to -->
        <p class="houselight">{t('Turned off by its update, which also asks to reach {places}.', { places: list(current.updateAsksToReach) })}</p>
      {:else if !current.configured}
        <p class="houselight">{t('Fill in the settings, then turn it on.')}</p>
      {:else}
        <p class="dust">{tc('a plugin', 'Off')}</p>
      {/if}
      <div class="status-actions">
        <!-- Translators: buttons that turn a plugin off or on; "Allow" lets its update connect to what it asks for -->
        <button class="btn {current.enabled ? '' : 'primary'}" disabled={!current.enabled && !current.configured} onclick={() => command(current!, current!.enabled ? 'disable' : 'enable')}>{current.enabled ? t('Turn off') : current.updateAsksToReach?.length ? t('Allow and turn on') : t('Turn on')}</button>
        {#if current.enabled}<button class="btn" onclick={() => command(current!, 'refresh')}>{t('Refresh now')}</button>{/if}
      </div>
    </section>

    {#if current.settings.length}
      <section class="section">
        <div class="section-head"><h3>{t('Settings')}</h3></div>
        {#each current.settings as st (st.key)}
          <div class="field">
            <!-- Translators: {label} is the label of a setting that may be left empty -->
            <label for={st.key}>{st.required || st.kind === 'toggle' ? st.label : t('{label} (optional)', { label: st.label })}</label>
            {#if st.kind === 'toggle'}
              {@const on = (st.value ?? st.defaultValue) === 'true'}
              <button id={st.key} class="list-row" role="switch" aria-checked={on} aria-label={st.label} onclick={() => act(t('Saved'), () => api.pluginSetting(current!.id, st.key, on ? 'false' : 'true')).then(load)}>
                <span class="grow title" aria-hidden="true">{on ? t('On') : t('Off')}</span><span class="toggle" class:on></span>
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
                placeholder={(st.kind === 'password' || st.kind === 'secret_url') && st.hasValue ? savedHint() : (st.defaultValue ?? '')}
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
      <!-- Translators: the heading over what a plugin has written down while working, and the button that hides or shows it -->
      <div class="section-head"><h3>{t('Log')}</h3><button class="btn ghost" onclick={() => { showLog = !showLog; load(); }}>{showLog ? tc('log', 'Hide') : tc('log', 'Show')}</button></div>
      {#if showLog}
        <pre class="log">{log.length ? log.map((l) => l.replace(/^\d+ /, '')).join('\n') : t('Nothing logged yet.')}</pre>
      {/if}
    </section>

    <section class="section">
      <!-- Translators: {key} is the first eight characters of the key the plugin's publisher signs with -->
      <p class="caption">{reachText(current)} {refreshText(current)} {t('Updates are taken only from its publisher, whose key starts {key}.', { key: `${current.publisher.slice(0, 4)} ${current.publisher.slice(4, 8)}` })}</p>
      {#if !current.builtIn}<Confirm label={t('Remove plugin')} confirm={t('Yes, remove it and its settings')} onconfirm={() => command(current!, 'remove')} />{/if}
    </section>
  {:else}
    <header class="page-head"><div><p class="meta">{app.state?.device.name ?? t('Your TV')}</p><h1>{t('Plugins')}</h1></div></header>
    <p class="dust">{t('They add rows, channels and guide data from your services. Each plugin can reach only the places it names, and sees only its own settings.')}</p>

    <section class="list">
      {#each plugins as p (p.id)}
        {@const s = status(p)}
        <button class="list-row" onclick={() => go('plugins', p.id)}>
          <!-- Translators: under a plugin's name: how many items it brings and its version number -->
          <div class="grow"><div class="title">{p.name}</div><div class="caption">{p.enabled && p.items ? tn(p.items, '{n} item, version {version}', '{n} items, version {version}', { version: p.version }) : t('version {version}', { version: p.version })}</div></div>
          <span class="pill {s.cls}">{s.text}</span>
        </button>
      {/each}
      {#if !loaded}
        <p class="empty">{app.away ? t("The TV isn't answering, so its plugins can't be listed yet.") : t('Reading the plugins on the TV…')}</p>
      {:else if plugins.length === 0}
        <p class="empty">{t('No plugins yet. Install one below.')}</p>
      {/if}
    </section>

    <section class="card install">
      <h3>{t('Install a plugin')}</h3>
      <p class="caption">{bundle[0]}<code>.lgp</code>{bundle[1]}</p>
      <button class="btn" disabled={installing} onclick={() => picker?.click()}>{installing ? t('Installing…') : t('Choose file')}</button>
      <input type="file" accept=".lgp,application/zip" onchange={install} hidden bind:this={picker} />
      <div class="url-row">
        <input placeholder="https://…/plugin.lgp" bind:value={url} inputmode="url" autocapitalize="off" aria-label={t('Link to a plugin')} />
        <button class="btn primary" onclick={installUrl} disabled={installing || !url.trim()}>{installing ? t('Installing…') : t('Install')}</button>
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
