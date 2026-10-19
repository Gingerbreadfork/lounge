<script lang="ts">
  import { api, argb, art, type Tile } from '../lib/api';
  import { act, app, connectionLabel } from '../lib/state.svelte';

  let snapshot = $derived(app.state?.snapshot ?? null);
  let spotlight = $derived(snapshot?.stage.spotlights[0] ?? null);
  let np = $derived(snapshot?.nowPlaying ?? null);

  function open(t: Tile) {
    act(`Opening ${t.title} on the TV`, () => api.open(t.action));
  }
  function media(cmd: string) {
    if (!np) return;
    act('', () => api.open(`media:${np!.package}:${cmd}`));
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

  {#if np}
    <section class="card np" style="--light: {argb(np.lightArgb)}">
      <div class="np-art">{#if np.artPath}<img src={art(np.artPath)} alt="" />{/if}</div>
      <div class="np-text">
        <p class="caption">{np.appLabel}</p>
        <p class="np-title">{np.title}</p>
        {#if np.subtitle}<p class="meta">{np.subtitle}</p>{/if}
        <div class="np-controls">
          <button class="round" disabled={!np.canSkipPrevious} onclick={() => media('previous')} aria-label="Previous">⏮</button>
          <button class="round big" onclick={() => media('play-pause')} aria-label={np.status === 'PLAYING' ? 'Pause' : 'Play'}>{np.status === 'PLAYING' ? '❚❚' : '▶'}</button>
          <button class="round" disabled={!np.canSkipNext} onclick={() => media('next')} aria-label="Next">⏭</button>
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
        <p class="meta">{spotlight.category}</p>
        <h2>{spotlight.title}</h2>
        {#each spotlight.lines as line}<p class="meta">{line}</p>{/each}
        {#if progressPct(spotlight.progress) != null}
          <div class="progress"><i style="width: {progressPct(spotlight.progress)}%"></i></div>
        {/if}
        <div class="actions">
          <button class="btn primary" onclick={() => act(`${spotlight!.primary.label} on the TV`, () => api.open(spotlight!.primary.action))}>{spotlight.primary.label}</button>
          {#if spotlight.secondary}<button class="btn" onclick={() => act('', () => api.open(spotlight!.secondary!.action))}>{spotlight.secondary.label}</button>{/if}
        </div>
      </div>
    </section>
  {/if}

  {#if snapshot}
    {#if snapshot.stage.strip.length}
      <section class="section">
        <div class="section-head"><h3>Stage</h3><span class="caption">{snapshot.stage.strip.length}</span></div>
        <div class="row-scroller">
          {#each snapshot.stage.strip as t (t.id)}
            <button class="tile landscape" style="--light: {argb(t.lightArgb)}" onclick={() => open(t)}>
              {#if t.artworkPath}<img src={art(t.artworkPath)} alt="" loading="lazy" />{/if}
              {#if t.kind !== 'APP_BANNER' || !t.artworkPath}<div class="scrim"><div class="t">{t.title}</div>{#if t.subtitle}<div class="s">{t.subtitle}</div>{/if}</div>{/if}
              {#if t.progress != null}<div class="bar"><i style="width: {progressPct(t.progress)}%"></i></div>{/if}
            </button>
          {/each}
        </div>
      </section>
    {/if}
    {#each snapshot.rows as row (row.id)}
      <section class="section">
        <div class="section-head"><h3>{row.title}</h3>{#if row.badge === 'LIVE'}<span class="pill bad"><span class="dot"></span>Live</span>{/if}</div>
        <div class="row-scroller">
          {#each row.tiles as t (t.id)}
            <button class="tile {row.kind.toLowerCase()}" style="--light: {argb(t.lightArgb)}" onclick={() => open(t)}>
              {#if row.kind === 'COMPACT'}
                {#if t.iconPath}<img class="icon" src={art(t.iconPath)} alt="" loading="lazy" />{/if}
                <span class="t">{t.title}</span>
              {:else}
                {#if t.artworkPath}<img src={art(t.artworkPath)} alt="" loading="lazy" />{:else}<div class="fill" style="background: color-mix(in srgb, {argb(t.lightArgb)} 35%, var(--curtain))"></div>{/if}
                {#if t.badge === 'LIVE'}<span class="badge"><span class="dot"></span>Live</span>{/if}
                {#if row.kind !== 'APP_BANNER' && row.kind !== 'POSTER'}<div class="scrim"><div class="t">{t.title}</div>{#if t.subtitle}<div class="s">{t.subtitle}</div>{/if}</div>{/if}
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

<style>
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
  .round { width: 40px; height: 40px; border-radius: 50%; background: var(--velvet); color: var(--limelight); font-size: 14px; display: grid; place-items: center; }
  .round.big { width: 48px; height: 48px; background: var(--light); color: var(--velvet); font-size: 16px; }
  .round:disabled { opacity: 0.35; }
  .tile .fill { position: absolute; inset: 0; }
  .tile .icon { width: 28px; height: 28px; border-radius: 7px; flex: none; object-fit: cover; }
  .tile.compact .t { font-size: 14px; font-weight: 600; white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
</style>
