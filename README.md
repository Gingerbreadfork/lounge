<p align="center">
  <img src="docs/logo.png" width="160" alt="">
</p>

<h1 align="center">Lounge</h1>

<p align="center">
  A home screen for Android TV and Google TV, laid out around what you actually watch.<br>
  Quick on the remote, calm to look at, and nothing leaves the house.
</p>

<p align="center">
  <img src="docs/screenshots/home.png" width="920" alt="The Lounge home screen: a still from Charade large on the right, the title, year, director and a Continue button on the left, and a strip of what to watch next along the bottom">
</p>

## At a glance

<table>
<tr>
<td width="33%" valign="top">
<img src="docs/icons/play.svg" width="28" height="28" alt=""><br>
<b>The Stage</b><br>
One thing at a time: the episode to continue, the channel that is on, the app you open at this hour.
</td>
<td width="33%" valign="top">
<img src="docs/icons/tv.svg" width="28" height="28" alt=""><br>
<b>The whole TV</b><br>
Inputs named by their devices and, on a streamer, the TV's power, input and sound.
</td>
<td width="33%" valign="top">
<img src="docs/icons/apps.svg" width="28" height="28" alt=""><br>
<b>Rows you own</b><br>
Pin, reorder and hide. Your apps in your order, and games with a badge when a controller is in.
</td>
</tr>
<tr>
<td width="33%" valign="top">
<img src="docs/icons/resume.svg" width="28" height="28" alt=""><br>
<b>Continue watching</b><br>
Every app that publishes Watch Next, the TV's own listings and your media server, one entry per title.
</td>
<td width="33%" valign="top">
<img src="docs/icons/theme.svg" width="28" height="28" alt=""><br>
<b>Light, not chrome</b><br>
The focused tile's own colour spills onto the wall. Ten themes, Blackout and High contrast among them.
</td>
<td width="33%" valign="top">
<img src="docs/icons/phone.svg" width="28" height="28" alt=""><br>
<b>Your phone as a remote</b><br>
A page the TV serves, with no account: it types, arranges the home screen and installs plugins.
</td>
</tr>
<tr>
<td width="33%" valign="top">
<img src="docs/icons/guide.svg" width="28" height="28" alt=""><br>
<b>Live TV and a guide</b><br>
Tuner and plugin channels in Live now, and a guide laid out to the minute.
</td>
<td width="33%" valign="top">
<img src="docs/icons/frame.svg" width="28" height="28" alt=""><br>
<b>A screensaver of its own</b><br>
Your artwork or nine abstract scenes, which can also run live behind the home screen.
</td>
<td width="33%" valign="top">
<img src="docs/icons/plugin.svg" width="28" height="28" alt=""><br>
<b>Fifteen plugins</b><br>
Jellyfin, Plex, Kodi, IPTV, Trakt, Twitch and more, each sandboxed WebAssembly in a signed bundle.
</td>
</tr>
</table>

<p align="center"><sub>Keeps itself current from this repository's releases &nbsp;·&nbsp; Android 9 and up, arm64 and 32-bit ARM</sub></p>

## Why

The home screen that comes with a TV works for the company that made it. It is slow on the
hardware it ships on, it fills the rows with things to buy, it plays trailers you did not ask
for, and it phones home about all of it. The thing you were in the middle of watching is in
there somewhere.

Lounge starts from that thing. One picture, the title, how much is left, and a button. Under it,
rows you arrange yourself: what you are watching, your apps in your order, live channels with a
guide, the inputs on the back of the TV. It reads what the apps on the TV already publish, what
your media server knows, and what you tell it, and it keeps all of that on the TV.

## What it does

**The Stage.** The top of the screen is given to one thing at a time: the episode to continue,
the channel that is on, the app you open every evening at this hour. A strip under it holds the
next few. Move down into the rows and the Stage lowers to a band with the title still on it;
move back up and it rises again, one continuous picture, nothing squeezed.

