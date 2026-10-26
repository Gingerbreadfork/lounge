# Writing a Lounge plugin

A plugin adds things to the home screen: episodes to continue, rows of its own, channels in
Live now and the guide, or a wallpaper. It is a WebAssembly module in a signed `.lgp` bundle that
runs inside the launcher's Rust core. It cannot touch Android, the file system, sockets or other
plugins. It can fetch from the hosts it names, keep a small private store, read its own
settings, and log. Everything it publishes is plain data; the launcher fetches the artwork,
draws the tiles and lights the room.

This guide takes you from nothing to a plugin on your TV in about twenty minutes, then covers
every part of the interface. The plugins in `plugins/` are complete examples, each showing a
different corner of the interface:

- `plugins/sample`: eighteen public-domain films with posters and stills, some part-watched, and
  a screensaver drawn on the TV's GPU. No settings, no network beyond the pictures. The best one
  to copy from.
- `plugins/iptv`: an M3U playlist becomes channels, an XMLTV feed fills the guide. Settings,
  a gzipped guide read in chunks within a memory and time budget, and the 2,000 places shared
  fairly between channels.
- `plugins/jellyfin` and `plugins/emby`: sign in to a server with a password, publish resume
  points and new arrivals, answer search. Secrets, a cached token in the key-value store, deep
  links into another app.
- `plugins/plex`: the same with a token instead of a password, and pictures through the
  server's resizer.
- `plugins/youtube`: a list setting parsed into sources, an Atom feed read with `quick-xml`,
  partial failure reported without giving up.
- `plugins/archive`: rows from a search API, and `resolve`, which looks up the file to play only
  when something is selected.
- `plugins/immich`: `Kind::Wallpaper` items that feed the screensaver instead of a row, read
  from a shared album's timeline a month at a time.
- `plugins/hdhomerun`: `Kind::Epg`, channels that play in the TV's player, a guide window from
  `RefreshContext`, and finding a device through a discovery service.
- `plugins/tvheadend`: answering the server's Digest or Basic challenge, a token on the
  addresses the TV fetches itself, paging through a large guide, recordings as a row.
- `plugins/channels`: a DVR's recordings split between Continue watching and a row by their
  playback position.
- `plugins/nextpvr`: a session signed in with a digest, a guide gathered across refreshes in a
  fixed set of key-value keys so one refresh never exceeds the fetch budget, and `resolve` so
  play addresses always carry a live session.

## Quick start

You need stable Rust with the wasm target, and the `lgp-tool` from this repository.

```sh
rustup target add wasm32-unknown-unknown
git clone <this repository> lounge
cd lounge/plugins && cargo build --release -p lgp-tool    # plugins/target/release/lgp-tool
```

Make a crate next to the others, or anywhere with a path dependency on the SDK:

```toml
# Cargo.toml
[package]
name = "my-plugin"
version = "0.1.0"
edition = "2024"

[lib]
crate-type = ["cdylib", "rlib"]

[dependencies]
lounge-plugin-sdk = { path = "../lounge/plugins/sdk" }
```

```rust
// src/lib.rs
use lounge_plugin_sdk::{self as sdk, host, Action, Item, ItemKind, Kind, Manifest, RefreshContext};

pub struct Mine;

impl sdk::Plugin for Mine {
    fn manifest() -> Manifest {
        Manifest {
            id: "com.example.mine".into(),
            name: "My plugin".into(),
            version: env!("CARGO_PKG_VERSION").into(),
            description: "What it puts on the screen, in one sentence.".into(),
            publisher_name: "Your name".into(),
            kinds: vec![Kind::Content],
            fetch_hosts: vec!["api.example.com".into()],
            refresh_minutes: 30,
            rows: vec![],
            settings: vec![],
            homepage: None,
        }
    }

    fn refresh(_ctx: &RefreshContext) -> Result<Vec<Item>, String> {
        let body = host::get_text("https://api.example.com/continue")?;
        sdk::info!("fetched {} bytes", body.len());
        Ok(vec![Item {
            id: "ep:42".into(),
            kind: ItemKind::Resume,
            title: "The Bear".into(),
            subtitle: Some("Honeydew".into()),
            season: Some("2".into()),
            episode: Some("4".into()),
            duration_ms: Some(30 * 60_000),
            position_ms: Some(12 * 60_000),
            last_used_ms: Some(host::now_ms() - 3_600_000),
            thumbnail: Some("https://api.example.com/art/42.jpg".into()),
            action: Action::view("myapp://episode/42"),
            ..Default::default()
        }])
    }
}

sdk::export_plugin!(Mine);
```

