use reqwest::blocking::Client;
use serde_json::Value;
use slint::{Image, ModelRc, SharedString, VecModel};
use std::collections::HashSet;
use std::sync::{Arc, Mutex};
use std::time::Duration;

slint::include_modules!();

const DEFAULT_API_BASE: &str = "https://public-miruro-consumet-api.vercel.app/";

#[cfg(target_os = "android")]
type PlatformApp = slint::android::AndroidApp;
#[cfg(not(target_os = "android"))]
type PlatformApp = ();

#[derive(Clone, Debug)]
struct AnimeRecord {
    id: String,
    title: String,
    details: String,
    image_url: String,
}

#[derive(Clone, Debug, Default)]
struct SelectedAnime {
    id: String,
    title: String,
    media_type: String,
}

type SharedApiBase = Arc<Mutex<String>>;
type SharedSelected = Arc<Mutex<SelectedAnime>>;
type SharedCard = Arc<Mutex<Option<AnimeCardData>>>;
type SharedSaved = Arc<Mutex<Vec<AnimeCardData>>>;
type SharedHistory = Arc<Mutex<Vec<AnimeCardData>>>;

#[derive(Clone, Default)]
struct SearchFilters {
    year: String,
    season: String,
    format: String,
    status: String,
    sort: String,
}

#[cfg(target_os = "android")]
#[unsafe(no_mangle)]
fn android_main(app: PlatformApp) {
    if let Err(error) = slint::android::init(app.clone()) {
        eprintln!("Could not initialize Slint Android backend: {error}");
        return;
    }
    if let Err(error) = run_window(Some(app)) {
        eprintln!("Zenime Android UI failed: {error}");
    }
}

#[cfg(not(target_os = "android"))]
pub fn run() -> Result<(), slint::PlatformError> {
    run_window(None)
}

