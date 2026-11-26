<script lang="ts">
  import { api, type Device, type Diagnostics } from '../lib/api';
  import Confirm from '../lib/Confirm.svelte';
  import { aboutLinks, coins, copyText, paypal, spaced, type Coin } from '../lib/giving';
  import { t, tr } from '../lib/lang.svelte';
  import { act, app, done, quiet, toast, unpair } from '../lib/state.svelte';
  import { ago, decimal, span } from '../lib/words';

  let diag = $state<Diagnostics | null>(null);
  let devices = $state<Device[]>([]);
  /** A backup chosen to restore, waiting for the person to say it may replace the TV's layout and settings. */
  let chosen = $state<File | null>(null);
  let restoring = $state(false);
  let picker = $state<HTMLInputElement>();
  /** Whether the TV holds a key for pop-ups, once it has said. */
  let keySet = $state<boolean | null>(null);
  /** A key just made, held only until the page is left. */
  let newKey = $state<string | null>(null);
  /** Counts the times a key was made or removed here, so an older answer from the TV is not believed. */
  let keyChanges = 0;

  $effect(() => {
    load();
    const timer = setInterval(load, 5000);
    return () => clearInterval(timer);
  });
  async function load() {
    diag = (await quiet(() => api.diagnostics())) ?? diag;
    devices = (await quiet(() => api.devices())) ?? devices;
    const asked = keyChanges;
    const said = (await quiet(() => api.alerts()))?.keySet;
    if (said !== undefined && asked === keyChanges) keySet = said;
  }
  async function makeKey() {
    const made = await act('', () => api.newAlertKey());
    if (!made) return;
    keyChanges++;
    newKey = made.key;
    keySet = true;
  }
  async function removeKey() {
    if (!(await done(t('Key removed. Nothing can show pop-ups now.'), () => api.clearAlertKey()))) return;
    keyChanges++;
    newKey = null;
    keySet = false;
  }
  /** What Home Assistant needs in configuration.yaml to show a pop-up on this TV. */
  let recipe = $derived(
    newKey == null
      ? ''
      : [
          'rest_command:',
          '  lounge_popup:',
          `    url: "${location.origin}/api/alert"`,
          '    method: post',
          '    headers:',
          `      authorization: "Bearer ${newKey}"`,
          '    content_type: "application/json"',
          '    payload: >-',
          '      {"title": {{ title | tojson }}, "message": {{ message | default("") | tojson }}, "image": {{ image | default("") | tojson }}}',
        ].join('\n'),
  );
  /** An automation's action that rings the TV with the camera's picture. */
  let ring = $derived(
    [
      '- action: rest_command.lounge_popup',
      '  data:',
      // Translators: the example title of a doorbell pop-up
      `    title: ${JSON.stringify(t('Someone is at the door'))}`,
      `    image: "http://192.168.1.10:8123{{ state_attr('camera.front_door', 'entity_picture') }}"`,
    ].join('\n'),
  );
  async function copyRecipe(text: string) {
    toast((await copyText(text)) ? t('Copied') : t("Couldn't copy it. Press and hold the text to copy it."));
  }
  function choose(e: Event) {
    const input = e.target as HTMLInputElement;
    chosen = input.files?.[0] ?? null;
    input.value = '';
  }
  async function restore() {
    const file = chosen;
    if (!file) return;
    restoring = true;
    if (await done(t('Layout and settings restored on the TV'), async () => api.restore(await file.text()))) chosen = null;
    restoring = false;
  }
  async function forget(d: Device) {
    // Translators: said once the TV has forgotten a paired phone; {name} is that phone's name
    const forgotten = await done(d.current ? t('The TV has forgotten this phone') : t('{name} forgotten', { name: d.name }), () => api.revokeDevice(d.id));
    if (!forgotten) return;
    if (d.current) unpair(null);
    else load();
  }
  /** The TV forgets this phone first, so its token stops working there too; the phone only lets
   *  go of it once the TV has. */
  async function forgetThisPhone() {
    const list = await act('', () => api.devices());
    const mine = list?.find((d) => d.current);
    if (mine) await forget(mine);
    else if (list) unpair(null);
  }
  async function copy(coin: Coin) {
    // Translators: {coin} is a currency's name, such as Bitcoin; "it" is the address to send that currency to
    toast((await copyText(coin.address)) ? t('{coin} address copied', { coin: coin.name }) : t("Couldn't copy it. Press and hold the address to copy it."));
  }
  function bytes(n: number) {
    // Translators: a size in megabytes or kilobytes
    return n >= 1048576 ? t('{size} MB', { size: decimal(n / 1048576, 1) }) : t('{size} KB', { size: Math.round(n / 1024) });
  }
  /** Which Lounge runs on which TV, as a sentence. */
  function running() {
    const version = app.state?.device.version ?? '';
    const model = app.state?.device.model;
    // Translators: {version} is a number such as 0.9.1 and {model} is the TV's make and model
    return model == null ? t('Lounge {version} on this TV.', { version }) : t('Lounge {version} on {model}.', { version, model });
  }
  let rows = $derived.by(() => {
    const d = diag;
    if (!d) return [] as [string, string][];
    const n = (k: string) => Number(d[k] ?? 0);
    // Translators: how many plugins are installed and how many of those are turned on
    const plugins = t('{installed} installed, {on} on', { installed: n('plugins'), on: n('pluginsEnabled') });
    return [
      // Translators: diagnostics: how many films, apps and other things the TV holds
      [t('Things Lounge knows about'), String(n('items'))],
      [t('Plugins'), plugins],
      // Translators: diagnostics: how long building the home screen last took, in milliseconds
      [t('Last rebuild took'), t('{n} ms', { n: n('lastBuildMs') })],
      // Translators: diagnostics: when the TV last took in something new, such as "3 minutes ago", or that it never has
      [t('Last change arrived'), n('lastIngestMs') ? ago(n('lastIngestMs')) : t('never')],
      // Translators: diagnostics: the size of what Lounge keeps on the TV
      [t('Saved data'), bytes(n('databaseBytes'))],
      // Translators: diagnostics: how many jobs are queued on the TV, or that none is
      [t('Work waiting'), n('queueDepth') === 0 ? t('none') : String(n('queueDepth'))],
      // Translators: diagnostics: how many requests the TV app has made of Lounge's core
      [t('Requests answered'), String(n('ffiCalls'))],
      // Translators: diagnostics: how long Lounge has been running, such as "2 hours 5 minutes"
      [t('Running for'), span(n('uptimeMs'))],
      // Translators: diagnostics: a number that goes up each time the home screen changes
      [t('Home screen version'), String(n('revision'))],
      // Translators: diagnostics: how capable the TV's hardware is
      [t('Device tier'), String(d['tier'] ?? '')],
    ] as [string, string][];
  });
