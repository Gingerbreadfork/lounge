<script lang="ts">
  // A sheet that rises over the page. It leaves on a tap outside, Escape or the Back button, and
  // Tab stays among its own buttons while it is open.
  import type { Snippet } from 'svelte';
  import Icon from './Icon.svelte';
  import { layer, type Opened } from './layers';

  let {
    title,
    subtitle = null,
    lead,
    onclose,
    children,
  }: { title: string; subtitle?: string | null; lead?: Snippet; onclose: () => void; children: Snippet } = $props();

  let sheet: HTMLDivElement;
  // Focus goes back to whatever had it before the sheet opened.
  const opener = document.activeElement instanceof HTMLElement ? document.activeElement : null;
  let opened: Opened | null = null;
  $effect(() => {
    opened = layer(() => {
      opened = null;
      onclose();
    });
    if (!sheet.contains(document.activeElement)) sheet.focus({ preventScroll: true });
    return () => {
      opened?.leave();
      opened = null;
      if (opener?.isConnected) opener.focus({ preventScroll: true });
    };
  });

  /** Closes the sheet, whatever is open over it. */
  export function close() {
    opened?.leave();
    opened = null;
    onclose();
  }

  function onkeydown(e: KeyboardEvent) {
    // A layer open over the sheet has the keys while it is there, and so does one that has
    // just taken this key to close itself.
    if (!opened?.top() || e.defaultPrevented) return;
    if (e.key === 'Escape') close();
    if (e.key !== 'Tab') return;
    const choices = [...sheet.querySelectorAll<HTMLElement>('button:not(:disabled), a[href]')];
    if (choices.length === 0) return;
    const [first, last] = [choices[0], choices[choices.length - 1]];
    const inside = sheet.contains(document.activeElement) && document.activeElement !== sheet;
    if (e.shiftKey && (document.activeElement === first || !inside)) { e.preventDefault(); last.focus(); }
    else if (!e.shiftKey && (document.activeElement === last || !inside)) { e.preventDefault(); first.focus(); }
  }
  // A tap that began on the scrim closes the sheet; the release of the hold that opened it does not.
  let scrimPress = false;
</script>

<svelte:window {onkeydown} />

<div class="scrim" role="presentation" onpointerdown={() => (scrimPress = true)} onclick={() => { if (scrimPress) close(); scrimPress = false; }}></div>
<div class="sheet" role="dialog" aria-modal="true" aria-label={title} tabindex="-1" bind:this={sheet}>
  <div class="sheet-head">
    {@render lead?.()}
    <div class="grow">
      <div class="title">{title}</div>
      {#if subtitle}<div class="caption">{subtitle}</div>{/if}
    </div>
    <button class="icon-btn" onclick={close} aria-label="Close"><Icon name="close" size={22} /></button>
  </div>
  {@render children()}
</div>

<style>
  .scrim { position: fixed; inset: 0; background: color-mix(in srgb, var(--velvet) 60%, transparent); z-index: 20; animation: fade 160ms ease-out; }
  .sheet { position: fixed; left: 0; right: 0; bottom: 0; z-index: 21; background: var(--curtain); border-radius: var(--radius-l) var(--radius-l) 0 0; border-top: 1px solid var(--curtain-edge); padding: 14px var(--side-r) calc(env(safe-area-inset-bottom) + 16px) var(--side-l); display: flex; flex-direction: column; gap: 14px; animation: rise 220ms cubic-bezier(0.2, 0.8, 0.2, 1); max-height: 85vh; overflow-y: auto; outline: none; }
  .sheet-head { display: flex; align-items: center; gap: 12px; }
  .sheet-head .title { font-weight: 600; font-size: 17px; white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
  .grow { flex: 1; min-width: 0; display: flex; flex-direction: column; gap: 1px; }
  .sheet :global(.list-row) { background: var(--velvet); }
  .sheet :global(.list-row .title), .sheet :global(.list-row .caption) { white-space: normal; }
  .sheet :global(.list-row:disabled) { opacity: 0.5; }
  .sheet :global(.list-row:focus-visible), .sheet :global(.icon-btn:focus-visible) { outline: 2px solid var(--light); outline-offset: -2px; }
  @keyframes rise { from { transform: translateY(24px); opacity: 0; } to { transform: none; opacity: 1; } }
  @keyframes fade { from { opacity: 0; } to { opacity: 1; } }
</style>
