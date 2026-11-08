# Watch Next and streaming apps

Lounge reads the Android TV provider with the user-granted `READ_TV_LISTINGS` permission.
Continue watching can include Netflix when Netflix publishes entries that this permission
allows Lounge to read. Being signed into Netflix does not, by itself, establish that such
entries exist. Lounge cannot reconstruct a Netflix watch history from an empty provider.

## Reading the provider

The AndroidX TV Provider library defines `TvContractCompat.AUTHORITY` as `android.media.tv`,
so its Watch Next URI is `content://android.media.tv/watch_next_program`. AndroidX is the
library's Java package, not a second provider authority, and no authority fallback is needed.
A device test checks these URIs.

The installed app already receives `READ_EPG_DATA` from the library's merged manifest.
Declaring it again in the app does not unlock any more data.

An ADB `content query` returning `No result found` is not sufficient evidence that a table is
empty. The provider filters rows by the caller: without `READ_TV_LISTINGS`, ordinary callers
see their own rows. With it, they can also see other apps' searchable rows. The ADB shell does
not hold that permission, whereas Lounge does. Use the permissioned probe below.

Sources: [AndroidX contract](https://developer.android.com/reference/androidx/tvprovider/media/tv/TvContractCompat),
[provider query filtering](https://android.googlesource.com/platform/packages/providers/TvProvider/+/master/src/com/android/providers/tv/TvProvider.java),
[provider permissions](https://android.googlesource.com/platform/packages/providers/TvProvider/+/refs/heads/main/AndroidManifest.xml).

## Launcher behaviour

- Each observer subscription requests a refresh after registering. This covers first launch,
  permission grants and returning home after playback, when the observer was stopped while
  the player was in front.
- A malformed metadata row is skipped individually, so one publisher's invalid value cannot
  discard every other entry from that table.
- Preview channels are discoverable in Lounge's channel settings even when the stock launcher
  marks the channel non-browsable. Lounge's own enabled-channel settings still decide which
  rows appear; individually hidden programmes and hidden tuner channels remain excluded.
- Collection logs show accepted counts by content kind; observer registration failures are
  logged instead of silently ignored.

## Artwork fallback

Listings are published with cached artwork before any downloads finish. A cache miss leaves
a playable tile with its title and theme colours, including in the Stage's filmstrip. An older
cached image remains usable while it refreshes or when the source is offline.

Four background workers fetch missing or expired artwork, prioritising Watch Next. Completed
images are applied in batches about every half-second. Updating progress, adding a title or
removing one does not wait for those workers; a late image can only update the current listings.
Disabled preview channels are still discoverable, but their images are not downloaded until
the row is enabled in Lounge. Shared image URLs are fetched once per refresh.

## Verification

The collector's instrumentation tests use an in-memory provider for synthetic metadata;
they do not insert anything into the TV's real listings or Netflix history. They cover
system URIs, resume metadata, hidden/untitled rows, stable IDs, malformed rows, channel discovery,
and refreshing after resubscription without a change notification. Artwork tests cover stalled
downloads, metadata updates and removals during a fetch, independent image completion, bounded
concurrency, disabled rows, shared URLs, stale cache fallback, and unavailable images.

```sh
cd android
ANDROID_SERIAL=<tv>:5555 ./gradlew :collector:tvprovider:connectedDebugAndroidTest
```

To also inspect the real provider, explicitly opt into the read-only probe on a TV with a
TV provider. It grants TV-listings permission to the separate test APK, logs counts grouped
by publisher and visibility, and checks the same Watch Next cursor parser Lounge uses.
It does not print titles, deep links or account information.

```sh
ANDROID_SERIAL=<tv>:5555 ./gradlew :collector:tvprovider:connectedDebugAndroidTest \
  -Pandroid.testInstrumentationRunnerArguments.tvProviderProbe=true
adb -s <tv>:5555 logcat -d -s LoungeTvTest:I LoungeTv:D
```

If Gradle's connected-test harness reports a failure after all test cases passed, the runner
can be invoked directly. This also sidesteps streamed-install stalls on some TVs:

```sh
./gradlew :collector:tvprovider:assembleDebugAndroidTest
adb -s <tv>:5555 install --no-streaming -r \
  collector/tvprovider/build/outputs/apk/androidTest/debug/tvprovider-debug-androidTest.apk
adb -s <tv>:5555 shell am instrument -w -e tvProviderProbe true \
  tv.lounge.collector.tvprovider.test/androidx.test.runner.AndroidJUnitRunner
```

For Netflix playback verification, open a profile, play an unfinished title, return home,
and rerun the probe after Netflix has had time to publish. If the permissioned probe still
finds no Netflix rows, Netflix is not publishing them on that device, and nothing in Lounge
can change that.
