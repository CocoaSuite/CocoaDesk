//! The CocoaDesk desktop shell: window, tray, background timer and the commands the UI calls.

use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, MutexGuard};
use std::time::Duration;

use chrono::{Local, NaiveDate, NaiveDateTime, Timelike, Utc};
use serde::Serialize;
use tauri::menu::{Menu, MenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Emitter, Manager, State, WindowEvent};
use tauri_plugin_dialog::DialogExt;
use tauri_plugin_notification::NotificationExt;
use tauri_plugin_opener::OpenerExt;

use cocoadesk_core::boards::{self, Board, BoardSummary, ImageStore};
use cocoadesk_core::import::WorkbookPreview;
use cocoadesk_core::model::{ClassInput, MarkStatus, Period};
use cocoadesk_core::news::store::{self as news, CheckOutcome, Filter, NewsView, Reading};
use cocoadesk_core::news::{feed, HttpFetcher, TopicInput};
use cocoadesk_core::service::{self, ImportDone, ImportRequest, Settings, WeekView};
use cocoadesk_core::store::{SemesterInput, Store};

/// Largest timetable file we read, to keep a wrong pick from eating memory.
const MAX_FILE_BYTES: u64 = 25 * 1024 * 1024;

struct Picked {
    bytes: Vec<u8>,
}

struct AppState {
    store: Mutex<Store>,
    files: Mutex<HashMap<String, Picked>>,
    next_file: AtomicU64,
    fetcher: HttpFetcher,
    /// Topics being checked right now, so a timer run and "Check now" don't overlap.
    checking: Mutex<HashSet<i64>>,
    /// Moodboard pictures.
    images: ImageStore,
    /// Where the window was before the moodboard was pinned on top, to put it back.
    overlay: Mutex<Option<WindowPlace>>,
}

#[derive(Clone, Copy)]
struct WindowPlace {
    maximized: bool,
    size: tauri::PhysicalSize<u32>,
    position: tauri::PhysicalPosition<i32>,
}

impl AppState {
    fn store(&self) -> Result<MutexGuard<'_, Store>, String> {
        self.store.lock().map_err(|_| "The data store is unavailable. Restart CocoaDesk.".to_string())
    }
    fn file<T>(&self, token: &str, f: impl FnOnce(&Picked) -> T) -> Result<T, String> {
        let files = self.files.lock().map_err(|_| "Internal error.".to_string())?;
        files.get(token).map(f).ok_or_else(|| "That file is no longer open. Choose it again.".to_string())
    }
}

/// The start of today on this computer's clock, as UTC.
fn day_start() -> chrono::DateTime<Utc> {
    let midnight = Local::now().date_naive().and_hms_opt(0, 0, 0).expect("valid time");
    midnight.and_local_timezone(Local).earliest().map(|d| d.with_timezone(&Utc)).unwrap_or_else(|| Utc::now() - chrono::Duration::hours(24))
}

fn now() -> NaiveDateTime {
    let n = Local::now().naive_local();
    n.with_nanosecond(0).unwrap_or(n)
}

fn err(e: cocoadesk_core::Error) -> String {
    e.to_string()
}

fn parse_date(s: &str) -> Result<NaiveDate, String> {
    NaiveDate::parse_from_str(s, "%Y-%m-%d").map_err(|_| format!("Invalid date «{s}»."))
}

// ---------- commands ----------

#[tauri::command]
fn week_view(state: State<AppState>, semester_id: Option<i64>, date: Option<String>) -> Result<WeekView, String> {
    let now = now();
    let date = match date {
        Some(d) => parse_date(&d)?,
        None => now.date(),
    };
    service::week_view(&*state.store()?, semester_id, date, now).map_err(err)
}

#[tauri::command]
fn save_semester(state: State<AppState>, id: Option<i64>, input: SemesterInput) -> Result<i64, String> {
    let store = state.store()?;
    match id {
        Some(id) => store.update_semester(id, &input).map(|_| id),
        None => store.create_semester(&input),
    }
    .map_err(err)
}

