//! The CocoaDesk desktop shell: window, tray, background timer and the commands the UI calls.

mod appicon;

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
use cocoadesk_core::research::sources::{Explore, Overview};
use cocoadesk_core::research::store::{
    self as research, Citations as PaperCitations, Filter as ResearchFilter, LibraryView, PaperView, Reading as PaperReading,
    SearchResult, Settings as ResearchSettings, Shelf, TopicInput as ResearchTopicInput, TopicsView,
};
use cocoadesk_core::research::Query;
use cocoadesk_core::notes::Note;
use cocoadesk_core::service::{self, ImportDone, ImportRequest, Settings, WeekView};
use cocoadesk_core::store::{SemesterInput, Store};
use cocoadesk_core::tasks::{self, BoardInput, CardInput, RateInfo, TasksView};

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
    /// When the exchange rate was last asked for, so a failing download isn't retried on every view.
    rate_tried: Mutex<Option<std::time::Instant>>,
    /// Research topics being checked right now.
    research_checking: Mutex<HashSet<i64>>,
    /// Downloaded paper PDFs, served to the reader at `paperpdf://localhost/<paper id>`.
    pdf_dir: std::path::PathBuf,
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

#[tauri::command]
fn app_icon(state: State<AppState>) -> Result<String, String> {
    Ok(appicon::current(&*state.store()?))
}

/// Switches the app icon; existing CocoaDesk shortcuts follow it.
#[tauri::command]
async fn set_app_icon(app: AppHandle, state: State<'_, AppState>, theme: String) -> Result<String, String> {
    if !appicon::valid(&theme) {
        return Err("Unknown icon.".into());
    }
    appicon::save(&*state.store()?, &theme)?;
    appicon::apply(&app, &theme, true);
    Ok(theme)
}

