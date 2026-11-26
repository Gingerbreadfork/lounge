<script lang="ts">
  // The home screen's shape, from the phone: which rows show and in what order, what sits on
  // the Stage, which apps stay at the front, and everything that is hidden.
  import { untrack } from 'svelte';
  import { api, art, type AppRef, type GroupRef, type Layout, type LayoutPatch, type NamedRef, type RowRef, type SourceRef } from '../lib/api';
  import Confirm from '../lib/Confirm.svelte';
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
  function save(patch: LayoutPatch, message = t('Saved to the TV')) {
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

  // App groups
  /** The group whose name and apps are open for change, by id. */
  let openGroup = $state<string | null>(null);
  let newGroup = $state('');
  let renamed = $state('');
  /** The name the field held when the group was opened; only a name typed since then is saved. */
  let renamedFrom = '';
  // The TV keeps no more groups than this.
  const MOST_GROUPS = 24;
  let known = $derived(new Set(layout?.apps.map((a) => a.package)));
  let groupApps = $derived.by(() => {
    const group = layout?.groups.find((g) => g.id === openGroup);
    if (!layout || !group) return { inside: [] as AppRef[], outside: [] as AppRef[] };
    const byPkg = new Map(layout.apps.map((a) => [a.package, a]));
    return { inside: group.apps.flatMap((p) => byPkg.get(p) ?? []), outside: layout.apps.filter((a) => !group.apps.includes(a.package)) };
  });
  // The TV is sent the one group that changed, so a group made there meanwhile is left alone.
  function setGroup(group: GroupRef, next: Partial<GroupRef>, message: string) {
    if (!layout) return;
    const changed = { ...group, ...next };
    layout = { ...layout, groups: layout.groups.map((g) => (g.id === group.id ? changed : g)) };
    save({ groups: [changed] }, message);
  }
  function makeGroup() {
    const name = newGroup.trim();
    if (!layout || !name) return;
    const id = `p${Date.now().toString(36)}`;
    newGroup = '';
    const made = { id, name, apps: [] };
    edit(made);
    layout = { ...layout, groups: [...layout.groups, made] };
    // Translators: {group} is the name of a group of apps just made; its apps are chosen next
    save({ newGroups: [made] }, t('{group} made. Now choose its apps.', { group: name }));
  }
  function edit(group: GroupRef) {
    openGroup = openGroup === group.id ? null : group.id;
    renamed = renamedFrom = group.name;
  }
  function renameGroup(group: GroupRef) {
    const name = renamed.trim();
    if (!name || name === renamedFrom) return;
    renamedFrom = name;
    // Translators: {group} is the new name of a group of apps
    setGroup(group, { name }, t('Renamed to {group}', { group: name }));
  }
  function addToGroup(group: GroupRef, a: AppRef) {
    // Translators: {app} is an app's name and {group} the name of a group of apps
    setGroup(group, { apps: [...group.apps, a.package] }, t('{app} added to {group}', { app: a.title, group: group.name }));
  }
  function removeFromGroup(group: GroupRef, a: AppRef) {
    // Translators: {app} is an app's name and {group} the name of a group of apps
    setGroup(group, { apps: group.apps.filter((p) => p !== a.package) }, t('{app} removed from {group}', { app: a.title, group: group.name }));
  }
  // Moves count the apps listed, so one that is no longer on the TV keeps to the end.
  function moveInGroup(group: GroupRef, from: number, to: number) {
    if (!layout) return;
    const listed = groupApps.inside.map((a) => a.package);
    const [p] = listed.splice(from, 1);
    listed.splice(to, 0, p);
    const apps = [...listed, ...group.apps.filter((x) => !listed.includes(x))];
    edits++;
    layout = { ...layout, groups: layout.groups.map((g) => (g.id === group.id ? { ...g, apps } : g)) };
  }
  function saveGroup(id: string) {
    const group = layout?.groups.find((g) => g.id === id);
    // Translators: the apps in a group are in a new order
    if (group) save({ groups: [group] }, t('Group reordered'));
  }
  function deleteGroup(group: GroupRef) {
    if (!layout) return;
    openGroup = null;
    layout = { ...layout, groups: layout.groups.filter((g) => g.id !== group.id) };
    // Translators: {group} is the name of a group of apps
    save({ deletedGroups: [group.id] }, t('{group} deleted', { group: group.name }));
  }
  /** How many of a group's apps are on the TV now. */
  function onTv(group: GroupRef) {
    return group.apps.filter((p) => known.has(p)).length;
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
      <!-- Translators: the heading over the groups of apps, each a row of its own on the TV -->
      <div class="section-head"><h3>{t('App groups')}</h3></div>
      <p class="caption">{t('A group is a row of apps on the TV under a name you give it, such as Kids or Sport. Drag it into place among the rows at the top of this page.')}</p>
      {#if layout.groups.length}
        <div class="list">
          {#each layout.groups as group (group.id)}
            <div class="list-row">
              <span class="glyph"><Icon name="apps" size={24} /></span>
              <div class="grow">
                <div class="title">{group.name}</div>
                <div class="caption">{onTv(group) === 0 ? t('No apps yet') : tn(onTv(group), '{n} app', '{n} apps')}</div>
              </div>
              <!-- Translators: buttons that open and close a group of apps for change -->
              <button class="btn ghost" aria-expanded={openGroup === group.id} onclick={() => edit(group)}>{openGroup === group.id ? t('Done') : t('Edit')}</button>
            </div>
            {#if openGroup === group.id}
              <div class="group">
                <form class="field" onsubmit={(e) => { e.preventDefault(); renameGroup(group); }}>
                  <!-- Translators: the label of the field that holds a group's name -->
                  <label for="group-name">{t('Name')}</label>
                  <input id="group-name" maxlength="40" autocomplete="off" bind:value={renamed} onblur={() => renameGroup(group)} />
                </form>
                {#if groupApps.inside.length}
                  <Reorder items={groupApps.inside} keyOf={(a) => a.package} nameOf={(a) => a.title} onmove={(from, to) => moveInGroup(group, from, to)} ondone={() => saveGroup(group.id)}>
                    {#snippet children(a)}
                      {#if a.iconPath}<img class="row-icon" src={art(a.iconPath)} alt="" />{/if}
                      <div class="grow title">{a.title}</div>
                      <!-- Translators: a button that takes an app out of a group -->
                      <button class="btn ghost" onclick={() => removeFromGroup(group, a)}>{t('Remove')}</button>
                    {/snippet}
                  </Reorder>
                {/if}
                <div class="list">
                  {#each groupApps.outside as a (a.package)}
                    <div class="list-row">
                      {#if a.iconPath}<img class="row-icon" src={art(a.iconPath)} alt="" />{:else}<span class="glyph"><Icon name="apps" size={24} /></span>{/if}
                      <div class="grow title">{a.title}</div>
                      <!-- Translators: a button that puts an app in a group -->
                      <button class="btn ghost" onclick={() => addToGroup(group, a)}>{t('Add')}</button>
                    </div>
                  {/each}
                </div>
                <!-- Translators: the button that deletes a group of apps, and the question it asks first -->
                <Confirm quiet label={t('Delete this group')} confirm={t('Yes, delete this group')} onconfirm={() => deleteGroup(group)} />
                <p class="caption center">{t('Its apps stay on the TV.')}</p>
              </div>
            {/if}
          {/each}
        </div>
      {/if}
      {#if layout.groups.length < MOST_GROUPS}
        <form class="new-group" onsubmit={(e) => { e.preventDefault(); makeGroup(); }}>
          <!-- Translators: the field that names a new group of apps, and the button that makes it -->
          <div class="field"><input maxlength="40" autocomplete="off" placeholder={t('Name a new group')} aria-label={t('Name a new group')} bind:value={newGroup} /></div>
          <button class="btn" disabled={!newGroup.trim()}>{t('Make it')}</button>
        </form>
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
  .group { display: flex; flex-direction: column; gap: 12px; padding: 4px 0 12px 16px; border-left: 2px solid var(--curtain-edge); margin-left: 12px; }
  .group .center { margin-top: -6px; }
  .new-group { display: flex; gap: 10px; align-items: center; }
  .new-group .field { flex: 1; }
</style>
