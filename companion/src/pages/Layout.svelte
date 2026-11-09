<script lang="ts">
  // The home screen's shape, from the phone: which rows show and in what order, what sits on
  // the Stage, which apps stay at the front, and everything that is hidden.
  import { untrack } from 'svelte';
  import { api, art, type AppRef, type Layout, type NamedRef, type RowRef, type SourceRef } from '../lib/api';
  import Icon from '../lib/Icon.svelte';
  import Reorder from '../lib/Reorder.svelte';
  import { act, app, quiet } from '../lib/state.svelte';
  import { words } from '../lib/words';

  let layout = $state<Layout | null>(null);
  let busy = $state(false);
  let appQuery = $state('');

  // Read once, then again with each snapshot, so a tile pinned with the remote shows up here too.
  $effect(() => {
    app.state?.snapshot?.revision;
    untrack(() => load());
  });

  // A read that began before a change was made here is thrown away, so it cannot put the
  // older layout back on screen.
  let edits = 0;
  async function load() {
    if (waiting > 0) return;
    const began = edits;
    const l = await quiet(() => api.layout());
    if (l && waiting === 0 && edits === began) layout = l;
  }
  // Saves run one at a time and in order, so the TV ends up with the last move. Only the last
  // answer is shown, so an older one cannot undo a newer move on screen.
  let queue: Promise<void> = Promise.resolve();
  let waiting = 0;
  function save(patch: Partial<Layout>, message = 'Saved to the TV') {
    edits++;
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
  // Each change shows at once, so the next one builds on it even before the TV has answered;
  // `local` is what the page shows meanwhile beyond what the TV is sent.
  function change(patch: Partial<Layout>, message: string, local: Partial<Layout> = {}) {
    if (!layout) return;
    layout = { ...layout, ...patch, ...local };
    save(patch, message);
  }

  // Rows
  let rows = $derived.by(() => {
    if (!layout) return [] as RowRef[];
    const byId = new Map(layout.availableRows.map((r) => [r.id, r]));
    return layout.rowOrder.map((id) => byId.get(id) ?? { id, title: id, shown: true, chosen: false, count: 0, detail: '' });
  });
  let hiddenRows = $derived(rows.filter((r) => !r.shown && r.chosen).length);
  // A drag reorders on screen as it goes and saves once the row is set down.
  function moveRow(from: number, to: number) {
    if (!layout) return;
    const order = [...layout.rowOrder];
    const [id] = order.splice(from, 1);
    order.splice(to, 0, id);
    edits++;
    layout = { ...layout, rowOrder: order };
  }
  function saveRows() {
    if (layout) save({ rowOrder: layout.rowOrder }, 'Rows reordered');
  }
  function toggleRow(row: RowRef) {
    if (!layout) return;
    const shown = !row.shown;
    change(
      { rowVisibility: { ...layout.rowVisibility, [row.id]: shown } },
      shown ? `${row.title} is back on the TV` : `${row.title} hidden on the TV`,
      { availableRows: layout.availableRows.map((r) => (r.id === row.id ? { ...r, shown } : r)) },
    );
  }
  function resetRows() {
    if (!layout) return;
    save({ rowOrder: [] }, 'Rows back to their usual order');
  }

  // Stage
  function unpinStage(item: NamedRef) {
    if (!layout) return;
    change({ stagePins: layout.stagePins.filter((p) => p !== item.id) }, 'Unpinned from the Stage', { stageItems: layout.stageItems.filter((i) => i.id !== item.id) });
  }

  // Apps
  let pinnedApps = $derived.by(() => {
    if (!layout) return [] as AppRef[];
    const byPkg = new Map(layout.apps.map((a) => [a.package, a]));
    return layout.pinnedApps.map((p) => byPkg.get(p) ?? { package: p, title: p, pinned: true });
  });
  let otherApps = $derived.by(() => {
    if (!layout) return [] as AppRef[];
    const q = appQuery.trim().toLowerCase();
    return layout.apps.filter((a) => !layout!.pinnedApps.includes(a.package)).filter((a) => !q || a.title.toLowerCase().includes(q));
  });
  function movePinned(from: number, to: number) {
    if (!layout) return;
    const pins = [...layout.pinnedApps];
    const [p] = pins.splice(from, 1);
    pins.splice(to, 0, p);
    edits++;
    layout = { ...layout, pinnedApps: pins };
  }
  function savePinned() {
    if (layout) save({ pinnedApps: layout.pinnedApps }, 'Your apps reordered');
  }
  function pinApp(a: AppRef) {
    if (!layout) return;
    change({ pinnedApps: [...layout.pinnedApps, a.package] }, `${a.title} pinned to the front of Your apps`);
  }
  function unpinApp(a: AppRef) {
    if (!layout) return;
    change({ pinnedApps: layout.pinnedApps.filter((p) => p !== a.package) }, `${a.title} unpinned from Your apps`);
  }

  function setRecent(n: number) {
    if (!layout || layout.recentApps === n) return;
    change({ recentApps: n }, n === 0 ? 'Your apps shows pinned apps only' : `Your apps shows ${n} recent apps`);
  }

  // Hidden
  function showItem(item: NamedRef) {
    if (!layout) return;
    change({ hidden: layout.hidden.filter((h) => h !== item.id) }, `${item.title} is back on the TV`, { hiddenItems: layout.hiddenItems.filter((i) => i.id !== item.id) });
  }
  // Continue watching's sources: everything is shown unless it is hidden here or from a tile.
  function toggleSource(source: SourceRef) {
    if (!layout) return;
    const shown = !source.shown;
    const others = layout.mutedProviders.filter((p) => p !== source.id);
    change(
      { mutedProviders: shown ? others : [...others, source.id] },
      shown ? `${source.title} is back in Continue watching` : `${source.title} hidden from Continue watching`,
      // One that brings nothing is only listed while it is hidden.
      { continueSources: layout.continueSources.filter((s) => s.id !== source.id || !shown || s.count > 0).map((s) => (s.id === source.id ? { ...s, shown } : s)) },
    );
  }
  function titles(n: number) {
    return n === 0 ? 'Nothing to continue now' : n === 1 ? '1 title' : `${n} titles`;
  }
</script>

<main class="page">
  <header class="page-head">
    <div><p class="meta">Home screen</p><h1>Layout</h1></div>
    {#if busy}<span class="pill">Saving…</span>{/if}
  </header>

  {#if layout}
    <section class="section">
      <div class="section-head"><h3>Rows, top to bottom</h3><span class="caption">{hiddenRows ? `${hiddenRows} hidden` : 'All shown'}</span></div>
      <p class="caption">Drag the dots to reorder. A row with nothing in it stays off the screen until it has something to show.</p>
      <Reorder items={rows} keyOf={(r) => r.id} nameOf={(r) => r.title} onmove={moveRow} ondone={saveRows}>
        {#snippet children(row)}
          <div class="grow" class:dim={!row.shown}>
            <div class="title">{row.title}</div>
            <div class="caption one-line">{row.shown ? row.detail : `Hidden · ${row.detail}`}</div>
          </div>
          <button class="icon-btn" class:quiet={row.shown} onclick={() => toggleRow(row)} aria-label={row.shown ? `Hide ${row.title}` : `Show ${row.title}`}>
            <Icon name={row.shown ? 'show' : 'hide'} size={22} />
          </button>
        {/snippet}
      </Reorder>
      <button class="btn ghost" onclick={resetRows}>Put the rows back in their usual order</button>
      <p class="caption center">Keeps what you have hidden.</p>
    </section>

    <section class="section">
      <div class="section-head"><h3>On the Stage</h3></div>
      <p class="caption">Pinned things stay among the big pictures at the top. Hold a tile on What's on and choose “{words.pinStage}”.</p>
      {#if layout.stageItems.length}
        <div class="list">
          {#each layout.stageItems as item (item.id)}
            <div class="list-row">
              {#if item.iconPath}<img class="row-icon" src={art(item.iconPath)} alt="" />{:else}<span class="glyph"><Icon name="pin" size={24} /></span>{/if}
              <div class="grow title">{item.title}</div>
              <button class="btn ghost" onclick={() => unpinStage(item)}>Unpin</button>
            </div>
          {/each}
        </div>
      {/if}
    </section>

    <section class="section">
      <div class="section-head"><h3>Your apps</h3></div>
      <p class="caption">Pinned apps stay at the front in this order, then the apps you opened last. Everything else is in All apps on the TV, where holding OK on an app pins it.</p>
      <div class="segments" role="radiogroup" aria-label="Recent apps shown after the pins">
        {#each [0, 5, 10, 15] as n}
          <button class="segment" class:on={layout.recentApps === n} role="radio" aria-checked={layout.recentApps === n} onclick={() => setRecent(n)}>{n === 0 ? 'Pinned only' : `${n} recent`}</button>
        {/each}
      </div>
      {#if pinnedApps.length}
        <Reorder items={pinnedApps} keyOf={(a) => a.package} nameOf={(a) => a.title} onmove={movePinned} ondone={savePinned}>
          {#snippet children(a)}
            {#if a.iconPath}<img class="row-icon" src={art(a.iconPath)} alt="" />{/if}
            <div class="grow"><div class="title">{a.title}</div><div class="caption">Pinned</div></div>
            <button class="btn ghost" onclick={() => unpinApp(a)}>Unpin</button>
          {/snippet}
        </Reorder>
      {/if}
      {#if layout.apps.length > 8}
        <div class="field"><input type="search" placeholder="Find an app to pin" bind:value={appQuery} aria-label="Find an app to pin" autocomplete="off" /></div>
      {/if}
      {#if otherApps.length}
        <div class="list">
          {#each otherApps as a (a.package)}
            <div class="list-row">
              {#if a.iconPath}<img class="row-icon" src={art(a.iconPath)} alt="" />{:else}<span class="glyph"><Icon name="pin" size={24} /></span>{/if}
              <div class="grow title">{a.title}</div>
              <button class="btn ghost" onclick={() => pinApp(a)}>Pin</button>
            </div>
          {/each}
        </div>
      {:else if appQuery}
        <p class="caption">No app called “{appQuery}”.</p>
      {/if}
    </section>

    <section class="section">
      <div class="section-head"><h3>Hidden from every row</h3></div>
      {#if layout.hiddenItems.length === 0}
        <p class="caption">Nothing is hidden. Hold a tile on What's on, or OK on the TV, and choose Hide.</p>
      {:else}
        <div class="list">
          {#each layout.hiddenItems as item (item.id)}
            <div class="list-row">
              {#if item.iconPath}<img class="row-icon" src={art(item.iconPath)} alt="" />{:else}<span class="glyph"><Icon name="hide" size={24} /></span>{/if}
              <div class="grow title">{item.title}</div>
              <button class="btn ghost" onclick={() => showItem(item)}>Show</button>
            </div>
          {/each}
        </div>
      {/if}
    </section>

    <section class="section">
      <div class="section-head"><h3>Continue watching comes from</h3></div>
      {#if layout.continueSources.length === 0}
        <p class="caption">Nothing to continue yet. Apps and plugins are listed here once they bring something.</p>
      {:else}
        <p class="caption">Everything is shown unless you hide it here, or hold a tile and hide everything from its app.</p>
        <div class="list">
          {#each layout.continueSources as source (source.id)}
            <div class="list-row">
              {#if source.iconPath}<img class="row-icon" src={art(source.iconPath)} alt="" />{:else}<span class="glyph"><Icon name="play" size={22} /></span>{/if}
              <div class="grow" class:dim={!source.shown}>
                <div class="title">{source.title}</div>
                <div class="caption one-line">{source.shown ? titles(source.count) : `Hidden · ${titles(source.count)}`}</div>
              </div>
              <button class="icon-btn" class:quiet={source.shown} onclick={() => toggleSource(source)} aria-label={source.shown ? `Hide ${source.title}` : `Show ${source.title}`}>
                <Icon name={source.shown ? 'show' : 'hide'} size={22} />
              </button>
            </div>
          {/each}
        </div>
      {/if}
    </section>
  {:else}
    <p class="empty">Loading the layout…</p>
  {/if}
</main>

<style>
  .row-icon { width: 32px; height: 32px; border-radius: 8px; object-fit: cover; flex: none; }
  .segments { display: flex; gap: 6px; padding: 4px; background: var(--curtain); border-radius: 14px; }
  .segment { flex: 1; min-height: 40px; border-radius: 10px; font-size: 14px; font-weight: 600; color: var(--dust); transition: background 120ms ease-out, color 120ms ease-out; }
  .segment.on { background: var(--velvet); color: var(--limelight); box-shadow: inset 0 0 0 1px var(--curtain-edge); }
  .dim .title { color: var(--dust); }
  .center { text-align: center; margin-top: -10px; }
  .one-line { white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
  .section > .caption { margin-top: -4px; }
</style>
