# Zenime Android

Rust-powered native Android client for [Zenime](https://github.com/Brentph505/Zenime). This is a separate app; it does not modify or embed the website.

## Native design

- Rust owns catalog parsing, AniList GraphQL requests, provider discovery, episode resolution, manga chapter lookup, image decoding and app state.
- Slint renders the interface as compiled native code; there is no website WebView.
- AndroidX Media3 / ExoPlayer plays direct HLS, MP4 and DASH media.
- Minimum Android version is Android 8.1 (API 27), suitable for a Vivo Y91c-class device.
- AniList is the canonical anime/manga metadata source. Adult-marked titles are filtered from results, and adult-only provider families are excluded.

## Data flow

The app queries AniList directly over HTTPS for catalog and details. Provider requests run from the device and attempt Zenime Web's regular anime provider priority: AniKoto, AnimeParadise, 4Animo, ReAnime, KickAssAnime, AnimePahe, XAnime, AniPM and AniDB. Manga source priority is MangaHere, Atsumaru, MangaPill and MangaKatana.

A legacy metadata endpoint can be configured in Settings as a fallback, but it is no longer the default source for the catalogue or detail metadata. Direct scraping still requires an internet connection to AniList and the upstream providers. Provider URLs are visible in network traffic and APK code; the app does not attempt to bypass provider access controls.

Upstream provider implementations may change or use anti-bot protections. Some hosts return embed-only players or encrypted JavaScript payloads rather than direct HLS/MP4/DASH streams. The native player supports direct media manifests and cannot play arbitrary embed pages. The current port includes device-side adapters for the provider-specific endpoints and common stream extraction paths, but it must not be considered full provider parity until every resolver, subtitle path, skip-time integration and manga image decoding path is verified against live provider responses.

## Features

The app includes AniList-backed trending/popular/top-rated catalogue views, title search with filters, anime/manga details, provider episode/chapter lists, manga reader, saved titles, local history, settings, AniList account/list hooks, and a native Media3 player. AniList OAuth and list mutations still require end-to-end verification using an OAuth client ID registered for the app's custom zenime://callback redirect.

## Validation

Builds and CI have intentionally not been run during this implementation pass, as requested. The source still needs a compile pass and on-device verification before it can be called release-ready.

## Attribution

Zenime is derived from Miruro no Kuon under the source repository's attribution and non-commercial license terms. Preserve those terms when modifying or redistributing the app.