fn run_window(platform_app: Option<PlatformApp>) -> Result<(), slint::PlatformError> {
    let ui = MainWindow::new()?;
    let weak = ui.as_weak();

    #[cfg(target_os = "android")]
    let saved_api = platform_app.as_ref().and_then(|app| read_android_preference(app, "api_base"));
    #[cfg(not(target_os = "android"))]
    let saved_api: Option<String> = None;

    let initial_api = std::env::var("ZENIME_API_BASE_URL")
        .ok()
        .or(saved_api)
        .unwrap_or_else(|| DEFAULT_API_BASE.to_string());
    let api_base: SharedApiBase = Arc::new(Mutex::new(normalize_base(&initial_api)));
    #[cfg(target_os = "android")]
    let initial_client_id = platform_app.as_ref().and_then(|app| read_android_preference(app, "anilist_client_id")).unwrap_or_default();
    #[cfg(not(target_os = "android"))]
    let initial_client_id = String::new();
    let client_id = Arc::new(Mutex::new(initial_client_id.clone()));
    let selected: SharedSelected = Arc::new(Mutex::new(SelectedAnime::default()));
    let selected_card: SharedCard = Arc::new(Mutex::new(None));
    let saved: SharedSaved = Arc::new(Mutex::new(Vec::new()));
    let history: SharedHistory = Arc::new(Mutex::new(Vec::new()));
    let current_category = Arc::new(Mutex::new("trending".to_string()));
    let current_query = Arc::new(Mutex::new(String::new()));

    #[cfg(target_os = "android")]
    if let Some(app) = platform_app.as_ref() {
        restore_cards(weak.clone(), app, "saved_items", saved.clone(), false);
        restore_cards(weak.clone(), app, "history_items", history.clone(), true);
    }

    ui.set_api_url(SharedString::from(initial_api));
    ui.set_anilist_client_id(SharedString::from(initial_client_id));
    ui.set_status(SharedString::from("Loading trending anime…"));

    {
        let weak = weak.clone();
        let api_base = api_base.clone();
        let category_state = current_category.clone();
        let query_state = current_query.clone();
        ui.on_category_selected(move |category| {
            let category = category.to_string();
            if let Ok(mut state) = category_state.lock() {
                *state = category.clone();
            }
            if let Ok(mut state) = query_state.lock() {
                state.clear();
            }
            if let Some(window) = weak.upgrade() {
                window.set_active_page(SharedString::from("home"));
            }
            load_catalog(weak.clone(), api_base.clone(), category, String::new());
        });
    }

    {
        let weak = weak.clone();
        let api_base = api_base.clone();
        let category_state = current_category.clone();
        let query_state = current_query.clone();
        ui.on_search_requested(move |query| {
            let query = query.trim().to_string();
            if query.is_empty() {
                set_status(&weak, "Type an anime title to search.");
                return;
            }
            if let Ok(mut state) = category_state.lock() {
                *state = "search".to_string();
            }
            if let Ok(mut state) = query_state.lock() {
                *state = query.clone();
            }
            if let Some(window) = weak.upgrade() {
                window.set_active_page(SharedString::from("search"));
            }
            load_catalog(
                weak.clone(),
                api_base.clone(),
                "search".to_string(),
                query,
            );
        });
    }

    {
        let weak = weak.clone();
        let api_base = api_base.clone();
        let selected = selected.clone();
        let selected_card = selected_card.clone();
        let category_state = current_category.clone();
        ui.on_anime_selected(move |id, title, image_url, poster| {
            let category = category_state.lock().map(|value| value.clone()).unwrap_or_else(|_| "trending".to_string());
            let media_type = if category == "manga" { "MANGA" } else { "ANIME" };
            open_anime_detail(
                weak.clone(),
                api_base.clone(),
                selected.clone(),
                selected_card.clone(),
                id.to_string(),
                title.to_string(),
                media_type.to_string(),
                image_url.to_string(),
                poster,
            );
        });
    }

    {
        let weak = weak.clone();
        let api_base = api_base.clone();
        let selected = selected.clone();
        let selected_card = selected_card.clone();
        let history = history.clone();
        let platform_app = platform_app.clone();
        ui.on_episode_selected(move |episode_id| {
            let chosen = selected.lock().map(|s| s.clone()).unwrap_or_default();
            if chosen.id.is_empty() {
                set_status(&weak, "Choose an anime first.");
                return;
            }

            set_status(&weak, "Finding a direct stream for the native player…");
            let weak = weak.clone();
            let base = api_base_for_selected(&api_base);
            let platform_app = platform_app.clone();
            let episode_id = episode_id.to_string();
            let anime_title = chosen.title.clone();
            let media_type = chosen.media_type.clone();
            if let Some(card) = selected_card.lock().ok().and_then(|card| card.clone()) {
                if let Ok(mut items) = history.lock() {
                    items.retain(|item| item.id != card.id);
                    items.insert(0, card);
                    items.truncate(50);
                    #[cfg(target_os = "android")]
                    if let Some(app) = platform_app.as_ref() {
                        write_android_preference(app, "history_items", &serialize_cards(&items));
                    }
                    let copy = items.clone();
                    let weak_for_ui = weak.clone();
                    let _ = slint::invoke_from_event_loop(move || {
                        if let Some(window) = weak_for_ui.upgrade() {
                            window.set_history_cards(ModelRc::new(VecModel::from(copy)));
                        }
                    });
                }
            }
            if media_type == "MANGA" {
                load_manga_pages(weak.clone(), base, episode_id, anime_title);
                return;
            }

            std::thread::spawn(move || {
                match find_direct_stream(&base, &episode_id) {
                    Ok((url, referer)) => {
                        #[cfg(target_os = "android")]
                        if let Some(app) = platform_app {
                            launch_native_player(app, url, anime_title, referer);
                            set_status(&weak, "Opened the native video player.");
                            return;
                        }

                        #[cfg(not(target_os = "android"))]
                        let _ = (platform_app, url, anime_title, referer);

                        set_status(
                            &weak,
                            "A direct stream was found. Install the Android APK to play it in the native player.",
                        );
                    }
                    Err(message) => set_status(&weak, &message),
                }
            });
        });
    }

    {
        let weak = weak.clone();
        let selected_card = selected_card.clone();
        let saved = saved.clone();
        let platform_app = platform_app.clone();
        ui.on_save_selected(move || {
            let current = selected_card.lock().ok().and_then(|value| value.clone());
            let Some(card) = current else {
                set_status(&weak, "Open an anime before saving it.");
                return;
            };

            let mut status = "Saved to your library.".to_string();
            if let Ok(mut items) = saved.lock() {
                if let Some(index) = items.iter().position(|item| item.id == card.id) {
                    items.remove(index);
                    status = "Removed from your library.".to_string();
                } else {
                    items.push(card);
                }

                let copy = items.clone();
                #[cfg(target_os = "android")]
                if let Some(app) = platform_app.as_ref() {
                    write_android_preference(app, "saved_items", &serialize_cards(&items));
                }
                if let Some(window) = weak.upgrade() {
                    window.set_saved_cards(ModelRc::new(VecModel::from(copy)));
                }
            }
            set_status(&weak, &status);
        });
    }

    {
        let weak = weak.clone();
        let api_base = api_base.clone();
        let category_state = current_category.clone();
        let query_state = current_query.clone();
        let platform_app = platform_app.clone();
        ui.on_save_api_url(move |input| {
            let base = normalize_base(input.as_str());
            if !base.starts_with("https://") && !base.starts_with("http://") {
                set_status(&weak, "The API URL must start with https:// or http://.");
                return;
            }
            if let Ok(mut state) = api_base.lock() {
                *state = base.clone();
            }
            #[cfg(target_os = "android")]
            if let Some(app) = platform_app.as_ref() {
                write_android_preference(app, "api_base", &base);
            }
            if let Some(window) = weak.upgrade() {
                window.set_api_url(SharedString::from(base));
                window.set_active_page(SharedString::from("home"));
                window.set_active_category(SharedString::from("trending"));
            }
            if let Ok(mut state) = category_state.lock() {
                *state = "trending".to_string();
            }
            if let Ok(mut state) = query_state.lock() {
                state.clear();
            }
            load_catalog(weak.clone(), api_base.clone(), "trending".to_string(), String::new());
        });
    }

    {
        let weak = weak.clone();
        let api_base = api_base.clone();
        let category_state = current_category.clone();
        let query_state = current_query.clone();
        let saved = saved.clone();
        ui.on_navigate(move |destination| {
            let page = destination.to_string();
            if let Some(window) = weak.upgrade() {
                window.set_active_page(SharedString::from(page.clone()));
            }

            match page.as_str() {
                "home" => {
                    let category = category_state.lock().map(|s| s.clone()).unwrap_or_else(|_| "trending".to_string());
                    let query = query_state.lock().map(|s| s.clone()).unwrap_or_default();
                    load_catalog(weak.clone(), api_base.clone(), category, query);
                }
                "search" => {
                    set_status(&weak, "Search the catalog by title or keyword.");
                }
                "saved" => {
                    let copy = saved.lock().map(|items| items.clone()).unwrap_or_default();
                    if let Some(window) = weak.upgrade() {
                        window.set_saved_cards(ModelRc::new(VecModel::from(copy.clone())));
                    }
                    set_status(
                        &weak,
                        if copy.is_empty() {
                            "Your library is empty. Open an anime and tap Save to library."
                        } else {
                            "Your saved anime (kept in memory for this app session)."
                        },
                    );
                }
                "settings" => {
                    set_status(&weak, "Set a compatible metadata API, then save and refresh.");
                }
                "history" => {
                    let copy = history.lock().map(|items| items.clone()).unwrap_or_default();
                    if let Some(window) = weak.upgrade() {
                        window.set_history_cards(ModelRc::new(VecModel::from(copy.clone())));
                    }
                    set_status(&weak, if copy.is_empty() {
                        "No local history yet. Played titles will appear here."
                    } else {
                        "Your recent titles are available to resume."
                    });
                }
                "manga" => {
                    if let Ok(mut state) = category_state.lock() { *state = "manga".to_string(); }
                    let query = query_state.lock().map(|s| s.clone()).unwrap_or_default();
                    load_catalog(weak.clone(), api_base.clone(), "manga".to_string(), query);
                }
                "profile" => {
                    set_status(&weak, "AniList account and list syncing requires OAuth client configuration.");
                }
                "about" => {
                    set_status(&weak, "Zenime native Android · Rust + Slint · minimum Android 8.1 (API 27).");
                }
                _ => {}
            }
        });
    }

    {
        let weak = weak.clone();
        let history = history.clone();
        let platform_app = platform_app.clone();
        ui.on_history_action(move |action| {
            match action.as_str() {
                "clear" => {
                    if let Ok(mut items) = history.lock() { items.clear(); }
                    #[cfg(target_os = "android")]
                    if let Some(app) = platform_app.as_ref() { write_android_preference(app, "history_items", "[]"); }
                    if let Some(window) = weak.upgrade() {
                        window.set_history_cards(ModelRc::new(VecModel::from(Vec::<AnimeCardData>::new())));
                    }
                    set_status(&weak, "Local history cleared.");
                }
                _ => {
                    let items = history.lock().map(|items| items.clone()).unwrap_or_default();
                    if let Some(window) = weak.upgrade() {
                        window.set_history_cards(ModelRc::new(VecModel::from(items.clone())));
                    }
                    set_status(&weak, if items.is_empty() {
                        "No viewing history yet. Start an episode to add it here."
                    } else {
                        "Recent titles are ready to resume."
                    });
                }
            }
        });
    }

    {
        let weak = weak.clone();
        let api_base = api_base.clone();
        let category_state = current_category.clone();
        let query_state = current_query.clone();
        ui.on_manga_action(move |query| {
            let query = query.trim().to_string();
            if let Ok(mut state) = category_state.lock() { *state = "manga".to_string(); }
            if let Ok(mut state) = query_state.lock() { *state = query.clone(); }
            if let Some(window) = weak.upgrade() {
                window.set_active_page(SharedString::from("manga"));
            }
            load_catalog(weak.clone(), api_base.clone(), "manga".to_string(), query);
        });
    }

    {
        let weak = weak.clone();
        let client_id = client_id.clone();
        let platform_app = platform_app.clone();
        ui.on_profile_action(move |action| {
            if action.as_str() == "login" {
                let id = client_id.lock().map(|value| value.clone()).unwrap_or_default();
                if id.trim().is_empty() {
                    set_status(&weak, "Add your AniList OAuth client ID in Settings first.");
                    return;
                }
                let url = format!(
                    "https://anilist.co/api/v2/oauth/authorize?client_id={}&redirect_uri=zenime%3A%2F%2Fcallback&response_type=token",
                    urlencoding::encode(id.trim())
                );
                #[cfg(target_os = "android")]
                if let Some(app) = platform_app.clone() {
                    open_external_url(app, url);
                    set_status(&weak, "Complete AniList authorization in your browser, then return to Zenime.");
                    return;
                }
                #[cfg(not(target_os = "android"))]
                let _ = url;
                set_status(&weak, "AniList authorization is available in the Android app.");
                return;
            }

            #[cfg(target_os = "android")]
            let token = platform_app.as_ref().and_then(|app| read_android_preference(app, "anilist_access_token"));
            #[cfg(not(target_os = "android"))]
            let token: Option<String> = None;

            let Some(token) = token else {
                set_status(&weak, "Connect AniList first to load your profile, lists, and statistics.");
                return;
            };
            set_status(&weak, "Loading your AniList profile and statistics…");
            let weak_for_profile = weak.clone();
            std::thread::spawn(move || {
                let result = fetch_anilist_profile(&token);
                let _ = slint::invoke_from_event_loop(move || {
                    if let Some(window) = weak_for_profile.upgrade() {
                        match result {
                            Ok((name, stats)) => {
                                window.set_anilist_username(SharedString::from(name));
                                window.set_status(SharedString::from(stats));
                            }
                            Err(message) => window.set_status(SharedString::from(message)),
                        }
                    }
                });
            });
        });
    }

    {
        let weak = weak.clone();
        let client_id = client_id.clone();
        let platform_app = platform_app.clone();
        ui.on_save_anilist_client_id(move |value| {
            let value = value.trim().to_string();
            if value.is_empty() {
                set_status(&weak, "Enter a valid AniList OAuth client ID.");
                return;
            }
            if let Ok(mut current) = client_id.lock() { *current = value.clone(); }
            #[cfg(target_os = "android")]
            if let Some(app) = platform_app.as_ref() {
                write_android_preference(app, "anilist_client_id", &value);
            }
            set_status(&weak, "AniList client ID saved. Register zenime://callback as the OAuth redirect URI.");
        });
    }

    {
        let weak = weak.clone();
        let platform_app = platform_app.clone();
        ui.on_open_link(move |url| {
            let url = url.to_string();
            #[cfg(target_os = "android")]
            if let Some(app) = platform_app.clone() {
                open_external_url(app, url);
                return;
            }
            #[cfg(not(target_os = "android"))]
            let _ = url;
            set_status(&weak, "External links can be opened from the Android app.");
        });
    }

    {
        let weak = weak.clone();
        let api_base = api_base.clone();
        let category_state = current_category.clone();
        let query_state = current_query.clone();
        ui.on_refresh_requested(move || {
            let category = category_state.lock().map(|s| s.clone()).unwrap_or_else(|_| "trending".to_string());
            let query = query_state.lock().map(|s| s.clone()).unwrap_or_default();
            load_catalog(weak.clone(), api_base.clone(), category, query);
        });
    }

    load_catalog(
        weak.clone(),
        api_base.clone(),
        "trending".to_string(),
        String::new(),
    );

    ui.run()
}

