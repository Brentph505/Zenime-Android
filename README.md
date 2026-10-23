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

Upstream provider implementations may change or use anti-bot protections. Some hosts return embed-only players or encrypted JavaScript payloads rather than direct HLS/MP4/DASH streams. Direct HLS playback uses an app-local loopback proxy that forwards Referer/Origin and Range headers and rewrites nested HLS playlists, keys and segment URLs. The proxy binds only to 127.0.0.1 and uses a per-session random token. Segment bytes are streamed instead of buffering full media files in memory. Providers that return an iframe rather than a direct stream can be opened in a dedicated Android WebView activity, with JavaScript and fullscreen support; it does not use an external public iframe proxy. Provider responses, anti-bot protections, subtitle paths, and skip-time integration still require live verification.

## Brand assets and UI

The Android app now carries the Zenime web project's launcher artwork, white/black logo and wordmark variants, discover banner, loading art and not-found art. The native Slint UI uses the actual Zenime wordmark and discover banner; a local SVG icon set covers common navigation and actions. Android launcher branding is set in the manifest. Assets are stored locally in the APK and do not require a network request.

## Web-to-native parity status

This is an active native port, not a claim of 100% feature parity yet. Website features that require dedicated native work are tracked here:

- [ ] Home airing schedule with day selector and full episode cards
- [ ] Search dropdown/keyboard navigation, genre filters, full sorting and pagination
- [ ] Full media info page: recommendations, related media, studios, trailer, metadata tabs, card/list/number episode views
- [ ] Native video player controls parity: source switching, subtitle tracks/styles, intro/outro timestamps, keyboard/touch shortcuts, watch progress resume and completion sync
- [ ] Manga reader parity: provider switching, continuous/single-page modes, chapter list/search, preloading, reading progress/bookmarks, fullscreen and chapter navigation
- [ ] AniList account parity: full editable list entries (score/progress/rewatch), full statistics/profile view, notification feed and read/mark-read actions, history/bookmark synchronization
- [ ] History parity: per-episode watch records, date groups, search/filter/sort, remove individual/group/all, continue watching, manga reading history and sync controls
- [ ] Full settings parity: theme, language/title preferences, provider defaults, sync threshold, spoiler/NSFW display and storage controls, restore defaults and data management
- [ ] Dedicated Studio, Policy/Terms, About and 404 screens
- [ ] Offline/error/loading states, accessible icon labels, screen rotation and small-device layout review

## Features

The app includes AniList-backed trending/popular/top-rated catalogue views, title search with filters, anime/manga details, provider episode/chapter lists, manga reader, saved titles, local history, settings, AniList account/list hooks, and a native Media3 player. AniList OAuth and list mutations still require end-to-end verification using an OAuth client ID registered for the app's custom zenime://callback redirect.

## Validation

Builds and CI have intentionally not been run during this implementation pass, as requested. The source still needs a compile pass and on-device verification before it can be called release-ready.

## Attribution

Zenime is derived from Miruro no Kuon under the source repository's attribution and non-commercial license terms. Preserve those terms when modifying or redistributing the app.
