<script module lang="ts">
  // Stroke icons on the same 28-unit grid as the TV's icon set, so both screens share one hand.
  type Shape = { d: string; fill?: boolean };
  const speaker: Shape = { d: 'M4 11h4l5-4v14l-5-4H4z', fill: true };
  const icons: Record<string, Shape[]> = {
    rewind: [{ d: 'M14 7L5 14l9 7z', fill: true }, { d: 'M23 7l-9 7 9 7z', fill: true }],
    fast_forward: [{ d: 'M5 7l9 7-9 7z', fill: true }, { d: 'M14 7l9 7-9 7z', fill: true }],
    play_pause: [{ d: 'M3 6l9 8-9 8z', fill: true }, { d: 'M15.5 6h3v16h-3z', fill: true }, { d: 'M21 6h3v16h-3z', fill: true }],
    play: [{ d: 'M9 5l13 9-13 9z', fill: true }],
    pause: [{ d: 'M7 5h4.5v18H7z', fill: true }, { d: 'M16.5 5H21v18h-4.5z', fill: true }],
    skip_previous: [{ d: 'M6 5h3v18H6z', fill: true }, { d: 'M22 5l-12 9 12 9z', fill: true }],
    skip_next: [{ d: 'M19 5h3v18h-3z', fill: true }, { d: 'M6 5l12 9-12 9z', fill: true }],
    volume_down: [speaker, { d: 'M17 10a5 5 0 0 1 0 8' }],
    volume_up: [speaker, { d: 'M17 10a5 5 0 0 1 0 8' }, { d: 'M20.5 7a9.5 9.5 0 0 1 0 14' }],
    mute: [speaker, { d: 'M17.5 11l6 6M23.5 11l-6 6' }],
    mic: [{ d: 'M14 3a4 4 0 0 1 4 4v6a4 4 0 0 1-8 0V7a4 4 0 0 1 4-4z' }, { d: 'M6 13a8 8 0 0 0 16 0' }, { d: 'M14 21v4M10 25h8' }],
    up: [{ d: 'M14 22V6M7 13l7-7 7 7' }],
    down: [{ d: 'M14 6v16M7 15l7 7 7-7' }],
    left: [{ d: 'M22 14H6M13 7l-7 7 7 7' }],
    right: [{ d: 'M6 14h16M15 7l7 7-7 7' }],
    back: [{ d: 'M12 6l-7 7 7 7' }, { d: 'M5 13h11a7 7 0 0 1 0 14h-3' }],
    home: [{ d: 'M4 13L14 4.5 24 13' }, { d: 'M7 11v12.5h14V11' }, { d: 'M12 23.5V17h4v6.5' }],
    menu: [{ d: 'M5 8h18M5 14h18M5 20h18' }],
    grip: [{ d: 'M10 7h.01M18 7h.01M10 14h.01M18 14h.01M10 21h.01M18 21h.01' }],
    show: [{ d: 'M3 14c3-6 7-8 11-8s8 2 11 8c-3 6-7 8-11 8s-8-2-11-8z' }, { d: 'M14 10.5a3.5 3.5 0 1 1 0 7 3.5 3.5 0 1 1 0-7z' }],
    hide: [{ d: 'M3 14c3-6 7-8 11-8s8 2 11 8c-3 6-7 8-11 8s-8-2-11-8z' }, { d: 'M5 23L23 5' }],
    pin: [{ d: 'M10 3h8l-1 8 4 4H7l4-4z' }, { d: 'M14 15v10' }],
    rows: [{ d: 'M4 7h20M4 14h20M4 21h20' }],
    close: [{ d: 'M7 7l14 14M21 7L7 21' }],
    open: [{ d: 'M9 5l13 9-13 9z', fill: true }],
    // The glyphs a plugin's control may name, as the TV draws them.
    battery: [{ d: 'M5.5 9h15a2 2 0 0 1 2 2v6a2 2 0 0 1-2 2h-15a2 2 0 0 1-2-2v-6a2 2 0 0 1 2-2z' }, { d: 'M25 12.5v3' }, { d: 'M7.5 12.5v3' }],
    blind: [{ d: 'M4 5h20' }, { d: 'M6 5v16.5h16V5' }, { d: 'M6 9.5h16M6 13.5h16M6 17.5h16' }, { d: 'M14 21.5v3' }],
    camera: [{ d: 'M5.5 8.5h13a2 2 0 0 1 2 2v7a2 2 0 0 1-2 2h-13a2 2 0 0 1-2-2v-7a2 2 0 0 1 2-2z' }, { d: 'M20.5 12.5l4.5-2.5v8l-4.5-2.5' }],
    door: [{ d: 'M7 24.5v-19A1.5 1.5 0 0 1 8.5 4h11A1.5 1.5 0 0 1 21 5.5v19' }, { d: 'M4.5 24.5h19' }, { d: 'M16.2 15a1.3 1.3 0 1 0 2.6 0a1.3 1.3 0 1 0 -2.6 0z', fill: true }],
    drop: [{ d: 'M14 3.5C14 3.5 6.5 12 6.5 16.5a7.5 7.5 0 0 0 15 0C21.5 12 14 3.5 14 3.5z' }],
    fan: [{ d: 'M11.8 14a2.2 2.2 0 1 0 4.4 0a2.2 2.2 0 1 0 -4.4 0z' }, { d: 'M14 11.6C14.6 7.5 12.5 3.8 9.6 4.6C7.2 5.3 8 9.6 12 12.4' }, { d: 'M16.1 15.2C19.3 17.8 23.6 17.8 24.3 14.9C24.9 12.5 20.8 11 16.4 13.1' }, { d: 'M11.9 15.2C8.1 16.7 5.9 20.4 8.1 22.5C9.9 24.2 13.2 21.4 13.6 16.5' }],
    garage: [{ d: 'M3.5 12L14 4.5 24.5 12' }, { d: 'M6 10.5V24M22 10.5V24' }, { d: 'M9.5 24v-9h9v9' }, { d: 'M9.5 18h9M9.5 21h9' }],
    humidity: [{ d: 'M8 23c-2-3 2-5 0-8s2-5 0-8' }, { d: 'M14 23c-2-3 2-5 0-8s2-5 0-8' }, { d: 'M20 23c-2-3 2-5 0-8s2-5 0-8' }],
    light: [{ d: 'M10 17.5C7.5 15.9 6 13.3 6 10.5a8 8 0 0 1 16 0c0 2.8-1.5 5.4-4 7V20h-8z' }, { d: 'M11 23.5h6' }],
    lock: [{ d: 'M8 12.5h12a2.5 2.5 0 0 1 2.5 2.5v6.5A2.5 2.5 0 0 1 20 24H8a2.5 2.5 0 0 1-2.5-2.5V15A2.5 2.5 0 0 1 8 12.5z' }, { d: 'M9.5 12.5V9a4.5 4.5 0 0 1 9 0v3.5' }, { d: 'M12.6 18.3a1.4 1.4 0 1 0 2.8 0a1.4 1.4 0 1 0 -2.8 0z', fill: true }],
    motion: [{ d: 'M12 14a2 2 0 1 0 4 0a2 2 0 1 0 -4 0z', fill: true }, { d: 'M9.5 9.5a6.4 6.4 0 0 0 0 9M18.5 9.5a6.4 6.4 0 0 1 0 9' }, { d: 'M6 6a11.3 11.3 0 0 0 0 16M22 6a11.3 11.3 0 0 1 0 16' }],
    person: [{ d: 'M10 9a4 4 0 1 0 8 0a4 4 0 1 0 -8 0z' }, { d: 'M5.5 24a8.5 8.5 0 0 1 17 0' }],
    plug: [{ d: 'M7.5 4h13A3.5 3.5 0 0 1 24 7.5v13a3.5 3.5 0 0 1-3.5 3.5h-13A3.5 3.5 0 0 1 4 20.5v-13A3.5 3.5 0 0 1 7.5 4z' }, { d: 'M11 10.5v3.5M17 10.5v3.5' }, { d: 'M12.5 18.5h3' }],
    scene: [{ d: 'M14 4c.8 5.5 4.5 9.2 10 10-5.5.8-9.2 4.5-10 10-.8-5.5-4.5-9.2-10-10 5.5-.8 9.2-4.5 10-10z' }],
    script: [{ d: 'M5 7h10M5 12h7M5 17h5' }, { d: 'M16 13l7 4.5-7 4.5z' }],
    shield: [{ d: 'M14 3.5l8.5 3v7c0 5-3.5 9-8.5 11-5-2-8.5-6-8.5-11v-7z' }],
    smoke: [{ d: 'M4 14a10 10 0 1 0 20 0a10 10 0 1 0 -20 0z' }, { d: 'M10.4 14a3.6 3.6 0 1 0 7.2 0a3.6 3.6 0 1 0 -7.2 0z' }, { d: 'M14 6.5v1.5M14 20v1.5M6.5 14H8M20 14h1.5' }],
    speaker: [{ d: 'M4 11h5l6-5v16l-6-5H4z' }, { d: 'M19 10a5 5 0 0 1 0 8' }],
    sun: [{ d: 'M9.5 14a4.5 4.5 0 1 0 9 0a4.5 4.5 0 1 0 -9 0z' }, { d: 'M14 3.5V6M14 22v2.5M3.5 14H6M22 14h2.5M6.6 6.6l1.8 1.8M19.6 19.6l1.8 1.8M6.6 21.4l1.8-1.8M19.6 8.4l1.8-1.8' }],
    thermostat: [{ d: 'M14 4a2.5 2.5 0 0 1 2.5 2.5v8.7a4.5 4.5 0 1 1-5 0V6.5A2.5 2.5 0 0 1 14 4z' }, { d: 'M12.2 19a1.8 1.8 0 1 0 3.6 0a1.8 1.8 0 1 0 -3.6 0z', fill: true }, { d: 'M19.5 7.5H22M19.5 11H22' }],
    toggle: [{ d: 'M9 8h10a6 6 0 0 1 0 12H9A6 6 0 0 1 9 8z' }, { d: 'M16.4 14a2.6 2.6 0 1 0 5.2 0a2.6 2.6 0 1 0 -5.2 0z', fill: true }],
    tv: [{ d: 'M5.5 5h17A2.5 2.5 0 0 1 25 7.5v10a2.5 2.5 0 0 1-2.5 2.5h-17A2.5 2.5 0 0 1 3 17.5v-10A2.5 2.5 0 0 1 5.5 5z' }, { d: 'M10 24h8' }],
    vacuum: [{ d: 'M4 14a10 10 0 1 0 20 0a10 10 0 1 0 -20 0z' }, { d: 'M7.2 9.5a8.3 8.3 0 0 1 13.6 0' }, { d: 'M11.8 15.5a2.2 2.2 0 1 0 4.4 0a2.2 2.2 0 1 0 -4.4 0z' }],
    window: [{ d: 'M5 4.5h18v19H5z' }, { d: 'M14 4.5v19M5 14h18' }],
    thing: [{ d: 'M8 14a6 6 0 1 0 12 0a6 6 0 1 0 -12 0z' }],
  };

  /** Whether an icon is drawn under this name. */
  export function drawn(name: string | null | undefined): name is string {
    return !!name && Object.hasOwn(icons, name);
  }
</script>

<script lang="ts">
  let { name, size = 24 }: { name: string; size?: number } = $props();
  let shapes = $derived(Object.hasOwn(icons, name) ? icons[name] : []);
</script>

<svg width={size} height={size} viewBox="0 0 28 28" aria-hidden="true" focusable="false">
  {#each shapes as s, i (i)}
    <path d={s.d} fill={s.fill ? 'currentColor' : 'none'} stroke={s.fill ? 'none' : 'currentColor'} stroke-width="2" stroke-linecap="round" stroke-linejoin="round" />
  {/each}
</svg>