/// Puts a CocoaDesk shortcut with the chosen icon on the desktop.
#[tauri::command]
async fn create_shortcut(app: AppHandle, state: State<'_, AppState>) -> Result<(), String> {
    let theme = appicon::current(&*state.store()?);
    appicon::shortcuts(&app, &theme, true)
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

// ---------- notes ----------

#[tauri::command]
fn notes_list(state: State<AppState>) -> Result<Vec<Note>, String> {
    state.store()?.notes().map_err(err)
}

#[tauri::command]
fn note_create(state: State<AppState>, body: String) -> Result<Note, String> {
    state.store()?.create_note(&body, now()).map_err(err)
}

#[tauri::command]
fn note_save(state: State<AppState>, id: i64, body: String) -> Result<Note, String> {
    state.store()?.save_note(id, &body, now()).map_err(err)
}

#[tauri::command]
fn note_pin(state: State<AppState>, id: i64, pinned: bool) -> Result<(), String> {
    state.store()?.set_note_pinned(id, pinned).map_err(err)
}

#[tauri::command]
fn note_delete(state: State<AppState>, id: i64) -> Result<(), String> {
    state.store()?.delete_note(id).map_err(err)
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
        // The pinned panel hides the top bar and its window controls, so it gets the native frame back.
        window.set_decorations(true).map_err(e)?;
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
        window.set_decorations(false).map_err(e)?;
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


// ---------- research ----------

#[tauri::command]
fn research_settings(state: State<AppState>) -> Result<ResearchSettings, String> {
    state.store()?.research_settings().map_err(err)
}

#[tauri::command]
fn research_save_settings(state: State<AppState>, settings: ResearchSettings) -> Result<(), String> {
    state.store()?.save_research_settings(&settings).map_err(err)
}

#[tauri::command]
async fn research_search(app: AppHandle, query: Query) -> Result<SearchResult, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<AppState>();
        research::search(&state.store, &state.fetcher, &query, Utc::now()).map_err(err)
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
async fn research_overview(app: AppHandle, text: String) -> Result<Option<Overview>, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<AppState>();
        research::overview(&state.fetcher, &text).map_err(err)
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
fn research_topics_view(state: State<AppState>, topic_id: Option<i64>, filter: Option<ResearchFilter>) -> Result<TopicsView, String> {
    research::topics_view(&*state.store()?, topic_id, filter.unwrap_or(ResearchFilter::All), Utc::now()).map_err(err)
}

#[tauri::command]
fn research_save_topic(app: AppHandle, state: State<AppState>, id: Option<i64>, input: ResearchTopicInput) -> Result<i64, String> {
    let id = {
        let store = state.store()?;
        match id {
            Some(id) => store.update_research_topic(id, &input).map(|_| id),
            None => store.create_research_topic(&input, now()),
        }
        .map_err(err)?
    };
    let handle = app.clone();
    std::thread::spawn(move || {
        run_research_check(&handle, id);
    });
    Ok(id)
}

#[tauri::command]
fn research_delete_topic(state: State<AppState>, id: i64) -> Result<(), String> {
    state.store()?.delete_research_topic(id).map_err(err)
}

#[tauri::command]
async fn research_check_now(app: AppHandle, id: i64) -> Result<research::CheckOutcome, String> {
    tauri::async_runtime::spawn_blocking(move || run_research_check(&app, id).ok_or_else(|| "This topic is already being checked.".to_string())?)
        .await
        .map_err(|e| e.to_string())?
}

#[tauri::command]
fn research_mark_topic_read(state: State<AppState>, topic_id: i64) -> Result<(), String> {
    state.store()?.mark_research_topic_read(topic_id).map_err(err)
}

#[tauri::command]
fn research_library(state: State<AppState>, shelf: Shelf) -> Result<LibraryView, String> {
    research::library_view(&*state.store()?, shelf).map_err(err)
}

#[tauri::command]
fn research_set_saved(state: State<AppState>, paper_id: i64, saved: bool) -> Result<(), String> {
    state.store()?.set_paper_saved(paper_id, saved, Utc::now()).map_err(err)
}

#[tauri::command]
fn research_set_read(state: State<AppState>, paper_id: i64, read: bool) -> Result<(), String> {
    state.store()?.set_paper_read(paper_id, read).map_err(err)
}

#[tauri::command]
fn research_set_note(state: State<AppState>, paper_id: i64, note: String) -> Result<(), String> {
    state.store()?.set_paper_note(paper_id, &note, Utc::now()).map_err(err)
}

#[tauri::command]
fn research_set_subject(state: State<AppState>, paper_id: i64, subject: String) -> Result<(), String> {
    state.store()?.set_paper_subject(paper_id, &subject, Utc::now()).map_err(err)
}

#[tauri::command]
fn research_set_collections(state: State<AppState>, paper_id: i64, collections: Vec<i64>) -> Result<(), String> {
    state.store()?.set_paper_collections(paper_id, &collections, Utc::now()).map_err(err)
}

#[tauri::command]
fn research_save_collection(state: State<AppState>, id: Option<i64>, name: String) -> Result<i64, String> {
    state.store()?.save_collection(id, &name).map_err(err)
}

#[tauri::command]
fn research_delete_collection(state: State<AppState>, id: i64) -> Result<(), String> {
    state.store()?.delete_collection(id).map_err(err)
}

#[tauri::command]
fn research_blocked(state: State<AppState>) -> Result<Vec<String>, String> {
    state.store()?.research_blocked().map_err(err)
}

#[tauri::command]
fn research_block(state: State<AppState>, value: String) -> Result<String, String> {
    state.store()?.block_research(&value).map_err(err)
}

#[tauri::command]
fn research_unblock(state: State<AppState>, value: String) -> Result<(), String> {
    state.store()?.unblock_research(&value).map_err(err)
}

#[tauri::command]
async fn research_read(app: AppHandle, paper_id: i64, refresh: Option<bool>) -> Result<PaperReading, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<AppState>();
        research::read_paper(&state.store, paper_id, &state.fetcher, Utc::now(), now().date(), refresh.unwrap_or(false)).map_err(err)
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
fn research_cite(state: State<AppState>, paper_id: i64) -> Result<PaperCitations, String> {
    let p = state.store()?.research_paper(paper_id).map_err(err)?;
    Ok(research::citations(&p, now().date()))
}

#[tauri::command]
async fn research_explore(app: AppHandle, paper_id: i64, kind: Explore) -> Result<Vec<PaperView>, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<AppState>();
        research::explore(&state.store, &state.fetcher, paper_id, kind, Utc::now()).map_err(err)
    })
    .await
    .map_err(|e| e.to_string())?
}

#[derive(Serialize)]
struct BoardChoice {
    id: i64,
    name: String,
}