**Rows you own.** Continue watching comes from every app that publishes it, from the TV's own
listings, and from plugins. Your apps sit in the order you pin them, sorted by how recently you
opened them until you do. Games get a row of their own, marked when a controller is connected.
Hold OK on any tile for a menu: pin, move, hide, pin it to the Stage, app info, the store page,
uninstall. Rows with nothing in them stay out of the way.

**Live TV with a guide.** Channels from the TV's tuner and from plugins show what is on now, and
a guide lays the schedule out as a grid of channels and time.

**The TV itself.** HDMI inputs with the names their devices announce over CEC and whether
anything is plugged in. On a streamer, a TV controls card turns the TV on and switches to the
right input, sends sound to a soundbar, changes the volume, or turns everything off.

**Now playing.** Music or a podcast playing in the background shows as a chip in the top bar and
a card with play, pause and skip.

**Search.** Apps, what you have been watching, and whatever the plugins can find, from one box.

**Screensaver and live backgrounds.** The screensaver shows the artwork from the Stage and the
rows with a slow push-in, or one of nine abstract scenes drawn on the TV's graphics chip in your
theme's colours. Plugins can add scenes ([PLUGINS.md](PLUGINS.md#screensavers)). Any scene can
also run behind the home screen, dimmed so the tiles stay the brightest thing in the room.

**Set up once.** Five screens on first run: how it should look, which accesses to allow, making
it the home screen, pairing a phone, done. Every one is reachable again from Settings.

<p align="center">
  <img src="docs/screenshots/rows.png" width="49%" alt="The Stage lowered to a band with the rows below it: Continue watching, Your apps, Games">
  <img src="docs/screenshots/tile-menu.png" width="49%" alt="The long-press menu beside an app tile, with the room dimmed around it">
</p>

## The phone

Settings, Phone companion shows a QR code and a six-digit code. The page at
`http://<tv>:8787` works in any browser on the same network and can be added to the phone's
home screen like an app. The TV serves it itself and remembers the phone until you forget it.

<p align="center">
  <img src="docs/screenshots/phone-home.png" width="23%" alt="What's on: the Stage and the app rows as tiles that open on the TV">
  &nbsp;
  <img src="docs/screenshots/phone-remote.png" width="23%" alt="Remote: a d-pad, media keys, a field that types on the TV, and voice">
  &nbsp;
  <img src="docs/screenshots/phone-layout.png" width="23%" alt="Layout: the rows in order with arrows to move them">
  &nbsp;
  <img src="docs/screenshots/phone-plugins.png" width="23%" alt="Plugins: the installed plugins and a place to install another from a file or a link">
</p>

- **What's on** mirrors the home screen. Tap a tile and it opens on the TV.
- **Remote** has a d-pad, Back, Home, Menu and the media keys, a field that types into whatever
  the TV has focused, and a place to paste a link that opens on the TV. With Power Setup
  connected, the keys reach whatever app is in front.
- **Layout** reorders the rows and your pinned apps, and shows what is pinned to the Stage.
- **Plugins** configures the installed plugins and installs new ones from a file or a link.
- **More** backs up your layout and settings as one file and restores it, lists the paired
  phones, and shows the TV's diagnostics.

## Plugins

A plugin is a WebAssembly module in a signed `.lgp` bundle. It runs inside the core under a fuel
budget and a memory cap, with no access to Android, the file system, sockets or other plugins:
it can only fetch from the hosts its manifest names, keep a small private store, read its
settings and log. What it publishes becomes tiles, Stage spotlights, channels in the guide or
rows of its own. The TV pins each publisher's key on first install and refuses an update signed
by anyone else. Any language that targets core WebAssembly will do: the interface is eight
imports and six exports with JSON in between.

Fifteen ship in the APK, written against the same SDK as anyone else's. Turn on the ones you use
from the phone or the TV's settings.

