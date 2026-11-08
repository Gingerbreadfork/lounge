<script lang="ts" generics="T">
  // A list whose rows can be dragged into a new order by their handle, or moved with the arrow
  // keys when the handle has focus. The row follows the finger, the others slide out of its
  // way, and the new order is reported once, when the row is set down.
  import { tick, type Snippet } from 'svelte';
  import { flip } from 'svelte/animate';
  import Icon from './Icon.svelte';

  let {
    items,
    keyOf,
    nameOf,
    onmove,
    ondone,
    children,
  }: {
    items: T[];
    keyOf: (item: T) => string;
    nameOf: (item: T) => string;
    /** The item at `from` now belongs at `to`; the caller reorders `items` on screen. */
    onmove: (from: number, to: number) => void;
    /** The order has settled and is worth saving. */
    ondone: () => void;
    children: Snippet<[T, number]>;
  } = $props();

  let container: HTMLDivElement;
  let lifted = $state<string | null>(null);
  let offset = $state(0);
  let announcement = $state('');
  let fingerY = 0;
  let startFinger = 0;
  let startTop = 0;
  let scroller: number | undefined;
  let moved = false;

  $effect(() => () => window.clearInterval(scroller));

  function indexOf(key: string): number {
    return items.findIndex((it) => keyOf(it) === key);
  }
  function rowAt(i: number): HTMLElement | null {
    return (container?.children[i] as HTMLElement | undefined) ?? null;
  }
  // Layout positions, which a sliding neighbour's animation does not disturb.
  function middle(i: number): number {
    const el = rowAt(i);
    return el ? el.offsetTop + el.offsetHeight / 2 : 0;
  }
  function inList(clientY: number): number {
    return clientY - container.getBoundingClientRect().top;
  }

  function begin(e: PointerEvent, i: number) {
    if (e.button !== 0 || lifted !== null) return;
    const grip = e.currentTarget as HTMLElement;
    grip.setPointerCapture(e.pointerId);
    lifted = keyOf(items[i]);
    moved = false;
    startFinger = fingerY = inList(e.clientY);
    startTop = rowAt(i)?.offsetTop ?? 0;
    offset = 0;
    navigator.vibrate?.(8);
    scroller = window.setInterval(edgeScroll, 16);
  }

  async function dragged(e: PointerEvent) {
    if (lifted === null) return;
    fingerY = inList(e.clientY);
    await settle();
  }

  /** Puts the lifted row where the finger is and keeps it under the finger. */
  async function settle() {
    if (lifted === null) return;
    const from = indexOf(lifted);
    if (from < 0) return;
    let to = from;
    while (to > 0 && fingerY < middle(to - 1)) to--;
    while (to < items.length - 1 && fingerY > middle(to + 1)) to++;
    if (to !== from) {
      onmove(from, to);
      moved = true;
      navigator.vibrate?.(6);
      await tick();
    }
    const top = rowAt(indexOf(lifted))?.offsetTop ?? startTop;
    offset = fingerY - startFinger - (top - startTop);
  }

  function end() {
    if (lifted === null) return;
    window.clearInterval(scroller);
    scroller = undefined;
    const key = lifted;
    lifted = null;
    offset = 0;
    if (moved) {
      const i = indexOf(key);
      if (i >= 0) announcement = `${nameOf(items[i])} is now ${i + 1} of ${items.length}`;
      ondone();
    }
  }

  /** The page scrolls on its own when the finger nears the top or bottom of the screen. */
  function edgeScroll() {
    if (lifted === null || !container) return;
    const y = fingerY + container.getBoundingClientRect().top;
    const edge = 72;
    const step = y < edge ? -Math.ceil((edge - y) / 6) : y > innerHeight - edge ? Math.ceil((y - (innerHeight - edge)) / 6) : 0;
    if (step === 0) return;
    const before = scrollY;
    window.scrollBy(0, step);
    // The list moved under a still finger, so the finger moved within the list.
    fingerY += scrollY - before;
    settle();
  }

  /** One step up or down, from the handle's arrow keys or the hidden buttons for screen readers. */
  async function step(i: number, by: -1 | 1, refocus: string) {
    const to = i + by;
    if (to < 0 || to >= items.length) return;
    const key = keyOf(items[i]);
    onmove(i, to);
    await tick();
    announcement = `${nameOf(items[to])} is now ${to + 1} of ${items.length}`;
    ondone();
    container.querySelector<HTMLElement>(`[data-key="${CSS.escape(key)}"] ${refocus}`)?.focus();
  }
  function keys(e: KeyboardEvent, i: number) {
    const by = e.key === 'ArrowUp' ? -1 : e.key === 'ArrowDown' ? 1 : 0;
    if (!by) return;
    e.preventDefault();
    step(i, by, '.grip');
  }
</script>

<div class="reorder" class:lifting={lifted !== null} bind:this={container}>
  {#each items as item, i (keyOf(item))}
    <div
      class="list-row reorder-item"
      class:lifted={lifted === keyOf(item)}
      data-key={keyOf(item)}
      style:transform={lifted === keyOf(item) ? `translateY(${offset}px)` : undefined}
      animate:flip={{ duration: lifted === keyOf(item) ? 0 : 180 }}
    >
      <button
        class="grip"
        aria-label="Drag to move {nameOf(item)}, or use the arrow keys"
        onpointerdown={(e) => begin(e, i)}
        onpointermove={dragged}
        onpointerup={end}
        onpointercancel={end}
        onkeydown={(e) => keys(e, i)}
      >
        <Icon name="grip" size={22} />
      </button>
      <span class="sr-only">
        <button class="sr-move up" disabled={i === 0} onclick={() => step(i, -1, '.sr-move.up')}>Move {nameOf(item)} up</button>
        <button class="sr-move down" disabled={i === items.length - 1} onclick={() => step(i, 1, '.sr-move.down')}>Move {nameOf(item)} down</button>
      </span>
      {@render children(item, i)}
    </div>
  {/each}
</div>
<div class="sr-only" aria-live="polite">{announcement}</div>

<style>
  .reorder { position: relative; display: flex; flex-direction: column; gap: 8px; }
  .reorder-item { transition: box-shadow 160ms ease-out, background 160ms ease-out; }
  .reorder-item.lifted { position: relative; z-index: 2; background: var(--curtain-edge); box-shadow: 0 14px 32px -12px rgb(0 0 0 / 0.7), 0 0 0 1px color-mix(in srgb, var(--light) 40%, transparent); }
  .lifting { user-select: none; -webkit-user-select: none; }
  .grip { width: 36px; height: 44px; margin-left: -8px; display: grid; place-items: center; color: var(--dust); touch-action: none; cursor: grab; border-radius: 10px; flex: none; }
  .grip:active, .lifted .grip { color: var(--limelight); cursor: grabbing; }
  .grip:focus-visible { outline: 2px solid var(--light); outline-offset: -2px; }
  /* Kept for screen readers and the keyboard, out of sight for everyone else. */
  .sr-only { position: absolute; width: 1px; height: 1px; overflow: hidden; clip: rect(0 0 0 0); white-space: nowrap; }
  .sr-only:focus-within { position: static; width: auto; height: auto; clip: auto; display: flex; gap: 4px; }
  .sr-move { font-size: 12px; color: var(--dust); padding: 4px 8px; border-radius: 8px; background: var(--velvet); }
</style>
