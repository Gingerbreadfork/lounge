<script lang="ts">
  // A plugin's panel on the phone: a room, a thermostat, a light's own settings. It is the sheet
  // the TV opens at its side, with the same rows, pressed here without touching the TV.
  import { api, type Panel, type PanelRow } from './api';
  import BottomSheet from './BottomSheet.svelte';
  import Icon, { drawn } from './Icon.svelte';
  import { layer, type Opened } from './layers';
  import { act, quiet, show, toast } from './state.svelte';

  let { plugin, id, onclose }: { plugin: string; id: string; onclose: () => void } = $props();

  /** How often the panel is read again: often for a minute after a tap, then now and then. */
  const SOON_MS = 3000;
  const LATER_MS = 15000;
  const SOON_FOR_MS = 60000;
  /** How long a confirm row keeps its question before it takes it back. */
  const ASK_FOR_MS = 8000;

  // The panels opened one from another; the last is the one showing, and Back goes up one.
  let trail = $state<string[]>([]);
  let steps: Opened[] = [];
  const current = $derived(trail[trail.length - 1] ?? id);
  let panel = $state<Panel | null>(null);
  let failed = $state<string | null>(null);
  let busy = $state<string | null>(null);
  let asking = $state<string | null>(null);
  let askTimer: ReturnType<typeof setTimeout> | undefined;
  let tappedAt = Date.now();

  async function read(of: string) {
    const fresh = await quiet(() => api.panel(plugin, of));
    if (of !== current) return;
    if (fresh) {
      panel = fresh;
      failed = null;
    } else if (!panel && !document.hidden) {
      failed = "The TV couldn't read this from the plugin. Close it and try again.";
    }
  }

  // The panel showing is read as it opens, and again as long as it stays open.
  $effect(() => {
    const of = current;
    panel = null;
    failed = null;
    asking = null;
    let stopped = false;
    let timer: ReturnType<typeof setTimeout>;
    const again = () => {
      if (stopped) return;
      timer = setTimeout(async () => {
        await read(of);
        again();
      }, Date.now() - tappedAt < SOON_FOR_MS ? SOON_MS : LATER_MS);
    };
    read(of).then(again);
    // A read still on its way when the panel goes must not start the clock again.
    return () => {
      stopped = true;
      clearTimeout(timer);
    };
  });

  $effect(() => () => {
    clearTimeout(askTimer);
    steps.forEach((s) => s.leave());
  });

  function open(sheet: string) {
    trail.push(sheet);
    steps.push(layer(() => { steps.pop(); trail.pop(); }));
  }
  function up() {
    steps.pop()?.leave();
    trail.pop();
  }
  // The sheet answers Escape while it is on top; over it, a panel opened from another goes up
  // one, and marks the key as taken so the sheet under it stays.
  function onkeydown(e: KeyboardEvent) {
    if (e.key !== 'Escape' || e.defaultPrevented || !steps[steps.length - 1]?.top()) return;
    e.preventDefault();
    up();
  }

  function ask(row: PanelRow) {
    asking = row.id;
    clearTimeout(askTimer);
    askTimer = setTimeout(() => (asking = null), ASK_FOR_MS);
  }

  async function press(row: PanelRow, which: 'press' | 'down' | 'up') {
    if (busy) return;
    tappedAt = Date.now();
    if (row.kind === 'confirm' && asking !== row.id) return ask(row);
    clearTimeout(askTimer);
    asking = null;
    const of = current;
    busy = row.id;
    const pressed = await act('', () => api.panelPress(plugin, of, row.id, which));
    busy = null;
    if (!pressed) return;
    if (pressed.say) toast(pressed.say);
    if (pressed.sheet && pressed.sheet.plugin === plugin) open(pressed.sheet.id);
    else show(pressed);
    // The thing answers a moment after it is told, and some answer late.
    setTimeout(() => read(of), 400);
    setTimeout(() => read(of), 2400);
  }

  /** What a screen reader says for a row: its title, then how things stand. */
  function describe(row: PanelRow) {
    return [row.title, row.subtitle, row.value].filter(Boolean).join(', ');
  }
</script>

<svelte:window {onkeydown} />