Build it, run it on your computer, then bundle it:

```sh
cargo build --release --target wasm32-unknown-unknown
lgp-tool run target/wasm32-unknown-unknown/release/my_plugin.wasm
lgp-tool keygen ~/.config/lounge/publisher.key                       # once, keep it safe
lgp-tool bundle target/wasm32-unknown-unknown/release/my_plugin.wasm ~/.config/lounge/publisher.key mine.lgp
lgp-tool inspect mine.lgp
```

`run` prints the log and every item the TV would show. It fetches live by default.

Install it on the TV from the phone: Settings, Phone companion on the TV, then on the phone open
the Plugins page and choose the file, or paste a link to it. New plugins start off; fill in
any settings, turn it on, and the first refresh runs. The TV's Plugins settings show the same
things, including the plugin's recent log lines, which is where your `sdk::info!` lines end up.

## How a plugin is run

The core loads your module fresh for every call, with a fuel budget, a memory cap and a time
limit, so a runaway loop or a leak ends that call and nothing else. Host functions cost fuel
too, so a loop of `kv_set` runs out like a loop of arithmetic. There is no state between calls
apart from the key-value store. `refresh` runs when the plugin is turned on, every
`refresh_minutes` while the screen is on, when a setting changes, and when someone asks for it
on the phone. Refreshes run one after another on their own thread; `resolve` and `search` run on
another, so pressing a tile never waits for someone's refresh.

`search` runs as the user types on the TV's search screen, for plugins whose manifest lists the
`search` kind and whose required settings are filled in. Each plugin that finds something gets a
row under its own name, with the first picture of each result; the results' actions work
exactly as they do in a refresh. A search that a newer one has replaced is not run at all.
`resolve` runs when the user selects an item whose action says `resolve`.

The host fetches artwork for you, four pictures at a time, caps each at 4 MB (12 MB for a
wallpaper), scales it to every size the screen needs and takes the palette that lights the
focused tile. Give it a direct image URL under one of your `fetch_hosts`; a picture that fails
to download is simply a tile without one, so prefer modestly sized files. Each URL is downloaded
once however many items name it, earliest items first. One refresh may download up to 600 new
pictures, 192 MB, in two minutes; what does not fit comes with later refreshes, since pictures
already downloaded cost nothing. Downloads a refresh no longer names are deleted.

## The manifest

```json
{
  "id": "tv.lounge.iptv",
  "name": "IPTV playlist",
  "version": "0.1.0",
  "description": "Channels from an M3U playlist, with programme information from an XMLTV guide.",
  "publisher_name": "Lounge",
  "kinds": ["content", "epg"],
  "fetch_hosts": ["$playlist_url", "*"],
  "refresh_minutes": 60,
  "rows": [{ "id": "recent", "title": "Recently added", "style": "poster" }],
  "settings": [
    { "key": "playlist_url", "label": "Playlist URL (M3U)", "kind": "url", "required": true, "help": "…" },
    { "key": "password", "label": "Password", "kind": "password" }
  ],
  "homepage": "https://example.com"
}
```

