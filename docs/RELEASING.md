# Releasing

Pushing a tag like `v0.2.0` builds the APK, runs the tests, and attaches the APK to a GitHub
release as `Lounge-0.2.0.apk` with a `SHA256SUMS` file. TVs running Lounge pick it up within
half a day.

Beside the APK go its baseline profiles: `Lounge-0.2.0.api28.dm` for Android 9 to 11 and
`Lounge-0.2.0.api31.dm` for Android 12 and later, both listed in `SHA256SUMS`. A TV installs the
one for its Android with the APK, and Android then compiles Lounge as part of the install instead
of leaving it to the next night. The profiles are built from
`android/app/src/main/baseline-prof.txt`; when the home screen's code has changed a good deal,
record that file again with `tools/baseline-profile.sh <adb serial>` before tagging, on a device
with Android 13 or later.

The release stops before it builds unless:

- the tag matches `versionName` in `android/app/build.gradle.kts`, and `versionCode` is higher
  than at the previous tag;
- the repository has the app's signing key as `LOUNGE_KEYSTORE_BASE64` (the keystore, base64
  encoded), with `LOUNGE_KEYSTORE_PASSWORD`, `LOUNGE_KEY_ALIAS` and `LOUNGE_KEY_PASSWORD`;
- the repository has the plugin publisher key as `LOUNGE_PUBLISHER_KEY`, the contents of
  `~/.config/lounge/publisher.key`.

Without the keys, a debug-signed APK would not install over a signed one, and the built-in
plugins would carry a key made on the spot, which every TV would then pin instead of yours.
Store the publisher key with:

```sh
gh secret set LOUNGE_PUBLISHER_KEY < ~/.config/lounge/publisher.key
```
