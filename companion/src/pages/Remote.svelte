<script lang="ts">
  import { api } from '../lib/api';
  import Icon from '../lib/Icon.svelte';
  import { act, app, connectionLabel, toast } from '../lib/state.svelte';

  let text = $state('');
  let listening = $state(false);
  let recognition: any = null;

  function key(k: string) {
    if (navigator.vibrate) navigator.vibrate(8);
    act('', () => api.key(k));
  }
  function send() {
    if (!text.trim()) return;
    const t = text;
    text = '';
    act('Typed on the TV', () => api.text(t));
  }
  function search() {
    if (!text.trim()) return;
    const t = text;
    text = '';
    act(`Searching for “${t}”`, () => api.search(t));
  }
  let link = $derived(text.trim().match(/^https?:\/\/\S+$/) ? text.trim() : null);
  function openLink() {
    if (!link) return;
    const l = link;
    text = '';
    act('Opened on the TV', () => api.open(`url:${l}`));
  }

  // Voice through the phone's own recogniser; the TV never needs one. Browsers give the microphone
  // only to secure pages, which the TV's plain http address is not, so elsewhere there is no button.
  const Speech = window.isSecureContext ? ((globalThis as any).SpeechRecognition ?? (globalThis as any).webkitSpeechRecognition) : undefined;
  function voice() {
    if (!Speech) return;
    if (listening) { recognition?.stop(); return; }
    recognition = new Speech();
    recognition.lang = navigator.language;
    recognition.interimResults = true;
    recognition.onresult = (e: any) => {
      let s = '';
      for (const r of e.results) s += r[0].transcript;
      text = s;
      if (e.results[e.results.length - 1].isFinal) { search(); }
    };
    recognition.onend = () => (listening = false);
    recognition.onerror = (e: any) => {
      listening = false;
      if (e?.error === 'no-speech') toast('Nothing was heard. Press the microphone and speak.');
      else if (e?.error === 'not-allowed' || e?.error === 'service-not-allowed') toast("The browser did not allow the microphone. The keyboard's own dictation works in the field.");
      else if (e?.error !== 'aborted') toast('Voice did not work this time. Try again, or use the keyboard.');
    };
    listening = true;
    recognition.start();
  }
</script>