| Field | Meaning |
|---|---|
| `id` | A reverse-domain name: lowercase labels of letters, digits and dashes joined by dots, at least two labels. It names the plugin's directory on the TV, so it never changes. |
| `name`, `description`, `publisher_name` | What the TV and the phone show. The description is one sentence. |
| `version` | Shown to the user; updates may carry any version, the signature is what matters. |
| `kinds` | What the plugin does: `content` (items and rows), `epg` (channels and schedule), `wallpaper`, `screensaver`, `search`. |
| `fetch_hosts` | Hosts `fetch` may reach, including on redirects, each shown to the user in words. A name or IP address is that host; `*.example.com` is its subdomains; `*` is any host on the public internet. `$key` is whatever host the user enters in the setting `key`, which is how a plugin reaches a server they name, at home or not. `local-network` is any home-network IP address or local name, for a plugin that finds devices itself, such as a tuner; a local name is one label, or one under `.local`, `.lan`, `.home.arpa` or `.internal`. The TV checks the address every name resolves to. The home network is reachable only through `$key`, `local-network`, or an entry that is itself an IP address or a local name, never through a wildcard or a public name. Loopback and the TV's own addresses are reachable only through an entry that is literally `localhost` or an IP address. |
| `refresh_minutes` | How often `refresh` runs while the screen is on; at least 5, 0 means the host's default. |
| `rows` | Rows of your own, in order. `style` is `landscape` (16:9 tiles), `poster` (portrait) or `compact`. Items name a row by its `id`. |
| `settings` | Fields the user fills in on the TV or the phone. `kind` is `text`, `url`, `password`, `number` or `toggle`. A `toggle` is stored as `true` or `false`. A `default` is what `setting` returns while nothing is set, and the TV and the phone show it in the field. A `password` is only readable through `secret`; `setting` returns nothing for it. It is never shown back. |
| `homepage` | A link shown on the plugin's page. |

The manifest ships as `manifest.json` in the bundle and is also returned by `lg_manifest`;
the installer refuses a bundle where the two disagree. The SDK writes both from
`Plugin::manifest()`.

## Items

```json
{
  "id": "resume:e1",
  "kind": "resume",
  "title": "The Bear",
  "subtitle": "Honeydew",
  "season": "2", "episode": "4",
  "duration_ms": 1800000, "position_ms": 900000,
  "last_used_ms": 1760800000000,
  "poster": "https://…/primary.jpg",
  "thumbnail": "https://…/thumb.jpg",
  "logo": "https://…/logo.png",
  "row": "recent",
  "action": { "view": "jellyfin://…", "fallback_launch": "org.jellyfin.androidtv" },
  "channel_id": "abc.au", "channel_name": "ABC TV", "channel_number": "2",
  "start_ms": 1760800000000, "end_ms": 1760803600000,
  "is_new": false, "weight": 0
}
```

