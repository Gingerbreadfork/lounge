<script lang="ts">
  // A tile's menu on the phone: the same choices as holding OK on the TV, each with a line on
  // what it does. It opens over the page and leaves on a tap outside, Escape or the Back button.
  import { api, art, type Layout, type Tile } from './api';
  import Icon from './Icon.svelte';
  import BottomSheet from './BottomSheet.svelte';
  import { t } from './lang.svelte';
  import { act, app, isControl, openOnTv, panelOf, press, quiet } from './state.svelte';
  import { words } from './words';

  let {
    tile,
    rowId,
    rowTitle,
    onclose,
  }: { tile: Tile; rowId: string | null; rowTitle: string | null; onclose: () => void } = $props();

  let layout = $state<Layout | null>(null);
  let busy = $state(false);
  let shell: BottomSheet;
  // The sheet's first choice takes focus as it opens.
  const focus = (el: HTMLElement) => el.focus();
  // Pinned state comes with the tile; the layout is read so a change can be written back whole.
  quiet(() => api.layout()).then((l) => (layout = l ?? null));
  const close = () => shell.close();

  const isApp = $derived(tile.id.startsWith('app:'));
  // A notice has nothing to pin or hide; it can only be told not to come back.
  const isNotice = $derived(tile.id.startsWith('notice:'));
  // A tile to read has nothing to open, on the TV or from the Stage.
  const opens = $derived(tile.action !== '');
  const pkg = $derived(tile.id.replace(/^app:/, ''));
  const rowShown = $derived(layout?.availableRows.find((r) => r.id === rowId)?.shown ?? true);

  // A layout that could not be read as the sheet opened is read again when a choice needs it.
  async function change(label: string, patch: (l: Layout) => Partial<Layout>) {
    if (busy) return;
    busy = true;
    const current = layout ?? (layout = (await act('', () => api.layout())) ?? null);
    const done = current && (await act(label, () => api.setLayout(patch(current))));
    busy = false;
    if (done) close();
  }
  // A control is pressed as a tap presses it; anything else opens on the TV, as the row says.
  function open() {
    if (isControl(tile)) press(tile);
    else openOnTv(tile);
    close();
  }
  function toggleStage() {
    if (tile.stagePinned) change(t('Unpinned from the Stage'), (l) => ({ stagePins: l.stagePins.filter((p) => p !== tile.id) }));
    // The Stage has six places, so the oldest pin makes room.
    else change(t('Pinned to the Stage'), (l) => ({ stagePins: [...l.stagePins.filter((p) => p !== tile.id), tile.id].slice(-6) }));
  }
  function toggleApp() {
    if (tile.pinned) change(t('Unpinned from Your apps'), (l) => ({ pinnedApps: l.pinnedApps.filter((p) => p !== pkg) }));
    else change(t('Pinned to the front of Your apps'), (l) => ({ pinnedApps: [...l.pinnedApps.filter((p) => p !== pkg), pkg] }));
  }
  function hide() {
    change(t('Hidden from every row'), (l) => ({ hidden: [...l.hidden.filter((h) => h !== tile.id), tile.id] }));
  }
  function hideAllFrom() {
    const from = tile.provider;
    if (!from) return;
    // Translators: {source} is the app or plugin whose titles are hidden from Continue watching
    change(t('Everything from {source} hidden', { source: tile.providerName ?? '' }), (l) => ({ mutedProviders: [...l.mutedProviders.filter((p) => p !== from), from] }));
  }
  function dismiss() {
    change(t('It will not be shown again'), (l) => ({ dismissedNotices: [...l.dismissedNotices.filter((n) => n !== tile.id), tile.id] }));
  }
  function hideRow() {
    if (!rowId) return;
    // Translators: {row} is a home screen row's title, such as "Continue watching"
    change(t('{row} row hidden', { row: rowTitle ?? '' }), (l) => ({ rowVisibility: { ...l.rowVisibility, [rowId]: false } }));
  }
  // A control's own panel opens here on the phone, in this sheet's place.
  const panel = $derived(panelOf(tile.more));
  function adjust() {
    const p = panel;
    close();
    app.panel = p;
  }
