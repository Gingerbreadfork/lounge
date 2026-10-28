# Lounge — Design system

This document is the authority for how Lounge looks, moves, sounds and speaks. Code follows it; when code and this document disagree, fix the code or change this document deliberately, never silently. Working title throughout: Lounge.

## 1. The idea

**The room is dark. The content is the light.**

A living room at night is a dim space with one bright thing in it. Lounge treats the home screen the same way. The backdrop is a deep, warm dark (velvet, not black). Artwork is the only light source: the focused tile is brightest, it spills coloured light onto the backdrop and its neighbours, and everything else sits a step back as if the house lights were down. Move the focus and the light moves with it.

That is the one bold idea. Everything else is quiet and disciplined so that this reads clearly: typography with character but no tricks, structure that encodes meaning (a border or a rule only when it tells you something), and motion that answers the remote.

Why it fits: it makes the user's own content, not our chrome, the hero. It is dynamic without video or ads. It is cheap: one radial light and a per-tile brightness curve, not a decoder. It gives the launcher an identity nobody else has.

## 2. Principles

1. **Light follows focus.** Focus is never a border. The focused element is the brightest thing on screen and the only one casting light.
2. **Content first, chrome last.** Full-bleed artwork, left-aligned text, controls that appear where they're needed. Nothing is decorated; every rule, badge or line carries information.
3. **One screen, one job.** The Stage answers "what should I watch now?". Rows answer "what else have I got?". Overlays answer one question each and then leave.
4. **The remote is the hand.** Every state is reachable with five keys (D-pad, OK) and a controller. Back always goes up one level; Home always returns to the Stage with focus restored.
5. **Earn every motion.** Motion shows what changed. Ambient motion is allowed only below the threshold of attention (the light field's clouds, the lowered Stage's light, a stepped push-in in the screensaver, the Now Playing bars at eight frames a second) and it stops after a minute without input. The orchestrated moments are the lamp warming up at launch, the Stage changing, the proscenium lowering, and the idle dim.
6. **Readable from the sofa.** Minimum 22 px type at 1080p, contrast at or above 4.5:1 for text, 3:1 for large display text, no thin weights.

## 3. Colour

### 3.1 Palette (static tokens)

The default theme is **Dusk**: the room after sunset, indigo walls going plum toward one corner, one coral lamp. It is the room the mark's sofa sits in, and its gradient is the sofa's own, slowed down to a whisper.

| Token | Dusk | Role |
|---|---|---|
| `velvet` | `#120E27` | Backdrop. Deep indigo, the walls with the lights down. |
| `horizon` | `#1C1130` | The backdrop's far corner: the light field runs `velvet` into this toward the bottom right, so the room is never one flat wall. Only the field draws it; every opaque fill is `velvet`. |
| `curtain` | `#211A3E` | Raised surfaces: side panels, overlay sheets, the playbill column, pills. |
| `curtain-edge` | `#342A58` | 1 px hairline where a surface must be separated from the backdrop (overlays only). |
| `limelight` | `#F6EFE7` | Primary text and icons. Warm white; never pure `#FFFFFF`. |
| `dust` | `#A8A0C2` | Secondary text, metadata, inactive icons. Lavender grey. |
| `houselight` | `#FFA86B` | The house accent: the coral lamp. Used when no artwork light is available (empty states, settings, onboarding, tiles without a palette) and for progress fills. |
| `ember` | `#FF5468` | The single semantic red: live indicator, recording, destructive confirmation. |
| `signal` | `#7CDFB3` | The single semantic green: connected, online, success. |

Contrast on Dusk `velvet` (WCAG relative luminance): `limelight` 16.5:1, `dust` 7.6:1, `houselight` 10.3:1, `ember` 6.0:1, `signal` 11.7:1. On `curtain`: `limelight` 14.4:1, `dust` 6.6:1. `velvet` on `houselight` 10.3:1, so buttons on the lamp take `velvet` text.

**House**, the warm alternative: `velvet` `#1A1016`, `curtain` `#2A1B24`, `curtain-edge` `#3B2A34`, `dust` `#A89AA3`, `houselight` `#F2B263` (tungsten), `ember` `#E35D5B`, `signal` `#7FD3A8`; `horizon` equals `velvet`. Contrast on its `velvet`: `limelight` 16.3:1, `dust` 6.9:1, `houselight` 10.0:1, `ember` 5.3:1, `signal` 10.4:1. The core's unit tests check these House constants, which are also the core's fallback light; the UI maps that fallback to the current theme's `houselight`.

OLED theme ("Blackout"): House with `velvet` → `#000000`, `curtain` → `#161016`, everything else unchanged. Light spill alpha is halved so the black stays black away from the focus.

**The ten rooms.** Every theme is a dim room with one lamp; they differ in the colour of the walls and the colour of the light, never in layout, motion or the light model. `limelight` is the same warm white in all of them, `horizon` sits within 5 % of `velvet`, and every one clears the same bar: `limelight` ≥ 15.9:1 and `dust` ≥ 6.9:1 on `velvet`, `houselight` ≥ 7.5:1 both ways, `ember` ≥ 5.3:1, `signal` ≥ 10.4:1.

