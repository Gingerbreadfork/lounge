<script lang="ts">
  // The TV's power and input, as the TV controls sheet on the TV has them. Each row is run by
  // the TV, and says beside itself how that went.
  import { api, explain, type TvControl, type TvControls } from './api';
  import BottomSheet from './BottomSheet.svelte';
  import Icon from './Icon.svelte';
  import { quiet, unpair } from './state.svelte';

  let { onclose }: { onclose: () => void } = $props();

  /** How long "Sent" stays beside a row. */
  const SENT_FOR_MS = 2500;

  type Run = { state: 'sending' | 'sent' } | { state: 'failed'; why: string };
  let tv = $state<TvControls | null>(null);
  let unread = $state(false);
  let runs = $state<Record<string, Run>>({});

  async function read() {
    const fresh = await quiet(() => api.tv());
    if (fresh) tv = fresh;
    else if (!tv) unread = true;
  }
  read();

  async function run(c: TvControl) {
    if (runs[c.id]?.state === 'sending') return;
    runs[c.id] = { state: 'sending' };
    try {
      await api.tvControl(c.id);
      const sent: Run = { state: 'sent' };
      runs[c.id] = sent;
      setTimeout(() => { if (runs[c.id]?.state === 'sent') delete runs[c.id]; }, SENT_FOR_MS);
      // Power and inputs read differently once the TV has done it.
      setTimeout(read, 1500);
    } catch (e: any) {
      if (e?.status === 401) return unpair();
      // A failure stays with its row until it is pressed again, so there is time to read it.
      runs[c.id] = { state: 'failed', why: explain(e) };
    }
  }

  function value(c: TvControl) {
    const r = runs[c.id];
    return r?.state === 'sending' ? 'Sending…' : r?.state === 'sent' ? 'Sent' : r?.state === 'failed' ? "Didn't work" : (c.value ?? '');
  }
  function why(c: TvControl) {
    const r = runs[c.id];
    return r?.state === 'failed' ? r.why : null;
  }
  const ready = $derived(!!tv && !tv.note);
  // A box plugged into a TV reaches it through the cable; a TV on its own can only sleep.
  const cable = $derived(!!tv?.controls.some((c) => c.id !== 'sleep'));
</script>

<BottomSheet title="TV controls" subtitle={ready && cable ? 'Turn the TV on or off and switch its input.' : null} {onclose}>
  {#if tv?.note}
    <p class="note">{tv.note}</p>
    <p class="caption note">Volume and playback are on the remote, and work without it.</p>
  {:else if tv}
    <div class="list">
      {#each tv.controls as c, i (c.id)}
        {#if c.group && c.group !== tv.controls[i - 1]?.group}<p class="caption group">{c.group}</p>{/if}
        <button class="list-row" onclick={() => run(c)}>
          <span class="glyph" class:off={c.id === 'sleep'}><Icon name={c.glyph} size={24} /></span>
          <span class="grow">
            <span class="title">{c.title}</span>
            {#if why(c)}<span class="caption">{why(c)}</span>{/if}
          </span>
          <span class="value {runs[c.id]?.state ?? ''}" aria-live="polite">{value(c)}</span>
        </button>
      {/each}
    </div>
    {#if cable}<p class="caption note">Lounge asks the TV through its HDMI cable; the TV decides what it does.</p>{/if}
  {:else}
    <p class="caption note">{unread ? "The TV isn't answering. Check it is on, then open this again." : 'Reading them from the TV…'}</p>
  {/if}
</BottomSheet>

<style>
  .note { color: var(--dust); padding: 0 4px; }
  p.note:not(.caption) { color: var(--limelight); }
  .group { margin: 8px 4px 0; color: var(--dust); }
  .grow { flex: 1; min-width: 0; display: flex; flex-direction: column; gap: 1px; }
  .glyph { width: 32px; height: 32px; display: grid; place-items: center; color: var(--dust); flex: none; }
  .glyph.off { color: var(--ember); }
  .value.sending { color: var(--houselight); }
  .value.sent { color: var(--signal); }
  .value.failed { color: var(--ember); }
</style>
