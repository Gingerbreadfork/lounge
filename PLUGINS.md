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

- `plugins/sample`: eighteen public-domain films with posters and stills, some part-watched. No
  settings, no network beyond the pictures. The best one to copy from.
- `plugins/iptv`: an M3U playlist becomes channels, an XMLTV feed fills the guide. Settings,
  gzip, parsing a large document.
- `plugins/jellyfin` and `plugins/emby`: sign in to a server with a password, publish resume
  points and new arrivals, answer search. Secrets, a cached token in the key-value store, deep
  links into another app.
- `plugins/plex`: the same with a token instead of a password, and pictures through the
  server's resizer.
- `plugins/youtube`: a list setting parsed into sources, an Atom feed read with `quick-xml`,
  partial failure reported without giving up.
- `plugins/archive`: rows from a search API, and `resolve`, which looks up the file to play only
  when something is selected.
- `plugins/immich`: `Kind::Wallpaper` items that feed the screensaver instead of a row.
- `plugins/hdhomerun`: `Kind::Epg`, channels that play in the TV's player, a guide window from
  `RefreshContext`, and finding a device through a discovery service.
- `plugins/tvheadend`: basic auth for the API and a token on the addresses the TV fetches
  itself, paging through a large guide, recordings as a row.
- `plugins/channels`: a DVR's recordings split between Continue watching and a row by their
  playback position.
- `plugins/nextpvr`: a session signed in with a digest, a guide gathered across refreshes with
  the key-value store so one refresh never exceeds the fetch budget, and `resolve` so play
  addresses always carry a live session.

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

The core loads your module fresh for every call, with a fuel budget and a memory cap, so a
runaway loop or a leak ends that call and nothing else. There is no state between calls apart
from the key-value store. `refresh` runs when the plugin is turned on, every `refresh_minutes`
while the screen is on, when a setting changes, and when someone asks for it on the phone.
`search` runs as the user types on the TV's search screen. `resolve` runs when the user
selects an item whose action says `resolve`.

The host fetches artwork for you, four pictures at a time, caps each at 4 MB, scales it to
every size the screen needs and takes the palette that lights the focused tile. Give it a
direct image URL under one of your `fetch_hosts`; a picture that fails to download is simply a
tile without one, so prefer modestly sized files.

## The manifest

```json
{
  "id": "tv.lounge.iptv",
  "name": "IPTV playlist",
  "version": "0.1.0",
  "description": "Channels from an M3U playlist, with programme information from an XMLTV guide.",
  "publisher_name": "Lounge",
  "kinds": ["content", "epg"],
  "fetch_hosts": ["*"],
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
| `kinds` | What the plugin does: `content` (items and rows), `epg` (channels and schedule), `wallpaper`, `search`. |
| `fetch_hosts` | Hosts `fetch` may reach, including on redirects. `*` means any host and is shown to the user as such; a plugin that talks to a server the user names must declare it. Loopback and the TV's own addresses are never implied by `*`. |
| `refresh_minutes` | How often `refresh` runs while the screen is on; at least 5, 0 means the host's default. |
| `rows` | Rows of your own, in order. `style` is `landscape` (16:9 tiles), `poster` (portrait) or `compact`. Items name a row by its `id`. |
| `settings` | Fields the user fills in on the TV or the phone. `kind` is `text`, `url`, `password`, `number` or `toggle`. A `password` is only readable through `secret`, never through `setting`, and is never shown back. |
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
picture for the screensaver: give it the picture as `thumbnail` (it is prepared at 1920×1080),
a `title` and, if you like, a `subtitle`; it is the one kind that needs no `action`.

**What a tile shows.** `title` is the first line; `subtitle` the second, in the launcher's own
voice (commas between facts, never dots: "S2 E4, Netflix"). `season` and `episode` are shown
as "S2 E4" when both are given. `duration_ms` with `position_ms`, or `progress` from 0 to 1,
draws the progress bar and "19 minutes left". `last_used_ms` orders Continue watching, newest
first. `added_ms` and `is_new` mark new arrivals. `weight` orders a row when recency does not.

**Artwork.** `thumbnail` is 16:9 and feeds landscape tiles and the Stage's full-bleed picture.
`poster` is portrait and feeds poster rows; on its own it is also cropped for landscape tiles and
framed on the Stage over a soft backdrop. `logo` is a channel or publisher mark, fitted and never
cropped. Give both `thumbnail` and `poster` when you have them.

**Channels.** A `channel` item carries `channel_id`, `channel_name` and `channel_number`;
`schedule` items name the same `channel_id` with `start_ms` and `end_ms`. The guide window the
host wants is in `RefreshContext.window_hours`.

## Actions

What happens when the user selects an item. The first field that is set wins, in this order:

| Field | What it does |
|---|---|
| `play` | A stream URL, opened in whatever video player the TV has. `title` is handed to the player. |
| `view` | An Android URI: a deep link such as `jellyfin://…`, a web address, or an `intent:` string. `fallback_launch` runs if nothing on the TV handles it. |
| `launch` | A package name to open. |
| `resolve` | A string handed back to your `resolve` at selection time, which returns one of the above. Use it when the stream URL is only known at the moment of playing. |