fn fetch_anilist_profile(token: &str) -> Result<(String, String), String> {
    let client = request_client()?;
    let query = "query { Viewer { name statistics { anime { count episodesWatched minutesWatched } manga { count chaptersRead volumesRead } } } }";
    let response = client.post("https://graphql.anilist.co")
        .bearer_auth(token)
        .json(&serde_json::json!({"query": query}))
        .send()
        .map_err(|error| format!("AniList request failed: {error}"))?
        .error_for_status()
        .map_err(|error| format!("AniList authorization or API error: {error}"))?
        .json::<Value>()
        .map_err(|error| format!("AniList returned invalid profile data: {error}"))?;
    if let Some(errors) = response.get("errors").and_then(Value::as_array) {
        let message = errors.first().and_then(|item| item.get("message")).and_then(Value::as_str).unwrap_or("AniList profile request failed.");
        return Err(message.to_string());
    }
    let viewer = response.get("data").and_then(|data| data.get("Viewer")).ok_or_else(|| "AniList did not return a signed-in user.".to_string())?;
    let name = first_text(viewer, &["name"]);
    let stats = viewer.get("statistics").unwrap_or(&Value::Null);
    let anime = stats.get("anime").unwrap_or(&Value::Null);
    let manga = stats.get("manga").unwrap_or(&Value::Null);
    let summary = format!(
        "Anime: {} titles · {} episodes watched · {} minutes. Manga: {} titles · {} chapters read.",
        first_text(anime, &["count"]),
        first_text(anime, &["episodesWatched"]),
        first_text(anime, &["minutesWatched"]),
        first_text(manga, &["count"]),
        first_text(manga, &["chaptersRead"]),
    );
    Ok((if name.is_empty() { "AniList user".to_string() } else { name }, summary))
}