#[tauri::command]
fn research_task_boards(state: State<AppState>) -> Result<Vec<BoardChoice>, String> {
    Ok(state.store()?.task_boards().map_err(err)?.into_iter().map(|b| BoardChoice { id: b.id, name: b.name }).collect())
}

#[tauri::command]
fn research_add_to_tasks(app: AppHandle, state: State<AppState>, paper_id: i64, board_id: i64) -> Result<i64, String> {
    let card = research::add_to_tasks(&*state.store()?, paper_id, board_id, Utc::now(), now()).map_err(err)?;
    let _ = app.emit("tasks:changed", ());
    Ok(card)
}

fn pdf_path(state: &AppState, paper_id: i64) -> std::path::PathBuf {
    state.pdf_dir.join(format!("{paper_id}.pdf"))
}

/// Downloads a paper's free PDF (once) so the reader can show it.
#[tauri::command]
async fn research_pdf(app: AppHandle, paper_id: i64) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<AppState>();
        let path = pdf_path(&state, paper_id);
        if path.exists() {
            return Ok(());
        }
        let url = state.store()?.research_paper(paper_id).map_err(err)?.pdf_url.ok_or_else(|| "There's no free PDF of this paper.".to_string())?;
        let bytes = research::fetch_pdf(&state.fetcher, &url).map_err(err)?;
        std::fs::create_dir_all(&state.pdf_dir).map_err(|e| format!("Can't create the PDF folder: {e}"))?;
        std::fs::write(&path, bytes).map_err(|e| format!("Can't save the PDF: {e}"))
    })
    .await
    .map_err(|e| e.to_string())?
}

/// Saves a copy of the downloaded PDF where the person chooses.
#[tauri::command]
async fn research_save_pdf(app: AppHandle, paper_id: i64) -> Result<bool, String> {
    research_pdf(app.clone(), paper_id).await?;
    let state = app.state::<AppState>();
    let paper = state.store()?.research_paper(paper_id).map_err(err)?;
    let first = paper.authors.first().map(|a| a.family.clone()).unwrap_or_default();
    let mut name: String = format!("{first} {} {}", paper.year.map(|y| y.to_string()).unwrap_or_default(), paper.title)
        .chars()
        .map(|c| if "\\/:*?\"<>|".contains(c) { ' ' } else { c })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    name = name.chars().take(120).collect();
    let target = app.dialog().file().set_title("Save the PDF").set_file_name(format!("{name}.pdf")).add_filter("PDF", &["pdf"]).blocking_save_file();
    let Some(target) = target else { return Ok(false) };
    let target = target.into_path().map_err(|e| e.to_string())?;
    std::fs::copy(pdf_path(&state, paper_id), target).map_err(|e| format!("Can't save the PDF: {e}"))?;
    Ok(true)
}

/// Opens a paper's page or PDF in the browser.
#[tauri::command]
fn research_open(app: AppHandle, state: State<AppState>, paper_id: i64, what: String) -> Result<(), String> {
    let p = state.store()?.research_paper(paper_id).map_err(err)?;
    let url = match what.as_str() {
        "pdf" => p.pdf_url.ok_or_else(|| "There's no free PDF of this paper.".to_string())?,
        "doi" => p.doi.map(|d| format!("https://doi.org/{d}")).unwrap_or(p.url),
        _ => p.url,
    };
    open_web(&app, &url)
}

/// Serves downloaded PDFs to the reader at `paperpdf://localhost/<paper id>`.
fn serve_pdf(app: &AppHandle, path: &str) -> tauri::http::Response<Vec<u8>> {
    let id: Option<i64> = path.trim_start_matches('/').trim_end_matches(".pdf").parse().ok();
    let bytes = match (app.try_state::<AppState>(), id) {
        (Some(state), Some(id)) => std::fs::read(pdf_path(&state, id)).ok(),
        _ => None,
    };
    match bytes {
        Some(b) => tauri::http::Response::builder().header("Content-Type", "application/pdf").header("X-Content-Type-Options", "nosniff").body(b),
        None => tauri::http::Response::builder().status(404).body(Vec::new()),
    }
    .unwrap_or_else(|_| tauri::http::Response::new(Vec::new()))
}