</script>

<BottomSheet bind:this={shell} title={tile.title} subtitle={tile.subtitle} {onclose}>
  {#snippet lead()}
    {#if tile.iconPath}<img class="sheet-icon" src={art(tile.iconPath)} alt="" />{:else if tile.artworkPath}<img class="sheet-art" src={art(tile.artworkPath)} alt="" />{/if}
  {/snippet}
  {#if isNotice}
    <div class="list">
      <button class="list-row" {@attach focus} disabled={busy} onclick={dismiss}>
        <span class="glyph"><Icon name="hide" size={24} /></span>
        <span class="grow title">{t("Don't show this again")}</span>
      </button>
    </div>
  {:else}
  <div class="list">
    {#if panel}
      <button class="list-row" {@attach focus} onclick={adjust}>
        <span class="glyph"><Icon name="adjust" size={24} /></span>
        <span class="grow"><span class="title">{words.adjust}</span><span class="caption">{words.adjustDetail}</span></span>
      </button>
    {/if}
    {#if opens}
      <!-- Translators: menu choices; "Press" works a control such as a light switch, as a tap on it does -->
      <button class="list-row" {@attach !panel && focus} onclick={open}><span class="glyph"><Icon name="open" size={22} /></span><span class="grow title">{isControl(tile) ? t('Press') : t('Open on the TV')}</span></button>
    {/if}
    {#if (opens && !tile.glyph) || tile.stagePinned}
      <button class="list-row" disabled={busy} onclick={toggleStage}>
        <span class="glyph" class:on={tile.stagePinned}><Icon name="pin" size={24} /></span>
        <span class="grow"><span class="title">{tile.stagePinned ? words.unpinStage : words.pinStage}</span><span class="caption">{tile.stagePinned ? words.unpinStageDetail : words.pinStageDetail}</span></span>
      </button>
    {/if}
    {#if isApp}
      <button class="list-row" disabled={busy} onclick={toggleApp}>
        <span class="glyph" class:on={tile.pinned}><Icon name="pin" size={24} /></span>
        <span class="grow"><span class="title">{tile.pinned ? words.unpinApp : words.pinApp}</span><span class="caption">{tile.pinned ? words.unpinAppDetail : words.pinAppDetail}</span></span>
      </button>
    {/if}
    <button class="list-row" {@attach !opens && !panel && focus} disabled={busy} onclick={hide}>
      <span class="glyph"><Icon name="hide" size={24} /></span>
      <span class="grow"><span class="title">{words.hide(tile.title)}</span><span class="caption">{words.hideDetail}</span></span>
    </button>
    {#if tile.provider && tile.providerName}
      <button class="list-row" disabled={busy} onclick={hideAllFrom}>
        <span class="glyph"><Icon name="hide" size={24} /></span>
        <!-- Translators: a menu choice; {source} is the app or plugin a title comes from -->
        <span class="grow"><span class="title">{t('Hide everything from {source}', { source: tile.providerName })}</span><span class="caption">{words.hideAllFromDetail}</span></span>
      </button>
    {/if}
    {#if rowId && rowTitle && rowShown}
      <button class="list-row" disabled={busy} onclick={hideRow}>
        <span class="glyph"><Icon name="rows" size={24} /></span>
        <span class="grow"><span class="title">{words.hideRow}</span><span class="caption">{words.hideRowDetail}</span></span>
      </button>
    {/if}
  </div>
  {/if}
</BottomSheet>

<style>
  .sheet-icon { width: 44px; height: 44px; border-radius: 11px; object-fit: cover; flex: none; }
  .sheet-art { width: 72px; height: 40px; border-radius: 8px; object-fit: cover; flex: none; }
  .grow { flex: 1; min-width: 0; display: flex; flex-direction: column; gap: 1px; }
  .glyph { width: 32px; height: 32px; display: grid; place-items: center; color: var(--dust); flex: none; }
  .glyph.on { color: var(--light); }
</style>
