<script lang="ts">
  import { api, type Device, type Diagnostics } from '../lib/api';
  import Confirm from '../lib/Confirm.svelte';
  import { act, app, done, quiet, unpair } from '../lib/state.svelte';
  import { ago, span } from '../lib/words';

  let diag = $state<Diagnostics | null>(null);
  let devices = $state<Device[]>([]);
  /** A backup chosen to restore, waiting for the person to say it may replace the TV's layout. */
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
  function bytes(n: number) {
    return n >= 1048576 ? `${(n / 1048576).toFixed(1)} MB` : `${Math.round(n / 1024)} KB`;
  }
  let rows = $derived.by(() => {
    const d = diag;
    if (!d) return [] as [string, string][];
    const n = (k: string) => Number(d[k] ?? 0);
    return [
      ['Items on the home screen', String(n('items'))],
      ['Plugins', `${n('plugins')} installed, ${n('pluginsEnabled')} on`],
      ['Last rebuild took', `${n('lastBuildMs')} ms`],
      ['Last change arrived', n('lastIngestMs') ? ago(n('lastIngestMs')) : 'never'],
      ['Database', bytes(n('databaseBytes'))],
      ['Work waiting', n('queueDepth') === 0 ? 'none' : String(n('queueDepth'))],
      ['Calls into the core', String(n('ffiCalls'))],
      ['Running for', span(n('uptimeMs'))],
      ['Snapshot revision', String(n('revision'))],
      ['Device tier', String(d['tier'] ?? '')],
    ] as [string, string][];
  });
</script>

<main class="page">
  <header class="page-head"><div><p class="meta">{app.state?.device.name ?? 'Your TV'}</p><h1>More</h1></div></header>

  <section class="section">
    <div class="section-head"><h3>Back up</h3></div>
    <p class="caption">Your layout, hidden items, pins and plugin settings as one JSON file. Passwords and private addresses are not included.</p>
    <div class="actions">
      <a class="btn" href={api.backupUrl()} download="lounge-backup.json">Download backup</a>
      {#if chosen}
        <Confirm label={restoring ? 'Restoring…' : `Restore ${chosen.name}`} confirm="Yes, replace the TV's layout" disabled={restoring} onconfirm={restore} />
        <button class="btn ghost" disabled={restoring} onclick={() => (chosen = null)}>Not now</button>
      {:else}
        <button class="btn" onclick={() => picker?.click()}>Restore from file</button>
      {/if}
      <input type="file" accept="application/json,.json" onchange={choose} hidden bind:this={picker} />
    </div>
    {#if chosen}<p class="caption" role="status">Restoring replaces the TV's rows, pins and hidden items with the ones in the file.</p>{/if}
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
    <div class="section-head"><h3>About</h3></div>
    <p class="caption">Lounge {app.state?.device.version ?? ''} on {app.state?.device.model ?? 'this TV'}. Everything on this page goes between this phone and the TV over your own network.</p>
    <Confirm quiet label="Forget this phone" confirm="Yes, forget this phone" onconfirm={forgetThisPhone} />
  </section>
</main>

<style>
  .actions { display: flex; gap: 10px; flex-wrap: wrap; }
  .diag { display: grid; grid-template-columns: 1fr 1fr; gap: 10px 16px; }
  .kv { display: flex; flex-direction: column; gap: 2px; }
  .mono { color: var(--limelight); }
</style>