#[tauri::command]
fn delete_semester(state: State<AppState>, id: i64) -> Result<(), String> {
    state.store()?.delete_semester(id).map_err(err)
}

#[tauri::command]
fn set_periods(state: State<AppState>, semester_id: i64, periods: Vec<Period>) -> Result<(), String> {
    state.store()?.set_periods(semester_id, &periods).map_err(err)
}

#[tauri::command]
fn add_break(state: State<AppState>, semester_id: i64, from: String, to: String, label: String) -> Result<i64, String> {
    state.store()?.add_break(semester_id, parse_date(&from)?, parse_date(&to)?, &label).map_err(err)
}

#[tauri::command]
fn delete_break(state: State<AppState>, id: i64) -> Result<(), String> {
    state.store()?.delete_break(id).map_err(err)
}

#[tauri::command]
fn save_class(state: State<AppState>, semester_id: i64, id: Option<i64>, input: ClassInput) -> Result<i64, String> {
    let store = state.store()?;
    match id {
        Some(id) => store.update_class(id, &input).map(|_| id),
        None => store.add_class(semester_id, &input, now()),
    }
    .map_err(err)
}

#[tauri::command]
fn delete_class(state: State<AppState>, id: i64) -> Result<(), String> {
    state.store()?.delete_class(id).map_err(err)
}

#[tauri::command]
fn set_status(state: State<AppState>, class_id: i64, date: String, status: Option<MarkStatus>) -> Result<(), String> {
    service::set_status(&*state.store()?, class_id, parse_date(&date)?, status, now()).map_err(err)
}

#[tauri::command]
fn save_settings(state: State<AppState>, settings: Settings) -> Result<(), String> {
    settings.save(&*state.store()?).map_err(err)
}

#[derive(Serialize)]
struct PickedFile {
    token: String,
    name: String,
}

/// Opens the system file picker. The file is read here, so the UI never handles paths.
#[tauri::command]
async fn pick_timetable(app: AppHandle, state: State<'_, AppState>) -> Result<Option<PickedFile>, String> {
    let picked = app.dialog().file().set_title("Choose your timetable").add_filter("Excel workbook", &["xlsx"]).blocking_pick_file();
    let Some(picked) = picked else { return Ok(None) };
    let path = picked.into_path().map_err(|e| e.to_string())?;
    let meta = std::fs::metadata(&path).map_err(|e| format!("Can't open the file: {e}"))?;
    if meta.len() > MAX_FILE_BYTES {
        return Err("That file is too large to be a timetable.".into());
    }
    let bytes = std::fs::read(&path).map_err(|e| format!("Can't read the file: {e}"))?;
    let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| "timetable.xlsx".into());
    let token = state.next_file.fetch_add(1, Ordering::Relaxed).to_string();
    let mut files = state.files.lock().map_err(|_| "Internal error.".to_string())?;
    // Only the latest pick is kept.
    files.clear();
    files.insert(token.clone(), Picked { bytes });
    Ok(Some(PickedFile { token, name }))
}

#[tauri::command]
fn import_preview(state: State<AppState>, token: String, sheet: Option<String>, group: Option<String>) -> Result<WorkbookPreview, String> {
    let store = state.store()?;
    state.file(&token, |f| service::preview_import(&store, &f.bytes, sheet.as_deref(), group.as_deref()))?.map_err(err)
}

#[tauri::command]
fn import_exclusions(state: State<AppState>, semester_id: i64) -> Result<Vec<String>, String> {
    state.store()?.import_exclusions(semester_id).map_err(err)
}

#[tauri::command]
fn import_commit(state: State<AppState>, token: String, request: ImportRequest) -> Result<ImportDone, String> {
    let mut store = state.store()?;
    state.file(&token, |f| service::commit_import(&mut store, &f.bytes, &request, now()))?.map_err(err)
}

// ---------- moodboards ----------

#[tauri::command]
fn boards_list(state: State<AppState>) -> Result<Vec<BoardSummary>, String> {
    state.store()?.boards().map_err(err)
}

#[tauri::command]
fn board_load(state: State<AppState>, id: i64) -> Result<Board, String> {
    state.store()?.board(id).map_err(err)
}

