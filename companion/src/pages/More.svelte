<script lang="ts">
  import { api, type Device, type Diagnostics } from '../lib/api';
  import Confirm from '../lib/Confirm.svelte';
  import { aboutLinks, coins, copyText, paypal, spaced, type Coin } from '../lib/giving';
  import { act, app, done, quiet, toast, unpair } from '../lib/state.svelte';
  import { ago, span } from '../lib/words';

  let diag = $state<Diagnostics | null>(null);
  let devices = $state<Device[]>([]);
  /** A backup chosen to restore, waiting for the person to say it may replace the TV's layout and settings. */
  let chosen = $state<File | null>(null);
  let restoring = $state(false);
  let picker = $state<HTMLInputElement>();

  $effect(() => {
    load();
    const t = setInterval(load, 5000);
    return () => clearInterval(t);
  });
  async function load() {
    diag = (await quiet(() => api.diagnostics())) ?? diag;
    devices = (await quiet(() => api.devices())) ?? devices;
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
    if (await done('Layout and settings restored on the TV', async () => api.restore(await file.text()))) chosen = null;
    restoring = false;
  }
  async function forget(d: Device) {
    const forgotten = await done(d.current ? 'The TV has forgotten this phone' : `${d.name} forgotten`, () => api.revokeDevice(d.id));
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
    toast((await copyText(coin.address)) ? `${coin.name} address copied` : `Couldn't copy it. Press and hold the address to copy it.`);
  }
  function bytes(n: number) {
    return n >= 1048576 ? `${(n / 1048576).toFixed(1)} MB` : `${Math.round(n / 1024)} KB`;
  }
  let rows = $derived.by(() => {
    const d = diag;
    if (!d) return [] as [string, string][];
    const n = (k: string) => Number(d[k] ?? 0);
    return [
      ['Things Lounge knows about', String(n('items'))],
      ['Plugins', `${n('plugins')} installed, ${n('pluginsEnabled')} on`],
      ['Last rebuild took', `${n('lastBuildMs')} ms`],
      ['Last change arrived', n('lastIngestMs') ? ago(n('lastIngestMs')) : 'never'],
      ['Saved data', bytes(n('databaseBytes'))],
      ['Work waiting', n('queueDepth') === 0 ? 'none' : String(n('queueDepth'))],
      ['Requests answered', String(n('ffiCalls'))],
      ['Running for', span(n('uptimeMs'))],
      ['Home screen version', String(n('revision'))],
      ['Device tier', String(d['tier'] ?? '')],
    ] as [string, string][];
  });
</script>

<main class="page">
  <header class="page-head"><div><p class="meta">{app.state?.device.name ?? 'Your TV'}</p><h1>More</h1></div></header>

  <section class="section">
    <div class="section-head"><h3>Back up</h3></div>
    <p class="caption">Your layout, hidden items, pins, plugin settings and the TV's theme, background and sounds as one JSON file. Passwords and private addresses are not included.</p>
    <div class="actions">
      <a class="btn" href={api.backupUrl()} download="lounge-backup.json">Download backup</a>
      {#if chosen}
        <Confirm label={restoring ? 'Restoring…' : `Restore ${chosen.name}`} confirm="Yes, replace the TV's layout and settings" disabled={restoring} onconfirm={restore} />
        <button class="btn ghost" disabled={restoring} onclick={() => (chosen = null)}>Not now</button>
      {:else}
        <button class="btn" onclick={() => picker?.click()}>Restore from file</button>
      {/if}
      <input type="file" accept="application/json,.json" onchange={choose} hidden bind:this={picker} />
    </div>
    {#if chosen}<p class="caption" role="status">Restoring replaces the TV's rows, pins and hidden items with the ones in the file, and its theme, background and sounds where the file has them.</p>{/if}
  </section>

  <section class="section">
    <div class="section-head"><h3>Paired phones</h3></div>
    <div class="list">
      {#each devices as d (d.id)}
        <div class="list-row">
          <div class="grow"><div class="title">{d.name}</div><div class="caption">{d.current ? 'This phone' : `Seen ${ago(d.lastSeenMs)}`}</div></div>
          {#if !d.current}<Confirm quiet label="Forget" confirm="Yes, forget it" onconfirm={() => forget(d)} />{/if}
        </div>
      {/each}
    </div>
  </section>

  <section class="section">
    <div class="section-head"><h3>Diagnostics</h3><span class="caption">updates every 5 seconds</span></div>
    {#if diag}
      <div class="card diag">
        {#each rows as [k, v] (k)}
          <div class="kv"><span class="caption">{k}</span><span class="meta mono">{v}</span></div>
        {/each}
      </div>
    {:else}
      <p class="caption">Waiting for the TV…</p>
    {/if}
  </section>

  <section class="section">
    <div class="section-head"><h3>Support Lounge</h3></div>
    <div class="card give">
      <p class="pitch">No adverts. No tracking. No price.</p>
      <p class="only">This is the one thing Lounge asks for.</p>
      <p>If it has earned its place on your TV, give what it has been worth to you. Even the price of a coffee keeps it going, and absolutely anything helps.</p>
      <a class="btn primary" href={paypal.href} target="_blank" rel="noopener">Give with PayPal</a>
      <p class="caption">You choose the amount. All of it goes straight to its maker, and Lounge stays just the same whether you give or not.</p>
    </div>
    <div class="list">
      {#each coins as coin (coin.name)}
        <div class="card coin">
          <div class="coin-head">
            <span class="title">{coin.name}</span>
            <button class="btn small" onclick={() => copy(coin)}>Copy address</button>
          </div>
          <p class="address">{spaced(coin.address)}</p>
          <p class="caption">Send only {coin.name} to this address. <a href={coin.href}>Open it in a wallet</a></p>
        </div>
      {/each}
    </div>
  </section>

  <section class="section">
    <div class="section-head"><h3>About</h3></div>
    <div class="card about">
      <div class="lockup"><img src="/mark.png" alt="" width="66" height="36" /><span class="name">Lounge</span></div>
      <p>A home screen for your TV, laid out around what you're watching. Free and open source under the MIT licence.</p>
      <p class="caption">Lounge {app.state?.device.version ?? ''} on {app.state?.device.model ?? 'this TV'}. Everything these pages show goes between this phone and the TV over your own network. The links below and the ways to give open on the internet.</p>
    </div>
    <div class="list">
      {#each aboutLinks as link (link.href)}
        <a class="list-row" href={link.href} target="_blank" rel="noopener">
          <span class="grow"><span class="title">{link.name}</span><span class="caption">{link.shown}</span></span>
        </a>
      {/each}
    </div>
    <div class="maker"><img src="/maker.png" alt="" width="32" height="32" /><span><span class="caption">Made by</span><span class="title">Gingerbreadfork</span></span></div>
    <Confirm quiet label="Forget this phone" confirm="Yes, forget this phone" onconfirm={forgetThisPhone} />
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
  .lockup { display: flex; align-items: flex-end; gap: 12px; }
  .lockup .name { font-family: var(--display); font-size: 30px; font-weight: 650; line-height: 1; }
  .list-row .grow { display: flex; flex-direction: column; gap: 2px; }
  .maker { display: flex; align-items: center; gap: 12px; }
  .maker > span { display: flex; flex-direction: column; }
</style>
