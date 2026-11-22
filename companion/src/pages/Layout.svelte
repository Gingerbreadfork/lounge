<script lang="ts">
  // The home screen's shape, from the phone: which rows show and in what order, what sits on
  // the Stage, which apps stay at the front, and everything that is hidden.
  import { untrack } from 'svelte';
  import { api, art, type AppRef, type Layout, type NamedRef, type RowRef, type SourceRef } from '../lib/api';
  import Icon from '../lib/Icon.svelte';
  import { t, tc, tn } from '../lib/lang.svelte';
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
  // Saves run in order and only the last answer is shown, so the TV and the page end on the
  // last move. When that save fails, the page reads the TV again.
  let queue: Promise<void> = Promise.resolve();
  let waiting = 0;
  function save(patch: Partial<Layout>, message = t('Saved to the TV')) {
    edits++;
    busy = true;
    waiting++;
    queue = queue.then(async () => {
      const l = await act(message, () => api.setLayout(patch));
      waiting--;
      if (waiting > 0) return;
      if (l) layout = l;
      else {
        const began = edits;
        const fresh = await quiet(() => api.layout());
        if (fresh && waiting === 0 && edits === began) layout = fresh;
      }
      if (waiting === 0) busy = false;
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
    if (layout) save({ rowOrder: layout.rowOrder }, t('Rows reordered'));
  }
  function toggleRow(row: RowRef) {
    if (!layout) return;
    const shown = !row.shown;
    change(
      { rowVisibility: { ...layout.rowVisibility, [row.id]: shown } },
      // Translators: {name} is the title of a home screen row
      shown ? back(row.title) : t('{name} hidden on the TV', { name: row.title }),
      { availableRows: layout.availableRows.map((r) => (r.id === row.id ? { ...r, shown } : r)) },
    );
  }
  function resetRows() {
    if (!layout) return;
    save({ rowOrder: [] }, t('Rows back to their usual order'));
  }

  // Stage
  function unpinStage(item: NamedRef) {
    if (!layout) return;
    change({ stagePins: layout.stagePins.filter((p) => p !== item.id) }, t('Unpinned from the Stage'), { stageItems: layout.stageItems.filter((i) => i.id !== item.id) });
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
    // Translators: "Your apps" is the row's name: its pinned apps are in a new order
    if (layout) save({ pinnedApps: layout.pinnedApps }, t('Your apps reordered'));
  }
  function pinApp(a: AppRef) {
    if (!layout) return;
    change({ pinnedApps: [...layout.pinnedApps, a.package] }, t('{app} pinned to the front of Your apps', { app: a.title }));
  }
  function unpinApp(a: AppRef) {
    if (!layout) return;
    change({ pinnedApps: layout.pinnedApps.filter((p) => p !== a.package) }, t('{app} unpinned from Your apps', { app: a.title }));
  }

  function setRecent(n: number) {
    if (!layout || layout.recentApps === n) return;
    // Translators: "Your apps" is the row's name; {n} is how many recently opened apps follow the pinned ones
    change({ recentApps: n }, n === 0 ? t('Your apps shows pinned apps only') : tn(n, 'Your apps shows {n} recent app', 'Your apps shows {n} recent apps'));
  }

  // Hidden
  function showItem(item: NamedRef) {
    if (!layout) return;
    change({ hidden: layout.hidden.filter((h) => h !== item.id) }, back(item.title), { hiddenItems: layout.hiddenItems.filter((i) => i.id !== item.id) });
  }
  // Continue watching's sources: everything is shown unless it is hidden here or from a tile.
  function toggleSource(source: SourceRef) {
    if (!layout) return;
    const shown = !source.shown;
    const others = layout.mutedProviders.filter((p) => p !== source.id);
    change(
      { mutedProviders: shown ? others : [...others, source.id] },
      // Translators: {source} is an app or plugin that brings titles to the Continue watching row
      shown ? t('{source} is back in Continue watching', { source: source.title }) : t('{source} hidden from Continue watching', { source: source.title }),
      // One that brings nothing is only listed while it is hidden.
      { continueSources: layout.continueSources.filter((s) => s.id !== source.id || !shown || s.count > 0).map((s) => (s.id === source.id ? { ...s, shown } : s)) },
    );
  }
  /** What a toast says once something hidden shows on the TV again. */
  function back(name: string) {
    // Translators: {name} is the title of a row, an app, a film or anything else that was hidden
    return t('{name} is back on the TV', { name });
  }
  /** How many titles a source brings to Continue watching, with its own wording while it is hidden. */
  function titles(n: number, shown: boolean) {
    // Translators: how many titles an app or plugin brings to Continue watching just now
    if (shown) return n === 0 ? t('Nothing to continue now') : tn(n, '{n} title', '{n} titles');
    // Translators: for an app or plugin hidden from Continue watching: how many titles it would bring
    return n === 0 ? t('Hidden, nothing to continue now') : tn(n, 'Hidden, {n} title', 'Hidden, {n} titles');
  }
</script>

<main class="page">
  <header class="page-head">
    <div><p class="meta">{t('Home screen')}</p><h1>{t('Layout')}</h1></div>
    {#if busy}<span class="pill">{t('Saving…')}</span>{/if}
  </header>

  {#if layout}
    <section class="section">
      <!-- Translators: the heading over the home screen's rows, and beside it how many of them are hidden -->
      <div class="section-head"><h3>{t('Rows, top to bottom')}</h3><span class="caption">{hiddenRows ? tn(hiddenRows, '{n} hidden', '{n} hidden') : t('All shown')}</span></div>
      <p class="caption">{t('Drag the dots to reorder. A row with nothing in it stays off the screen until it has something to show.')}</p>
      <Reorder items={rows} keyOf={(r) => r.id} nameOf={(r) => r.title} onmove={moveRow} ondone={saveRows}>
        {#snippet children(row)}
          <div class="grow" class:dim={!row.shown}>
            <div class="title">{row.title}</div>
            <!-- Translators: under a hidden row's title; {detail} is a line on what the row holds -->
            <div class="caption one-line">{row.shown ? row.detail : row.detail ? t('Hidden, {detail}', { detail: row.detail }) : t('Hidden')}</div>
          </div>
          <!-- Translators: a button; {title} is the title of a home screen row -->
          <button class="icon-btn" class:quiet={row.shown} onclick={() => toggleRow(row)} aria-label={row.shown ? t('Hide {title}', { title: row.title }) : t('Show {title}', { title: row.title })}>
            <Icon name={row.shown ? 'show' : 'hide'} size={22} />
          </button>
        {/snippet}
      </Reorder>
      <button class="btn ghost" onclick={resetRows}>{t('Put the rows back in their usual order')}</button>
      <!-- Translators: under the button "Put the rows back in their usual order": hidden rows stay hidden -->
      <p class="caption center">{t('Keeps what you have hidden.')}</p>
    </section>

    <section class="section">
      <!-- Translators: the heading over the things pinned to the Stage -->
      <div class="section-head"><h3>{t('On the Stage')}</h3></div>
      <!-- Translators: "What's on" is the phone's first page; {choice} is the menu choice "Pin to the Stage" -->
      <p class="caption">{t("Pinned things stay among the big pictures at the top. Hold a tile on What's on and choose “{choice}”.", { choice: words.pinStage })}</p>
      {#if layout.stageItems.length}
        <div class="list">
          {#each layout.stageItems as item (item.id)}
            <div class="list-row">
              {#if item.iconPath}<img class="row-icon" src={art(item.iconPath)} alt="" />{:else}<span class="glyph"><Icon name="pin" size={24} /></span>{/if}
              <div class="grow title">{item.title}</div>
              <button class="btn ghost" onclick={() => unpinStage(item)}>{t('Unpin')}</button>
            </div>
          {/each}
        </div>
      {/if}
    </section>

    <section class="section">
      <div class="section-head"><h3>{t('Your apps')}</h3></div>
      <p class="caption">{t('Pinned apps stay at the front in this order, then the apps you opened last. Everything else is in All apps on the TV, where holding OK on an app pins it.')}</p>
      <div class="segments" role="radiogroup" aria-label={t('Recent apps shown after the pins')}>
        {#each [0, 5, 10, 15] as n (n)}
          <!-- Translators: a choice of how many recently opened apps follow the pinned ones in Your apps -->
          <button class="segment" class:on={layout.recentApps === n} role="radio" aria-checked={layout.recentApps === n} onclick={() => setRecent(n)}>{n === 0 ? t('Pinned only') : tn(n, '{n} recent', '{n} recent')}</button>
        {/each}
      </div>
      {#if pinnedApps.length}
        <Reorder items={pinnedApps} keyOf={(a) => a.package} nameOf={(a) => a.title} onmove={movePinned} ondone={savePinned}>
          {#snippet children(a)}
            {#if a.iconPath}<img class="row-icon" src={art(a.iconPath)} alt="" />{/if}
            <!-- Translators: under the name of an app that is pinned to the front of Your apps -->
            <div class="grow"><div class="title">{a.title}</div><div class="caption">{t('Pinned')}</div></div>
            <button class="btn ghost" onclick={() => unpinApp(a)}>{t('Unpin')}</button>
          {/snippet}
        </Reorder>
      {/if}
      {#if layout.apps.length > 8}
        <div class="field"><input type="search" placeholder={t('Find an app to pin')} bind:value={appQuery} aria-label={t('Find an app to pin')} autocomplete="off" /></div>
      {/if}
      {#if otherApps.length}
        <div class="list">
          {#each otherApps as a (a.package)}
            <div class="list-row">
              {#if a.iconPath}<img class="row-icon" src={art(a.iconPath)} alt="" />{:else}<span class="glyph"><Icon name="pin" size={24} /></span>{/if}
              <div class="grow title">{a.title}</div>
              <!-- Translators: a button that pins the app to the front of Your apps -->
              <button class="btn ghost" onclick={() => pinApp(a)}>{t('Pin')}</button>
            </div>
          {/each}
        </div>
      {:else if appQuery}
        <p class="caption">{t('No app called “{query}”.', { query: appQuery })}</p>
      {/if}
    </section>

    <section class="section">
      <!-- Translators: the heading over the list of things hidden from every row -->
      <div class="section-head"><h3>{tc('heading', 'Hidden from every row')}</h3></div>
      {#if layout.hiddenItems.length === 0}
        <!-- Translators: "What's on" is the phone's first page, OK is the remote's button and Hide is the menu choice -->
        <p class="caption">{t("Nothing is hidden. Hold a tile on What's on, or OK on the TV, and choose Hide.")}</p>
      {:else}
        <div class="list">
          {#each layout.hiddenItems as item (item.id)}
            <div class="list-row">
              {#if item.iconPath}<img class="row-icon" src={art(item.iconPath)} alt="" />{:else}<span class="glyph"><Icon name="hide" size={24} /></span>{/if}
              <div class="grow title">{item.title}</div>
              <!-- Translators: a button that puts a hidden thing back on the home screen -->
              <button class="btn ghost" onclick={() => showItem(item)}>{t('Show')}</button>
            </div>
          {/each}
        </div>
      {/if}
    </section>

    <section class="section">
      <!-- Translators: the heading over the apps and plugins that bring titles to the Continue watching row -->
      <div class="section-head"><h3>{t('Continue watching comes from')}</h3></div>
      {#if layout.continueSources.length === 0}
        <p class="caption">{t('Nothing to continue yet. Apps and plugins are listed here once they bring something.')}</p>
      {:else}
        <p class="caption">{t('Everything is shown unless you hide it here, or hold a tile and hide everything from its app.')}</p>
        <div class="list">
          {#each layout.continueSources as source (source.id)}
            <div class="list-row">
              {#if source.iconPath}<img class="row-icon" src={art(source.iconPath)} alt="" />{:else}<span class="glyph"><Icon name="play" size={22} /></span>{/if}
              <div class="grow" class:dim={!source.shown}>
                <div class="title">{source.title}</div>
                <div class="caption one-line">{titles(source.count, source.shown)}</div>
              </div>
              <!-- Translators: a button; {title} is the name of an app or plugin -->
              <button class="icon-btn" class:quiet={source.shown} onclick={() => toggleSource(source)} aria-label={source.shown ? t('Hide {title}', { title: source.title }) : t('Show {title}', { title: source.title })}>
                <Icon name={source.shown ? 'show' : 'hide'} size={22} />
              </button>
            </div>
          {/each}
        </div>
      {/if}
    </section>
  {:else}
    <p class="empty">{t('Loading the layout…')}</p>
  {/if}
</main>

<style>
  .row-icon { width: 32px; height: 32px; border-radius: 8px; object-fit: cover; flex: none; }
  .segments { display: flex; gap: 6px; padding: 4px; background: var(--curtain); border-radius: 14px; }
  .segment { flex: 1; min-height: 44px; border-radius: 10px; font-size: 14px; font-weight: 600; color: var(--dust); transition: background 120ms ease-out, color 120ms ease-out; }
  .segment.on { background: var(--velvet); color: var(--limelight); box-shadow: inset 0 0 0 1px var(--curtain-edge); }
  .dim .title { color: var(--dust); }
  .center { text-align: center; margin-top: -10px; }
  .one-line { white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
  .section > .caption { margin-top: -4px; }
</style>