#[tauri::command]
fn board_create(state: State<AppState>, name: String) -> Result<i64, String> {
    state.store()?.create_board(&name, now()).map_err(err)
}

#[tauri::command]
fn board_rename(state: State<AppState>, id: i64, name: String) -> Result<(), String> {
    state.store()?.rename_board(id, &name).map_err(err)
}

#[tauri::command]
fn board_duplicate(state: State<AppState>, id: i64) -> Result<i64, String> {
    state.store()?.duplicate_board(id, now()).map_err(err)
}

#[tauri::command]
fn board_delete(state: State<AppState>, id: i64) -> Result<(), String> {
    state.store()?.delete_board(id).map_err(err)
}

#[tauri::command]
fn board_save(state: State<AppState>, id: i64, scene: String) -> Result<(), String> {
    state.store()?.save_board(id, &scene, now()).map_err(err)
}

/// Stores a pasted, dropped or chosen picture. The bytes arrive as the raw request body.
#[tauri::command]
async fn board_add_image(state: State<'_, AppState>, request: tauri::ipc::Request<'_>) -> Result<String, String> {
    match request.body() {
        tauri::ipc::InvokeBody::Raw(bytes) => state.images.put(bytes).map_err(err),
        _ => Err("No image was sent.".into()),
    }
}

/// Downloads a picture dragged or pasted from a web page.
#[tauri::command]
async fn board_fetch_image(app: AppHandle, url: String) -> Result<String, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<AppState>();
        boards::download_image(&state.fetcher, &state.images, &url).map_err(err)
    })
    .await
    .map_err(|e| e.to_string())?
}

/// Pins the window on top of other apps as a small reference panel, or puts it back.
#[tauri::command]
fn board_overlay(window: tauri::WebviewWindow, state: State<AppState>, on: bool) -> Result<(), String> {
    let mut saved = state.overlay.lock().map_err(|_| "Internal error.".to_string())?;
    let e = |e: tauri::Error| e.to_string();
    if on {
        if saved.is_none() {
            *saved = Some(WindowPlace {
                maximized: window.is_maximized().map_err(e)?,
                size: window.outer_size().map_err(e)?,
                position: window.outer_position().map_err(e)?,
            });
        }
        window.unmaximize().map_err(e)?;
        window.set_min_size(Some(tauri::LogicalSize::new(240.0, 180.0))).map_err(e)?;
        let scale = window.scale_factor().map_err(e)?;
        let size = tauri::LogicalSize::new(560.0, 440.0).to_physical::<u32>(scale);
        window.set_size(size).map_err(e)?;
        // Top-right corner of the screen the window is on, clear of the edge.
        if let Ok(Some(m)) = window.current_monitor() {
            let margin = (24.0 * scale) as i32;
            let x = m.position().x + m.size().width as i32 - size.width as i32 - margin;
            let y = m.position().y + margin * 2;
            window.set_position(tauri::PhysicalPosition::new(x, y)).map_err(e)?;
        }
        window.set_always_on_top(true).map_err(e)?;
    } else {
        window.set_always_on_top(false).map_err(e)?;
        window.set_min_size(Some(tauri::LogicalSize::new(900.0, 560.0))).map_err(e)?;
        if let Some(place) = saved.take() {
            window.set_size(place.size).map_err(e)?;
            window.set_position(place.position).map_err(e)?;
            if place.maximized {
                window.maximize().map_err(e)?;
            }
        }
    }
    Ok(())
}

/// Serves moodboard pictures to the window at `boardimg://localhost/<id>`.
fn serve_image(app: &AppHandle, path: &str) -> tauri::http::Response<Vec<u8>> {
    let id = path.trim_start_matches('/');
    let found = app.try_state::<AppState>().map(|s| s.images.get(id));
    match found {
        Some(Ok((bytes, kind))) => tauri::http::Response::builder()
            .header("Content-Type", kind.mime())
            // An id is the hash of the picture, so it never changes.
            .header("Cache-Control", "public, max-age=31536000, immutable")
            .header("X-Content-Type-Options", "nosniff")
            .body(bytes),
        _ => tauri::http::Response::builder().status(404).body(Vec::new()),
    }
    .unwrap_or_else(|_| tauri::http::Response::new(Vec::new()))
}