/// Checks one research topic, sends its notifications and tells the UI.
fn run_research_check(app: &AppHandle, id: i64) -> Option<Result<research::CheckOutcome, String>> {
    let state = app.state::<AppState>();
    {
        let mut busy = state.research_checking.lock().ok()?;
        if !busy.insert(id) {
            return None;
        }
    }
    let _ = app.emit("research:checking", id);
    let result = research::check_topic(&state.store, id, &state.fetcher, Utc::now(), now()).map_err(err);
    if let Ok(mut busy) = state.research_checking.lock() {
        busy.remove(&id);
    }
    if let Ok(out) = &result {
        for p in &out.notify {
            let mut body = p.paper.title.clone();
            let by = p.paper.short_authors();
            if !by.is_empty() {
                body.push_str(&format!(" ({by})"));
            }
            let _ = app.notification().builder().title(format!("New paper: {}", out.topic_name)).body(body).show();
        }
        if out.notify_more > 0 {
            let _ = app.notification().builder().title(&out.topic_name).body(format!("…and {} more new papers", out.notify_more)).show();
        }
    }
    let _ = app.emit("research:changed", id);
    Some(result)
}

/// Checks research topics whose interval has passed, every minute.
fn start_research_timer(app: AppHandle) {
    std::thread::spawn(move || {
        let mut last_prune: Option<std::time::Instant> = None;
        // Give the window a moment before the first round of downloads.
        std::thread::sleep(Duration::from_secs(20));
        loop {
            let due = {
                let state = app.state::<AppState>();
                let ids = state.store().ok().and_then(|s| research::due_topics(&s, now()).ok()).unwrap_or_default();
                if last_prune.is_none_or(|t| t.elapsed() > Duration::from_secs(3600)) {
                    if let Ok(s) = state.store() {
                        let _ = s.prune_research(Utc::now());
                    }
                    last_prune = Some(std::time::Instant::now());
                }
                ids
            };
            for id in due {
                let handle = app.clone();
                std::thread::spawn(move || {
                    if let Some(Err(e)) = run_research_check(&handle, id) {
                        eprintln!("research: {e}");
                    }
                });
            }
            std::thread::sleep(Duration::from_secs(60));
        }
    });
}

// ---------- tasks ----------

/// Downloads today's dollar rate in the background when the kept one is from an earlier day,
/// at most once every ten minutes, and tells the window when it arrives.
fn refresh_rate_if_stale(app: &AppHandle) {
    let state = app.state::<AppState>();
    let stale = state.store().ok().and_then(|s| s.rate_is_stale(now().date()).ok()).unwrap_or(false);
    if !stale {
        return;
    }
    {
        let Ok(mut tried) = state.rate_tried.lock() else { return };
        if tried.is_some_and(|t| t.elapsed() < Duration::from_secs(600)) {
            return;
        }
        *tried = Some(std::time::Instant::now());
    }
    let handle = app.clone();
    std::thread::spawn(move || {
        let state = handle.state::<AppState>();
        if tasks::refresh_rate(&state.store, &state.fetcher).is_ok() {
            let _ = handle.emit("tasks:changed", ());
        }
    });
}

#[tauri::command]
fn tasks_view(app: AppHandle, state: State<AppState>, board_id: Option<i64>) -> Result<TasksView, String> {
    refresh_rate_if_stale(&app);
    tasks::tasks_view(&*state.store()?, board_id, now()).map_err(err)
}

#[tauri::command]
fn task_board_save(state: State<AppState>, id: Option<i64>, input: BoardInput) -> Result<i64, String> {
    let store = state.store()?;
    match id {
        Some(id) => store.update_task_board(id, &input).map(|_| id),
        None => store.create_task_board(&input, now()),
    }
    .map_err(err)
}

#[tauri::command]
fn task_board_delete(state: State<AppState>, id: i64) -> Result<(), String> {
    state.store()?.delete_task_board(id).map_err(err)
}

#[tauri::command]
fn task_list_save(state: State<AppState>, board_id: i64, id: Option<i64>, name: String, is_done: bool) -> Result<i64, String> {
    let store = state.store()?;
    match id {
        Some(id) => store.update_task_list(id, &name, is_done, now()).map(|_| id),
        None => store.add_task_list(board_id, &name, is_done),
    }
    .map_err(err)
}