</script>

<main class="page">
  <header class="page-head"><div><p class="meta">{app.state?.device.name ?? t('Your TV')}</p><h1>{t('More')}</h1></div></header>

  <section class="section">
    <div class="section-head"><h3>{t('Back up')}</h3></div>
    <p class="caption">{t("Your layout, hidden items, pins, plugin settings and the TV's theme, background and sounds as one JSON file. Passwords and private addresses are not included.")}</p>
    <div class="actions">
      <a class="btn" href={api.backupUrl()} download="lounge-backup.json">{t('Download backup')}</a>
      {#if chosen}
        <!-- Translators: the button that restores a backup, and the question it asks first; {file} is the name of the file chosen -->
        <Confirm label={restoring ? t('Restoring…') : t('Restore {file}', { file: chosen.name })} confirm={t("Yes, replace the TV's layout and settings")} disabled={restoring} onconfirm={restore} />
        <button class="btn ghost" disabled={restoring} onclick={() => (chosen = null)}>{t('Not now')}</button>
      {:else}
        <button class="btn" onclick={() => picker?.click()}>{t('Restore from file')}</button>
      {/if}
      <input type="file" accept="application/json,.json" onchange={choose} hidden bind:this={picker} />
    </div>
    {#if chosen}<p class="caption" role="status">{t("Restoring replaces the TV's rows, pins and hidden items with the ones in the file, and its theme, background and sounds where the file has them.")}</p>{/if}
  </section>

  <section class="section">
    <div class="section-head"><h3>{t('Paired phones')}</h3></div>
    <div class="list">
      {#each devices as d (d.id)}
        <div class="list-row">
          <!-- Translators: under a paired phone's name; {when} is how long ago the TV heard from it, such as "3 minutes ago" or "just now" -->
          <div class="grow"><div class="title">{d.name}</div><div class="caption">{d.current ? t('This phone') : t('Seen {when}', { when: ago(d.lastSeenMs) })}</div></div>
          <!-- Translators: a button that makes the TV forget a paired phone, and the question it asks first -->
          {#if !d.current}<Confirm quiet label={t('Forget')} confirm={t('Yes, forget it')} onconfirm={() => forget(d)} />{/if}
        </div>
      {/each}
    </div>
  </section>

  <section class="section">
    <div class="section-head"><h3>{t('Pop-ups from your home')}</h3></div>
    <p class="caption">{t("Home Assistant, or anything in your home that can call an address, can put a pop-up on the TV: the camera's picture when someone rings the door, say. It needs a key made here, which lets it show pop-ups and nothing else.")}</p>
    {#if newKey}
      <div class="card recipe">
        <p class="caption">{t('This key is shown once. Put this in configuration.yaml in Home Assistant, then restart it.')}</p>
        <pre class="yaml">{recipe}</pre>
        <button class="btn small" onclick={() => copyRecipe(recipe)}>{t('Copy')}</button>
        <p class="caption">{t("Then add this to the actions of an automation that runs when the bell rings, with your Home Assistant's address and your camera in place of the ones here.")}</p>
        <pre class="yaml">{ring}</pre>
        <button class="btn small" onclick={() => copyRecipe(ring)}>{t('Copy')}</button>
      </div>
    {/if}
    <div class="actions">
      {#if keySet}
        <button class="btn" onclick={() => done(t('Sent. Look at the TV.'), () => api.testAlert())}>{t('Send a test pop-up')}</button>
        <!-- Translators: buttons under Pop-ups from your home, and the question each asks first -->
        <Confirm quiet label={t('Make a new key')} confirm={t('Yes, the old key stops working')} onconfirm={makeKey} />
        <Confirm quiet label={t('Remove the key')} confirm={t('Yes, remove it')} onconfirm={removeKey} />
      {:else if keySet === false}
        <button class="btn" onclick={makeKey}>{t('Make a key')}</button>
      {/if}
    </div>
    <p class="caption">{t('A pop-up shows over whatever is on the TV once Power Setup on the TV has run “Show pop-ups over other apps”. Until then it shows on the home screen only.')}</p>
  </section>

  <section class="section">
    <div class="section-head"><h3>{t('Diagnostics')}</h3><span class="caption">{t('updates every 5 seconds')}</span></div>
    {#if diag}
      <div class="card diag">
        {#each rows as [k, v] (k)}
          <div class="kv"><span class="caption">{k}</span><span class="meta mono">{v}</span></div>
        {/each}
      </div>
    {:else}
      <p class="caption">{t('Waiting for the TV…')}</p>
    {/if}
  </section>

  <section class="section">
    <div class="section-head"><h3>{t('Support Lounge')}</h3></div>
    <div class="card give">
      <p class="pitch">{t('No adverts. No tracking. No price.')}</p>
      <p class="only">{t('This is the one thing Lounge asks for.')}</p>
      <p>{t('If it has earned its place on your TV, give what it has been worth to you. Even the price of a coffee keeps it going, and absolutely anything helps.')}</p>
      <a class="btn primary" href={paypal.href} target="_blank" rel="noopener">{t('Give with PayPal')}</a>
      <p class="caption">{t('You choose the amount. All of it goes straight to its maker, and Lounge stays just the same whether you give or not.')}</p>
    </div>
    <div class="list">
      {#each coins as coin (coin.name)}
        <div class="card coin">
          <div class="coin-head">
            <span class="title">{coin.name}</span>
            <button class="btn small" onclick={() => copy(coin)}>{t('Copy address')}</button>
          </div>
          <p class="address">{spaced(coin.address)}</p>
          <!-- Translators: {coin} is a currency's name, such as Bitcoin; the link opens the address in a wallet app -->
          <p class="caption">{t('Send only {coin} to this address.', { coin: coin.name })} <a href={coin.href}>{t('Open it in a wallet')}</a></p>
        </div>
      {/each}
    </div>
  </section>

  <section class="section">
    <div class="section-head"><h3>{t('About')}</h3></div>
    <div class="card about">
      <div class="lockup"><img src="/mark.png" alt="" width="66" height="36" /><span class="name">Lounge</span></div>
      <p>{t("A home screen for your TV, laid out around what you're watching. Free and open source under the MIT licence.")}</p>
      <p class="caption">{running()} {t('Everything these pages show goes between this phone and the TV over your own network. The links below and the ways to give open on the internet.')}</p>
    </div>
    <div class="list">
      {#each aboutLinks as link (link.href)}
        <a class="list-row" href={link.href} target="_blank" rel="noopener">
          <span class="grow"><span class="title">{tr(link.name)}</span><span class="caption">{link.shown}</span></span>
        </a>
      {/each}
    </div>
    <!-- Translators: a label above the name of the person who makes Lounge -->
    <div class="maker"><img src="/maker.png" alt="" width="32" height="32" /><span><span class="caption">{t('Made by')}</span><span class="title">Gingerbreadfork</span></span></div>
    <Confirm quiet label={t('Forget this phone')} confirm={t('Yes, forget this phone')} onconfirm={forgetThisPhone} />
  </section>
</main>

<style>
  .actions { display: flex; gap: 10px; flex-wrap: wrap; }
  .diag { display: grid; grid-template-columns: 1fr 1fr; gap: 10px 16px; }
  .kv { display: flex; flex-direction: column; gap: 2px; }
  .mono { color: var(--limelight); }
  .give, .about, .coin { display: flex; flex-direction: column; gap: 10px; }
  .give .btn { align-self: flex-start; }
  .pitch { font-family: var(--headline); font-size: 22px; font-weight: 600; line-height: 1.2; }
  .only { color: var(--houselight); font-weight: 600; }
  .coin-head { display: flex; align-items: center; justify-content: space-between; gap: 12px; }
  .coin-head .title, .maker .title { font-weight: 600; font-size: 16px; }
  .btn.small { min-height: 36px; padding: 0 14px; font-size: 14px; background: var(--curtain-edge); }
  /* The address is one thing to copy: a hold on it selects all of it. */
  .address { font-variant-numeric: tabular-nums; font-size: 15px; line-height: 1.5; overflow-wrap: anywhere; user-select: all; -webkit-user-select: all; }
  .coin a { color: var(--limelight); text-decoration: underline; }
  .recipe { display: flex; flex-direction: column; align-items: flex-start; gap: 10px; }
  .yaml { align-self: stretch; margin: 0; padding: 12px 14px; border-radius: 10px; background: var(--velvet); color: var(--limelight); font: 13px/1.5 ui-monospace, monospace; white-space: pre; overflow-x: auto; user-select: all; -webkit-user-select: all; }
  .lockup { display: flex; align-items: flex-end; gap: 12px; }
  .lockup .name { font-family: var(--display); font-size: 30px; font-weight: 650; line-height: 1; }
  .list-row .grow { display: flex; flex-direction: column; gap: 2px; }
  .maker { display: flex; align-items: center; gap: 12px; }
  .maker > span { display: flex; flex-direction: column; }
</style>