<BottomSheet title={panel?.title ?? 'One moment'} subtitle={panel?.subtitle} {onclose}>
  {#snippet lead()}
    {#if trail.length > 0}<button class="icon-btn" onclick={up} aria-label="Back"><Icon name="left" size={22} /></button>{/if}
  {/snippet}
  {#if failed}<p class="caption note">{failed}</p>{/if}
  {#if panel}
    <div class="list" aria-live="polite">
      {#each panel.rows as row (row.id)}
        {#if row.kind === 'group'}
          <p class="caption group">{row.title}</p>
        {:else if row.kind === 'step'}
          <div class="list-row step" class:busy={busy === row.id}>
            {#if row.press}
              <button class="body" onclick={() => press(row, 'press')} aria-label={describe(row)}>
                {@render glyph(row)}
                {@render words(row, row.title)}
              </button>
            {:else}
              <div class="body">
                {@render glyph(row)}
                {@render words(row, row.title)}
              </div>
            {/if}
            <button class="icon-btn" disabled={!row.down} onclick={() => press(row, 'down')} aria-label="{row.title}, less"><Icon name="minus" size={22} /></button>
            <span class="value amount">{row.value ?? ''}</span>
            <button class="icon-btn" disabled={!row.up} onclick={() => press(row, 'up')} aria-label="{row.title}, more"><Icon name="plus" size={22} /></button>
          </div>
        {:else if row.kind === 'open'}
          <button class="list-row" onclick={() => { tappedAt = Date.now(); if (row.sheet) open(row.sheet); }} aria-label={describe(row)}>
            {@render glyph(row)}
            {@render words(row, row.title)}
            {#if row.value}<span class="value">{row.value}</span>{/if}
            <span class="more"><Icon name="chevron" size={20} /></span>
          </button>
        {:else if row.kind === 'press' || row.kind === 'confirm'}
          <button class="list-row" class:busy={busy === row.id} class:asking={asking === row.id} onclick={() => press(row, 'press')} aria-label={asking === row.id ? row.ask : describe(row)}>
            {@render glyph(row)}
            {@render words(row, asking === row.id ? (row.ask ?? row.title) : row.title)}
            {#if row.value && asking !== row.id}<span class="value">{row.value}</span>{/if}
          </button>
        {:else}
          <div class="list-row read">
            {@render glyph(row)}
            {@render words(row, row.title)}
            {#if row.value}<span class="value">{row.value}</span>{/if}
          </div>
        {/if}
      {/each}
    </div>
    {#if panel.footnote}<p class="caption note">{panel.footnote}</p>{/if}
  {:else if !failed}
    <p class="caption note">Reading it from the TV…</p>
  {/if}
</BottomSheet>

{#snippet glyph(row: PanelRow)}
  {#if row.glyph}
    <span class="glyph" class:on={row.on === true} aria-hidden="true"><Icon name={drawn(row.glyph) ? row.glyph : 'thing'} size={24} /></span>
  {/if}
{/snippet}

{#snippet words(row: PanelRow, title: string)}
  <span class="grow">
    <span class="title">{title}</span>
    {#if row.subtitle}<span class="caption">{row.subtitle}</span>{/if}
  </span>
{/snippet}

<style>
  .group { margin: 8px 4px 0; color: var(--dust); }
  .note { color: var(--dust); padding: 0 4px; }
  .grow { flex: 1; min-width: 0; display: flex; flex-direction: column; gap: 1px; text-align: left; }
  .glyph { width: 32px; height: 32px; display: grid; place-items: center; color: var(--dust); flex: none; }
  .glyph.on { color: var(--light); }
  .more { color: var(--dust); display: grid; place-items: center; flex: none; }
  .busy { opacity: 0.6; }
  .asking { box-shadow: inset 0 0 0 2px var(--ember); }
  .asking .title { color: var(--ember); }
  .read { color: var(--limelight); }
  .step { gap: 8px; padding-right: 10px; }
  .step .body { flex: 1; min-width: 0; display: flex; align-items: center; gap: 14px; color: var(--limelight); min-height: 44px; }
  .step .icon-btn { background: var(--curtain); }
  .amount { min-width: 44px; text-align: center; color: var(--limelight); font-weight: 600; }
</style>
