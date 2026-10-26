<script lang="ts">
  import { api, type Device, type Diagnostics } from '../lib/api';
  import { act, app, toast, unpair } from '../lib/state.svelte';

  let diag = $state<Diagnostics | null>(null);
  let devices = $state<Device[]>([]);
  let restoring = $state(false);

  $effect(() => {
    load();
    const t = setInterval(load, 5000);
    return () => clearInterval(t);
  });
  async function load() {
    diag = (await act('', () => api.diagnostics())) ?? diag;
    devices = (await act('', () => api.devices())) ?? devices;
  }
  async function restore(e: Event) {
    const file = (e.target as HTMLInputElement).files?.[0];
    if (!file) return;
    restoring = true;
    const text = await file.text();
    await act('Layout and settings restored on the TV', () => api.restore(text));
    restoring = false;
    (e.target as HTMLInputElement).value = '';
  }
  async function revoke(d: Device) {
    await act(d.current ? 'This phone is no longer paired' : `${d.name} forgotten`, () => api.revokeDevice(d.id));
    if (d.current) unpair(); else load();
  }
  function bytes(n: number) {
    return n >= 1048576 ? `${(n / 1048576).toFixed(1)} MB` : `${Math.round(n / 1024)} KB`;
  }
  function duration(ms: number) {
    const m = Math.floor(ms / 60000);
    if (m < 1) return 'under a minute';
    if (m < 60) return `${m} min`;
    const h = Math.floor(m / 60);
    return h < 48 ? `${h} h ${m % 60} min` : `${Math.floor(h / 24)} days`;
  }
  function ago(ms: number) {
    if (!ms) return 'never';
    const d = Date.now() - ms;
    return d < 60000 ? 'just now' : `${duration(d)} ago`;
  }
  let rows = $derived.by(() => {
    const d = diag;
    if (!d) return [] as [string, string][];
    const n = (k: string) => Number(d[k] ?? 0);
    return [
      ['Items on the home screen', String(n('items'))],
      ['Plugins', `${n('plugins')} installed · ${n('pluginsEnabled')} on`],
      ['Last rebuild took', `${n('lastBuildMs')} ms`],
      ['Last change arrived', ago(n('lastIngestMs'))],
      ['Database', bytes(n('databaseBytes'))],
      ['Work waiting', n('queueDepth') === 0 ? 'none' : String(n('queueDepth'))],
      ['Calls into the core', String(n('ffiCalls'))],
      ['Running for', duration(n('uptimeMs'))],
      ['Snapshot revision', String(n('revision'))],
      ['Device tier', String(d['tier'] ?? '')],
    ] as [string, string][];
  });
</script>

<main class="page">
  <header class="page-head"><div><p class="meta">{app.state?.device.name ?? 'Your TV'}</p><h1>More</h1></div></header>

  <section class="section">
    <div class="section-head"><h3>Back up</h3></div>
    <p class="caption">Your layout, hidden items, pins and plugin settings as one JSON file. Passwords are not included.</p>
    <div class="actions">
      <a class="btn" href={api.backupUrl()} download="lounge-backup.json">Download backup</a>
      <label class="btn">{restoring ? 'Restoring…' : 'Restore from file'}<input type="file" accept="application/json,.json" onchange={restore} hidden /></label>
    </div>
  </section>

  <section class="section">
    <div class="section-head"><h3>Paired phones</h3></div>
    <div class="list">
      {#each devices as d (d.id)}
        <div class="list-row">
          <div class="grow"><div class="title">{d.name}{d.current ? ' (this phone)' : ''}</div><div class="caption">last seen {new Date(d.lastSeenMs).toLocaleString()}</div></div>
          <button class="btn ghost" onclick={() => revoke(d)}>Forget</button>
        </div>
      {/each}
    </div>
  </section>

  <section class="section">
    <div class="section-head"><h3>Diagnostics</h3><span class="caption">updates every 5 s</span></div>
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
    <button class="btn ghost danger" onclick={unpair}>Unpair this phone</button>
  </section>
</main>

<style>
  .actions { display: flex; gap: 10px; flex-wrap: wrap; }
  .diag { display: grid; grid-template-columns: 1fr 1fr; gap: 10px 16px; }
  .kv { display: flex; flex-direction: column; gap: 2px; }
  .mono { color: var(--limelight); }
</style>