**Where an item goes.** A `resume` item joins Continue watching and is a candidate for the
Stage. A `channel` joins Live now and the guide, and `schedule` items fill in what is on it. A
`program` goes into the row its `row` names; without one it goes nowhere. A `wallpaper` is a
picture for the screensaver: give it the picture as `thumbnail`, a `title` and, if you like, a
`subtitle`, which goes under the title on the screen; it needs no `action`. The picture is
prepared for the panel, at 1920×1080 or 3840×2160, and never made larger than it came, so send
the largest picture you have up to 12 MB and 13 million pixels. A wallpaper with `framed: true` is kept whole in its own shape and hung on the dark
wall with a shadow, as a painting; the rest fill the screen. A `screensaver` is an abstract
screensaver the TV draws itself; see [Screensavers](#screensavers).

**Ids.** An item's `id` must be unique within one refresh and stay the same from one refresh to
the next. When two items share an id, the first is kept and the plugin's log says how many were
left out. The first 2,000 items of a refresh are kept, so put what matters first.

**What a tile shows.** `title` is the first line; `subtitle` the second, in the launcher's own
voice (commas between facts, never dots: "S2 E4, Netflix"). `season` and `episode` are shown
as "S2 E4" when both are given. `duration_ms` with `position_ms`, or `progress` from 0 to 1,
draws the progress bar and "19 minutes left". `last_used_ms` orders Continue watching, newest
first; a resume item without one keeps the time Lounge first saw it. `added_ms` and `is_new` mark
new arrivals. `weight` orders a row when recency does not. A row on the home shows its first 60
items in that order; search and the guide still reach the rest.

**Artwork.** `thumbnail` is 16:9 and feeds landscape tiles and the Stage's full-bleed picture.
`poster` is portrait and feeds poster rows; on its own it is also cropped for landscape tiles and
framed on the Stage over a soft backdrop. `logo` is a channel or publisher mark, fitted and never
cropped. Give both `thumbnail` and `poster` when you have them.

**Channels.** A `channel` item carries `channel_id`, `channel_name` and `channel_number`;
`schedule` items name the same `channel_id` with `start_ms` and `end_ms`. A `schedule` item needs
no `action`: selecting it plays its channel. One with a `start_ms` and no `end_ms` is on from its
start until something else is known, which is what a live stream is; its `subtitle` joins the
Stage's lines while it is on. The guide window the host wants is in `RefreshContext.window_hours`.

## Actions

What happens when the user selects an item. The first field that is set wins, in this order:

| Field | What it does |
|---|---|
| `play` | A stream URL, opened in whatever video player the TV has. `title` is handed to the player. |
| `view` | An Android URI: a deep link such as `jellyfin://…`, a web address, or an `intent:` string. `fallback_launch` runs if nothing on the TV handles it. |
| `launch` | A package name to open. |
| `resolve` | A string handed back to your `resolve` at selection time, which returns one of the actions here. Use it when the stream URL is only known at the moment of playing, or when the plugin does the work itself, such as telling a player on the network to start: then return an empty action, and nothing more is opened. |
| `search` | A query for Lounge's own search: for a title that lives in some app the plugin cannot name. A search result never carries one. |

## Screensavers

A plugin can add abstract screensavers that the TV draws on its own GPU. The plugin never draws:
it publishes a program and values for the program's uniforms, and Lounge runs it behind the clock
thirty times a second. The same scene can be chosen as the home's live background, dimmed, with
the room's light over it. Settings, Screensaver, Style lists them next to Lounge's own, and Preview it
now shows one straight away.

```rust
Item {
    id: "projector".into(),
    kind: ItemKind::Screensaver,
    title: "Projector".into(),
    subtitle: Some("Smoke in the beam, from the back row".into()),
    shader: Some(Shader::new(include_str!("projector.agsl")).float("drift", 0.08).color("beam", "#FFE2B8")),
    ..Default::default()
}
```

Declare `screensaver` in the manifest's `kinds`. The `title` is the style's name and the
`subtitle` its one-line description. Every refresh replaces the plugin's screensavers, so values
can change over time, a palette that follows the weather for instance. The TV keeps the last set,
so the screensaver has them after a restart before the plugin has refreshed.

**The program** is [AGSL](https://developer.android.com/develop/ui/views/graphics/agsl), Android's
shading language. Its entry point is `half4 main(float2 coord)`, called for every pixel with
`coord` in pixels from the top left. Lounge sets these uniforms when the program declares them:

| Uniform | Value |
|---|---|
| `uniform float2 resolution` | The size being drawn, in pixels. It changes when Lounge lowers the resolution, so work in `coord / resolution.y` and the scene keeps its shape. |
| `uniform float time` | Seconds since the screensaver started. |
| `uniform float hour` | The local time of day, from 0 to 24. |
| `layout(color) uniform half4 velvet` | The theme's background. `horizon`, `curtain`, `houselight`, `ember`, `signal` and `dust` are the theme's other colours (DESIGN.md §3), so a scene follows whichever theme the person picked. |
| `uniform shader noise` | Three smooth noises that tile, in the red, green and blue of one 256-pixel square: `noise.eval(p * 16.0).r` reads one noise cell per unit of `p`. Sample it for noise and fbm instead of computing them; a TV's GPU fetches a texture far faster than it does the arithmetic. The only `shader` input a program may declare. |

Every other uniform takes its value from `shader.uniforms`: a number for a `float` or `int`, a
list of numbers for vectors, matrices and arrays, as many as the uniform holds, and a colour such
as `"#FFE2B8"` for a `layout(color) uniform half4`. A uniform left unset is zero. A program cannot
take other shaders, colour filters or blenders as input.

**What Lounge adds.** Every scene goes through the same finishing pass: the room's vignette, the
house lights, which go down a step after half an hour, a ceiling on highlights so nothing glares
in a dark room, and dither so dark gradients do not band. The clock and the style's name sit
bottom left. Design for that: keep scenes dark, slow and calm, because they run for hours.

**Cost.** Lounge draws the program at a third of the screen's resolution and upscales it, which
suits soft, flowing scenes. It watches each frame's GPU time, and when a frame takes more than
about 24 ms it drops to a quarter, then a fifth, down to a sixth, and remembers where the program
settled on that TV. A program too heavy even at a sixth falls back to pictures. A few octaves of
value noise, warped two or three times, sits comfortably on a Google TV Streamer; Lounge's Ink is
about as heavy as a scene should get.

**Trying one out.** A `.agsl` file in the TV's `screensavers` folder appears under Style as
"This TV", with no plugin and no build:

```sh
adb push smoke.agsl /sdcard/Android/data/tv.lounge.launcher/files/screensavers/
adb logcat -s Lounge | grep screensaver
```

Choose it, preview it, push again after each change and preview again. The log says what the
program costs (`screensaver: local:smoke.agsl at 1/3, 33.3 ms a frame, 0.5 ms late, GPU 14.2
ms`), and when it does not compile it falls back to pictures with the compiler's reason. Lounge's
own scenes are in `android/feature/home/src/main/assets/screensavers` to read and borrow from.

## The host

Everything a plugin can do, as the SDK exposes it under `host`:

| Function | What it does |
|---|---|
| `fetch(request)` | An HTTP request to a host in `fetch_hosts`. Bodies are capped at 8 MB, and a larger one fails with `host::TOO_LARGE`; a response body is text when it is valid UTF-8, otherwise base64 in `body_base64`, which `sdk::body_bytes` turns into bytes. Redirects are followed only to allowed hosts, and your headers go only to the origin you addressed: a redirect to another host, port or scheme drops them. `timeout_ms` shortens one request; none takes longer than 20 s. At most 64 calls per invocation, sharing a 60 s budget. |
| `get_text(url)` | `fetch` for a GET that must return text, failing on anything but 2xx. Its errors name the address without its query string or password, so they are safe to log; `sdk::redact` does the same for your own messages. |
| `kv_get`, `kv_get_string`, `kv_set`, `kv_delete` | A private store: 256 keys of up to 256 KB each. Survives restarts and updates; cleared when the plugin is removed. Cache tokens and the last refresh's result here. |
| `setting(key)` | A setting's value, or its manifest default. Nothing for a `password`. |
| `required_setting(key, label)` | The same, or an error naming the field for the user. |
| `secret(key)` | A `password` setting's value. Only keys the manifest declares as passwords. |
| `now_ms()` | Milliseconds since the epoch. |
| `log(level, message)` | 0 debug, 1 info, 2 warn, 3 error. `sdk::info!`, `warn!` and `error!` format for you. Shown on the plugin's page on the TV and by `lgp-tool run`. |

`RefreshContext` tells `refresh` the time, the UTC offset in minutes, the device tier (`a`, `b`
or `c`, so a plugin can fetch less on a small TV), when the last successful refresh finished,
and how many hours of schedule are wanted. `epoch_ms(...)` in the SDK turns a civil date into
milliseconds without a calendar crate, and `parse_iso8601` reads a timestamp, returning nothing
for anything malformed.

A refresh keeps at most `sdk::MAX_ITEMS`. `sdk::dedup_ids` drops repeated ids, and
`fit_schedule` or, for a long guide read as it arrives, `ScheduleBudget` share the places left
after the channels evenly between them, earliest programmes first. `for_each_in_array` reads a
large JSON array one entry at a time.

## Testing without a TV

```sh
lgp-tool run mine.wasm --set playlist_url=https://example.com/list.m3u --secret password=hunter2
lgp-tool run mine.lgp --search "bear"
lgp-tool run mine.lgp --resolve "play:his_girl_friday"
lgp-tool run mine.wasm --fixtures ./fixtures --now 1767225600000
```

`--set` and `--secret` stand in for the fields the user would fill. Live fetches follow the
TV's rules, the allow list on every redirect and loopback and the home network only where the TV
would allow them, so a host refused here is refused on the TV too. `--state file` keeps the
key-value store in a JSON file between runs, as the TV keeps it between refreshes, so a sign-in
that takes a refresh or two can be walked through. `--search` runs `search`
instead of `refresh`, and `--resolve action` runs `resolve` for one action and prints what came
back. Each run keeps what the TV would keep, the first item with each id and at most 2000 of
them, or 12 search results, and prints the TV's notes about the rest. It prints the fuel it
used; `--json` prints the items as JSON. After the items come any pictures the TV would not show:
one from a host outside `fetch_hosts`, or a tile's picture over 4 MB among the fixtures.
`--artwork` downloads every picture the way the TV does to find the rest. `--fixtures dir` answers every `fetch` from a file
named `<sha256 of the URL>.body` in that directory. A request that is not a GET, or has a body,
first looks for `<sha256 of "METHOD url\nbody">.body`, so each step of a sign-in or each
JSON-RPC call can have its own answer; when a fixture is missing, the error names both files.
`--now` gives the refresh a fixed time in milliseconds, so with fixtures a test runs the same
way every time and offline. A binary body reaches the plugin as `body_base64`, as
on the TV, and one over 8 MB fails the same way. An optional `<sha256>.head` beside it gives the
status on its first line and headers on the lines after; without one the answer is 200 with no
headers. Unit-test your parsing in ordinary Rust: the SDK's host functions are absent off wasm,
so keep parsing in functions that take strings or readers, as `plugins/iptv` does with
`parse_m3u` and `parse_xmltv`.

## Bundling, signing and updates

A `.lgp` is a zip with `manifest.json`, `plugin.wasm`, `publisher.pub` (an Ed25519 public key,
32 bytes or 64 hex characters) and `signature.sig` (64 bytes or 128 hex characters) over
`"lounge-plugin-v1\0"`, the length-prefixed manifest, then the length-prefixed module.

`lgp-tool keygen` writes a signing key once, readable only by you, and refuses to overwrite one
that exists; keep it private and back it up. The TV pins your
public key the first time it installs your plugin and refuses a later bundle signed by any other
key, so a new key means your users have to remove and reinstall. `lgp-tool bundle` signs,
`lgp-tool inspect` verifies and prints the manifest. Installing a bundle with the same `id`
again is an update: the module and manifest are replaced, settings and the key-value store are
kept, and the plugin refreshes.

## Limits

| | |
|---|---|
| Memory | 32 MB of linear memory per call, one memory, tables of up to 100,000 entries |
| Fuel | 20 billion instructions per call, handed out in slices; a runaway loop ends at its time limit or with "out of fuel". A start function gets the first 10 million |
| Host calls | 10,000 fuel each, plus one per byte they move |
| Time | 3 minutes for a refresh, 12 s for `search`, `resolve` and the manifest |
| Fetches | 64 per call, 60 s of waiting in total, 20 s and 8 MB each, and 16 MB once encoded for the plugin |
| Result | 8 MB of JSON from a refresh, 256 KB from `search`, `resolve` and the manifest |
| Items | 2,000 per refresh, each id once; past the first 4,000 sent, items are only counted |
| Artwork | 4 MB per picture, 12 MB for a wallpaper; 600 new pictures, 192 MB and two minutes per refresh |
| Key-value | 256 keys of up to 256 bytes, values of 256 KB, 1,000 writes per call |
| Log | 500 lines per call of up to 4 KB each, the last 200 kept |
| Screensavers | 8 per plugin, each a program of up to 32 KB with up to 24 uniforms of up to 16 numbers |

## The wire interface, for other languages

Any language that targets core WebAssembly works; the SDK is a convenience. A module exports
`memory`, `lg_alloc`, `lg_free`, `lg_manifest`, `lg_refresh`, and optionally `lg_search` and
`lg_resolve`, and imports the host functions from a module named `lounge`. Strings cross the
boundary as UTF-8 JSON in linear memory, and a `(ptr, len)` pair is packed into one `i64` as
`ptr << 32 | len`; `0` means nothing.

| Export | Signature | Meaning |
|---|---|---|
| `memory` | | linear memory |
| `lg_alloc(len) -> ptr` | `i32 -> i32` | allocate `len` bytes the host may write into |
| `lg_free(ptr, len)` | `i32, i32` | free memory the plugin returned |
| `lg_manifest() -> packed` | `-> i64` | `{"ok": Manifest}` |
| `lg_refresh(ptr, len) -> packed` | `i32, i32 -> i64` | `RefreshContext` in; `{"ok": [Item]}` or `{"err": "…"}` out |
| `lg_search(ptr, len) -> packed` | optional | `SearchQuery` (`query`, `limit`) in; items out |
| `lg_resolve(ptr, len) -> packed` | optional | action string in; `Action` out |

| Import (module `lounge`) | Signature | Meaning |
|---|---|---|
| `fetch(req_ptr, req_len) -> packed` | | `HttpRequest` in; `{"ok": HttpResponse}` or `{"err": …}` out |
| `kv_get(k, kl) -> packed`, `kv_set(k, kl, v, vl)`, `kv_delete(k, kl)` | | the private store |
| `now() -> i64` | | milliseconds since the epoch |
| `log(level, ptr, len)` | | 0 debug, 1 info, 2 warn, 3 error |
| `secret(k, kl) -> packed` | | a `password` setting's value |
| `setting(k, kl) -> packed` | | any other setting's value, or its default |

Memory the host hands to the plugin (fetch responses, settings) is allocated with the plugin's
`lg_alloc` and belongs to the plugin; memory the plugin returns is freed by the host with
`lg_free`. `lg_alloc` must not call host functions: the call fails if it does. Keys longer
than 256 bytes are an error, not cut short. The JSON shapes are the structs in `core/lounge-plugin-api/src/lib.rs`, which is the
one definition the host and the SDK share.

## Checklist before you share it

- The `id` is yours and will never change.
- `fetch_hosts` lists only what the plugin needs: `$key` for a server the user names, `*` only for
  pictures and pages that may come from anywhere.
- Passwords are `password` settings read through `secret`, never logged.
- `lgp-tool run --artwork` shows the items you expect and no pictures the TV would not show,
  and the log says what happened in a sentence a user could read on the TV.
- A refresh that fails returns an error naming the cause and the fix ("Jellyfin didn't answer
  at 192.168.0.20. Check the server, then try again."), not a stack trace.
- Items keep stable `id`s between refreshes, so pins and hidden tiles survive.
- A screensaver looks right in every theme, Blackout included, and its log line shows it holding
  a third or a quarter of the resolution on the slowest TV you have.
- The bundle is signed with a key you have backed up.
