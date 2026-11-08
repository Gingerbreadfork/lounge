<script lang="ts">
  import { api, argb, art, tag, type Tile } from '../lib/api';
  import { hold } from '../lib/hold';
  import Icon from '../lib/Icon.svelte';
  import { act, app, connectionLabel, dismissShare, openShare } from '../lib/state.svelte';
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
  function open(t: Tile) {
    act(`Opening ${t.title} on the TV`, () => api.open(t.action));
  }
  function menu(t: Tile, rowId: string | null, rowTitle: string | null) {
    sheet = { tile: t, rowId, rowTitle };
    if (!hintSeen) {
      hintSeen = true;
      try { localStorage.setItem('lounge.heldTile', '1'); } catch { /* private mode */ }
    }
  }
  // A menu left open for a tile the TV has since dropped closes on its own.
  $effect(() => {
    if (!sheet || !snapshot) return;
    const all = [...snapshot.stage.strip, ...snapshot.rows.flatMap((r) => r.tiles)];
    if (!all.some((t) => t.id === sheet!.tile.id)) sheet = null;
  });
  function media(cmd: string) {
    if (!np) return;
    act('', () => api.open(`media:${np!.package}:${cmd}`));
  }
  /** What a screen reader says for a tile: the title, then what the eye would see on it. */
  function describe(t: Tile) {
    const parts = [t.title];
    if (t.subtitle) parts.push(t.subtitle);
    if (tag(t.badge) === 'LIVE') parts.push('live');
    if (t.stagePinned) parts.push('pinned to the Stage');
    else if (t.pinned) parts.push('pinned');
    return parts.join(', ');
  }
  function progressPct(p: number | null | undefined) {
    return p == null ? null : Math.round(Math.max(0, Math.min(1, p)) * 100);
  }
</script>

