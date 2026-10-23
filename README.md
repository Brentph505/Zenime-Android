# Zenime Android

Rust-powered native Android client for [Zenime](https://github.com/Brentph505/Zenime). This is a standalone mobile project; it does not replace the Zenime website.

## App architecture

- **Rust** owns catalog requests, title search, anime details, episode lookup, and networking.
- **Slint** draws the interface as compiled native code; there is no WebView and no embedded website.
- **AndroidX Media3 / ExoPlayer** handles direct HLS/MP4 playback in a native Android activity.
- The visual style follows Zenime's existing dark palette, cards, and lavender accent.
- Results marked as adult by the metadata API are filtered out.

The native player accepts direct HLS and media URLs. It does not load provider embed pages inside a WebView, so embed-only sources may not play.

## Build requirements

Install Rust, Java 17, Android SDK / NDK, CMake, Gradle 8.13, and `cargo-ndk`. The GitHub Actions workflow installs the Android SDK and NDK for hosted builds.

## Build locally

From the repository root:

```bash
rustup target add aarch64-linux-android
cargo install cargo-ndk --locked
cargo ndk -t arm64-v8a -o android/app/src/main/jniLibs build --release --lib
cd android
gradle --no-daemon :app:assembleDebug
```

The resulting debug APK is at `android/app/build/outputs/apk/debug/app-debug.apk`. It is intended for testing, not Play Store distribution.

## CI build

Push to `main` or use the workflow's **Run workflow** button in GitHub Actions. The workflow builds the arm64 APK and publishes an artifact named `zenime-native-debug-apk`. Download it from the completed workflow run.

For public distribution, create a signed release build with a protected release keystore. Do not commit keystores, signing passwords, API keys, or other secrets.

## Configuration and limitations

The initial app includes Trending, Popular, Top Rated, Latest, title search, anime details, episode listing, a session-only saved list, and native direct-stream playback. The metadata / video API is configurable in the app's Settings screen. The default endpoint is a configurable example, not a guaranteed service; if it is unavailable, set a compatible Zenime / Miruro API endpoint.

Backend responses and provider formats vary. In particular, direct playback is supported, but embedded-only streams are not. The APK build and app behavior must be validated on a real device before treating the app as a stable release.

## Attribution and license

Zenime is derived from Miruro no Kuon under the repository's custom attribution, non-commercial license. Keep the original root project's license terms and required attribution when modifying or redistributing this derivative. This project is intended for non-commercial use.