// ---------- news ----------

#[tauri::command]
fn news_view(state: State<AppState>, topic_id: Option<i64>, filter: Option<Filter>) -> Result<NewsView, String> {
    news::news_view(&*state.store()?, topic_id, filter.unwrap_or(Filter::All), now(), day_start()).map_err(err)
}

#[tauri::command]
fn save_topic(app: AppHandle, state: State<AppState>, id: Option<i64>, input: TopicInput) -> Result<i64, String> {
    let id = {
        let store = state.store()?;
        match id {
            Some(id) => store.update_topic(id, &input).map(|_| id),
            None => store.create_topic(&input, now()),
        }
        .map_err(err)?
    };
    // A new or changed topic is checked straight away.
    let handle = app.clone();
    std::thread::spawn(move || {
        run_check(&handle, id);
    });
    Ok(id)
}

#[tauri::command]
fn delete_topic(state: State<AppState>, id: i64) -> Result<(), String> {
    state.store()?.delete_topic(id).map_err(err)
}

#[tauri::command]
async fn check_topic_now(app: AppHandle, id: i64) -> Result<CheckOutcome, String> {
    tauri::async_runtime::spawn_blocking(move || run_check(&app, id).ok_or_else(|| "This topic is already being checked.".to_string())?)
        .await
        .map_err(|e| e.to_string())?
}

#[derive(Serialize)]
struct FoundFeed {
    url: String,
    title: String,
}

#[tauri::command]
async fn find_feed(app: AppHandle, address: String) -> Result<FoundFeed, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<AppState>();
        feed::discover_feed(&state.fetcher, &address).map(|(url, title)| FoundFeed { url, title }).map_err(err)
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
fn set_story_read(state: State<AppState>, item_id: i64, read: bool) -> Result<(), String> {
    state.store()?.set_story_read(item_id, read).map_err(err)
}

#[tauri::command]
fn mark_topic_read(state: State<AppState>, topic_id: i64) -> Result<(), String> {
    state.store()?.mark_topic_read(topic_id).map_err(err)
}

#[tauri::command]
fn set_saved(state: State<AppState>, item_id: i64, saved: bool) -> Result<(), String> {
    state.store()?.set_saved(item_id, saved).map_err(err)
}

#[tauri::command]
fn block_site(state: State<AppState>, site: String) -> Result<String, String> {
    state.store()?.block_site(&site).map_err(err)
}

#[tauri::command]
fn unblock_site(state: State<AppState>, domain: String) -> Result<(), String> {
    state.store()?.unblock_site(&domain).map_err(err)
}

/// Opens a story in the default browser and marks it read. Uses the article's own address
/// when CocoaDesk has already found it.
#[tauri::command]
fn open_story(app: AppHandle, state: State<AppState>, item_id: i64) -> Result<(), String> {
    let url = {
        let store = state.store()?;
        let item = store.news_item(item_id).map_err(err)?;
        store.set_story_read(item_id, true).map_err(err)?;
        match store.cached_article(item_id).map_err(err)? {
            Some((Some(a), _)) => a.url,
            _ => item.url,
        }
    };
    open_web(&app, &url)
}

fn open_web(app: &AppHandle, url: &str) -> Result<(), String> {
    if !(url.starts_with("https://") || url.starts_with("http://")) {
        return Err("That isn't a web address.".into());
    }
    app.opener().open_url(url, None::<&str>).map_err(|e| e.to_string())
}

/// Opens a link from inside an article in the browser.
#[tauri::command]
fn open_link(app: AppHandle, url: String) -> Result<(), String> {
    open_web(&app, &url)
}

/// The article text for the reader, fetched the first time and kept afterwards.
#[tauri::command]
async fn read_story(app: AppHandle, item_id: i64, refresh: Option<bool>) -> Result<Reading, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<AppState>();
        let r = news::read_story(&state.store, item_id, &state.fetcher, Utc::now(), refresh.unwrap_or(false)).map_err(err);
        let _ = app.emit("news:read", item_id);
        r
    })
    .await
    .map_err(|e| e.to_string())?
}

