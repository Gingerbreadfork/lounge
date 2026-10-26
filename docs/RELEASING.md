# Releasing

Pushing a tag like `v0.2.0` builds the APK, runs the tests, and attaches the APK to a GitHub
release as `Lounge-0.2.0.apk` with a `SHA256SUMS` file. TVs running Lounge pick it up within
half a day.

The release stops before it builds unless:

- the tag matches `versionName` in `android/app/build.gradle.kts`, and `versionCode` is higher
  than at the previous tag;
- the repository has the app's signing key as `LOUNGE_KEYSTORE_BASE64` (the keystore, base64
  encoded), with `LOUNGE_KEYSTORE_PASSWORD`, `LOUNGE_KEY_ALIAS` and `LOUNGE_KEY_PASSWORD`;
- the repository has the plugin publisher key as `LOUNGE_PUBLISHER_KEY`, the contents of
  `~/.config/lounge/publisher.key`.

Without the keys, a debug-signed APK would not install over a signed one, and plugins signed
with another key would be refused by every TV that pinned yours. Store the publisher key with:

```sh
gh secret set LOUNGE_PUBLISHER_KEY < ~/.config/lounge/publisher.key
```
