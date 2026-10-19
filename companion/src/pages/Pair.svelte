<script lang="ts">
  import { api, setToken } from '../lib/api';
  import { app, connect, go, toast } from '../lib/state.svelte';

  let code = $state('');
  let busy = $state(false);

  async function pair() {
    if (code.replace(/\D/g, '').length < 6) return;
    busy = true;
    try {
      const r = await api.pair(code.replace(/\D/g, ''));
      setToken(r.token);
      app.paired = true;
      connect();
      go('home');
      toast(`Paired with ${r.device}`);
    } catch (e: any) {
      toast(e?.status === 403 ? 'That code is wrong or has expired. Open Settings › Phone companion on the TV for a new one.' : (e?.message ?? 'The TV did not answer.'));
    } finally {
      busy = false;
    }
  }
</script>

<main class="page pair">
  <img class="mark" src="/mark.png" alt="" aria-hidden="true" />
  <h1>Pair with your TV</h1>
  <p class="dust">On the TV, open <b>Settings › Phone companion</b>. Scan the QR code, or type the six digits it shows.</p>
  <form class="field" onsubmit={(e) => { e.preventDefault(); pair(); }}>
    <label for="code">Pairing code</label>
    <input id="code" inputmode="numeric" autocomplete="one-time-code" pattern="[0-9 ]*" maxlength="7" placeholder="000 000" bind:value={code} />
    <button class="btn primary" type="submit" disabled={busy || code.replace(/\D/g, '').length < 6}>{busy ? 'Pairing…' : 'Pair'}</button>
  </form>
  <p class="caption">Everything stays on your network. The TV remembers this phone until you forget it from the TV.</p>
</main>

<style>
  .pair { justify-content: center; gap: 20px; padding-bottom: 40px; max-width: 420px; margin: 0 auto; width: 100%; }
  .mark { width: 88px; height: auto; margin-bottom: 8px; }
  .field input { font-family: var(--headline); font-size: 28px; letter-spacing: 0.18em; text-align: center; min-height: 64px; }
  form { gap: 12px; }
</style>
