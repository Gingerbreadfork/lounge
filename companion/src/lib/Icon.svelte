<script lang="ts">
  // Stroke icons on the same 28-unit grid as the TV's icon set, so both screens share one hand.
  let { name, size = 24 }: { name: string; size?: number } = $props();

  type Shape = { d: string; fill?: boolean };
  const speaker: Shape = { d: 'M4 11h4l5-4v14l-5-4H4z', fill: true };
  const icons: Record<string, Shape[]> = {
    rewind: [{ d: 'M14 7L5 14l9 7z', fill: true }, { d: 'M23 7l-9 7 9 7z', fill: true }],
    fast_forward: [{ d: 'M5 7l9 7-9 7z', fill: true }, { d: 'M14 7l9 7-9 7z', fill: true }],
    play_pause: [{ d: 'M3 6l9 8-9 8z', fill: true }, { d: 'M15.5 6h3v16h-3z', fill: true }, { d: 'M21 6h3v16h-3z', fill: true }],
    volume_down: [speaker, { d: 'M17 10a5 5 0 0 1 0 8' }],
    volume_up: [speaker, { d: 'M17 10a5 5 0 0 1 0 8' }, { d: 'M20.5 7a9.5 9.5 0 0 1 0 14' }],
    mute: [speaker, { d: 'M17.5 11l6 6M23.5 11l-6 6' }],
    mic: [{ d: 'M14 3a4 4 0 0 1 4 4v6a4 4 0 0 1-8 0V7a4 4 0 0 1 4-4z' }, { d: 'M6 13a8 8 0 0 0 16 0' }, { d: 'M14 21v4M10 25h8' }],
    up: [{ d: 'M14 22V6M7 13l7-7 7 7' }],
    down: [{ d: 'M14 6v16M7 15l7 7 7-7' }],
    left: [{ d: 'M22 14H6M13 7l-7 7 7 7' }],
    right: [{ d: 'M6 14h16M15 7l7 7-7 7' }],
    back: [{ d: 'M12 6l-7 7 7 7' }, { d: 'M5 13h11a7 7 0 0 1 0 14h-3' }],
    home: [{ d: 'M4 13L14 4l10 9' }, { d: 'M7 11v12h14V11' }, { d: 'M12 23v-6h4v6' }],
    menu: [{ d: 'M5 8h18M5 14h18M5 20h18' }],
  };
  let shapes = $derived(icons[name] ?? []);
</script>

<svg width={size} height={size} viewBox="0 0 28 28" aria-hidden="true" focusable="false">
  {#each shapes as s}
    <path d={s.d} fill={s.fill ? 'currentColor' : 'none'} stroke={s.fill ? 'none' : 'currentColor'} stroke-width="2" stroke-linecap="round" stroke-linejoin="round" />
  {/each}
</svg>
