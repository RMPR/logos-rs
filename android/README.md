# LeVerbe for Android

The desktop app's library (`logos_rs::app`) run by an Android
`NativeActivity`, in its touch layout. It is aimed at e-ink tablets: the
E-ink theme is the default, pages turn instead of scrolling, and nothing
animates. The Bible database ships in the APK (`assets/bible.db`, which the
APK's own compression packs to a third) and is unpacked into the app's storage on first launch, and again after an
update that brings a different one. The session (`session.json`) is kept
beside it.

Not in this build: importing translations from files. That needs Android's
file picker (the Storage Access Framework), which is a Java API.

## Building

You need the Android SDK (platform 34, and build-tools 35 or later: the
`d8` of build-tools 34 fails on classes compiled by JDK 21), the NDK (r26 or
later) and a JDK, then:

```sh
rustup target add aarch64-linux-android
cargo install cargo-apk

export ANDROID_HOME=/path/to/sdk ANDROID_NDK_ROOT=/path/to/ndk

cargo run --release --bin import        # data/bible.db, if not built yet
tools/android-db.sh                      # → android/assets/bible.db
cargo apk build -p leverbe-android --release
```

The APK lands in `target/release/apk/LeVerbe.apk`. A release build has to be
signed: point `CARGO_APK_RELEASE_KEYSTORE` and
`CARGO_APK_RELEASE_KEYSTORE_PASSWORD` at a keystore (see `keytool
-genkeypair`). Keep using the same one: Android installs an update only over
an app signed with the same key. `cargo apk build -p leverbe-android` (no
`--release`) signs with the debug key instead, and `cargo apk run -p
leverbe-android` installs and starts it on a tablet connected over `adb`.

The release workflow builds the APK alongside the desktop packages. It signs
with the keystore in the `ANDROID_KEYSTORE` secret (base64) and its password
in `ANDROID_KEYSTORE_PASSWORD`; without them it makes a throwaway key, and an
APK signed with one cannot update the previous release in place — uninstall
first.

## Trying the touch layout on a desktop

```sh
LEVERBE_TOUCH=1 cargo run --bin logos-rs
```

runs the desktop build with the tablet's layout, theme and fonts; narrow the
window below 1100 px to see the upright (portrait) arrangement.