| Theme | The room | `velvet` | `horizon` | `curtain` / edge | `dust` | `houselight` | `ember` | `signal` |
|---|---|---|---|---|---|---|---|---|
| Dusk (default) | Indigo night, coral lamp | `#120E27` | `#1C1130` | `#211A3E` / `#342A58` | `#A8A0C2` | `#FFA86B` | `#FF5468` | `#7CDFB3` |
| House | Warm plum, tungsten light | `#1A1016` | = velvet | `#2A1B24` / `#3B2A34` | `#A89AA3` | `#F2B263` | `#E35D5B` | `#7FD3A8` |
| Hearth | Umber walls, firelight | `#17100D` | `#22130D` | `#291C17` / `#3D2B23` | `#B3A196` | `#FF7E4D` | `#FF4A52` | `#8ED9A0` |
| Opera | Red velvet and gilt | `#1E0B12` | `#26101A` | `#311520` / `#48222F` | `#B89DA6` | `#F3D08C` | `#FF5C63` | `#86DBAE` |
| Marquee | Navy, marquee bulbs | `#0C1226` | `#141A30` | `#172040` / `#26305A` | `#A1A9C4` | `#FFC85C` | `#FF5A6A` | `#7FE0B6` |
| Green room | Deep green, brass lamp | `#0E1814` | `#131F1C` | `#192822` / `#283B33` | `#A0B2A8` | `#E9C47F` | `#FF6260` | `#8FE8BC` |
| Moonlight | Cool slate, moonlight | `#10161F` | `#151C27` | `#1B2430` / `#2B3747` | `#A3ADBB` | `#C4D9F2` | `#FF6773` | `#86DEB1` |
| Graphite | No colour, warm white | `#141414` | `#191919` | `#222222` / `#343434` | `#A6A3A0` | `#E8DFD4` | `#F25C5C` | `#86D9AC` |
| Blackout | True black for OLED panels | `#000000` | = velvet | `#161016` / `#3B2A34` | `#A89AA3` | `#F2B263` | `#E35D5B` | `#7FD3A8` |
| High contrast | Bolder text and edges | `#000000` | = velvet | `#2A1B24` / `#3B2A34` | `#D5CBD1` | `#F2B263` | `#E35D5B` | `#7FD3A8` |

Settings cycles them in this order; onboarding shows them as a two-column grid with the live preview beside it.

### 3.2 Artwork light (dynamic tokens)

Every artwork gets a palette at index time (Rust, `lounge-artwork`): `dominant`, `vibrant`, `muted`. From these we derive, per tile:

- `light`: the colour the tile emits. Take `vibrant` if its chroma is above 0.12, else `dominant`; clamp lightness to 0.55–0.72 so dark posters still glow and white posters don't blind. Fallback: `houselight`.
- `light-text`: `limelight` or `velvet`, whichever contrasts ≥ 4.5:1 against `light` (used on primary buttons sitting on the light).

The light is applied in three places when a tile is focused:

| Effect | Spec |
|---|---|
| Glow | Outer shadow, `light` at 34 % alpha, blur 32 px, spread 0, offset 0 0. |
| Shader wallpaper | Beneath the spill: a slow field of `light`-tinted clouds over the ground (`velvet` running into `horizon` toward the bottom right) at ±4 % luminance with a soft leak around the focus and fine grain; drifts imperceptibly (one cycle ≈ a minute), quieter by day, half in Blackout, still under reduced motion, absent in Lite. It is texture for the dark, never a picture. |
| Backdrop spill | Radial gradient centred on the tile's centre, radius 1100 px, `light` at 16 % alpha fading to 0, drawn over `velvet` behind all rows. Moves with focus over 220 ms. Where the shader wallpaper runs, the ground is drawn at the screen's own size from a baked noise tile and redrawn only when its clock steps, the spill and the playbill's light are a half-size layer over it since they move every frame of a focus change, and the dither goes over both at full size (`LightField`); the look is identical. |
| Neighbour lift | Tiles immediately left and right rise from 0.78 to 0.86 brightness (see 6.1), as if lit from the side. |

The Stage's playbill column takes a stronger version: `light` at 22 % alpha spilling leftward from the artwork edge, so the title appears lit by the picture.

Dynamic colour never recolours text, icons or the house palette. It is light, not paint.

## 4. Typography

Two families, clearly distinct, both SIL Open Font License, shipped as variable fonts in the APK.