fn serialize_cards(cards: &[AnimeCardData]) -> String {
    let records = cards.iter().map(|card| serde_json::json!({
        "id": card.id.to_string(),
        "title": card.title.to_string(),
        "details": card.details.to_string(),
        "image_url": card.image_url.to_string(),
    })).collect::<Vec<_>>();
    serde_json::to_string(&records).unwrap_or_else(|_| "[]".to_string())
}

#[cfg(target_os = "android")]
fn restore_cards(
    weak: slint::Weak<MainWindow>,
    app: &PlatformApp,
    key: &'static str,
    target: SharedSaved,
    is_history: bool,
) {
    let Some(raw) = read_android_preference(app, key) else { return; };
    let Ok(value) = serde_json::from_str::<Value>(&raw) else { return; };
    let Some(items) = value.as_array() else { return; };
    let records = items.iter().filter_map(|item| {
        let id = first_text(item, &["id"]);
        let title = first_text(item, &["title"]);
        if id.is_empty() || title.is_empty() { return None; }
        Some(AnimeRecord {
            id,
            title,
            details: first_text(item, &["details"]),
            image_url: first_text(item, &["image_url"]),
        })
    }).collect::<Vec<_>>();
    if records.is_empty() { return; }
    std::thread::spawn(move || {
        let Ok(client) = request_client() else { return; };
        let mut cards = Vec::new();
        for record in records {
            let pixels = load_poster(&client, &record.image_url);
            cards.push(AnimeCardData {
                id: SharedString::from(record.id),
                title: SharedString::from(record.title),
                details: SharedString::from(record.details),
                image_url: SharedString::from(record.image_url),
                poster: Image::from_rgba8(pixels),
            });
        }
        if let Ok(mut current) = target.lock() { *current = cards.clone(); }
        let _ = slint::invoke_from_event_loop(move || {
            if let Some(window) = weak.upgrade() {
                if is_history {
                    window.set_history_cards(ModelRc::new(VecModel::from(cards)));
                } else {
                    window.set_saved_cards(ModelRc::new(VecModel::from(cards)));
                }
            }
        });
    });
}

fn api_base_for_selected(base: &SharedApiBase) -> String {
    base.lock()
        .map(|value| value.clone())
        .unwrap_or_else(|_| DEFAULT_API_BASE.to_string())
}

fn normalize_base(value: &str) -> String {
    let trimmed = value.trim().trim_end_matches('/');
    if trimmed.is_empty() {
        DEFAULT_API_BASE.to_string()
    } else {
        format!("{trimmed}/")
    }
}

fn set_status(weak: &slint::Weak<MainWindow>, message: &str) {
    let weak = weak.clone();
    let message = message.to_string();
    let _ = slint::invoke_from_event_loop(move || {
        if let Some(window) = weak.upgrade() {
            window.set_status(SharedString::from(message));
        }
    });
}

fn api_url(base: &str, path: &str) -> String {
    format!("{}{}", normalize_base(base), path.trim_start_matches('/'))
}

fn request_client() -> Result<Client, String> {
    Client::builder()
        .user_agent("Zenime-Android/0.1.0 (native client)")
        .timeout(Duration::from_secs(25))
        .connect_timeout(Duration::from_secs(12))
        .build()
        .map_err(|error| format!("Could not start the network client: {error}"))
}

fn fetch_json(client: &Client, url: &str) -> Result<Value, String> {
    let mut request = client.get(url);
    if let Some(key) = option_env!("ZENIME_API_KEY").filter(|key| !key.trim().is_empty()) {
        request = request.header("X-API-Key", key);
    }
    let response = request.send().map_err(|error| {
        format!("Network request failed. Check the API endpoint and connection. ({error})")
    })?;
    let response = response.error_for_status().map_err(|error| {
        format!("The API returned an error. Check the endpoint or API access. ({error})")
    })?;
    response
        .json::<Value>()
        .map_err(|error| format!("The API response was not valid JSON: {error}"))
}

fn result_array(payload: &Value) -> Option<&Vec<Value>> {
    if let Some(items) = payload.as_array() {
        return Some(items);
    }
    if let Some(items) = payload.get("results").and_then(Value::as_array) {
        return Some(items);
    }
    if let Some(items) = payload.get("episodes").and_then(Value::as_array) {
        return Some(items);
    }
    let data = payload.get("data")?;
    data.as_array()
        .or_else(|| data.get("results").and_then(Value::as_array))
        .or_else(|| data.get("episodes").and_then(Value::as_array))
        .or_else(|| data.get("chapters").and_then(Value::as_array))
        .or_else(|| data.get("pages").and_then(Value::as_array))
        .or_else(|| data.get("Page").and_then(|page| page.get("media")).and_then(Value::as_array))
        .or_else(|| data.get("media").and_then(Value::as_array))
        .or_else(|| payload.get("chapters").and_then(Value::as_array))
        .or_else(|| payload.get("pages").and_then(Value::as_array))
}

