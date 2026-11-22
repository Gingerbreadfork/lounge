<script lang="ts">
  import { api, art, light, lit, tag, type Tile } from '../lib/api';
  import { hold } from '../lib/hold';
  import Icon, { drawn } from '../lib/Icon.svelte';
  import { t } from '../lib/lang.svelte';
  import { act, app, connectionLabel, dismissShare, go, openShare, press } from '../lib/state.svelte';
  import TileSheet from '../lib/TileSheet.svelte';

  let snapshot = $derived(app.state?.snapshot ?? null);
  let spotlight = $derived(snapshot?.stage.spotlights[0] ?? null);
  let np = $derived(snapshot?.nowPlaying ?? null);
  /** The tile whose menu is open, and the home row it was held in. */
  let sheet = $state<{ tile: Tile; rowId: string | null; rowTitle: string | null } | null>(null);
  // The hint goes once a tile has been held; the gesture is then known.
  let hintSeen = $state(read('lounge.heldTile'));

  function read(key: string) {
    try { return localStorage.getItem(key) === '1'; } catch { return false; }
  }
  function open(tile: Tile) {
    // A tile to read opens nothing on the TV, so a tap is not passed on.
    if (!tile.action) return;
    // The TV's controls and its apps have a place of their own on the phone.
    if (tile.action === 'control:tv:open') return void (app.tvControls = true);
    if (tile.action === 'control:allapps') return go('home', 'apps');
    // A suggested plugin is set up here on the phone, where its settings are easy to type.
    if (tile.action.startsWith('control:plugin:')) return go('plugins', tile.action.slice('control:plugin:'.length));
    press(tile);
  }
  function menu(tile: Tile, rowId: string | null, rowTitle: string | null) {
    sheet = { tile, rowId, rowTitle };
    if (!hintSeen) {
      hintSeen = true;
      try { localStorage.setItem('lounge.heldTile', '1'); } catch { /* private mode */ }
    }
  }
  // A menu left open for a tile the TV has since dropped closes on its own.
  $effect(() => {
    if (!sheet || !snapshot) return;
    const all = [...snapshot.stage.strip, ...snapshot.rows.flatMap((r) => r.tiles)];
    if (!all.some((tile) => tile.id === sheet!.tile.id)) sheet = null;
  });
  function media(cmd: string) {
    if (!np) return;
    act('', () => api.open(`media:${np!.package}:${cmd}`));
  }
  /** What a screen reader says for a tile: the title, then what the eye would see on it. */
  function describe(tile: Tile) {
    const parts = [tile.title];
    if (tile.subtitle) parts.push(tile.subtitle);
    // Translators: read out after a tile's title, for a control such as a light that is on or off
    else if (tile.on != null) parts.push(tile.on ? t('on') : t('off'));
    // Translators: read out after a tile's title, for something broadcast live
    if (tag(tile.badge) === 'LIVE') parts.push(t('live'));
    // Translators: read out after a tile's title, for something pinned to the Stage
    if (tile.stagePinned) parts.push(t('pinned to the Stage'));
    // Translators: read out after a tile's title, for an app pinned to the front of Your apps
    else if (tile.pinned) parts.push(t('pinned'));
    return parts.join(', ');
  }
  function progressPct(p: number | null | undefined) {
    return p == null ? null : Math.round(Math.max(0, Math.min(1, p)) * 100);
  }
</script>