<main class="page">
  <header class="page-head">
    <div>
      <p class="meta">{app.state?.device.name ?? 'Your TV'}</p>
      <h1>What's on</h1>
    </div>
    <span class="pill" class:on={app.connected}>{connectionLabel()}</span>
  </header>

  {#if app.error}
    <p class="empty">{app.error}</p>
  {/if}

  {#if app.share}
    <section class="card share" aria-live="polite">
      <p class="caption">Shared with Lounge</p>
      <p class="share-link">{app.share}</p>
      <div class="share-actions">
        <button class="btn primary" onclick={openShare}>Open on the TV</button>
        <button class="btn ghost" onclick={dismissShare}>Not now</button>
      </div>
    </section>
  {/if}

  {#if np}
    <section class="card np" style="--light: {argb(np.lightArgb)}">
      <div class="np-art">{#if np.artPath}<img src={art(np.artPath)} alt="" />{/if}</div>
      <div class="np-text">
        <p class="caption">{np.appLabel}</p>
        <p class="np-title">{np.title}</p>
        {#if np.subtitle}<p class="meta">{np.subtitle}</p>{/if}
        <div class="np-controls">
          <button class="round" disabled={!np.canSkipPrevious} onclick={() => media('previous')} aria-label="Previous"><Icon name="skip_previous" size={20} /></button>
          <button class="round big" onclick={() => media('play-pause')} aria-label={tag(np.status) === 'PLAYING' ? 'Pause' : 'Play'}><Icon name={tag(np.status) === 'PLAYING' ? 'pause' : 'play'} size={24} /></button>
          <button class="round" disabled={!np.canSkipNext} onclick={() => media('next')} aria-label="Next"><Icon name="skip_next" size={20} /></button>
        </div>
      </div>
    </section>
  {/if}

  {#if spotlight}
    <section class="stage" style="--light: {argb(spotlight.lightArgb)}">
      <div class="stage-art">
        {#if spotlight.artworkPath}<img src={art(spotlight.artworkPath)} alt="" />{/if}
      </div>
      <div class="stage-text">
        <p class="meta">{spotlight.category}{#if spotlight.source}<span class="source">{` · ${spotlight.source}`}</span>{/if}</p>
        <h2>{spotlight.title}</h2>
        {#each spotlight.lines as line}<p class="meta">{line}</p>{/each}
        {#if progressPct(spotlight.progress) != null}
          <div class="progress"><i style="width: {progressPct(spotlight.progress)}%"></i></div>
        {/if}
        <div class="actions">
          <button class="btn primary" onclick={() => act(`${spotlight!.primary.label} on the TV`, () => api.open(spotlight!.primary.action))}>{spotlight.primary.label}</button>
          {#if spotlight.secondary}<button class="btn" onclick={() => act('', () => api.open(spotlight!.secondary!.action))}>{spotlight.secondary.label}</button>{/if}
        </div>
        {#if spotlight.explanation}<p class="caption">{spotlight.explanation}</p>{/if}
      </div>
    </section>
  {/if}

  {#if snapshot}
    {#if !hintSeen && (snapshot.stage.strip.length || snapshot.rows.length)}
      <p class="caption hint">Hold a tile to pin it to the Stage or the front of Your apps, or to hide it.</p>
    {/if}
    {#if snapshot.stage.strip.length}
      <section class="section">
        <div class="section-head"><h3>Stage</h3></div>
        <div class="row-scroller">
          {#each snapshot.stage.strip as t (t.id)}
            <button class="tile landscape" class:borrowed={t.borrowedArt} style="--light: {argb(t.lightArgb)}" aria-label={describe(t)} onclick={() => open(t)} use:hold={() => menu(t, null, null)}>
              {#if t.artworkPath}<img src={art(t.artworkPath)} alt="" loading="lazy" />{/if}
              {#if tag(t.kind) !== 'APP_BANNER' || !t.artworkPath}<div class="scrim"><div class="t">{t.title}</div>{#if t.subtitle}<div class="s">{t.subtitle}</div>{/if}</div>{/if}
              {#if t.stagePinned}<span class="pinmark" aria-hidden="true"><Icon name="pin" size={13} /></span>{/if}
              {#if t.progress != null}<div class="bar"><i style="width: {progressPct(t.progress)}%"></i></div>{/if}
            </button>
          {/each}
        </div>
      </section>
    {/if}
    {#each snapshot.rows as row (row.id)}
      <section class="section">
        <div class="section-head"><h3>{row.title}</h3>{#if tag(row.badge) === 'LIVE'}<span class="pill bad"><span class="dot"></span>Live</span>{/if}</div>
        <div class="row-scroller">
          {#each row.tiles as t (t.id)}
            <button class="tile {tag(row.kind).toLowerCase()}" class:borrowed={t.borrowedArt} style="--light: {argb(t.lightArgb)}" aria-label={describe(t)} onclick={() => open(t)} use:hold={t.id.startsWith('notice:') || t.id.startsWith('control:') ? null : () => menu(t, row.id, row.title)}>
              {#if tag(row.kind) === 'COMPACT'}
                {#if t.iconPath}<img class="icon" src={art(t.iconPath)} alt="" loading="lazy" />{/if}
                <span class="t">{t.title}</span>
                {#if t.pinned || t.stagePinned}<span class="pin-inline" aria-hidden="true"><Icon name="pin" size={14} /></span>{/if}
              {:else}
                {#if t.artworkPath}<img src={art(t.artworkPath)} alt="" loading="lazy" />{:else}<div class="fill" style="background: color-mix(in srgb, {argb(t.lightArgb)} 35%, var(--curtain))"></div>{/if}
                {#if tag(t.badge) === 'LIVE'}<span class="badge"><span class="dot"></span>Live</span>{/if}
                {#if (tag(row.kind) !== 'APP_BANNER' && tag(row.kind) !== 'POSTER') || !t.artworkPath}<div class="scrim"><div class="t">{t.title}</div>{#if t.subtitle}<div class="s">{t.subtitle}</div>{/if}</div>{/if}
                {#if t.pinned || t.stagePinned}<span class="pinmark" aria-hidden="true"><Icon name="pin" size={13} /></span>{/if}
                {#if t.progress != null}<div class="bar"><i style="width: {progressPct(t.progress)}%"></i></div>{/if}
              {/if}
            </button>
          {/each}
        </div>
      </section>
    {/each}
  {:else if !app.error}
    <p class="empty">Waiting for the TV…</p>
  {/if}
</main>

{#if sheet}
  <TileSheet tile={sheet.tile} rowId={sheet.rowId} rowTitle={sheet.rowTitle} onclose={() => (sheet = null)} />
{/if}

<style>
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
  .round { width: 40px; height: 40px; border-radius: 50%; background: var(--velvet); color: var(--limelight); display: grid; place-items: center; }
  .round.big { width: 48px; height: 48px; background: var(--light); color: var(--velvet); }
  .source { color: var(--dust); }
  .round:disabled { opacity: 0.35; }
  .tile .fill { position: absolute; inset: 0; }
  .tile .icon { width: 28px; height: 28px; border-radius: 7px; flex: none; object-fit: cover; }
  .tile.compact .t { font-size: 14px; font-weight: 600; white-space: nowrap; overflow: hidden; text-overflow: ellipsis; flex: 1; min-width: 0; }
  .hint { margin: -12px 0 -8px; }
  .pinmark { position: absolute; top: 6px; right: 6px; width: 22px; height: 22px; border-radius: 50%; display: grid; place-items: center; background: color-mix(in srgb, var(--velvet) 72%, transparent); color: var(--limelight); }
  .pin-inline { color: var(--dust); display: grid; place-items: center; flex: none; }
  /* A tile wearing its app's banner: a deeper scrim with the title on it, so it reads as something from the app. */
  .tile.borrowed .scrim { inset: 0; padding-top: 0; display: flex; flex-direction: column; justify-content: flex-end; background: linear-gradient(to top, color-mix(in srgb, var(--velvet) 94%, transparent), color-mix(in srgb, var(--velvet) 70%, transparent) 55%, color-mix(in srgb, var(--velvet) 10%, transparent)); }
</style>