fn value_text(value: &Value) -> String {
    if let Some(text) = value.as_str() {
        return text.trim().to_string();
    }
    if let Some(number) = value.as_i64() {
        return number.to_string();
    }
    if let Some(number) = value.as_f64() {
        return format!("{number:.1}");
    }
    String::new()
}

fn nested_title(value: &Value) -> String {
    if let Some(title) = value.as_str() {
        return title.trim().to_string();
    }
    for field in ["userPreferred", "english", "romaji", "native"] {
        let title = value.get(field).and_then(Value::as_str).unwrap_or("").trim();
        if !title.is_empty() {
            return title.to_string();
        }
    }
    String::new()
}

fn first_text(value: &Value, fields: &[&str]) -> String {
    for field in fields {
        let found = value_text(value.get(*field).unwrap_or(&Value::Null));
        if !found.is_empty() {
            return found;
        }
    }
    String::new()
}

fn image_url(value: &Value) -> String {
    for field in ["image", "poster", "cover", "coverImage", "banner", "thumbnail"] {
        let candidate = value.get(field).unwrap_or(&Value::Null);
        let url = if candidate.is_object() {
            first_text(candidate, &["extraLarge", "large", "medium", "url"])
        } else {
            value_text(candidate)
        };
        if url.starts_with("http://") || url.starts_with("https://") {
            return url;
        }
    }
    String::new()
}

fn fetch_anilist_catalog(query_text: &str, category: &str) -> Result<Value, String> {
    let client = request_client()?;
    let media_type = if category == "manga" { "MANGA" } else { "ANIME" };
    let sort = match category {
        "popular" => "POPULARITY_DESC",
        "top-rated" => "SCORE_DESC",
        "latest" => "TRENDING_DESC",
        _ => "TRENDING_DESC",
    };
    let query = "query ($page: Int, $perPage: Int, $search: String, $type: MediaType, $sort: [MediaSort]) { Page(page: $page, perPage: $perPage) { pageInfo { hasNextPage } media(type: $type, search: $search, sort: $sort, isAdult: false) { id title { userPreferred english romaji native } coverImage { extraLarge large medium } description format status episodes chapters averageScore popularity genres seasonYear isAdult } } }";
    let response = client.post("https://graphql.anilist.co")
        .json(&serde_json::json!({
            "query": query,
            "variables": {
                "page": 1,
                "perPage": 24,
                "search": if query_text.trim().is_empty() { Value::Null } else { Value::String(query_text.to_string()) },
                "type": media_type,
                "sort": [sort]
            }
        }))
        .send()
        .map_err(|error| format!("AniList catalog request failed: {error}"))?
        .error_for_status()
        .map_err(|error| format!("AniList catalog returned an error: {error}"))?
        .json::<Value>()
        .map_err(|error| format!("AniList returned invalid catalog data: {error}"))?;
    if let Some(errors) = response.get("errors").and_then(Value::as_array) {
        let message = errors.first().and_then(|item| item.get("message")).and_then(Value::as_str).unwrap_or("AniList catalog query failed.");
        return Err(message.to_string());
    }
    Ok(response)
}

fn parse_anime_records(payload: &Value, latest_mode: bool) -> Vec<AnimeRecord> {
    let Some(items) = result_array(payload) else {
        return Vec::new();
    };

    let mut out = Vec::new();
    let mut seen = HashSet::new();
    for item in items {
        let anime = item
            .get("anime")
            .or_else(|| item.get("media"))
            .unwrap_or(item);

        if anime.get("isAdult").and_then(Value::as_bool) == Some(true)
            || item.get("isAdult").and_then(Value::as_bool) == Some(true)
        {
            continue;
        }

        let id = first_text(anime, &["id", "anilistId", "malId"]);
        let title_value = anime.get("title").unwrap_or(&Value::Null);
        let mut title = nested_title(title_value);
        if title.is_empty() {
            title = first_text(anime, &["name", "englishTitle", "romaji"]);
        }
        if id.is_empty() || title.is_empty() || !seen.insert(id.clone()) {
            continue;
        }

        let year = first_text(anime, &["releaseDate", "year"]);
        let rating = first_text(anime, &["rating", "score"]);
        let kind = first_text(anime, &["type", "format"]);
        let status = first_text(anime, &["status"]);
        let mut details = Vec::new();
        if !kind.is_empty() {
            details.push(kind);
        }
        if !year.is_empty() && year != "0" {
            details.push(year);
        }
        if !rating.is_empty() && rating != "0" {
            details.push(format!("★ {rating}"));
        } else if !status.is_empty() {
            details.push(status);
        }
        if latest_mode {
            let ep_no = first_text(item, &["number", "episodeNumber", "episode"]);
            if !ep_no.is_empty() {
                details.insert(0, format!("EP {ep_no}"));
            }
        }

        out.push(AnimeRecord {
            id,
            title,
            details: if details.is_empty() {
                "Anime".to_string()
            } else {
                details.join("  ·  ")
            },
            image_url: image_url(anime),
        });
    }
    out
}

type PosterBuffer = slint::SharedPixelBuffer<slint::Rgba8Pixel>;

fn decode_poster(bytes: &[u8]) -> Option<PosterBuffer> {
    let decoded = image::load_from_memory(bytes).ok()?.thumbnail(260, 360).to_rgba8();
    Some(PosterBuffer::clone_from_slice(
        decoded.as_raw(),
        decoded.width(),
        decoded.height(),
    ))
}

fn blank_poster() -> PosterBuffer {
    PosterBuffer::new(1, 1)
}

fn load_poster(client: &Client, url: &str) -> PosterBuffer {
    if url.is_empty() {
        return blank_poster();
    }

    let response = client.get(url).timeout(Duration::from_secs(8)).send();
    match response {
        Ok(response) => match response.error_for_status() {
            Ok(response) => response
                .bytes()
                .ok()
                .and_then(|bytes| decode_poster(&bytes))
                .unwrap_or_else(blank_poster),
            Err(_) => blank_poster(),
        },
        Err(_) => blank_poster(),
    }
}

fn load_catalog(
    weak: slint::Weak<MainWindow>,
    api_base: SharedApiBase,
    category: String,
    query: String,
) {
    load_catalog_with_filters(weak, api_base, category, query, SearchFilters::default());
}