#[tauri::command]
fn task_list_delete(state: State<AppState>, id: i64) -> Result<(), String> {
    state.store()?.delete_task_list(id).map_err(err)
}

#[tauri::command]
fn task_list_move(state: State<AppState>, id: i64, index: usize) -> Result<(), String> {
    state.store()?.move_task_list(id, index).map_err(err)
}

#[tauri::command]
fn task_card_save(state: State<AppState>, list_id: i64, id: Option<i64>, input: CardInput) -> Result<i64, String> {
    let store = state.store()?;
    match id {
        Some(id) => store.update_task_card(id, &input, now()).map(|_| id),
        None => store.add_task_card(list_id, &input, now()),
    }
    .map_err(err)
}

#[tauri::command]
fn task_card_delete(state: State<AppState>, id: i64) -> Result<(), String> {
    state.store()?.delete_task_card(id).map_err(err)
}

#[tauri::command]
fn task_card_move(state: State<AppState>, id: i64, list_id: i64, index: usize) -> Result<(), String> {
    state.store()?.move_task_card(id, list_id, index, now()).map_err(err)
}

#[tauri::command]
fn task_card_paid(state: State<AppState>, id: i64, paid: bool) -> Result<(), String> {
    state.store()?.set_task_card_paid(id, paid, now()).map_err(err)
}

#[tauri::command]
async fn rate_refresh(app: AppHandle) -> Result<RateInfo, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<AppState>();
        tasks::refresh_rate(&state.store, &state.fetcher).map_err(err)
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
fn rate_set_manual(state: State<AppState>, value: Option<f64>) -> Result<(), String> {
    state.store()?.set_manual_rate(value).map_err(err)
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
            let body = if item.source_name.is_empty() { item.title.clone() } else { format!("{} ({})", item.title, item.source_name) };
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
    let note = MenuItem::with_id(app, "note", "Quick note", true, None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", "Quit CocoaDesk", true, None::<&str>)?;
    let menu = Menu::with_items(app, &[&open, &note, &quit])?;
    let mut tray = TrayIconBuilder::with_id("main")
        .tooltip("CocoaDesk")
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_menu_event(|app, e| match e.id.as_ref() {
            "open" => show_main(app),
            "note" => {
                show_main(app);
                let _ = app.emit("notes:quick", ());
            }
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
                            body.push_str(&format!(" in {}", r.room));
                        }
                        body.push_str(&format!(", starts in {} min", r.minutes_until));
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
        .register_asynchronous_uri_scheme_protocol("paperpdf", |ctx, request, responder| {
            let app = ctx.app_handle().clone();
            let path = request.uri().path().to_string();
            std::thread::spawn(move || responder.respond(serve_pdf(&app, &path)));
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
                rate_tried: Mutex::new(None),
                research_checking: Mutex::new(HashSet::new()),
                pdf_dir: dir.join("research-pdf"),
            });
            build_tray(app.handle())?;
            let theme = appicon::current(&*app.state::<AppState>().store()?);
            if theme != appicon::THEMES[0] {
                appicon::apply(app.handle(), &theme, false);
            }
            start_timer(app.handle().clone());
            start_news_timer(app.handle().clone());
            start_research_timer(app.handle().clone());
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
            app_icon,
            set_app_icon,
            create_shortcut,
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
            board_overlay,
            tasks_view,
            task_board_save,
            task_board_delete,
            task_list_save,
            task_list_delete,
            task_list_move,
            task_card_save,
            task_card_delete,
            task_card_move,
            task_card_paid,
            rate_refresh,
            rate_set_manual,
            research_settings,
            research_save_settings,
            research_search,
            research_overview,
            research_topics_view,
            research_save_topic,
            research_delete_topic,
            research_check_now,
            research_mark_topic_read,
            research_library,
            research_set_saved,
            research_set_read,
            research_set_note,
            research_set_subject,
            research_set_collections,
            research_save_collection,
            research_delete_collection,
            research_blocked,
            research_block,
            research_unblock,
            research_read,
            research_cite,
            research_explore,
            research_task_boards,
            research_add_to_tasks,
            research_pdf,
            research_save_pdf,
            research_open,
            notes_list,
            note_create,
            note_save,
            note_pin,
            note_delete
        ])
        .run(tauri::generate_context!())
        .expect("error while running CocoaDesk");
}