- **Bricolage Grotesque** (display). Wide, confident, a touch theatrical, with optical sizes. Used for the Stage title, row headers, the clock, screen titles and large numbers. Weight 600–700 (500 for the clock), optical size axis set to the rendered size, width axis 100 (the family's range is 75–100).
- **Figtree** (text). Friendly, legible, quiet. Everything else: metadata, buttons, descriptions, settings, the guide. Weights 400–600. Tabular numerals (`tnum`) on in the guide, Now Playing times and the clock seconds.

### 4.1 Scale at 1080p

| Style | Family | Size / line | Weight | Tracking | Use |
|---|---|---|---|---|---|
| `display-xl` | Bricolage | 72 / 76 | 700 | −0.02 em | Stage title (max 2 lines) |
| `display-l` | Bricolage | 48 / 54 | 650 | −0.015 em | Screen titles, Live Guide title, Now Playing title |
| `headline` | Bricolage | 34 / 40 | 600 | −0.01 em | Row headers, overlay headings |
| `clock` | Bricolage | 40 / 40 | 500 | 0 | Top-bar clock |
| `title` | Figtree | 28 / 34 | 600 | 0 | Tile titles in landscape tiles, button labels, guide programme names |
| `body` | Figtree | 26 / 34 | 400 | 0 | Descriptions, settings values |
| `meta` | Figtree | 24 / 30 | 500 | 0 | Metadata lines, subtitles, "why this" line |
| `caption` | Figtree | 22 / 26 | 500 | +0.01 em | Badges, timestamps, channel numbers (the floor: nothing smaller) |

Text scaling: the system font scale is honoured up to 1.3×; above that, row heights grow and the Stage title drops to one line with an ellipsis. At 4K-rendered UIs all values are doubled (density handles this).

### 4.2 Rules

- Sentence case everywhere. No all-caps labels, ever, including badges ("Live", not "LIVE").
- No eyebrow labels above headings. A row header is the heading.
- Metadata is written as a sentence fragment with commas or as two lines, never joined with dots or slashes: "Season 2, Episode 4" on one line, "23 minutes left" on the next.
- Numbers that change (clock, time left, channel numbers) use tabular figures so they don't jitter.
- Titles are never truncated mid-word at display sizes; wrap to two lines first, then ellipsis.

## 5. Layout

Canvas: 1920 × 1080 logical pixels (both reference devices render the UI at 1080p; 4K-rendered devices scale by density). Safe area: 96 px left and right, 54 px top and bottom. Everything left-aligns to x = 96. Right edges bleed off-screen intentionally (a partially visible tile is the scroll affordance).

### 5.1 Home, Stage expanded (focus in the Stage strip)

```
y 0    ┌──────────────────────────────────────────────────────────────────────────────┐
       │ (54)                                                                          │
y 54   │ ◖ Lounge                                 Search   TV controls   ⚙    21:42   │  top bar, 48 high
y 102  │                                                                               │
y 136  │ ┌─ playbill (560 w) ─────────┐  ┌─ artwork 1120 × 630 ───────────────────────┐│
       │ │ Continue watching          │  │                                            ││
       │ │                            │  │                                            ││
       │ │ The Bear                   │  │            (artwork, 16:9)                 ││
       │ │ (display-xl, 2 lines max)  │  │                                            ││
       │ │                            │  │                                            ││
       │ │ Season 2, Episode 4        │  │                                            ││
       │ │ 23 minutes left            │  │                                            ││
       │ │ ▮▮▮▮▮▮▮▯▯▯  (progress)     │  │                                            ││
       │ │ [ Continue ]  [ Details ]  │  │                                            ││
       │ │ Because you watch at 9 pm  │  └────────────────────────────────────────────┘│
y 766  │ └────────────────────────────┘                                                │
y 798  │ ┌────┐ ┌────┐ ┌────┐ ┌────┐ ┌────┐ ┌────┐ ┌──     stage strip: 6 × (232×130)  │
y 928  │ └────┘ └────┘ └────┘ └────┘ └────┘ └────┘ └──     gap 24, focused = shown above│
y 960  │ Your apps                                              (row header, peeking)  │
y 1004 │ ┌──────┐ ┌──────┐ ┌──────┐ ┌──────┐ ┌──────┐ ┌─      (tiles cut by the fold) │
y 1080 └──────────────────────────────────────────────────────────────────────────────┘
```

- The playbill is the Stage's text column. Level with the artwork's top edge sits the source chip: the provider's icon at 40 px (radius 10) and its name in `meta`/`dust`, saying where OK will take you. Bottom-aligned: category line (`meta`, `dust`), title (`display-xl`, wraps to two lines before an ellipsis), metadata lines (`meta`: "Season 1, Episode 6: The Fly", then "23 minutes left"), a 6 px progress bar (only when there is progress), primary and secondary buttons, and the explanation line ("Because you watch at 9 pm", `caption`, `dust`) which is present only when the ranking has a reason. Every slot keeps its full height whatever the item (two title lines, two metadata lines, the bar, the explanation), so moving along the strip changes the words and nothing else moves. The source never appears among the lines.
- The artwork is 1120 × 630 with a 20 px radius, its vignette baked in at index time; it is the brightest object on screen and the spill source for the playbill.
- The stage strip is the Stage's own row: 6 landscape tiles, 232 × 130, radius 12, artwork only (no title, no scrim: the title is the 72 px heading above). A strip tile with no artwork keeps its icon and one `meta` line. The focused tile is what the Stage shows. Moving along the strip crossfades the Stage (6.3).
- The first content row's header and the top 76 px of its tiles are visible under the Stage: the fold is honest about there being more.

### 5.2 Home, Stage collapsed (focus in any row)

The Stage lowers to a 200 px band at y 136, full width between the safe margins: the artwork's frame travels from 1120 × 630 at x 704 to 1728 × 200 at x 96 over `proscenium`, and as it lowers the picture goes out of focus into its own light. The playbill and strip fade out over the first 60 % of the lowering and back in over the last 60 % of the raising; rows slide up beneath. UP from the first row focuses the band and raises the Stage again, the light focusing back into the picture.

- **The light.** At index time the key art is read in six columns across a strip around its focal line (the horizontal band of the 16:9 frame with the most edge energy, weighted toward the upper third). Each column's colour leans toward its most saturated pixels, so a yellow coat survives a grey park, and is toned as light: OKLab lightness 0.2–0.5, chroma lifted by half up to 0.17. The six pools ease into one another across the band in OKLab. Over them the picture's `light` blooms right of middle, wider than tall; the top, bottom and right edges fall away as if seen through glass, and blue-noise dither keeps the gradients from banding. A picture without colour blooms silver, never the house lamp; one without key art is a single pool of its `light`.
- **The text.** The left 20 % is the room's own `velvet`, faintly lit by the picture (8 %), and the colour rises over the next half. The title is `display-l` in `limelight` at x 48, one line; 18 px below it the 6 px progress bar, 300 wide, with how long is left beside it in `meta` at 84 %. With no progress the line is the first metadata line. Text sits at 9:1 or better on the band.
- **The lowering.** The light takes over from the picture between 85 % and 20 % of the way down; because the pools come from the picture's own columns, its colours land where they were and the change reads as a defocus. The title and bar fade in over the last half, riding in with the frame's left edge so the frame never cuts them.
- **The drift.** The pools sway sideways and the bloom wanders and breathes, each on its own long period (29–71 s), on the light field's clock: stepped, below the threshold of attention, still after a minute without input, under reduced motion and in Lite. Each part is drawn once into a layer and the drift only moves layers, so it costs no redraw.

### 5.3 Rows

- Header: `headline`, left at x 96, 58 px tall: a 40 px line and 18 px of clear air, so a focused tile lifts into space, not type. The focused row's header is `limelight`; every other header is `limelight` at 62 %, animated over `settle`, so moving between rows reads. A row header may carry one trailing badge: a live dot (`ember`, 10 px, pulsing per 7.3) for Live now; a controller glyph for Games when a controller is connected.
- Tiles start at x 96, gutter 24, and run off the right edge. Row vertical gap 32 px.
- Every focusable container reserves 56 px of glow room on all sides; light is never clipped. A d-pad press off either end of a row nudges the focused tile 6 px that way over `snap` and back over `settle`, without a sound.
- Tile styles and sizes (width × height, radius 12 everywhere except the Stage):

| Style | S | M (default) | L |
|---|---|---|---|
| Poster 2:3 | 176 × 264 | 208 × 312 | 248 × 372 |
| Landscape 16:9 | 320 × 180 | 384 × 216 | 448 × 252 |
| App banner 16:9 | 256 × 144 | 288 × 162 | 320 × 180 |
| Compact (icon + text) | 288 × 72 | 320 × 80 | 352 × 88 |

- A row with nothing to show is not rendered. There are no empty rows and no placeholder tiles.
- Focused tile scale 1.06 (poster/landscape/app), 1.04 (compact). Rows do not scroll vertically until the focused row would leave the viewport; the focused row sits at a fixed y when scrolling (the "focus line" at y 334 in collapsed mode) so the eye never hunts.

### 5.4 Overlays

Overlays are `curtain` sheets with a 1 px `curtain-edge` top or left hairline, slid in from the edge they belong to, over a 40 % `velvet` scrim. They never cover the whole screen. Every overlay has an enter and an exit; focus moves at the start of the exit, so the key press is answered at once and the motion follows.

- **Right sheets** (TV controls / Inputs, Quick settings, and everything Settings opens) — 560 wide. Enter: slide from +560 px with a fade over `proscenium`; scrim 0 → 1 over 200 ms. Exit: slide and fade over `settle`; scrim out over 180 ms. One panel serves them all: moving between sheets (Settings → Plugins → a plugin) crossfades the content over 180 ms with an 8 px shift in the direction of travel while the panel stays put.
- **Live Guide** — bottom sheet, 680 tall, grid of channels × time. Slides up over `proscenium`, down over `settle`.
- **Now Playing** — bottom-right card, 640 × 200, artwork left, title/artist, transport buttons. Rises half its height over `proscenium`, leaves over `settle`.
- **Long-press tile menu** — anchored popover beside the tile, 360 wide, `curtain`, radius 16, header with the item's icon at 40 px and its title in `title`, items with 56 px rows. It grows out of the edge facing the tile (scale 0.96 → 1, 8 px slide, fade) over `snap`; exit is a 90 ms fade. There is no scrim: the room dims with house lights (every tile drops to brightness 0.60, the band and top bar to 50 %), and the tile that opened it stays lit.
- **Sheet rows** are 72 px: glyph, `title`, optional value in `meta`; a row with a second line grows when large text needs the room. A rested row paints nothing; focus is a `curtainEdge` surface (in over `snap`, out over `settle`) with the glow at half strength and the glyph in `houselight`. A row whose state is "on" shows a 10 px `signal` dot before its value; glyphs are never tinted `signal`. Rows with a cyclable value show `‹ value ›` when focused and change it with Left and Right as well as OK, the value swapping with an 8 px slide and a 180 ms crossfade. Groups of rows carry a `caption`/`dust` label above them ("Look and sound", "What Lounge can see", "This TV").

### 5.5 Other screens

- **Search**: top 1/3 is the query field (`display-l` as the typed text, `dust` placeholder "Search your TV"), bottom 2/3 is one row per result type (Apps, Continue watching, Live now, Jellyfin…), each a normal row. Results update per keystroke. Voice results arrive the same way.
- **Settings (on TV)**: two-column like the system: list left (`title`), detail right (`body`). Only essentials; the last item is "Open on your phone" with the pairing QR.
- **Onboarding**: five full-screen steps, each one question, `display-l` heading, one primary button (Next / Start, with Skip beside it on the optional steps). Progress is five 8 px dots, not numbers. The left column (820 px) holds the question and its rows; the right holds a lamp in `houselight` with the step's glyph, or the live pairing QR on the phone step. Back goes to the previous step.
- **Ambient (screensaver)**: artwork full-bleed, dimmed to 60 %, clock `clock` style bottom-left, optional now/next line; elements drift 2 px every 60 s and the whole frame dims a further 20 % after 30 minutes (OLED care).
- **Ambient, abstract scenes**: instead of artwork, a slow scene drawn on the GPU in the theme's own colours (Ink, Current, Rain, Smoke, Embers, Shallows, Lava, Aurora, Silk, or one a plugin publishes), with the same clock and two lines naming the scene and where it came from. A scene is dark with one lit thing in it, the room after the lights go down: `velvet` and `horizon` for the body, `houselight` and `ember` for the light. Every scene passes through one finishing step: a vignette, the house lights (100 %, then 80 % after 30 minutes), a ceiling on highlights at about 88 % so nothing glares, and blue-noise dither against banding, all drawn at the screen's own size over the scaled-up scene. It draws at 30 fps at half the screen's resolution, or on a Tier A TV at full size for a scene with fine detail (Rain, Embers). When a frame costs the GPU more than about 24 ms it slows to 20 fps before it shrinks, and so on down to a sixth of the screen, so a big panel keeps its pixels for as long as the TV can afford them.
- **Live backgrounds**: Settings → Background puts one of the same scenes behind the home instead of the still light field. The scene is dimmed to about 72 % with a ceiling at 50 %, so the tiles stay the light in the room, and the scene is drawn at a fraction of the screen while the dimming, the focused tile's spill, the playbill's light and the dither go over it at the screen's own size, so a big panel sees smooth light and fine grain rather than the scene's pixels. It freezes with the field's clock, after a minute without input or under an opaque overlay; when a frame costs the GPU more than about 24 ms it slows to 20 fps, then draws at a third, and gives way to the still field beyond that, with a toast saying so. Still in Lite mode and with reduced motion.

### 5.x The mark

The mark is the plush coral sofa: a soft two-seater lit from above, apricot fading to coral,
on tapered wooden legs. It is the one bright thing in the dim room, which is the home screen's
own idea. Sources: `design/logo/sofa.png` (on transparency) and `design/logo/sofa-gradient.png`
(on its indigo-to-orange gradient); `design/logo/make.py` renders every size from them, and
nothing is traced or redrawn by hand. The sofa appears on exactly two grounds: on `velvet`
inside the TV UI, and on its own
gradient wherever it stands for the app outside the UI: the adaptive launcher icon
(background: the gradient; foreground: the sofa at 58 dp inside the safe zone; monochrome: its
silhouette), the launcher banner (the sofa on the left, "Lounge" in Bricolage Grotesque Bold,
`limelight`, to its right) and the phone app's icon. The top bar carries the banner's lockup at
TV size, nothing behind it: the sofa 56 dp wide, then "Lounge" 14 dp after it in the same
Bricolage cut (weight 700, optical size 96) at 30, `limelight`, tracked −0.01 em, its baseline on
the line where the sofa's seat meets its legs, so the legs hang below the line as the g's tail
does. Scaled down from the banner the word would fall under the 22 px floor, so at TV size the
word is set larger against the sofa than on the banner. The gradient belongs to the mark and never
appears elsewhere in the UI. Never outline it, never flip it, never recolour it, never put text
on it, and never show it smaller than 36 dp wide.

## 6. Focus and motion

### 6.1 Brightness states (house lights)

| State | Brightness | Saturation | Notes |
|---|---|---|---|
| Focused | 1.00 | 1.00 | Glow + spill + scale, and a 2 px `limelight` rim at 62 % just inside the tile's edge |
| Neighbour (±1 in the same row) | 0.86 | 1.00 | Lit from the side |
| Rested (same row) | 0.78 | 0.92 | |
| Other rows | 0.70 | 0.88 | |
| Idle dim (no input for 20 s) | all states −0.12 | −0.05 | Glow and spill halve. Going down takes `dim` (1.2 s); any key restores over `settle` |

Brightness is a colour-matrix on the tile image, not an overlay, so blacks stay black.

### 6.2 Curves and durations

| Name | Duration | Easing | Use |
|---|---|---|---|
| `snap` | 120 ms | cubic-bezier(0.2, 0, 0, 1) | Focus scale and glow in |
| `settle` | 180 ms | cubic-bezier(0.2, 0, 0, 1) | Focus out, brightness changes |
| `travel` | 220 ms | cubic-bezier(0.3, 0, 0.1, 1) | Row horizontal scroll, backdrop spill moving |
| `proscenium` (Stage) | — | — | The Stage's height eases between 864 and 200 px while one picture's frame travels from the artwork's place to the band (5.2), going out of focus into its light, and the playbill and strip fade beneath it; the rows slide up. Nothing is squeezed. Text swaps in the playbill are one in-place fade per spotlight change, lines staggered by 40 ms. |
| `proscenium` | 260 ms | cubic-bezier(0.35, 0, 0.15, 1) | Stage collapse/expand, overlay enter |
| `curtain` | 320 ms | cubic-bezier(0.3, 0, 0.1, 1) | Stage artwork crossfade, playbill swap, theme change |
| `dim` | 1200 ms | cubic-bezier(0.4, 0, 0.2, 1) | The house lights going down when idle; the screensaver's late dim over 60 s is the same idea stepped |
| `lamp` | 600 ms | cubic-bezier(0.2, 0, 0, 1) | The lamp warming up after a cold start: content is there from the first frame, only the light ramps in |

No overshoot, no bounce, no spring physics. Reduced motion: all durations become 1 ms except `travel` (kept at 120 ms so scrolling stays trackable), the backdrop spill snaps, parallax is off. The first light is placed, never swept into: the spill snaps to the first focused tile and animates from then on.

### 6.3 The Stage change

When the strip focus moves: the outgoing artwork fades to 0 while sliding 12 px in the direction of travel; the incoming artwork fades in from 0.92 scale to 1.0 and from 12 px behind; the playbill's old text fades out as one in the first 40 % of 320 ms and the new text fades in exactly where it will sit, title from 112 ms, metadata 40 ms later, buttons and the explanation 80 ms later; nothing in the column moves, and old and new never overlap. A change that arrives while a swap is still running carries on from whatever is on screen: each of up to three copies fades from where it is, the newest drawn on top, so fast presses never cut a picture or a line off mid-fade. The spill colour tweens to the new light over the same 320 ms. Nothing else on screen moves.

### 6.4 Focus behaviour

- Each row remembers its last focused index; returning to the row restores it.
- Home from anywhere: Stage expanded, focus on the strip's first tile, or on the last Stage tile if Home was pressed within the last 10 minutes.
- Back from a row returns focus to the strip, not to the top bar. Back from the strip does nothing (we are home). Back from an overlay closes it and restores focus to the element that opened it.
- Long-press OK opens the tile menu; long-press Back is a no-op; long-press Home is reserved for the system unless the accessibility module owns it.
- Focus never lands on a non-interactive element. The focus trap test in CI enforces this.

### 6.5 Parallax

On Tier A and B, the Stage artwork has three layers if the source provides them (background, foreground cut-out, logo) or one layer otherwise. Horizontal focus travel along the strip shifts the layers by 0 / 6 / 10 px over `travel`. That is the only parallax in the product. Off in Lite mode and reduced motion.

## 7. Components

### 7.1 Tiles

- **Poster**: artwork fills the tile; no text on the tile. Title appears only in the playbill or, in rows, as a `meta` line below the focused tile (fades in with focus, 120 ms). A `caption` badge top-left for "Live" (`ember` dot + text) or "New" (`houselight` text), never both.
- **Landscape**: artwork plus a bottom scrim (velvet 0 → 55 % → 90 % over the bottom 60 %) carrying one `title` line and one `meta` line. Titles are cut at a word boundary with an ellipsis, never mid-word. A tile whose artwork is an app banner carries no scrim and no text: the banner names itself. Progress bar 4 px, inset 16 px from the sides and 12 px from the bottom, track `limelight` at 22 %, fill `houselight` multiplied by the tile's brightness so a rested tile's bar is not its brightest thing.
- **App banner**: the app's banner as supplied, inside the brightness model like any artwork; apps with no banner get a `curtain` tile with the icon at 72 px and the name in `title`. A tile with no picture at all shows a diagonal of its light into `curtain`, opaque, so it reads as made rather than missing.
- **Live tile** (landscape variant): channel logo top-left at 56 px, programme title, "Now, 42 minutes left" as meta, progress bar is elapsed time; the `ember` live badge moves to the top-right so it never sits on the logo.
- **Input / device tile** (compact or landscape): device glyph (HDMI, console, soundbar), CEC name if known else port name, a `signal` dot when connected. Disconnected ports are not shown.
- **Game tile**: landscape with the game's artwork or the app's banner; a controller glyph badge when a controller is connected.
- **TV controls card** (streamer): a 384 × 216 landscape tile with four compact actions inside: Turn on TV and switch here, Sound to soundbar, Volume, Turn everything off. Each is a `title` line with a glyph; focus moves inside the card.

- **Notice tile** (compact, size L): a `houselight` info glyph, a `title` line that names the
  thing to turn on ("Turn on TV listings") and a `caption` line that says why ("See what you
  were watching"). It sits where the row it stands in for would be, one per row, and its only
  menu item is "Don't show this again". The update notice is the one exception to the
  placement: "Lounge 0.1.2 is ready" sits in a row of its own at the top, and only when the
  install needs the person (a confirmation, or permission to install).
- **Now Playing chip** (top bar, between Search and Settings): three bars in the artwork's `light`
  that move while sound is playing (static in reduced motion), then the title in `meta`, at most
  280 px. Selecting it opens the Now Playing card (5.4): artwork 200 × 200 on the left, app name
  in `caption`, title in `title`, artist in `meta`, round 56 px transport buttons (previous,
  play/pause filled with the `light`, next), an "Open" secondary button and a 4 px elapsed bar
  that advances locally while playing.
- **Guide cell** (Live Guide, 5.4): rows 72 px tall, channel column 240 px (40 px logo, number in
  `caption`, name in `meta`), programme cells laid out to the minute across 2 h 30 min, `meta`
  title over a `caption` time span. The present moment is a 2 px `houselight` line down the grid;
  the programme on air is underlined in `houselight`. OK tunes the channel.

- **Plugin sheet rows**: a plugin is a row with its name and one status word — On · Off · Needs
  setup · Problem — in `signal`, `dust` or `ember`. Its detail sheet reads top to bottom: name and
  version, description, one status sentence ("76 items · refreshed 3 min ago"), the on/off row,
  settings as fields (label in `caption`, value in `title`, 56 px, saved on Done or when focus
  leaves; passwords show "•••••••• (saved)"), Refresh now, Log, Remove (never for built-in).
- **Phone companion sheet**: the on/off row, then the QR code at 300 px on `limelight`, "Or type
  this code" and the six digits in `display-l` grouped in threes, the address in `caption`, New
  code, and Paired phones. The phone app mirrors these tokens (`companion/src/app.css`) so TV and
  phone read as one product; on the phone the focused tile's light becomes the accent.

- **Power Setup sheet**: heading, one status sentence, then a single row that is the only thing
  to do right now — Connect (houselight), Waiting for you to allow it…, Connected (signal), or
  Open Developer options. Under "What it can do", each action is a row (title · Run/Done/Failed)
  followed by its reason in `caption` and the exact shell command in a monospace `velvet` box, so
  nothing runs that the person has not read. Forget this connection sits last; the footnote
  shows the key fingerprint the device's prompt displays.
- **TV controls sheet**: the privileged rows first — Wake the TV and switch to me, TV power,
  Everything off (ember), soundbar/TV sound on televisions, then "Switch the TV to" with the CEC
  devices on the bus — each row's value flips to Sending… / Sent / the failure for 2.5 s. Volume,
  mute and play/pause follow and work without Power Setup. Without it the first row is the one
  step to set it up.

- **Screensaver sheet**: one status row naming the current screensaver (ours in `signal`), a Style
  row that cycles Pictures and the abstract scenes with Left and Right and names the current one
  underneath, then the single next step — Use Lounge as the screensaver, or Back to the previous
  one — and Preview it now, which draws the chosen style in-app until any key.

### 7.2 Buttons

- Primary: `light-text` on the artwork `light` (or `velvet` on `houselight` when no artwork), 56 px tall, 24 px radius pill, `title` label, 32 px horizontal padding. Only one primary per screen region.
- Secondary: `limelight` on `curtain` with a 1 px `curtain-edge` hairline, same size.
- Icon buttons in the top bar: 48 px, glyph 28 px, label in `meta` to the right; the label is always visible (no icon-only buttons on TV).
- Focus on a button: scale 1.04 and glow, same model as tiles.

### 7.3 Badges and indicators

- Live: 10 px `ember` dot + "Live" in `caption`. The dot pulses once every 4 s (opacity 1 → 0.6 → 1 over 600 ms); off in reduced motion.
- Connected: 10 px `signal` dot, no pulse.
- Progress: 4 px (tiles) or 6 px (Stage) bar, `houselight` on `limelight` at 22 %, radius 3, no animation except when the value changes (tween 320 ms).

### 7.4 Toasts

Bottom-left (x 96, 54 from the bottom), `curtain` card, 1 px hairline, radius 16, `title` text, auto-dismiss after 4 s, at most one at a time. Enter: rise 16 px with a fade over `proscenium`; exit: fade over `settle`. Used for confirmations ("Hidden from Continue watching") and for errors written as sentences ("Nothing on this TV can open Alien: Earth. Is its app still installed?"), never an exception's text. The system toast is never used.

## 8. States

- **Loading**: never a spinner on the home screen. The persisted snapshot renders first; tiles whose artwork isn't on disk yet show their dominant colour as a flat tile and fade the image in over 180 ms when it arrives. Before the first index the Stage says "Setting up your home screen"; "Nothing here yet" is only for a TV that has been indexed and really has nothing.
- **Search, nothing typed**: the row of recently watched things and the apps row, so the screen is never blank. Results reflow as they change (120 ms in, `travel` placement, 90 ms out) instead of jumping.
- **Empty row**: not rendered (5.3).
- **Empty home** (first run, nothing configured): the Stage shows the house light, the title "Let's set up your home screen", one primary button "Start", and the strip shows the setup steps as tiles.
- **Offline**: rows that need the network show their cached content with a `dust` "Offline, showing what we have" meta line under the header. Nothing else changes; apps, inputs and live TV work.
- **Permission missing**: Continue watching, without TV listings, shows one compact tile in the row's place: "Turn on TV listings to see what you were watching", primary action opens the system prompt. Dismissible; stays dismissed. Live now and Now playing stay away until their access is given; Settings has the switches.
- **Error**: toasts explain and offer the fix in the same sentence. Never "Something went wrong".

## 9. Sound

Three sounds, 48 kHz, under 40 KB total, each at −18 LUFS, played through the system UI sound stream so the TV's volume applies. Off by default while any media session is playing; user-toggleable. One family, rendered by `design/sounds/make.py` rather than sampled: felt and wood in a quiet room, with a touch of that room on every sound, nothing glassy, clicky or electronic. `open` and `close` are the same instrument a fourth apart, so the pair reads as a question and its answer.

- `tick`: focus moved. A felt-covered key: a wooden tap at 640 Hz over a 215 Hz body, the pitch dipping as it lands, 40 ms with 18 ms of room. Pitch jitters ±3 % so held keys never machine-gun; at most one per 90 ms, at 30 % volume while a key auto-repeats; a row change plays it at 0.94× pitch, a quiet spatial cue. Changing a settings value ticks too.
- `open`: a selection opened something (tile, overlay). A felt mallet on a warm wooden bar tuned to G3 (196 Hz, partials at 2×, 4× and 10×) with a breath of air rising behind it, the lamp coming on: 170 ms.
- `close`: Back / dismiss. The same mallet a fourth lower, D3, darker and without the air, settling 1.5 % down in pitch as it fades: 190 ms.

No sound on Home, on scroll end, or on errors (the toast is enough).

## 10. Voice (copy)

- Sentence case, plain verbs, present tense. "Continue", "Open", "Turn on TV listings", "Hide".
- Buttons say what will happen. The action keeps its name through the flow: "Hide" → toast "Hidden".
- Metadata helps decide, not describe: time left beats duration; "Live now" beats a start time.
- Explanations are honest and short: "Because you watch at 9 pm", "Added to Jellyfin today", "Connected on HDMI 2".
- Empty states invite: "Pair a controller to see your games here".
- Errors state the cause and the fix: "Jellyfin didn't answer at 192.168.0.20. Check the server, then try again."
- Never "Oops", never exclamation marks, never emoji.

## 11. Accessibility

- Every tile's content description: "<title>, <row name>, <state>" (e.g. "The Bear, Continue watching, 23 minutes left").
- Overlays announce their heading on open; toasts are live regions.
- High-contrast theme: `velvet` → `#000000`, `dust` → `#D5CBD1`, glow alpha doubled, focus adds a 3 px `limelight` ring (the one case where a ring is allowed, because the user asked for it).
- Reduced motion: section 6.2.
- Text scale to 1.3× without clipping; at 1.3× the playbill drops the explanation line first, then the secondary button.
- Controller: D-pad/left stick move, A select, B back, Y long-press menu, Start search, Select/View quick settings. Shown once in onboarding when a controller is detected.

## 12. Tier adaptations

| | Tier A (Streamer) | Tier B (TCL) | Tier C / Lite |
|---|---|---|---|
| Backdrop spill | Shader, moves over `travel` | Shader, moves over `travel` | Static `velvet`, glow only |
| Artwork light | Full | Full | Glow at 24 % alpha |
| Stage parallax | 3 layers | 3 layers | Off |
| Shader wallpaper | On | On | Off |
| Previews | 1080p | 720p | Off |
| Idle dim | On | On | On (free) |
| Light field clock | 333 ms steps; sleeps after 60 s idle | 500 ms steps; sleeps after 60 s idle | Off |
| Live background | Optional; 30 fps at half resolution, slowing to 20 fps and then a third by GPU time | Same | Off |
| Screensaver push-in | Stepped at 4 Hz | Stepped at 2 Hz | Off |
| Abstract scenes | 30 fps, half the resolution, slowing to 20 fps before each step down to a sixth by GPU time | Same | Same; Android 13 or later |

## 13. Streamer-specific (primary device)

- Remote keys: Home → Stage; Back → up one level; Assistant → system (we never intercept); YouTube/Netflix → launch (we never intercept); star button → whatever the user set in Google's own settings (we suggest our Search).
- TV controls card replaces the Inputs row; it is the last Stage-strip tile and also lives in the top bar.
- Default home layout: Stage, Continue watching, Your apps, Games, All apps. The TV controls row appears only on a TV with inputs; on the Streamer the card is the last strip tile and lives in the top bar, nowhere else.
- Google Photos ambient remains the user's screensaver unless they pick ours.

## 14. Design QA checklist (per release)

- [ ] No pure white, no fake black, no colour used as paint for text/icons.
- [ ] No all-caps, no eyebrows, no dot-joined metadata.
- [ ] Only the focused element glows; the spill is centred on it.
- [ ] Stage change performs exactly the 6.3 choreography and nothing else moves.
- [ ] Every row that renders has content; every overlay closes with Back and restores focus.
- [ ] Reduced motion and high contrast verified on the Streamer with TalkBack on.
- [ ] 1080p screenshots of Home (expanded and collapsed), Search, Live Guide, TV controls, Now Playing and Ambient attached to the release, taken from the HDMI capture rig.
