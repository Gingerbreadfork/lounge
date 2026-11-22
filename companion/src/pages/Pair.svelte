<script lang="ts">
  import { api, explain, setToken } from '../lib/api';
  import { t } from '../lib/lang.svelte';
  import { app, paired, toast } from '../lib/state.svelte';
  import { around } from '../lib/words';

  let code = $state('');
  let busy = $state(false);
  // Translators: {screen} is where "Phone companion" goes, the name of a screen in the TV's Settings
  const how = $derived(around(t('In Settings on the TV, open {screen}. Scan the QR code, or type the six digits it shows.'), 'screen'));

  async function pair() {
    if (code.replace(/\D/g, '').length < 6) return;
    busy = true;
    try {
      const r = await api.pair(code.replace(/\D/g, ''));
      setToken(r.token);
      code = '';
      paired(r.device);
    } catch (e) {
      toast(explain(e));
    } finally {
      busy = false;
    }
  }
</script>

<main class="page pair">
  <img class="mark" src="/mark.png" alt="" aria-hidden="true" />
  <h1>{t('Pair with your TV')}</h1>
  {#if app.pairNote}<p class="note" role="status">{app.pairNote}</p>{/if}
  <p class="dust">{how[0]}<b>{t('Phone companion')}</b>{how[1]}</p>
  <form class="field" onsubmit={(e) => { e.preventDefault(); pair(); }}>
    <label for="code">{t('Pairing code')}</label>
    <input id="code" inputmode="numeric" autocomplete="one-time-code" pattern="[0-9 ]*" maxlength="7" placeholder="000 000" bind:value={code} />
    <!-- Translators: the button that pairs this phone with the TV, and what it says while it does -->
    <button class="btn primary" type="submit" disabled={busy || code.replace(/\D/g, '').length < 6}>{busy ? t('Pairing…') : t('Pair')}</button>
  </form>
  <p class="caption">{t('Everything stays on your network. The TV remembers this phone until you forget it, from the TV or from More here.')}</p>
</main>

<style>
  .pair { justify-content: center; gap: 20px; padding-bottom: 40px; max-width: 420px; margin: 0 auto; width: 100%; }
  .mark { width: 88px; height: auto; margin-bottom: 8px; }
  .note { color: var(--houselight); font-weight: 600; }
  .field input { font-family: var(--headline); font-size: 28px; letter-spacing: 0.18em; text-align: center; min-height: 64px; }
  form { gap: 12px; }
</style>
