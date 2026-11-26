<script lang="ts">
  import { api } from '../lib/api';
  import Icon from '../lib/Icon.svelte';
  import { t, tc } from '../lib/lang.svelte';
  import { app, connectionLabel, done, toast } from '../lib/state.svelte';


  let text = $state('');
  /** A few words to show on the TV, and whether they are on their way. */
  let words = $state('');
  let saying = $state(false);
  async function say() {
    const said = words.trim();
    if (!said || saying) return;
    saying = true;
    if (await done(t('Shown on the TV'), () => api.message(said))) words = '';
    saying = false;
  }
  let listening = $state(false);
  let recognition: any = null;

  // Keys go to the TV one at a time, in the order pressed. A held key's repeat is dropped while
  // one is waiting, and a refused key drops the rest, so its reason is said once.
  let waiting: string[] = [];
  let sending = false;
  function key(k: string, repeat = false) {
    if (navigator.vibrate) navigator.vibrate(8);
    if (repeat && waiting.length > 0) return;
    waiting.push(k);
    if (!sending) sendKeys();
  }
  async function sendKeys() {
    sending = true;
    while (waiting.length > 0) {
      const k = waiting.shift()!;
      if (!(await done('', () => api.key(k)))) waiting = [];
    }
    sending = false;
  }

  // A key is sent when it is let go, so a scroll that starts on one sends nothing, and keyboard
  // and switch users press it like any button. Arrows, volume and seeking repeat while held.
  let repeat: ReturnType<typeof setTimeout> | undefined;
  let repeated = false;
  function stop() {
    clearTimeout(repeat);
    repeat = undefined;
  }
  function press(k: string, repeats = false) {
    return {
      onclick: () => {
        if (repeated) repeated = false;
        else key(k);
      },
      onpointerdown: () => {
        repeated = false;
        stop();
        if (!repeats) return;
        const tick = (wait: number) => (repeat = setTimeout(() => { repeated = true; key(k, true); tick(120); }, wait));
        tick(450);
      },
      onpointerup: stop,
      onpointercancel: () => { stop(); repeated = false; },
      onpointerleave: stop,
    };
  }
  // What was typed stays in the field until the TV has it, so a failure loses nothing.
  async function sendText(label: string, f: (typed: string) => Promise<unknown>) {
    const typed = text;
    if (!typed.trim()) return;
    if ((await done(label, () => f(typed))) && text === typed) text = '';
  }
  function send() {
    sendText(t('Sent to Lounge on the TV'), (typed) => api.text(typed));
  }
  function search() {
    sendText(t('Searching the TV for “{query}”', { query: text.trim() }), (typed) => api.search(typed));
  }
  let link = $derived(text.trim().match(/^https?:\/\/\S+$/) ? text.trim() : null);
  function openLink() {
    if (!link) return;
    const l = link;
    sendText(t('Opened on the TV'), () => api.open(`url:${l}`));
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
      if (e?.error === 'no-speech') toast(t('Nothing was heard. Press the microphone and speak.'));
      else if (e?.error === 'not-allowed' || e?.error === 'service-not-allowed') toast(t("The browser did not allow the microphone. The keyboard's own dictation works in the field."));
      else if (e?.error !== 'aborted') toast(t('Voice did not work this time. Try again, or use the keyboard.'));
    };
    listening = true;
    recognition.start();
  }
</script>

