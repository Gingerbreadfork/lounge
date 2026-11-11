<script lang="ts">
  // A tile's menu on the phone: the same choices as holding OK on the TV, each with a line on
  // what it does. It opens over the page and leaves on a tap outside, Escape or the Back button.
  import { api, art, type Layout, type Tile } from './api';
  import Icon from './Icon.svelte';
  import { act, quiet } from './state.svelte';
  import { words } from './words';

  let {
    tile,
    rowId,
    rowTitle,
    onclose,
  }: { tile: Tile; rowId: string | null; rowTitle: string | null; onclose: () => void } = $props();

  let layout = $state<Layout | null>(null);
  let busy = $state(false);
  let sheet: HTMLDivElement;
  // The sheet's first choice takes focus as it opens.
  const focus = (el: HTMLElement) => el.focus();
  // The sheet is one history entry, so Back closes it, and closing it any other way pops that
  // entry. Focus goes back to whatever had it before the sheet opened.
  let pushed = false;
  $effect(() => {
    const opener = document.activeElement instanceof HTMLElement ? document.activeElement : null;
    history.pushState({ tileSheet: true }, '');
    pushed = true;
    // Pinned state comes with the tile; the layout is read so a change can be written back whole.
    quiet(() => api.layout()).then((l) => (layout = l ?? null));
    return () => {
      if (pushed) {
        pushed = false;
        history.back();
      }
      if (opener?.isConnected) opener.focus({ preventScroll: true });
    };
  });
  function onpopstate() {
    if (!pushed) return;
    pushed = false;
    onclose();
  }
  function close() {
    if (pushed) history.back();
    else onclose();
  }

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
  function open() {
    act(`Opening ${tile.title} on the TV`, () => api.open(tile.action));
    close();
  }
  function toggleStage() {
    if (tile.stagePinned) change('Unpinned from the Stage', (l) => ({ stagePins: l.stagePins.filter((p) => p !== tile.id) }));
    // The Stage has six places, so the oldest pin makes room.
    else change('Pinned to the Stage', (l) => ({ stagePins: [...l.stagePins.filter((p) => p !== tile.id), tile.id].slice(-6) }));
  }
  function toggleApp() {
    if (tile.pinned) change('Unpinned from Your apps', (l) => ({ pinnedApps: l.pinnedApps.filter((p) => p !== pkg) }));
    else change('Pinned to the front of Your apps', (l) => ({ pinnedApps: [...l.pinnedApps.filter((p) => p !== pkg), pkg] }));
  }
  function hide() {
    change('Hidden from every row', (l) => ({ hidden: [...l.hidden.filter((h) => h !== tile.id), tile.id] }));
  }
  function hideAllFrom() {
    const from = tile.provider;
    if (!from) return;
    change(`Everything from ${tile.providerName} hidden`, (l) => ({ mutedProviders: [...l.mutedProviders.filter((p) => p !== from), from] }));
  }
  function dismiss() {
    change('It will not be shown again', (l) => ({ dismissedNotices: [...l.dismissedNotices.filter((n) => n !== tile.id), tile.id] }));
  }
  function hideRow() {
    if (!rowId) return;
    change(`${rowTitle} row hidden`, (l) => ({ rowVisibility: { ...l.rowVisibility, [rowId]: false } }));
  }
  function onkeydown(e: KeyboardEvent) {
    if (e.key === 'Escape') close();
    // Tab stays among the sheet's own choices while it is open.
    if (e.key !== 'Tab' || !sheet) return;
    const choices = [...sheet.querySelectorAll<HTMLElement>('button:not(:disabled)')];
    if (choices.length === 0) return;
    const [first, last] = [choices[0], choices[choices.length - 1]];
    const inside = sheet.contains(document.activeElement);
    if (e.shiftKey && (document.activeElement === first || !inside)) { e.preventDefault(); last.focus(); }
    else if (!e.shiftKey && (document.activeElement === last || !inside)) { e.preventDefault(); first.focus(); }
  }
  // A tap that began on the scrim closes the sheet; the release of the hold that opened it does not.
  let scrimPress = false;