<main class="page remote">
  <header class="page-head">
    <div><p class="meta">{app.state?.device.name ?? 'Your TV'}</p><h1>Remote</h1></div>
    <span class="pill" class:on={app.connected}>{connectionLabel()}</span>
  </header>

  <section class="dpad" aria-label="Direction pad">
    <button class="pad up" onpointerdown={() => key('up')} aria-label="Up"><Icon name="up" size={28} /></button>
    <button class="pad left" onpointerdown={() => key('left')} aria-label="Left"><Icon name="left" size={28} /></button>
    <button class="pad ok" onpointerdown={() => key('center')} aria-label="Select">OK</button>
    <button class="pad right" onpointerdown={() => key('right')} aria-label="Right"><Icon name="right" size={28} /></button>
    <button class="pad down" onpointerdown={() => key('down')} aria-label="Down"><Icon name="down" size={28} /></button>
  </section>

  <section class="keys">
    <button class="key labelled" onpointerdown={() => key('back')}><Icon name="back" size={22} /><span>Back</span></button>
    <button class="key labelled" onpointerdown={() => key('home')}><Icon name="home" size={22} /><span>Home</span></button>
    <button class="key labelled" onpointerdown={() => key('menu')}><Icon name="menu" size={22} /><span>Menu</span></button>
  </section>

  <section class="keys media" aria-label="Playback and volume">
    <button class="key" onpointerdown={() => key('rewind')} aria-label="Rewind"><Icon name="rewind" /></button>
    <button class="key" onpointerdown={() => key('play_pause')} aria-label="Play or pause"><Icon name="play_pause" size={26} /></button>
    <button class="key" onpointerdown={() => key('fast_forward')} aria-label="Fast forward"><Icon name="fast_forward" /></button>
    <button class="key" onpointerdown={() => key('volume_down')} aria-label="Volume down"><Icon name="volume_down" /></button>
    <button class="key" onpointerdown={() => key('mute')} aria-label="Mute"><Icon name="mute" /></button>
    <button class="key" onpointerdown={() => key('volume_up')} aria-label="Volume up"><Icon name="volume_up" /></button>
  </section>

  <section class="card type">
    <label class="caption" for="type">Type on the TV</label>
    <div class="type-row">
      <input id="type" bind:value={text} placeholder="Lands in the TV's focused field" enterkeyhint="send" onkeydown={(e) => { if (e.key === 'Enter') send(); }} autocapitalize="off" autocorrect="off" />
      {#if Speech}<button class="round" class:listening onclick={voice} aria-label={listening ? 'Stop listening' : 'Speak'}><Icon name="mic" size={22} /></button>{/if}
    </div>
    <div class="type-actions">
      {#if link}
        <button class="btn" onclick={send}>Type</button>
        <button class="btn primary" onclick={openLink}>Open on the TV</button>
      {:else}
        <button class="btn" onclick={send} disabled={!text.trim()}>Type</button>
        <button class="btn primary" onclick={search} disabled={!text.trim()}>Search the TV</button>
      {/if}
    </div>
    <p class="caption">Paste a link to open it on the TV.</p>
  </section>
</main>

<style>
  .remote { gap: 26px; }
  .dpad { display: grid; grid-template-areas: '. up .' 'left ok right' '. down .'; grid-template-columns: 1fr 1fr 1fr; gap: 10px; width: min(78vw, 300px); margin: 4px auto 0; }
  .pad { aspect-ratio: 1; border-radius: 24px; background: var(--curtain); color: var(--limelight); display: grid; place-items: center; box-shadow: inset 0 0 0 1px var(--curtain-edge); transition: transform 90ms ease-out, background 90ms ease-out; user-select: none; touch-action: manipulation; }
  .pad:active { transform: scale(0.95); background: var(--curtain-edge); }
  .pad.ok { border-radius: 50%; background: var(--light); color: var(--velvet); font-family: var(--headline); font-size: 22px; font-weight: 600; box-shadow: 0 10px 40px -14px var(--light); }
  .pad.up { grid-area: up; } .pad.down { grid-area: down; } .pad.left { grid-area: left; } .pad.right { grid-area: right; } .pad.ok { grid-area: ok; }
  .keys { display: flex; justify-content: center; align-items: center; gap: 10px; }
  /* Six keys share the width whatever the phone, with a breath between playback and volume. */
  .keys.media { display: grid; grid-template-columns: repeat(6, minmax(0, 1fr)); gap: 8px; }
  .keys.media .key:nth-child(4) { margin-left: 8px; }
  .keys.media .key:nth-child(3) { margin-right: 8px; }
  .key { min-width: 44px; height: 52px; padding: 0 10px; border-radius: 26px; background: var(--curtain); color: var(--limelight); display: inline-flex; align-items: center; justify-content: center; gap: 8px; box-shadow: inset 0 0 0 1px var(--curtain-edge); touch-action: manipulation; user-select: none; transition: transform 90ms ease-out, background 90ms ease-out; }
  .key:active { transform: scale(0.95); background: var(--curtain-edge); }
  .key.labelled { padding: 0 18px 0 14px; font-weight: 600; }
  .type { display: flex; flex-direction: column; gap: 10px; }
  .type-row { display: flex; gap: 8px; align-items: center; }
  .type-row input { flex: 1; min-width: 0; min-height: 50px; padding: 0 14px; border-radius: var(--radius); background: var(--velvet); border: 1px solid var(--curtain-edge); color: var(--limelight); outline: none; }
  .type-row input:focus { border-color: var(--light); }
  .round { width: 50px; height: 50px; border-radius: 50%; background: var(--velvet); border: 1px solid var(--curtain-edge); color: var(--limelight); display: grid; place-items: center; flex: none; }
  .round.listening { background: var(--ember); border-color: var(--ember); animation: pulse 1s ease-in-out infinite; }
  @keyframes pulse { 50% { transform: scale(1.08); } }
  @media (prefers-reduced-motion: reduce) { .round.listening { animation: none; } }
  .type-actions { display: flex; gap: 10px; justify-content: flex-end; }
</style>
