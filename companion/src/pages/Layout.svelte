<script lang="ts">
  import { api, type Layout } from '../lib/api';
  import Icon from '../lib/Icon.svelte';
  import { act, app } from '../lib/state.svelte';

  let layout = $state<Layout | null>(null);
  let busy = $state(false);

  $effect(() => { load(); });

  async function load() {
    layout = (await act('', () => api.layout())) ?? null;
  }
  // Saves run one at a time and in order, so the TV ends up with the last move. Only the last
  // answer is shown, so an older one cannot undo a newer move on screen.
  let queue: Promise<void> = Promise.resolve();
  let waiting = 0;
  function save(patch: Partial<Layout>, message = 'Saved to the TV') {
    busy = true;
    waiting++;
    queue = queue.then(async () => {
      const l = await act(message, () => api.setLayout(patch));
      waiting--;
      if (waiting === 0) {
        if (l) layout = l;
        busy = false;
      }
    });
  }
  function move(id: string, dir: -1 | 1) {
    if (!layout) return;
    const order = [...layout.rowOrder];
    const i = order.indexOf(id);
    const j = i + dir;
    if (i < 0 || j < 0 || j >= order.length) return;
    [order[i], order[j]] = [order[j], order[i]];
    layout = { ...layout, rowOrder: order };
    save({ rowOrder: order });
  }
  function titleOf(id: string) {
    return layout?.availableRows.find((r) => r.id === id)?.title ?? id;
  }
  function sourceOf(id: string) {
    if (id.startsWith('plugin:')) return 'From a plugin';
    if (id.startsWith('channel:')) return 'A channel another app publishes';
    return null;
  }
  // Each change shows at once, so the next one builds on it even before the TV has answered.
  function change(patch: Partial<Layout>, message: string) {
    if (!layout) return;
    layout = { ...layout, ...patch };
    save(patch, message);
  }
  function toggleHidden(id: string) {
    if (!layout) return;
    const shown = layout.hidden.includes(id);
    change({ hidden: shown ? layout.hidden.filter((h) => h !== id) : [...layout.hidden, id] }, shown ? 'Shown again' : 'Hidden on the TV');
  }
  function unpinStage(id: string) {
    if (!layout) return;
    change({ stagePins: layout.stagePins.filter((p) => p !== id) }, 'Unpinned from the Stage');
  }
  function unpinApp(pkg: string) {
    if (!layout) return;
    change({ pinnedApps: layout.pinnedApps.filter((p) => p !== pkg) }, 'Unpinned');
  }
  let snapshotTiles = $derived(app.state?.snapshot.rows.flatMap((r) => r.tiles) ?? []);
  function tileTitle(id: string) {
    return snapshotTiles.find((t) => t.id === id)?.title ?? app.state?.snapshot.stage.strip.find((t) => t.id === id)?.title ?? id;
  }
  function appTitle(pkg: string) {
    return snapshotTiles.find((t) => t.id === `app:${pkg}`)?.title ?? pkg;
  }
</script>

<main class="page">
  <header class="page-head">
    <div><p class="meta">Home screen</p><h1>Layout</h1></div>
    {#if busy}<span class="pill">Saving…</span>{/if}
  </header>

  {#if layout}
    <section class="section">
      <div class="section-head"><h3>Rows, top to bottom</h3><span class="caption">Empty rows stay hidden</span></div>
      <div class="list">
        {#each layout.rowOrder as id, i (id)}
          <div class="list-row">
            <div class="grow"><div class="title">{titleOf(id)}</div>{#if sourceOf(id)}<div class="caption">{sourceOf(id)}</div>{/if}</div>
            <button class="icon-btn" onclick={() => move(id, -1)} disabled={i === 0} aria-label="Move up"><Icon name="up" size={22} /></button>
            <button class="icon-btn" onclick={() => move(id, 1)} disabled={i === layout.rowOrder.length - 1} aria-label="Move down"><Icon name="down" size={22} /></button>
          </div>
        {/each}
      </div>
      {#if layout.rowOrder.length}
        <button class="btn ghost" onclick={() => save({ rowOrder: [] }, 'Default order restored')}>Reset order</button>
      {/if}
    </section>

    <section class="section">
      <div class="section-head"><h3>Pinned to the Stage</h3></div>
      {#if layout.stagePins.length === 0}
        <p class="caption">Long-press a tile on the TV and choose “Pin to Stage”.</p>
      {:else}
        <div class="list">
          {#each layout.stagePins as id (id)}
            <div class="list-row"><div class="grow title">{tileTitle(id)}</div><button class="btn ghost" onclick={() => unpinStage(id)}>Unpin</button></div>
          {/each}
        </div>
      {/if}
    </section>

    <section class="section">
      <div class="section-head"><h3>Your apps, in order</h3></div>
      {#if layout.pinnedApps.length === 0}
        <p class="caption">Apps follow recency until you pin one.</p>
      {:else}
        <div class="list">
          {#each layout.pinnedApps as pkg (pkg)}
            <div class="list-row"><div class="grow"><div class="title">{appTitle(pkg)}</div><div class="caption">{pkg}</div></div><button class="btn ghost" onclick={() => unpinApp(pkg)}>Unpin</button></div>
          {/each}
        </div>
      {/if}
    </section>

    <section class="section">
      <div class="section-head"><h3>Hidden</h3></div>
      {#if layout.hidden.length === 0}
        <p class="caption">Nothing hidden. Long-press a tile on the TV and choose “Hide”.</p>
      {:else}
        <div class="list">
          {#each layout.hidden as id (id)}
            <div class="list-row"><div class="grow title">{tileTitle(id)}</div><button class="btn ghost" onclick={() => toggleHidden(id)}>Show</button></div>
          {/each}
        </div>
      {/if}
    </section>
  {:else}
    <p class="empty">Loading the layout…</p>
  {/if}
</main>

<style>
  .icon-btn { width: 44px; height: 44px; border-radius: 12px; background: var(--velvet); color: var(--limelight); display: grid; place-items: center; flex: none; }
  .icon-btn:disabled { opacity: 0.3; }
</style>