fn load_catalog_with_filters(
    weak: slint::Weak<MainWindow>,
    api_base: SharedApiBase,
    category: String,
    query: String,
    filters: SearchFilters,
) {
    set_status(&weak, "Loading catalog…");
    let base = api_base_for_selected(&api_base);

    std::thread::spawn(move || {
        let result = (|| -> Result<Vec<AnimeCardData>, String> {
            let client = request_client()?;
            let route = match category.as_str() {
                "popular" => "meta/anilist/advanced-search?type=ANIME&sort=%5B%22POPULARITY_DESC%22%5D&page=1&perPage=24".to_string(),
                "top-rated" => "meta/anilist/advanced-search?type=ANIME&sort=%5B%22SCORE_DESC%22%5D&page=1&perPage=24".to_string(),
                "top-airing" => "meta/anilist/advanced-search?type=ANIME&status=RELEASING&sort=%5B%22POPULARITY_DESC%22%5D&page=1&perPage=24".to_string(),
                "latest" => "meta/anilist/recent-episodes?page=1&perPage=24&provider=anikoto".to_string(),
                "search" => format!(
                    "meta/anilist/advanced-search?query={}&type=ANIME&page=1&perPage=24",
                    urlencoding::encode(&query)
                ),
                "manga" => format!(
                    "meta/anilist/advanced-search?query={}&type=MANGA&page=1&perPage=24",
                    urlencoding::encode(&query)
                ),
                _ => "meta/anilist/trending?page=1&perPage=24".to_string(),
            };
            let url = api_url(&base, &route);
            let payload = match fetch_json(&client, &url) {
                Ok(payload) => payload,
                Err(_) => fetch_anilist_catalog(&query, &category)?,
            };
            let records = parse_anime_records(&payload, category == "latest");
            if records.is_empty() {
                return Err("The API returned no anime. Try another category or check the API URL in Settings.".to_string());
            }

            let records: Vec<AnimeRecord> = records.into_iter().take(24).collect();
            let mut loaded = Vec::with_capacity(records.len());
            for group in records.chunks(4) {
                let mut workers = Vec::new();
                for record in group.iter().cloned() {
                    let client = client.clone();
                    workers.push(std::thread::spawn(move || {
                        let poster = load_poster(&client, &record.image_url);
                        (record, poster)
                    }));
                }
                for worker in workers {
                    if let Ok(item) = worker.join() {
                        loaded.push(item);
                    }
                }
            }
            if loaded.is_empty() {
                return Err("Anime data loaded, but no valid results could be displayed.".to_string());
            }
            Ok(loaded)
        })();

        match result {
            Ok(loaded) => {
                let count = loaded.len();
                let status = format!("Showing {count} titles · {}", category_label(&category));
                let weak_for_ui = weak.clone();
                let _ = slint::invoke_from_event_loop(move || {
                    let cards: Vec<AnimeCardData> = loaded
                        .into_iter()
                        .map(|(record, pixels)| AnimeCardData {
                            id: SharedString::from(record.id),
                            title: SharedString::from(record.title),
                            details: SharedString::from(record.details),
                            image_url: SharedString::from(record.image_url),
                            poster: Image::from_rgba8(pixels),
                        })
                        .collect();
                    if let Some(window) = weak_for_ui.upgrade() {
                        window.set_anime_cards(ModelRc::new(VecModel::from(cards)));
                        window.set_status(SharedString::from(status));
                    }
                });
            }
            Err(message) => set_status(&weak, &message),
        }
    });
}

fn category_label(category: &str) -> &'static str {
    match category {
        "popular" => "Popular",
        "top-rated" => "Top rated",
        "top-airing" => "Top airing",
        "latest" => "Latest episodes",
        "search" => "Search results",
        "manga" => "Manga search",
        _ => "Trending",
    }
}

fn open_anime_detail(
    weak: slint::Weak<MainWindow>,
    api_base: SharedApiBase,
    selected: SharedSelected,
    selected_card: SharedCard,
    id: String,
    title: String,
    media_type: String,
    image_url: String,
    poster: Image,
) {
    let card = AnimeCardData {
        id: SharedString::from(id.clone()),
        title: SharedString::from(title.clone()),
        details: SharedString::from(if media_type == "MANGA" { "Manga" } else { "Anime" }),
        image_url: SharedString::from(image_url.clone()),
        poster: poster.clone(),
    };

    if let Ok(mut current) = selected.lock() {
        current.id = id.clone();
        current.title = title.clone();
        current.media_type = media_type.clone();
    }
    if let Ok(mut current) = selected_card.lock() {
        *current = Some(card);
    }

    if let Some(window) = weak.upgrade() {
        window.set_selected_id(SharedString::from(id.clone()));
        window.set_selected_title(SharedString::from(title.clone()));
        window.set_selected_poster(poster);
        window.set_selected_description(SharedString::from("Loading synopsis…"));
        window.set_selected_meta(SharedString::from("Loading details…"));
        window.set_episodes(ModelRc::new(VecModel::from(Vec::<EpisodeRowData>::new())));
        window.set_active_page(SharedString::from("detail"));
    }
    set_status(&weak, if media_type == "MANGA" {
        "Loading manga information and chapters…"
    } else {
        "Loading anime details and episodes…"
    });

    let base = api_base_for_selected(&api_base);
    std::thread::spawn(move || {
        let detail = (|| -> Result<(String, String, Vec<EpisodeRowData>), String> {
            let client = request_client()?;
            let detail_path = if media_type == "MANGA" {
                format!("meta/anilist-manga/info/{}?provider=mangahere", urlencoding::encode(&id))
            } else {
                format!("meta/anilist/data/{}?provider=anikoto", urlencoding::encode(&id))
            };
            let detail_url = api_url(&base, &detail_path);
            let payload = fetch_json(&client, &detail_url)?;
            let data = payload.get("data").unwrap_or(&payload);
            let description = strip_markup(
                data.get("description")
                    .and_then(Value::as_str)
                    .unwrap_or("Synopsis not available."),
            );
            let status = first_text(data, &["status", "format", "type", "chapters", "episodes"]);
            let episodes = if media_type == "MANGA" {
                parse_episode_rows(&payload)
            } else {
                let episodes_url = api_url(
                    &base,
                    &format!(
                        "meta/anilist/episodes/{}?provider=anikoto&dub=false",
                        urlencoding::encode(&id)
                    ),
                );
                let episodes_payload = fetch_json(&client, &episodes_url).unwrap_or(Value::Null);
                parse_episode_rows(&episodes_payload)
            };
            Ok((description, status, episodes))
        })();

        let _ = slint::invoke_from_event_loop(move || {
            if let Some(window) = weak.upgrade() {
                match detail {
                    Ok((description, meta, episodes)) => {
                        let count = episodes.len();
                        window.set_selected_description(SharedString::from(description));
                        window.set_selected_meta(SharedString::from(if meta.is_empty() {
                            if media_type == "MANGA" { "Manga".to_string() } else { "Anime".to_string() }
                        } else {
                            meta
                        }));
                        window.set_episodes(ModelRc::new(VecModel::from(episodes)));
                        window.set_status(SharedString::from(if count == 0 {
                            if media_type == "MANGA" {
                                "No chapters were returned by this manga provider.".to_string()
                            } else {
                                "No episodes came back from this provider. Try another title or API endpoint.".to_string()
                            }
                        } else if media_type == "MANGA" {
                            format!("{count} chapters available · tap one to read")
                        } else {
                            format!("{count} episodes available · tap one to play")
                        }));
                    }
                    Err(message) => {
                        window.set_selected_description(SharedString::from("Details are temporarily unavailable."));
                        window.set_status(SharedString::from(message));
                    }
                }
            }
        });
    });
}

