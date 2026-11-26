<script lang="ts">
  // The TV's own settings, in the groups and the words Settings on the TV has them. The TV lists
  // them with what each can be set to, so a setting it gains later shows here by itself.
  import { api, explain, type TvSetting, type TvSettings } from '../lib/api';
  import { t } from '../lib/lang.svelte';
  import { app, goBack, quiet, toast, unpair } from '../lib/state.svelte';

  let settings = $state<TvSettings | null>(null);
  let unread = $state(false);
  let saving = $state<string | null>(null);

  async function read() {
    const fresh = await quiet(() => api.settings());
    if (fresh) settings = fresh;
    else if (!settings) unread = true;
  }
  read();

  async function set(s: TvSetting, value: string) {
    if (saving || value === s.value) return;
    const before = s.value;
    saving = s.key;
    // Shown at once, and put back if the TV says no.
    s.value = value;
    try {
      await api.setSetting(s.key, value);
      toast(t('Saved'));
    } catch (e: any) {
      if (e?.status === 401) return unpair();
      s.value = before;
      toast(explain(e));
    } finally {
      saving = null;
    }
    // One setting can change what another says, so the TV is asked again.
    read();
  }
</script>

<main class="page">
  <header class="page-head">
    <div>
      <button class="btn ghost back" onclick={() => goBack('more')}>← {t('More')}</button>
      <h1>{t('TV settings')}</h1>
      <p class="meta">{app.state?.device.name ?? t('Your TV')}</p>
    </div>
  </header>

  {#if settings}
    {#each settings.groups as group (group.title)}
      <section class="section">
        <div class="section-head"><h3>{group.title}</h3></div>
        {#each group.settings as s (s.key)}
          {#if s.switch}
            {@const on = s.value === 'true'}
            <button class="list-row" role="switch" aria-checked={on} disabled={saving !== null} onclick={() => set(s, on ? 'false' : 'true')}>
              <span class="grow">
                <span class="title">{s.title}</span>
                {#if s.about}<span class="caption about">{s.about}</span>{/if}
              </span>
              <span class="toggle" class:on></span>
            </button>
          {:else}
            <div class="field">
              <label for={s.key}>{s.title}</label>
              <select id={s.key} value={s.value} disabled={saving !== null} onchange={(e) => set(s, (e.target as HTMLSelectElement).value)}>
                {#each s.choices as c (c.value)}<option value={c.value}>{c.label}</option>{/each}
              </select>
              {#if s.about}<p class="caption">{s.about}</p>{/if}
            </div>
          {/if}
        {/each}
      </section>
    {/each}
    <p class="caption">{t('Rows, groups of apps and what is hidden are under Layout.')}</p>
  {:else}
    <!-- Translators: on the TV settings page; "this" is the page and "them" are the TV's settings -->
    <p class="empty">{unread ? t("The TV isn't answering. Check it is on, then open this again.") : t('Reading them from the TV…')}</p>
  {/if}
</main>

<style>
  .back { padding-left: 0; }
  .section { display: flex; flex-direction: column; gap: 10px; }
  .grow { flex: 1; min-width: 0; display: flex; flex-direction: column; gap: 2px; }
  .list-row .title { white-space: normal; }
  .about { white-space: normal; }
</style>
