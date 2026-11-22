<script lang="ts">
  // A plugin's still picture, full screen and fetched again about once a second, as the TV shows
  // it. It rests after five minutes without a tap, and a new address is asked for when the old
  // one stops answering.
  import { api, type Live } from './api';
  import Icon from './Icon.svelte';
  import { t } from './lang.svelte';
  import { layer, type Opened } from './layers';
  import { done, unpair } from './state.svelte';

  let { live, onclose }: { live: Live; onclose: () => void } = $props();

  const EVERY_MS = 1000;
  const RESTS_AFTER_MS = 5 * 60000;

  let frame = $state<string | null>(null);
  let misses = $state(0);
  let resting = $state(false);
  let tappedAt = Date.now();
  let wake: (() => void) | null = null;
  let view: HTMLDivElement;
  /** Where the picture is now; it moves when the first address stops answering. */
  let address = '';

  const note = $derived(
    resting ? t('Resting. Tap to carry on.')
    : misses >= 3 ? (frame ? t('The picture has stopped arriving. Still trying.') : t("The picture isn't arriving. Still trying."))
    : frame ? null : t('One moment'),
  );

  let opened: Opened | null = null;
  $effect(() => {
    opened = layer(() => {
      opened = null;
      onclose();
    });
    view.focus({ preventScroll: true });
    return () => {
      opened?.leave();
      opened = null;
    };
  });
  function close() {
    opened?.leave();
    opened = null;
    onclose();
  }

  $effect(() => {
    address = live.url;
    let stopped = false;
    let timer: ReturnType<typeof setTimeout>;
    const pause = (ms: number) => new Promise<void>((r) => { timer = setTimeout(r, ms); });
    (async () => {
      while (!stopped) {
        if (Date.now() - tappedAt >= RESTS_AFTER_MS) {
          resting = true;
          await new Promise<void>((r) => { wake = r; });
          resting = false;
          continue;
        }
        if (document.hidden) {
          await pause(EVERY_MS);
          continue;
        }
        const started = Date.now();
        try {
          const blob = await api.livePicture({ ...live, url: address });
          if (stopped) return;
          const old = frame;
          frame = URL.createObjectURL(blob);
          if (old) URL.revokeObjectURL(old);
          misses = 0;
        } catch (e: any) {
          if (stopped) return;
          if (e?.status === 401) return unpair();
          misses++;
          // An address is good for a few minutes; the plugin gives the one that works now.
          try { address = (await api.liveAgain({ ...live, url: address })).url; } catch { /* tried again below */ }
        }
        const wait = misses > 0 ? Math.min(EVERY_MS * misses, 8000) : EVERY_MS;
        await pause(Math.max(100, wait - (Date.now() - started)));
      }
    })();
    return () => {
      stopped = true;
      clearTimeout(timer);
      wake?.();
      if (frame) URL.revokeObjectURL(frame);
    };
  });

  function tap() {
    tappedAt = Date.now();
    wake?.();
    wake = null;
  }
  function onkeydown(e: KeyboardEvent) {
    if (e.key !== 'Escape' || e.defaultPrevented || !opened?.top()) return;
    e.preventDefault();
    close();
  }
  function onTv() {
    tap();
    // Translators: {title} is the name of a live picture, such as a camera
    done(t('{title} is on the TV', { title: live.title }), () => api.liveOnTv({ ...live, url: address }));
  }
</script>

<svelte:window {onkeydown} />

<div class="live" role="dialog" aria-modal="true" aria-label={live.title} tabindex="-1" bind:this={view} onpointerdown={tap}>
  {#if frame}<img src={frame} alt="" />{/if}
  <div class="top">
    <button class="icon-btn" onclick={close} aria-label={t('Close')}><Icon name="close" size={22} /></button>
  </div>
  <div class="foot">
    <div class="grow">
      <p class="title">{live.title}</p>
      <p class="caption" aria-live="polite">{note ?? ''}</p>
    </div>
    <!-- Translators: a button that puts this live picture on the TV's screen -->
    <button class="btn" onclick={onTv}><Icon name="tv" size={20} />{t('On the TV')}</button>
  </div>
</div>

<style>
  .live { position: fixed; inset: 0; z-index: 28; background: #000; display: grid; place-items: center; outline: none; animation: fade 160ms ease-out; }
  .live img { max-width: 100%; max-height: 100%; width: 100%; height: 100%; object-fit: contain; }
  .top { position: absolute; top: calc(env(safe-area-inset-top) + 10px); right: var(--side-r); }
  .top .icon-btn { background: rgb(0 0 0 / 0.5); color: #fff; }
  .foot { position: absolute; left: 0; right: 0; bottom: 0; display: flex; align-items: flex-end; gap: 12px; padding: 72px var(--side-r) calc(env(safe-area-inset-bottom) + 18px) var(--side-l); background: linear-gradient(transparent, rgb(0 0 0 / 0.75)); }
  .grow { flex: 1; min-width: 0; }
  .title { font-family: var(--headline); font-size: 22px; font-weight: 600; color: #fff; white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
  .caption { color: rgb(255 255 255 / 0.7); min-height: 1.3em; }
  .btn { display: inline-flex; align-items: center; gap: 8px; flex: none; }
  @keyframes fade { from { opacity: 0; } to { opacity: 1; } }
</style>