fn strip_markup(value: &str) -> String {
    let mut output = String::with_capacity(value.len());
    let mut inside_tag = false;
    for ch in value.chars() {
        match ch {
            '<' => inside_tag = true,
            '>' => inside_tag = false,
            _ if !inside_tag => output.push(ch),
            _ => {}
        }
    }
    output
        .replace("&amp;", "&")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .trim()
        .to_string()
}

fn parse_episode_rows(payload: &Value) -> Vec<EpisodeRowData> {
    let Some(items) = result_array(payload) else {
        return Vec::new();
    };
    items
        .iter()
        .enumerate()
        .filter_map(|(index, item)| {
            let id = first_text(item, &["id", "episodeId"]);
            if id.is_empty() {
                return None;
            }
            let number = first_text(item, &["number", "episodeNumber", "episode"]);
            let title = first_text(item, &["title", "name"]);
            let label = if title.is_empty() {
                format!("Episode {}", if number.is_empty() { (index + 1).to_string() } else { number.clone() })
            } else if number.is_empty() {
                title
            } else {
                format!("Episode {number} · {title}")
            };
            Some(EpisodeRowData {
                id: SharedString::from(id),
                title: SharedString::from(label),
                details: SharedString::from("Select to open"),
            })
        })
        .collect()
}

fn find_direct_stream(base: &str, episode_id: &str) -> Result<(String, String), String> {
    let client = request_client()?;
    for provider in ["anikoto", "animepahe", "reanime", "kickassanime"] {
        let route = format!(
            "meta/anilist/watch?episodeId={}&provider={}",
            urlencoding::encode(episode_id),
            urlencoding::encode(provider)
        );
        let url = api_url(base, &route);
        let Ok(payload) = fetch_json(&client, &url) else {
            continue;
        };
        if let Some((stream, source_referer)) = direct_source(&payload) {
            let referer = if source_referer.is_empty() {
                provider_referer(provider).to_string()
            } else {
                source_referer
            };
            return Ok((stream, referer));
        }
    }

    Err("No direct HLS/MP4 source was returned. This app avoids WebViews, so embed-only provider pages are not playable in the native player.".to_string())
}

fn direct_source(payload: &Value) -> Option<(String, String)> {
    let data = payload.get("data").unwrap_or(payload);
    let sources = data.get("sources").and_then(Value::as_array)?;
    for source in sources {
        let url = first_text(source, &["url", "file", "src"]);
        if !is_direct_media(&url) {
            continue;
        }
        let referer = source
            .get("headers")
            .and_then(|headers| headers.get("Referer").or_else(|| headers.get("referer")))
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string();
        return Some((url, referer));
    }
    None
}

fn is_direct_media(url: &str) -> bool {
    if !(url.starts_with("https://") || url.starts_with("http://")) {
        return false;
    }
    let path = url.split('?').next().unwrap_or(url).to_ascii_lowercase();
    [".m3u8", ".mp4", ".mpd", ".m4v", ".webm"]
        .iter()
        .any(|suffix| path.ends_with(suffix))
}

fn provider_referer(provider: &str) -> &'static str {
    match provider {
        "animepahe" => "https://animepahe.com/",
        "reanime" => "https://reanime.to/",
        "kickassanime" => "https://krussdomi.com/",
        _ => "https://anikoto.to/",
    }
}

#[cfg(target_os = "android")]
fn read_android_preference(app: &PlatformApp, key: &str) -> Option<String> {
    let vm_ptr = app.vm_as_ptr() as *mut jni::sys::JavaVM;
    let vm = unsafe { jni::JavaVM::from_raw(vm_ptr) }.ok()?;
    vm.attach_current_thread(|mut env| -> jni::errors::Result<Option<String>> {
        let activity_raw = app.activity_as_ptr() as jni::sys::jobject;
        let activity = unsafe { jni::objects::JObject::from_raw(activity_raw) };
        let name = env.new_string("zenime_native")?;
        let name_obj: jni::objects::JObject = name.into();
        let prefs = env.call_method(
            &activity, "getSharedPreferences",
            "(Ljava/lang/String;I)Landroid/content/SharedPreferences;",
            &[jni::objects::JValue::Object(&name_obj), jni::objects::JValue::Int(0)],
        )?.l()?;
        let key_string = env.new_string(key)?;
        let key_obj: jni::objects::JObject = key_string.into();
        let default = env.new_string("")?;
        let default_obj: jni::objects::JObject = default.into();
        let value = env.call_method(
            &prefs, "getString",
            "(Ljava/lang/String;Ljava/lang/String;)Ljava/lang/String;",
            &[jni::objects::JValue::Object(&key_obj), jni::objects::JValue::Object(&default_obj)],
        )?.l()?;
        if value.is_null() { return Ok(None); }
        let value: jni::objects::JString = value.into();
        let text: String = env.get_string(&value)?.into();
        Ok(if text.trim().is_empty() { None } else { Some(text) })
    }).ok().flatten()
}

