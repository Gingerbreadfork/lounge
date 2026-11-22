<script lang="ts">
  // Every app on the TV, to open from the phone. The home shows only the pinned and the recent.
  import { api, art, type AppRef } from '../lib/api';
  import Icon from '../lib/Icon.svelte';
  import { language, t } from '../lib/lang.svelte';
  import { act, app, goBack, openingOnTv, quiet } from '../lib/state.svelte';

  let apps = $state<AppRef[] | null>(null);
  let unread = $state(false);
  let query = $state('');
  let opening = $state<string | null>(null);

  quiet(() => api.layout()).then((l) => {
    if (l) apps = [...l.apps].sort((a, b) => a.title.localeCompare(b.title, language()));
    else unread = true;
  });

  const found = $derived.by(() => {
    const q = query.trim().toLowerCase();
    return (apps ?? []).filter((a) => !q || a.title.toLowerCase().includes(q));
  });

  async function open(a: AppRef) {
    if (opening) return;
    opening = a.package;
    await act(openingOnTv(a.title), () => api.launch(a.package));
    opening = null;
  }
</script>

<main class="page">
  <header class="page-head">
    <div>
      <button class="btn ghost back" onclick={() => goBack('home')}>← {t('Home')}</button>
      <h1>{t('All apps')}</h1>
      <p class="meta">{app.state?.device.name ?? t('Your TV')}</p>
    </div>
  </header>

  {#if apps}
    <div class="find">
      <Icon name="search" size={20} />
      <input type="search" placeholder={t('Find an app')} bind:value={query} aria-label={t('Find an app')} autocomplete="off" autocapitalize="off" enterkeyhint="search" />
    </div>
    {#if found.length}
      <div class="grid">
        {#each found as a (a.package)}
          <button class="app" class:opening={opening === a.package} onclick={() => open(a)} aria-label={t('Open {title} on the TV', { title: a.title })}>
            {#if a.iconPath}<img src={art(a.iconPath)} alt="" loading="lazy" />{:else}<span class="blank" aria-hidden="true"><Icon name="apps" size={26} /></span>{/if}
            <span class="name">{a.title}</span>
          </button>
        {/each}
      </div>
    {:else}
      <p class="empty">{apps.length ? t('No app is called “{query}”.', { query: query.trim() }) : t('The TV has no apps to open yet.')}</p>
    {/if}
  {:else}
    <p class="empty">{unread ? t("The TV isn't answering, so its apps can't be listed yet.") : t('Reading the apps from the TV…')}</p>
  {/if}
</main>

<style>
  .back { padding: 0; min-height: 36px; }
  .find { display: flex; align-items: center; gap: 10px; padding: 0 14px; min-height: 50px; border-radius: var(--radius); background: var(--curtain); border: 1px solid var(--curtain-edge); color: var(--dust); }
  .find:focus-within { border-color: var(--light); }
  .find input { flex: 1; min-width: 0; background: none; border: 0; outline: none; color: var(--limelight); min-height: 48px; }
  .grid { display: grid; grid-template-columns: repeat(auto-fill, minmax(92px, 1fr)); gap: 14px 10px; }
  .app { display: flex; flex-direction: column; align-items: center; gap: 8px; padding: 8px 4px; border-radius: 16px; color: var(--limelight); transition: background 120ms ease-out, transform 90ms ease-out; }
  .app:active { background: var(--curtain); transform: scale(0.97); }
  .app.opening { opacity: 0.6; }
  .app img, .blank { width: 100%; aspect-ratio: 16 / 9; border-radius: 10px; object-fit: cover; background: var(--curtain); box-shadow: inset 0 0 0 1px var(--curtain-edge); }
  .blank { display: grid; place-items: center; color: var(--dust); }
  .name { font-size: 13px; line-height: 1.25; text-align: center; display: -webkit-box; -webkit-line-clamp: 2; line-clamp: 2; -webkit-box-orient: vertical; overflow: hidden; }
</style>