<main class="page">
  <header class="page-head">
    <div>
      <p class="meta">{app.state?.device.name ?? t('Your TV')}</p>
      <h1>{t("What's on")}</h1>
    </div>
    <span class="pill" class:on={app.connected}>{connectionLabel()}</span>
  </header>

  {#if app.error && !app.away}
    <p class="empty">{app.error}</p>
  {/if}

  {#if app.share}
    <section class="card share" aria-live="polite">
      <!-- Translators: above a link another app shared with Lounge, which can then be opened on the TV -->
      <p class="caption">{t('Shared with Lounge')}</p>
      <p class="share-link">{app.share}</p>
      <div class="share-actions">
        <button class="btn primary" onclick={openShare}>{t('Open on the TV')}</button>
        <button class="btn ghost" onclick={dismissShare}>{t('Not now')}</button>
      </div>
    </section>
  {/if}

  {#if np}
    <section class="card np" style={lit(np.lightArgb)}>
      <div class="np-art">{#if np.artPath}<img src={art(np.artPath)} alt="" />{/if}</div>
      <div class="np-text">
        <p class="caption">{np.appLabel}</p>
        <p class="np-title">{np.title}</p>
        {#if np.subtitle}<p class="meta">{np.subtitle}</p>{/if}
        <div class="np-controls">
          <!-- Translators: a button for what is playing on the TV: skips back to the track or episode before -->
          <button class="round" disabled={!np.canSkipPrevious} onclick={() => media('previous')} aria-label={t('Previous')}><Icon name="skip_previous" size={20} /></button>
          <!-- Translators: a button for what is playing on the TV: pauses it, or plays it again -->
          <button class="round big" onclick={() => media('play-pause')} aria-label={tag(np.status) === 'PLAYING' ? t('Pause') : t('Play')}><Icon name={tag(np.status) === 'PLAYING' ? 'pause' : 'play'} size={24} /></button>
          <!-- Translators: a button for what is playing on the TV: skips to the next track or episode -->
          <button class="round" disabled={!np.canSkipNext} onclick={() => media('next')} aria-label={t('Next')}><Icon name="skip_next" size={20} /></button>
        </div>
      </div>
    </section>
  {/if}

  {#if spotlight}
    <section class="stage" style={lit(spotlight.lightArgb)}>
      <div class="stage-art">
        {#if spotlight.artworkPath}<img src={art(spotlight.artworkPath)} alt="" />{/if}
      </div>
      <div class="stage-text">
        <p class="meta">{spotlight.category}{#if spotlight.source}<span class="source">{`, ${spotlight.source}`}</span>{/if}</p>
        <h2>{spotlight.title}</h2>
        {#each spotlight.lines as line, i (i)}<p class="meta">{line}</p>{/each}
        {#if progressPct(spotlight.progress) != null}
          <div class="progress"><i style="width: {progressPct(spotlight.progress)}%"></i></div>
        {/if}
        <div class="actions">
          <!-- Translators: said after a button is pressed; {action} is the button's own label, such as "Play" or "Resume" -->
          <button class="btn primary" onclick={() => act(t('{action} on the TV', { action: spotlight!.primary.label }), () => api.open(spotlight!.primary.action))}>{spotlight.primary.label}</button>
          {#if spotlight.secondary}<button class="btn" onclick={() => act('', () => api.open(spotlight!.secondary!.action))}>{spotlight.secondary.label}</button>{/if}
        </div>
        {#if spotlight.explanation}<p class="caption">{spotlight.explanation}</p>{/if}
      </div>
    </section>
  {/if}

  {#if snapshot}
    {#if !hintSeen && (snapshot.stage.strip.length || snapshot.rows.length)}
      <p class="caption hint">{t('Hold a tile to pin it to the Stage or the front of Your apps, or to hide it.')}</p>
    {/if}
    {#if snapshot.stage.strip.length}
      <section class="section">
        <!-- Translators: Lounge's name for the big pictures at the top of the TV's home screen -->
        <div class="section-head"><h3>{t('Stage')}</h3></div>
        <div class="row-scroller">
          {#each snapshot.stage.strip as tile (tile.id)}
            <button class="tile landscape" class:borrowed={tile.borrowedArt} style={lit(tile.lightArgb)} aria-label={describe(tile)} onclick={() => open(tile)} use:hold={() => menu(tile, null, null)}>
              {#if tile.artworkPath}<img src={art(tile.artworkPath)} alt="" loading="lazy" />{/if}
              {#if tag(tile.kind) !== 'APP_BANNER' || !tile.artworkPath}<div class="scrim"><div class="t">{tile.title}</div>{#if tile.subtitle}<div class="s">{tile.subtitle}</div>{/if}</div>{/if}
              {#if tile.stagePinned}<span class="pinmark" aria-hidden="true"><Icon name="pin" size={13} /></span>{/if}
              {#if tile.progress != null}<div class="bar"><i style="width: {progressPct(tile.progress)}%"></i></div>{/if}
            </button>
          {/each}
        </div>
      </section>
    {/if}
    {#each snapshot.rows as row (row.id)}
      <section class="section">
        <div class="section-head">
          <h3>{row.title}</h3>
          <!-- Translators: a small label on something broadcast live -->
          {#if tag(row.badge) === 'LIVE'}<span class="pill bad"><span class="dot"></span>{t('Live')}</span>{/if}
          {#if row.id === 'apps'}<button class="all" onclick={() => go('home', 'apps')}>{t('All apps')}<Icon name="chevron" size={16} /></button>{/if}
        </div>
        {#if row.note}<p class="caption dust">{row.note}</p>{/if}
        <div class="row-scroller">
          {#each row.tiles.filter((tile) => !tile.id.startsWith('control:')) as tile (tile.id)}
            <button class="tile {tag(row.kind).toLowerCase()}" class:borrowed={tile.borrowedArt} style={lit(tile.lightArgb)} aria-label={describe(tile)} onclick={() => open(tile)} use:hold={tile.id.startsWith('control:') ? null : () => menu(tile, row.id, row.title)}>
              {#if tag(row.kind) === 'COMPACT'}
                {#if drawn(tile.glyph) || (tile.glyph && !tile.iconPath)}
                  <span class="glyph" class:on={tile.on === true} class:off={tile.on === false} aria-hidden="true"><Icon name={drawn(tile.glyph) ? tile.glyph : 'thing'} size={26} /></span>
                {:else if tile.iconPath}<img class="icon" src={art(tile.iconPath)} alt="" loading="lazy" />{/if}
                <span class="lines">
                  <span class="t">{tile.title}</span>
                  {#if tile.subtitle}<span class="s">{tile.subtitle}</span>{/if}
                </span>
                {#if tile.pinned || tile.stagePinned}<span class="pin-inline" aria-hidden="true"><Icon name="pin" size={14} /></span>{/if}
              {:else}
                {#if tile.artworkPath}<img src={art(tile.artworkPath)} alt="" loading="lazy" />{:else}<div class="fill" style="background: color-mix(in srgb, {light(tile.lightArgb)} 35%, var(--curtain))"></div>{/if}
                <!-- Translators: a small label on something broadcast live -->
                {#if tag(tile.badge) === 'LIVE' && tag(row.badge) !== 'LIVE'}<span class="badge"><span class="dot"></span>{t('Live')}</span>{/if}
                {#if (tag(row.kind) !== 'APP_BANNER' && tag(row.kind) !== 'POSTER') || !tile.artworkPath}<div class="scrim">{#if (tile.artTitle ?? tile.title)}<div class="t">{tile.artTitle ?? tile.title}</div>{/if}{#if tile.subtitle}<div class="s">{tile.subtitle}</div>{/if}</div>{/if}
                {#if tile.pinned || tile.stagePinned}<span class="pinmark" aria-hidden="true"><Icon name="pin" size={13} /></span>{/if}
                {#if tile.progress != null}<div class="bar"><i style="width: {progressPct(tile.progress)}%"></i></div>{/if}
              {/if}
            </button>
          {/each}
        </div>
      </section>
    {/each}
    {#if !snapshot.rows.some((r) => r.id === 'apps')}
      <button class="list-row" onclick={() => go('home', 'apps')}>
        <span class="glyph"><Icon name="apps" size={24} /></span>
        <span class="grow"><span class="title">{t('All apps')}</span><span class="caption">{t('Open any app on the TV')}</span></span>
        <Icon name="chevron" size={20} />
      </button>
    {/if}
  {:else if !app.error}
    <p class="empty">{t('Waiting for the TV…')}</p>
  {/if}
</main>

{#if sheet}
  <TileSheet tile={sheet.tile} rowId={sheet.rowId} rowTitle={sheet.rowTitle} onclose={() => (sheet = null)} />
{/if}

<style>
  .all { display: inline-flex; align-items: center; gap: 2px; color: var(--dust); font-size: 14px; font-weight: 600; min-height: 32px; margin-left: auto; }
  .list-row .grow { display: flex; flex-direction: column; gap: 1px; }
  .list-row { color: var(--limelight); }
  .share { display: flex; flex-direction: column; gap: 10px; }
  .share-link { overflow-wrap: anywhere; font-weight: 600; display: -webkit-box; -webkit-line-clamp: 3; line-clamp: 3; -webkit-box-orient: vertical; overflow: hidden; }
  .share-actions { display: flex; gap: 10px; flex-wrap: wrap; }
  .stage { border-radius: var(--radius-l); overflow: hidden; background: var(--curtain); box-shadow: 0 0 0 1px var(--curtain-edge), 0 24px 60px -30px color-mix(in srgb, var(--light) 60%, transparent); }
  .stage-art { aspect-ratio: 16 / 9; background: linear-gradient(135deg, color-mix(in srgb, var(--light) 45%, var(--velvet)), var(--velvet)); }
  .stage-art img { width: 100%; height: 100%; object-fit: cover; display: block; }
  .stage-text { padding: 16px; display: flex; flex-direction: column; gap: 6px; }
  .stage-text h2 { font-family: var(--display); font-weight: 650; font-size: 26px; line-height: 1.1; }
  .progress { height: 4px; border-radius: 2px; background: var(--curtain-edge); margin: 6px 0 2px; max-width: 280px; }
  .progress i { display: block; height: 100%; border-radius: 2px; background: var(--houselight); }
  .actions { display: flex; gap: 10px; margin-top: 10px; }
  .np { display: flex; gap: 14px; padding: 12px; align-items: center; }
  .np-art { width: 84px; height: 84px; border-radius: 12px; flex: none; overflow: hidden; background: color-mix(in srgb, var(--light) 30%, var(--curtain)); }
  .np-art img { width: 100%; height: 100%; object-fit: cover; }
  .np-text { flex: 1; min-width: 0; display: flex; flex-direction: column; gap: 2px; }
  .np-title { font-weight: 600; white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
  .np-controls { display: flex; align-items: center; gap: 10px; margin-top: 8px; }
  .round { width: 44px; height: 44px; border-radius: 50%; background: var(--velvet); color: var(--limelight); display: grid; place-items: center; }
  .round.big { width: 48px; height: 48px; background: var(--light); color: var(--on-light); }
  .source { color: var(--dust); }
  .round:disabled { opacity: 0.35; }
  .tile .fill { position: absolute; inset: 0; }
  .tile .icon { width: 28px; height: 28px; border-radius: 7px; flex: none; object-fit: cover; }
  /* A compact tile reads like the TV's: its title, then the line that says why, as a caption. */
  .tile.compact .lines { flex: 1; min-width: 0; display: flex; flex-direction: column; gap: 1px; }
  .tile.compact .t { font-size: 14px; font-weight: 600; white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
  .tile.compact .s { font-size: 12px; font-weight: 500; color: var(--dust); white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
  /* A control's glyph takes the thing's own light while it is on and rests in dust while it is off. */
  .tile.compact .glyph { flex: none; display: grid; place-items: center; color: var(--houselight); transition: color 0.2s; }
  .tile.compact .glyph.on { color: var(--light); }
  .tile.compact .glyph.off { color: var(--dust); }
  .hint { margin: -12px 0 -8px; }
  .pinmark { position: absolute; top: 6px; right: 6px; width: 22px; height: 22px; border-radius: 50%; display: grid; place-items: center; background: color-mix(in srgb, var(--velvet) 72%, transparent); color: var(--limelight); }
  .pin-inline { color: var(--dust); display: grid; place-items: center; flex: none; }
  /* A tile wearing its app's banner: a deeper scrim with the title on it, so it reads as something from the app. */
  .tile.borrowed .scrim { inset: 0; padding-top: 0; display: flex; flex-direction: column; justify-content: flex-end; background: linear-gradient(to top, color-mix(in srgb, var(--velvet) 94%, transparent), color-mix(in srgb, var(--velvet) 70%, transparent) 55%, color-mix(in srgb, var(--velvet) 10%, transparent)); }
</style>
