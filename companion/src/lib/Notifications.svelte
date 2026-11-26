<script lang="ts">
  // What apps on the TV have posted, as the TV's own panel lists them, to read and dismiss from here.
  import { api, type TvNotification, type TvNotifications } from './api';
  import BottomSheet from './BottomSheet.svelte';
  import Icon from './Icon.svelte';
  import { t } from './lang.svelte';
  import { done, quiet } from './state.svelte';
  import { ago } from './words';

  let { onclose, onchange }: { onclose: () => void; onchange?: (count: number) => void } = $props();

  let notices = $state<TvNotifications | null>(null);
  let unread = $state(false);

  async function read() {
    const fresh = await quiet(() => api.notifications());
    if (fresh) {
      notices = fresh;
      onchange?.(fresh.on ? fresh.list.length : 0);
    } else if (!notices) unread = true;
  }
  $effect(() => {
    read();
    const timer = setInterval(read, 5000);
    return () => clearInterval(timer);
  });

  async function dismiss(n?: TvNotification) {
    if (await done(t('Dismissed on the TV'), () => api.dismissNotification(n?.key))) read();
  }
  const dismissible = $derived(!!notices?.list.some((n) => n.dismissible));
</script>

<BottomSheet title={t('Notifications')} subtitle={notices?.on && notices.list.length ? t('What apps on the TV want to tell you.') : null} {onclose}>
  {#if notices && !notices.on}
    <p class="caption note">{t("The TV isn't showing notifications. Turn them on in Settings on the TV.")}</p>
  {:else if notices && notices.list.length === 0}
    <p class="caption note">{t('Nothing on the TV to read.')}</p>
  {:else if notices}
    <div class="list">
      {#each notices.list as n (n.key)}
        <div class="list-row notice">
          <div class="grow">
            <!-- Translators: over a notification: the app it is from and how long ago, such as "Plex, 3 minutes ago" -->
            <div class="caption">{t('{app}, {when}', { app: n.app, when: ago(n.postedAt) })}</div>
            <div class="title">{n.title}</div>
            {#if n.text}<div class="text">{n.text}</div>{/if}
          </div>
          <!-- Translators: a button that removes one notification from the TV; {title} is the notification's title -->
          {#if n.dismissible}<button class="close" onclick={() => dismiss(n)} aria-label={t('Dismiss {title}', { title: n.title })}><Icon name="close" size={20} /></button>{/if}
        </div>
      {/each}
    </div>
    {#if dismissible}<div class="actions"><button class="btn" onclick={() => dismiss()}>{t('Dismiss all')}</button></div>{/if}
  {:else}
    <!-- Translators: in the notifications sheet; "this" is the sheet and "them" are the TV's notifications -->
    <p class="caption note">{unread ? t("The TV isn't answering. Check it is on, then open this again.") : t('Reading them from the TV…')}</p>
  {/if}
</BottomSheet>

<style>
  .note { color: var(--dust); padding: 0 4px; }
  .notice { align-items: flex-start; }
  .grow { flex: 1; min-width: 0; display: flex; flex-direction: column; gap: 2px; }
  .notice .title { white-space: normal; }
  .text { color: var(--dust); font-size: 14px; line-height: 1.4; white-space: pre-line; overflow-wrap: anywhere; }
  .close { width: 40px; height: 40px; border-radius: 50%; display: grid; place-items: center; color: var(--dust); flex: none; }
  .close:active { background: var(--curtain-edge); }
  .actions { display: flex; justify-content: flex-end; margin-top: 12px; }
</style>