## The host

Everything a plugin can do, as the SDK exposes it under `host`:

| Function | What it does |
|---|---|
| `fetch(request)` | An HTTP request to a host in `fetch_hosts`. Bodies are capped at 8 MB; a response body is text when it is valid UTF-8, otherwise base64 in `body_base64`. Redirects are followed only to allowed hosts. At most 64 calls per invocation, sharing a 60 s budget. |
| `get_text(url)` | `fetch` for a GET that must return text, failing on anything but 2xx. |
| `kv_get`, `kv_get_string`, `kv_set`, `kv_delete` | A private store: 256 keys of up to 256 KB each. Survives restarts and updates; cleared when the plugin is removed. Cache tokens and the last refresh's result here. |
| `setting(key)` | A setting's value, or its manifest default. |
| `required_setting(key, label)` | The same, or an error naming the field for the user. |
| `secret(key)` | A `password` setting's value. Only keys the manifest declares as passwords. |
| `now_ms()` | Milliseconds since the epoch. |
| `log(level, message)` | 0 debug, 1 info, 2 warn, 3 error. `sdk::info!`, `warn!` and `error!` format for you. Shown on the plugin's page on the TV and by `lgp-tool run`. |

`RefreshContext` tells `refresh` the time, the UTC offset in minutes, the device tier (`a`, `b`
or `c`, so a plugin can fetch less on a small TV), when the last successful refresh finished,
and how many hours of schedule are wanted. `epoch_ms(...)` in the SDK turns a civil date into
milliseconds without a calendar crate.

## Testing without a TV

```sh
lgp-tool run mine.wasm --set playlist_url=https://example.com/list.m3u --secret password=hunter2
lgp-tool run mine.lgp --search "bear"
lgp-tool run mine.lgp --resolve "play:his_girl_friday"
lgp-tool run mine.wasm --fixtures ./fixtures
```

`--set` and `--secret` stand in for the fields the user would fill. `--search` runs `search`
instead of `refresh`, and `--resolve action` runs `resolve` for one action and prints what came
back. `--fixtures dir` answers every `fetch` from a file named
`<sha256 of the URL>.body` in that directory, so a test runs the same way every time and
offline. Unit-test your parsing in ordinary Rust: the SDK's host functions are absent off wasm,
so keep parsing in functions that take strings, as `plugins/iptv` does with `parse_m3u` and
`parse_xmltv`.

## Bundling, signing and updates

A `.lgp` is a zip with `manifest.json`, `plugin.wasm`, `publisher.pub` (an Ed25519 public key,
32 bytes or 64 hex characters) and `signature.sig` (64 bytes or 128 hex characters) over
`"lounge-plugin-v1\0"`, the length-prefixed manifest, then the length-prefixed module.

`lgp-tool keygen` writes a signing key once; keep it private and back it up. The TV pins your
public key the first time it installs your plugin and refuses a later bundle signed by any other
key, so a new key means your users have to remove and reinstall. `lgp-tool bundle` signs,
`lgp-tool inspect` verifies and prints the manifest. Installing a bundle with the same `id`
again is an update: the module and manifest are replaced, settings and the key-value store are
kept, and the plugin refreshes.

## Limits

| | |
|---|---|
| Memory | 32 MB of linear memory per call |
| Fuel | 20 billion instructions per call; a runaway loop ends with "out of fuel" |
| Fetches | 64 per call, 60 s of waiting in total, 8 MB per body |
| Result | 8 MB of JSON per call; keep item counts in the hundreds |
| Artwork | 4 MB per picture, fetched by the host |
| Key-value | 256 keys of 256 KB |
| Log | 500 lines per call, the last 200 kept |

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
| `setting(k, kl) -> packed` | | any setting's value, or its default |

Memory the host hands to the plugin (fetch responses, settings) is allocated with the plugin's
`lg_alloc` and belongs to the plugin; memory the plugin returns is freed by the host with
`lg_free`. The JSON shapes are the structs in `core/lounge-plugin-api/src/lib.rs`, which is the
one definition the host and the SDK share.

## Checklist before you share it

- The `id` is yours and will never change.
- `fetch_hosts` lists only what the plugin needs; `*` only when the user names the server.
- Passwords are `password` settings read through `secret`, never logged.
- `lgp-tool run` shows the items you expect, with artwork URLs that resolve, and the log says
  what happened in a sentence a user could read on the TV.
- A refresh that fails returns an error naming the cause and the fix ("Jellyfin didn't answer
  at 192.168.0.20. Check the server, then try again."), not a stack trace.
- Items keep stable `id`s between refreshes, so pins and hidden tiles survive.
- The bundle is signed with a key you have backed up.