<main class="page remote">
  <header class="page-head">
    <div><p class="meta">{app.state?.device.name ?? t('Your TV')}</p><h1>{t('Remote')}</h1></div>
    <div class="head-end">
      <span class="pill" class:on={app.connected}>{connectionLabel()}</span>
      <button class="power" onclick={() => (app.tvControls = true)} aria-label={t('TV power and input')}><Icon name="power" size={24} /></button>
    </div>
  </header>

  <section class="dpad" aria-label={t('Direction pad')}>
    <button class="pad up" {...press('up', true)} aria-label={tc('remote key', 'Up')}><Icon name="up" size={28} /></button>
    <button class="pad left" {...press('left', true)} aria-label={tc('remote key', 'Left')}><Icon name="left" size={28} /></button>
    <button class="pad ok" {...press('center')}>{tc('remote key', 'OK')}</button>
    <button class="pad right" {...press('right', true)} aria-label={tc('remote key', 'Right')}><Icon name="right" size={28} /></button>
    <button class="pad down" {...press('down', true)} aria-label={tc('remote key', 'Down')}><Icon name="down" size={28} /></button>
  </section>

  <section class="keys nav">
    <button class="key labelled" {...press('back')}><Icon name="back" size={22} /><span>{tc('remote key', 'Back')}</span></button>
    <button class="key labelled" {...press('home')}><Icon name="home" size={22} /><span>{tc('remote key', 'Home')}</span></button>
    <button class="key labelled" {...press('menu')}><Icon name="menu" size={22} /><span>{tc('remote key', 'Menu')}</span></button>
  </section>

  <section class="keys media" aria-label={t('Playback and volume')}>
    <button class="key" {...press('rewind', true)} aria-label={tc('remote key', 'Rewind')}><Icon name="rewind" /></button>
    <button class="key" {...press('play_pause')} aria-label={tc('remote key', 'Play or pause')}><Icon name="play_pause" size={26} /></button>
    <button class="key" {...press('fast_forward', true)} aria-label={tc('remote key', 'Fast forward')}><Icon name="fast_forward" /></button>
    <button class="key" {...press('volume_down', true)} aria-label={tc('remote key', 'Volume down')}><Icon name="volume_down" /></button>
    <button class="key" {...press('mute')} aria-label={tc('remote key', 'Mute')}><Icon name="mute" /></button>
    <button class="key" {...press('volume_up', true)} aria-label={tc('remote key', 'Volume up')}><Icon name="volume_up" /></button>
  </section>

  <section class="card type">
    <label class="caption" for="type">{t('Type on the TV')}</label>
    <div class="type-row">
      <input id="type" bind:value={text} placeholder={t('Types into the field open in Lounge')} enterkeyhint="send" onkeydown={(e) => { if (e.key === 'Enter') send(); }} autocapitalize="off" autocorrect="off" />
      {#if Speech}<button class="round" class:listening onclick={voice} aria-label={listening ? t('Stop listening') : t('Speak')}><Icon name="mic" size={22} /></button>{/if}
    </div>
    <div class="type-actions">
      {#if link}
        <!-- Translators: a button that types the text into the field open on the TV -->
        <button class="btn" onclick={send}>{t('Type')}</button>
        <button class="btn primary" onclick={openLink}>{t('Open on the TV')}</button>
      {:else}
        <!-- Translators: a button that types the text into the field open on the TV -->
        <button class="btn" onclick={send} disabled={!text.trim()}>{t('Type')}</button>
        <button class="btn primary" onclick={search} disabled={!text.trim()}>{t('Search the TV')}</button>
      {/if}
    </div>
    <p class="caption">{t('Paste a link to open it on the TV.')}</p>
  </section>

  <section class="card type">
    <label class="caption" for="say">{t('Say it on the TV')}</label>
    <div class="type-row">
      <input id="say" bind:value={words} maxlength="120" placeholder={t('A few words to show on the TV')} enterkeyhint="send" onkeydown={(e) => { if (e.key === 'Enter') say(); }} />
      <!-- Translators: a button that shows the words just typed as a pop-up on the TV -->
      <button class="btn primary" onclick={say} disabled={!words.trim() || saying}>{t('Show')}</button>
    </div>
    <p class="caption">{t('It shows in the corner of the TV for a few seconds, over whatever is on.')}</p>
  </section>
</main>

<style>
  .remote { gap: 26px; }
  .head-end { display: flex; align-items: center; gap: 12px; }
  .power { width: 48px; height: 48px; border-radius: 50%; background: var(--curtain); color: var(--limelight); display: grid; place-items: center; flex: none; box-shadow: inset 0 0 0 1px var(--curtain-edge); transition: transform 90ms ease-out, background 90ms ease-out; }
  .power:active { transform: scale(0.95); background: var(--curtain-edge); }
  .dpad { display: grid; grid-template-areas: '. up .' 'left ok right' '. down .'; grid-template-columns: 1fr 1fr 1fr; gap: 10px; width: min(78vw, 300px); margin: 4px auto 0; }
  .pad { aspect-ratio: 1; border-radius: 24px; background: var(--curtain); color: var(--limelight); display: grid; place-items: center; box-shadow: inset 0 0 0 1px var(--curtain-edge); transition: transform 90ms ease-out, background 90ms ease-out; user-select: none; touch-action: manipulation; }
  .pad:active { transform: scale(0.95); background: var(--curtain-edge); }
  .pad.ok { border-radius: 50%; background: var(--light); color: var(--on-light); font-family: var(--headline); font-size: 22px; font-weight: 600; box-shadow: 0 10px 40px -14px var(--light); }
  .pad.up { grid-area: up; } .pad.down { grid-area: down; } .pad.left { grid-area: left; } .pad.right { grid-area: right; } .pad.ok { grid-area: ok; }
  .keys { display: flex; justify-content: center; align-items: center; gap: 10px; }
  /* Six keys share the width whatever the phone, with a breath between playback and volume. */
  .keys.media { display: grid; grid-template-columns: repeat(6, minmax(0, 1fr)); gap: 8px; }
  .keys.media .key:nth-child(4) { margin-left: 8px; }
  .keys.media .key:nth-child(3) { margin-right: 8px; }
  .key { min-width: 44px; height: 52px; padding: 0 10px; border-radius: 26px; background: var(--curtain); color: var(--limelight); display: inline-flex; align-items: center; justify-content: center; gap: 8px; box-shadow: inset 0 0 0 1px var(--curtain-edge); touch-action: manipulation; user-select: none; transition: transform 90ms ease-out, background 90ms ease-out; }
  .key:active { transform: scale(0.95); background: var(--curtain-edge); }
  .key.labelled { padding: 0 18px 0 14px; font-weight: 600; }
  /* Back, Home and Menu share the width, so they fit a 320 px phone. */
  .keys.nav { display: grid; grid-template-columns: repeat(3, minmax(0, max-content)); justify-content: center; }
  @media (max-width: 360px) { .keys.nav { grid-template-columns: repeat(3, minmax(0, 1fr)); gap: 6px; } .key.labelled { padding: 0 8px; gap: 6px; } }
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