| Plugin | What it adds |
|---|---|
| Jellyfin, Emby, Plex | Your server's resume points in Continue watching, its newest additions as a row, and search |
| Kodi | A Kodi library: resume points, the next episodes, recent additions and Live TV, played on whichever Kodi you choose |
| Trakt | What you paused, what is next and your watchlist, whichever service they live on |
| IPTV | An M3U playlist as channels, with a guide from XMLTV |
| HDHomeRun | The tuner's channels and guide, played in the TV's own player |
| Tvheadend, Channels DVR, NextPVR | A DVR's channels, guide and recordings |
| Twitch | The channels you follow that are live, in Live now and the guide |
| YouTube channels | Channels and playlists through their public feeds, no account needed |
| Internet Archive | Free films, cartoons and television from archive.org, with search |
| Immich photos | A shared Immich album as the screensaver |
| Art mode | Public-domain paintings from the Met and the Cleveland Museum of Art on the screensaver |

A sixteenth, **Sample library**, lives in the repository but not the APK: eighteen public-domain
films with their posters, so you can see Lounge with something on it. The pictures in this file
use it. [PLUGINS.md](PLUGINS.md) takes you from nothing to a plugin on your TV.

## The look

The idea is a dim living room at night with one bright thing in it. The backdrop is dark velvet
and the focused tile is the lamp: its artwork's own colour spills onto the wall and glows around
it, the tiles beside it lift as if lit from the side, and after a minute without the remote the
lights go down. Ten themes change the walls and the lamp and nothing else.

<p align="center">
  <img src="docs/screenshots/themes.png" width="920" alt="The ten themes, each shown as the home screen with the Settings sheet open">
</p>

Type is Bricolage Grotesque and Figtree, both bundled, the three sounds are rendered rather than
sampled, and the mark is a plush coral sofa. Reduced motion follows the system or your choice,
and Lite mode turns the shader and the spill off for a TV that needs it. The whole system is
written down in [DESIGN.md](DESIGN.md).

<p align="center">
  <img src="docs/screenshots/onboarding.png" width="49%" alt="The first setup screen: ten themes in a grid with a live preview of the home screen beside them">
  <img src="docs/screenshots/settings.png" width="49%" alt="The Settings sheet over the home screen">
</p>

## Installing on a TV

Grab `Lounge-<version>.apk` from the [Releases](../../releases) page, with a `SHA256SUMS` file
beside it, or build it yourself (below). Install it over ADB from a computer on the same network,
or copy the file to the TV and open it with a file manager.

```sh
adb connect <tv>:5555
adb install -r Lounge-<version>.apk
adb shell am start -n tv.lounge.launcher/.MainActivity
```

The first launch runs the setup steps. Android does not let an app make itself the home screen,
so that step needs either a command from a computer,

```sh
adb shell cmd package set-home-activity tv.lounge.launcher/.MainActivity
```

or **Power Setup** on the TV (Settings, Power Setup, Connect). On Google TV the stock home and
its setup app outrank any home you choose, so they have to be switched off as well, which leaves
them installed:

```sh
adb shell pm disable-user --user 0 com.google.android.apps.tv.launcherx
adb shell pm disable-user --user 0 com.google.android.tungsten.setupwraith
```

`pm enable` with the same names turns them back on. Do that before you uninstall Lounge, or the
TV is left without a home screen. Power Setup does both for you and can undo them.

Power Setup talks to the TV's own debugging service once you allow it on screen. It can take the
home role, grant the optional accesses, drive the TV over HDMI-CEC, set the screensaver and
compile Lounge ahead of time. Each action shows its command first, and its page lists what ran.

After the first install Lounge keeps itself current. Twice a day it checks the
[Releases](../../releases) page, verifies a newer APK against its published hash and installs it
while you are in another app. From Android 12 there is no prompt after the first update, and
with Power Setup there is none at all; otherwise a tile on the home screen offers the update.
The first time, Play Protect on Google TV may warn that it does not know the developer: choose
More details, then Install anyway. Settings, Updates checks now or turns this off.

## Under the hood