</script>

<svelte:window {onkeydown} {onpopstate} />

<div class="scrim" role="presentation" onpointerdown={() => (scrimPress = true)} onclick={() => { if (scrimPress) close(); scrimPress = false; }}></div>
<div class="sheet" role="dialog" aria-modal="true" aria-label={tile.title} bind:this={sheet}>
  <div class="sheet-head">
    {#if tile.iconPath}<img class="sheet-icon" src={art(tile.iconPath)} alt="" />{:else if tile.artworkPath}<img class="sheet-art" src={art(tile.artworkPath)} alt="" />{/if}
    <div class="grow">
      <div class="title">{tile.title}</div>
      {#if tile.subtitle}<div class="caption">{tile.subtitle}</div>{/if}
    </div>
    <button class="icon-btn" onclick={close} aria-label="Close"><Icon name="close" size={22} /></button>
  </div>
  {#if isNotice}
    <div class="list">
      <button class="list-row" {@attach focus} disabled={busy} onclick={dismiss}>
        <span class="glyph"><Icon name="hide" size={24} /></span>
        <span class="grow title">Don't show this again</span>
      </button>
    </div>
  {:else}
  <div class="list">
    {#if opens}
      <button class="list-row" {@attach focus} onclick={open}><span class="glyph"><Icon name="open" size={22} /></span><span class="grow title">Open on the TV</span></button>
    {/if}
    {#if opens || tile.stagePinned}
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
    <button class="list-row" {@attach !opens && focus} disabled={busy} onclick={hide}>
      <span class="glyph"><Icon name="hide" size={24} /></span>
      <span class="grow"><span class="title">{words.hide} {tile.title}</span><span class="caption">{words.hideDetail}</span></span>
    </button>
    {#if tile.provider && tile.providerName}
      <button class="list-row" disabled={busy} onclick={hideAllFrom}>
        <span class="glyph"><Icon name="hide" size={24} /></span>
        <span class="grow"><span class="title">Hide everything from {tile.providerName}</span><span class="caption">{words.hideAllFromDetail}</span></span>
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
</div>

<style>
  .scrim { position: fixed; inset: 0; background: color-mix(in srgb, var(--velvet) 60%, transparent); z-index: 20; animation: fade 160ms ease-out; }
  .sheet { position: fixed; left: 0; right: 0; bottom: 0; z-index: 21; background: var(--curtain); border-radius: var(--radius-l) var(--radius-l) 0 0; border-top: 1px solid var(--curtain-edge); padding: 14px var(--side-r) calc(env(safe-area-inset-bottom) + 16px) var(--side-l); display: flex; flex-direction: column; gap: 14px; animation: rise 220ms cubic-bezier(0.2, 0.8, 0.2, 1); max-height: 85vh; overflow-y: auto; }
  .sheet-head { display: flex; align-items: center; gap: 12px; }
  .sheet-icon { width: 44px; height: 44px; border-radius: 11px; object-fit: cover; flex: none; }
  .sheet-art { width: 72px; height: 40px; border-radius: 8px; object-fit: cover; flex: none; }
  .sheet-head .title { font-weight: 600; font-size: 17px; white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
  .grow { flex: 1; min-width: 0; display: flex; flex-direction: column; gap: 1px; }
  .sheet .list-row { background: var(--velvet); }
  .sheet .list-row .title, .sheet .list-row .caption { white-space: normal; }
  .sheet .list-row:disabled { opacity: 0.5; }
  .sheet .list-row:focus-visible { outline: 2px solid var(--light); outline-offset: -2px; }
  .glyph { width: 32px; height: 32px; display: grid; place-items: center; color: var(--dust); flex: none; }
  .glyph.on { color: var(--light); }
  @keyframes rise { from { transform: translateY(24px); opacity: 0; } to { transform: none; opacity: 1; } }
  @keyframes fade { from { opacity: 0; } to { opacity: 1; } }
</style>
