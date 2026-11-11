<script lang="ts">
  import { AWAY, light, onLight } from './lib/api';
  import { app, go } from './lib/state.svelte';
  import Home from './pages/Home.svelte';
  import Remote from './pages/Remote.svelte';
  import Layout from './pages/Layout.svelte';
  import Plugins from './pages/Plugins.svelte';
  import More from './pages/More.svelte';
  import Pair from './pages/Pair.svelte';

  const tabs = [
    { page: 'home', label: 'Home', d: 'M4 11 12 4l8 7v9H4z M10 20v-6h4v6' },
    { page: 'remote', label: 'Remote', d: 'M8 3h8a2 2 0 0 1 2 2v14a2 2 0 0 1-2 2H8a2 2 0 0 1-2-2V5a2 2 0 0 1 2-2z M12 9v6 M9 12h6' },
    { page: 'layout', label: 'Layout', d: 'M4 5h16v4H4z M4 11h16v4H4z M4 17h16v2H4z' },
    { page: 'plugins', label: 'Plugins', d: 'M9 4h6v3a2 2 0 0 0 2 2h3v6h-3a2 2 0 0 0-2 2v3H9v-3a2 2 0 0 0-2-2H4V9h3a2 2 0 0 0 2-2z' },
    { page: 'more', label: 'More', d: 'M5 12h.01 M12 12h.01 M19 12h.01' },
  ] as const;

  // The light follows what is on the TV: the spotlight's palette tints the companion's accent,
  // and without one the room's own lamp stands in.
  $effect(() => {
    const sp = app.state?.snapshot?.stage.spotlights[0];
    const root = document.documentElement.style;
    if (sp) {
      const l = light(sp.lightArgb);
      root.setProperty('--light', l);
      root.setProperty('--on-light', onLight(l));
    } else {
      root.removeProperty('--light');
      root.removeProperty('--on-light');
    }
  });
</script>

{#if app.route.page === 'pair' || !app.paired}
  <Pair />
{:else}
  <div class="away-region" role="status">
    {#if app.away}<p class="away">{AWAY}</p>{/if}
  </div>

  {#if app.route.page === 'home'}<Home />
  {:else if app.route.page === 'remote'}<Remote />
  {:else if app.route.page === 'layout'}<Layout />
  {:else if app.route.page === 'plugins'}<Plugins id={app.route.id} />
  {:else}<More />{/if}

  <nav class="tabs" aria-label="Sections">
    {#each tabs as t (t.page)}
      <button class="tab" class:active={app.route.page === t.page} onclick={() => go(t.page, undefined, { replace: true })} aria-current={app.route.page === t.page ? 'page' : undefined}>
        <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d={t.d} /></svg>
        {t.label}
      </button>
    {/each}
  </nav>
{/if}

<div class="toast-region" role="status">
  {#if app.toast}<div class="toast">{app.toast}</div>{/if}
</div>

<style>
  .away-region { position: fixed; top: calc(env(safe-area-inset-top) + 8px); left: var(--side-l); right: var(--side-r); z-index: 25; pointer-events: none; }
  .away { background: var(--curtain); border: 1px solid var(--curtain-edge); border-left: 3px solid var(--houselight); border-radius: 14px; padding: 12px 16px; font-weight: 600; box-shadow: 0 12px 32px -16px rgb(0 0 0 / 0.8); }
</style>