#[cfg(target_os = "android")]
fn write_android_preference(app: &PlatformApp, key: &str, value: &str) {
    let vm_ptr = app.vm_as_ptr() as *mut jni::sys::JavaVM;
    let Ok(vm) = (unsafe { jni::JavaVM::from_raw(vm_ptr) }) else { return; };
    let _ = vm.attach_current_thread(|mut env| -> jni::errors::Result<()> {
        let activity_raw = app.activity_as_ptr() as jni::sys::jobject;
        let activity = unsafe { jni::objects::JObject::from_raw(activity_raw) };
        let name = env.new_string("zenime_native")?;
        let name_obj: jni::objects::JObject = name.into();
        let prefs = env.call_method(
            &activity, "getSharedPreferences",
            "(Ljava/lang/String;I)Landroid/content/SharedPreferences;",
            &[jni::objects::JValue::Object(&name_obj), jni::objects::JValue::Int(0)],
        )?.l()?;
        let editor = env.call_method(&prefs, "edit", "()Landroid/content/SharedPreferences$Editor;", &[])?.l()?;
        let jkey = env.new_string(key)?;
        let jvalue = env.new_string(value)?;
        let key_obj: jni::objects::JObject = jkey.into();
        let value_obj: jni::objects::JObject = jvalue.into();
        env.call_method(
            &editor, "putString",
            "(Ljava/lang/String;Ljava/lang/String;)Landroid/content/SharedPreferences$Editor;",
            &[jni::objects::JValue::Object(&key_obj), jni::objects::JValue::Object(&value_obj)],
        )?;
        env.call_method(&editor, "apply", "()V", &[])?;
        Ok(())
    });
}

#[cfg(target_os = "android")]
fn open_external_url(app: PlatformApp, url: String) {
    app.run_on_java_main_thread(Box::new(move || {
        let vm_ptr = app.vm_as_ptr() as *mut jni::sys::JavaVM;
        let Ok(vm) = (unsafe { jni::JavaVM::from_raw(vm_ptr) }) else { return; };
        let _ = vm.attach_current_thread(|mut env| -> jni::errors::Result<()> {
            let activity_raw = app.activity_as_ptr() as jni::sys::jobject;
            let activity = unsafe { jni::objects::JObject::from_raw(activity_raw) };
            let intent = env.new_object(
                "android/content/Intent", "()V", &[]
            )?;
            let action = env.new_string("android.intent.action.VIEW")?;
            let action_obj: jni::objects::JObject = action.into();
            env.call_method(
                &intent, "setAction",
                "(Ljava/lang/String;)Landroid/content/Intent;",
                &[jni::objects::JValue::Object(&action_obj)],
            )?;
            let uri_class = env.find_class("android/net/Uri")?;
            let url_string = env.new_string(&url)?;
            let url_obj: jni::objects::JObject = url_string.into();
            let uri = env.call_static_method(
                uri_class, "parse", "(Ljava/lang/String;)Landroid/net/Uri;",
                &[jni::objects::JValue::Object(&url_obj)],
            )?.l()?;
            env.call_method(
                &intent, "setData", "(Landroid/net/Uri;)Landroid/content/Intent;",
                &[jni::objects::JValue::Object(&uri)],
            )?;
            env.call_method(
                &activity, "startActivity", "(Landroid/content/Intent;)V",
                &[jni::objects::JValue::Object(&intent)],
            )?;
            Ok(())
        });
    }));
}

#[cfg(target_os = "android")]
fn launch_native_player(app: PlatformApp, url: String, title: String, referer: String) {
    app.run_on_java_main_thread(Box::new(move || {
        let vm_ptr = app.vm_as_ptr() as *mut jni::sys::JavaVM;
        let vm = unsafe { jni::JavaVM::from_raw(vm_ptr) };
        let vm = match vm {
            Ok(vm) => vm,
            Err(error) => {
                eprintln!("Could not access Android Java VM: {error}");
                return;
            }
        };

        let result = vm.attach_current_thread(|mut env| -> jni::errors::Result<()> {
            let activity_raw = app.activity_as_ptr() as jni::sys::jobject;
            // The activity reference is a borrowed JNI global reference owned by android-activity.
            let activity = unsafe { jni::objects::JObject::from_raw(activity_raw) };
            let intent = env.new_object("android/content/Intent", "()V", &[])?;
            let class_name = env.new_string("com.brentph505.zenime.PlayerActivity")?;
            let class_name_obj: jni::objects::JObject = class_name.into();

            env.call_method(
                &intent,
                "setClassName",
                "(Landroid/content/Context;Ljava/lang/String;)Landroid/content/Intent;",
                &[
                    jni::objects::JValue::Object(&activity),
                    jni::objects::JValue::Object(&class_name_obj),
                ],
            )?;

            for (key, value) in [
                ("media_url", url.as_str()),
                ("media_title", title.as_str()),
                ("media_referer", referer.as_str()),
            ] {
                let key_string = env.new_string(key)?;
                let value_string = env.new_string(value)?;
                let key_object: jni::objects::JObject = key_string.into();
                let value_object: jni::objects::JObject = value_string.into();
                env.call_method(
                    &intent,
                    "putExtra",
                    "(Ljava/lang/String;Ljava/lang/String;)Landroid/content/Intent;",
                    &[
                        jni::objects::JValue::Object(&key_object),
                        jni::objects::JValue::Object(&value_object),
                    ],
                )?;
            }

            env.call_method(
                &activity,
                "startActivity",
                "(Landroid/content/Intent;)V",
                &[jni::objects::JValue::Object(&intent)],
            )?;
            Ok(())
        });

        if let Err(error) = result {
            eprintln!("Could not open native video player: {error}");
        }
    }));
}