The shell is Kotlin and Compose for TV. Everything that decides what to show is a Rust core
behind a UniFFI boundary: a graph of apps, programmes, channels and inputs with their usage,
ranking into the Stage and the rows, a SQLite store, and an artwork pipeline that decodes each
picture once and writes every size the screen needs. Plugins run in
[wasmi](https://github.com/wasmi-labs/wasmi) inside that core. The companion is an
[axum](https://github.com/tokio-rs/axum) server in the same process, with the Svelte 5 app
embedded at build time, about 30 KB gzipped.

The APK is 19 MB. On a Google TV Streamer the release build is on screen in about a second
from a cold start, sits a little over 100 MB, and keeps the main thread under 5 ms on nine
frames out of ten while you move around. A TCL television with a Mali GPU runs the same build
at about 75 MB and is bound by its GPU rather than its CPU. Both are measured with the scripts
in `tools/`.

Lounge reaches only what you gave it an address for, the artwork the TV's other apps publish,
and GitHub twice a day for updates. There are no accounts and no analytics.

## Building from source

You need JDK 21, the Android SDK with platform 37 and NDK 27, stable Rust with `cargo-ndk` and
the `wasm32-unknown-unknown` target, and Node 24 with pnpm 10.

```sh
rustup target add wasm32-unknown-unknown aarch64-linux-android armv7-linux-androideabi
cargo install cargo-ndk
(cd companion && pnpm install)
tools/plugins/build.sh                      # bundles and signs the built-in plugins into app assets
cd android && ./gradlew :app:assembleRelease
```

Gradle runs the Rust and web builds itself: it compiles the core for both ABIs with cargo-ndk,
generates the Kotlin bindings, and builds the companion with pnpm when `node_modules` is
present. The plugin publisher key is created on first use at `~/.config/lounge/publisher.key`
and never enters the tree.

There are three build types: `debug` installs beside a release as `tv.lounge.launcher.debug`,
`release` is shrunk with R8 and carries a baseline profile, and `profile` exists only to capture
that profile with `tools/baseline-profile.sh`. Release builds are signed with the debug key
unless `lounge.storeFile`, `lounge.storePassword`, `lounge.keyAlias` and `lounge.keyPassword`
are set in `~/.gradle/gradle.properties`.

Every push builds and tests the whole thing on GitHub and keeps the APK as an artifact named
`lounge-apk`. Pushing a tag publishes a release; [docs/RELEASING.md](docs/RELEASING.md) has the
rules and the secrets it needs.

## Working on it

```sh
cd core && cargo test --workspace            # the core: graph, ranking, colour, store, artwork, plugins
cd plugins && cargo test --workspace         # the SDK and the built-in plugins
cd android && ./gradlew :power:testDebugUnitTest
cd companion && pnpm exec svelte-check
cd companion && LOUNGE_TV=http://<tv>:8787 pnpm dev    # the phone app against a real TV
```

On a device, `tools/drive.sh <serial> <out dir>` walks a fixed path through the launcher and
saves a screenshot after every step, `tools/framestats.sh <serial>` prints per-phase frame times
for that path, and `tools/baseline-profile.sh <serial>` captures a fresh baseline profile from
the `profile` build. The scripts compile the installed build the way a real install ends up, so
frame times mean something.

```
android/     Gradle project: app, core/{bindings,design,platform}, collector/*, feature/home, power
core/        Rust workspace: lounge-core (UniFFI), -graph, -store, -artwork, -plugins, -companion
plugins/     Rust workspace: the plugin SDK, the built-in plugins, lgp-tool
companion/   Svelte 5 phone app, embedded into the core at build time
design/      fonts, the mark's sources and renderer, the sound renderer
docs/        the pictures in this file, and how to release
tools/       device drives, frame timing, the baseline profile, builds, the README's icons
workdir/     ignored by git: your own notes, logs and measurements
```

[DESIGN.md](DESIGN.md) is the design system every screen follows. [PLUGINS.md](PLUGINS.md) is
for writing a plugin.

## License

[MIT](LICENSE). The bundled fonts are under the SIL Open Font License; see `design/fonts/`.
