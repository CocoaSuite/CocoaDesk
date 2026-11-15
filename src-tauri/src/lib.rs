//! The CocoaDesk desktop shell: window, tray, background timer and the commands the UI calls.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, MutexGuard};
use std::time::Duration;

use chrono::{Local, NaiveDate, NaiveDateTime, Timelike};
use serde::Serialize;
use tauri::menu::{Menu, MenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Emitter, Manager, State, WindowEvent};
use tauri_plugin_dialog::DialogExt;
use tauri_plugin_notification::NotificationExt;

use cocoadesk_core::import::WorkbookPreview;
use cocoadesk_core::model::{ClassInput, MarkStatus, Period};
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
        .setup(|app| {
            let dir = app.path().app_data_dir()?;
            let store = Store::open(&dir.join("cocoadesk.db")).map_err(|e| Box::<dyn std::error::Error>::from(e.to_string()))?;
            app.manage(AppState { store: Mutex::new(store), files: Mutex::new(HashMap::new()), next_file: AtomicU64::new(1) });
            build_tray(app.handle())?;
            start_timer(app.handle().clone());
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
            import_commit
        ])
        .run(tauri::generate_context!())
        .expect("error while running CocoaDesk");
}