/// Checks one topic, sends its notifications and tells the UI. Returns `None` if a check of
/// the same topic is already running.
fn run_check(app: &AppHandle, id: i64) -> Option<Result<CheckOutcome, String>> {
    let state = app.state::<AppState>();
    {
        let mut busy = state.checking.lock().ok()?;
        if !busy.insert(id) {
            return None;
        }
    }
    let _ = app.emit("news:checking", id);
    let result = news::check_topic(&state.store, id, &state.fetcher, Utc::now(), now()).map_err(err);
    if let Ok(mut busy) = state.checking.lock() {
        busy.remove(&id);
    }
    if let Ok(out) = &result {
        for item in &out.notify {
            let body = if item.source_name.is_empty() { item.title.clone() } else { format!("{} · {}", item.title, item.source_name) };
            let _ = app.notification().builder().title(&out.topic_name).body(body).show();
        }
        if out.notify_more > 0 {
            let _ = app.notification().builder().title(&out.topic_name).body(format!("…and {} more new stories", out.notify_more)).show();
        }
    }
    let _ = app.emit("news:changed", id);
    // Read today's most covered stories in the background, so the day's summary can quote
    // their opening lines.
    let handle = app.clone();
    std::thread::spawn(move || {
        let state = handle.state::<AppState>();
        let targets = state.store().ok().and_then(|s| news::digest_prefetch(&s, id, day_start()).ok()).unwrap_or_default();
        if !targets.is_empty() {
            for item in targets {
                let _ = news::prefetch_article(&state.store, item, &state.fetcher, Utc::now());
            }
            let _ = handle.emit("news:changed", id);
        }
    });
    Some(result)
}

/// Checks topics whose interval has passed. Runs every 30 seconds; each topic is fetched on
/// its own thread so one slow website doesn't hold up the rest.
fn start_news_timer(app: AppHandle) {
    std::thread::spawn(move || {
        let mut last_prune: Option<std::time::Instant> = None;
        loop {
            let due = {
                let state = app.state::<AppState>();
                let ids = state.store().ok().and_then(|s| news::due_topics(&s, now()).ok()).unwrap_or_default();
                if last_prune.is_none_or(|t| t.elapsed() > Duration::from_secs(3600)) {
                    if let Ok(s) = state.store() {
                        let _ = s.prune_news(Utc::now());
                    }
                    last_prune = Some(std::time::Instant::now());
                }
                ids
            };
            for id in due {
                let handle = app.clone();
                std::thread::spawn(move || {
                    if let Some(Err(e)) = run_check(&handle, id) {
                        eprintln!("news: {e}");
                    }
                });
            }
            std::thread::sleep(Duration::from_secs(30));
        }
    });
}

// ---------- window and tray ----------

fn show_main(app: &AppHandle) {
    if let Some(w) = app.get_webview_window("main") {
        let _ = w.unminimize();
        let _ = w.show();
        let _ = w.set_focus();
    }
}

fn build_tray(app: &AppHandle) -> tauri::Result<()> {
    let open = MenuItem::with_id(app, "open", "Open CocoaDesk", true, None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", "Quit CocoaDesk", true, None::<&str>)?;
    let menu = Menu::with_items(app, &[&open, &quit])?;
    let mut tray = TrayIconBuilder::with_id("main")
        .tooltip("CocoaDesk")
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_menu_event(|app, e| match e.id.as_ref() {
            "open" => show_main(app),
            "quit" => app.exit(0),
            _ => {}
        })
        .on_tray_icon_event(|tray, e| {
            if let TrayIconEvent::Click { button: MouseButton::Left, button_state: MouseButtonState::Up, .. } = e {
                show_main(tray.app_handle());
            }
        });
    if let Some(icon) = app.default_window_icon() {
        tray = tray.icon(icon.clone());
    }
    tray.build(app)?;
    Ok(())
}

// ---------- background timer ----------

/// Marks ended classes, sends reminders and nudges the UI. It runs every 15 seconds and
/// works from the time of the previous run, so it also catches up after sleep or shutdown.
fn start_timer(app: AppHandle) {
    std::thread::spawn(move || {
        let mut last_minute = None;
        loop {
            let t = now();
            let outcome = {
                let state = app.state::<AppState>();
                let result = match state.store() {
                    Ok(store) => service::tick(&store, t).map_err(err),
                    Err(e) => Err(e),
                };
                result
            };
            match outcome {
                Ok(o) => {
                    for r in &o.reminders {
                        let mut body = format!("{} at {}", cocoadesk_kind(r.kind), r.start.format("%H:%M"));
                        if !r.room.is_empty() {
                            body.push_str(&format!(" · {}", r.room));
                        }
                        body.push_str(&format!(" · in {} min", r.minutes_until));
                        let _ = app.notification().builder().title(&r.subject).body(body).show();
                    }
                    let minute = (t.date(), t.hour(), t.minute());
                    if o.marked > 0 || !o.reminders.is_empty() || last_minute != Some(minute) {
                        last_minute = Some(minute);
                        let _ = app.emit("classes:changed", ());
                    }
                }
                Err(e) => eprintln!("timer: {e}"),
            }
            std::thread::sleep(Duration::from_secs(15));
        }
    });
}

fn cocoadesk_kind(k: cocoadesk_core::model::ClassKind) -> &'static str {
    use cocoadesk_core::model::ClassKind::*;
    match k {
        Lecture => "Lecture",
        Seminar => "Seminar",
        Lab => "Lab",
        Practice => "Practice",
        Sport => "Sport",
        Other => "Class",
    }
}

pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| show_main(app)))
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_opener::init())
        .register_asynchronous_uri_scheme_protocol("boardimg", |ctx, request, responder| {
            let app = ctx.app_handle().clone();
            let path = request.uri().path().to_string();
            std::thread::spawn(move || responder.respond(serve_image(&app, &path)));
        })
        .setup(|app| {
            let dir = app.path().app_data_dir()?;
            let store = Store::open(&dir.join("cocoadesk.db")).map_err(|e| Box::<dyn std::error::Error>::from(e.to_string()))?;
            let images = ImageStore::new(dir.join("board-images"));
            // Pictures no board uses any more are removed at start, never while boards are being edited.
            if let Ok(in_use) = store.board_images_in_use() {
                images.collect_garbage(&in_use);
            }
            app.manage(AppState {
                store: Mutex::new(store),
                files: Mutex::new(HashMap::new()),
                next_file: AtomicU64::new(1),
                fetcher: HttpFetcher::new(),
                checking: Mutex::new(HashSet::new()),
                images,
                overlay: Mutex::new(None),
            });
            build_tray(app.handle())?;
            start_timer(app.handle().clone());
            start_news_timer(app.handle().clone());
            Ok(())
        })
        .on_window_event(|window, event| {
            if let WindowEvent::CloseRequested { api, .. } = event {
                let keep = window
                    .app_handle()
                    .try_state::<AppState>()
                    .and_then(|s| s.store().ok().and_then(|st| Settings::load(&st).ok()))
                    .map(|s| s.close_to_tray)
                    .unwrap_or(false);
                if keep {
                    api.prevent_close();
                    let _ = window.hide();
                }
            }
        })
        .invoke_handler(tauri::generate_handler![
            week_view,
            save_semester,
            delete_semester,
            set_periods,
            add_break,
            delete_break,
            save_class,
            delete_class,
            set_status,
            save_settings,
            pick_timetable,
            import_preview,
            import_exclusions,
            import_commit,
            news_view,
            save_topic,
            delete_topic,
            check_topic_now,
            find_feed,
            set_story_read,
            mark_topic_read,
            set_saved,
            block_site,
            unblock_site,
            open_story,
            open_link,
            read_story,
            boards_list,
            board_load,
            board_create,
            board_rename,
            board_duplicate,
            board_delete,
            board_save,
            board_add_image,
            board_fetch_image,
            board_overlay
        ])
        .run(tauri::generate_context!())
        .expect("error while running CocoaDesk");
}
