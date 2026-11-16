//! The app icon choice (Amethyst or Midnight) and the desktop shortcut.

use tauri::image::Image;
use tauri::{AppHandle, Manager};

pub const THEMES: [&str; 2] = ["amethyst", "midnight"];
const KEY: &str = "app_icon";

fn png(theme: &str) -> &'static [u8] {
    match theme {
        "midnight" => include_bytes!("../icons/themes/midnight.png"),
        _ => include_bytes!("../icons/themes/amethyst.png"),
    }
}

#[cfg_attr(not(windows), allow(dead_code))]
fn ico(theme: &str) -> &'static [u8] {
    match theme {
        "midnight" => include_bytes!("../icons/themes/midnight.ico"),
        _ => include_bytes!("../icons/themes/amethyst.ico"),
    }
}

pub fn valid(theme: &str) -> bool {
    THEMES.contains(&theme)
}

/// The saved choice; Amethyst, the installed icon, when nothing is saved.
pub fn current(store: &cocoadesk_core::store::Store) -> String {
    store.setting(KEY).ok().flatten().filter(|t| valid(t)).unwrap_or_else(|| THEMES[0].to_string())
}

pub fn save(store: &cocoadesk_core::store::Store, theme: &str) -> Result<(), String> {
    store.set_setting(KEY, theme).map_err(|e| e.to_string())
}

/// Puts the icon on the window and the notification-area icon, and on CocoaDesk's shortcuts.
pub fn apply(app: &AppHandle, theme: &str, update_shortcuts: bool) {
    if let Ok(img) = Image::from_bytes(png(theme)) {
        if let Some(w) = app.get_webview_window("main") {
            let _ = w.set_icon(img.clone());
        }
        if let Some(tray) = app.tray_by_id("main") {
            let _ = tray.set_icon(Some(img));
        }
    }
    if update_shortcuts {
        let _ = shortcuts(app, theme, false);
    }
}

/// Creates a desktop shortcut with the chosen icon (when `create`), and points CocoaDesk's
/// existing Desktop, Start menu and pinned taskbar shortcuts at the same icon.
#[cfg(windows)]
pub fn shortcuts(app: &AppHandle, theme: &str, create: bool) -> Result<(), String> {
    use std::os::windows::process::CommandExt;
    use std::path::PathBuf;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    const NAME: &str = "CocoaDesk.lnk";

    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    // Shortcuts need the icon as a file on disk.
    let dir = app.path().app_data_dir().map_err(|e| e.to_string())?.join("appicons");
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let icon = dir.join(format!("{theme}.ico"));
    if std::fs::read(&icon).ok().as_deref() != Some(ico(theme)) {
        std::fs::write(&icon, ico(theme)).map_err(|e| e.to_string())?;
    }

    let desktop = app.path().desktop_dir().map_err(|e| e.to_string())?.join(NAME);
    let mut update: Vec<PathBuf> = vec![desktop.clone()];
    if let Ok(roam) = std::env::var("APPDATA") {
        let roam = PathBuf::from(roam).join("Microsoft");
        update.push(roam.join(r"Windows\Start Menu\Programs").join(NAME));
        update.push(roam.join(r"Internet Explorer\Quick Launch\User Pinned\TaskBar").join(NAME));
    }
    if let Ok(public) = std::env::var("PUBLIC") {
        update.push(PathBuf::from(public).join("Desktop").join(NAME));
    }
    if let Ok(pd) = std::env::var("ProgramData") {
        update.push(PathBuf::from(pd).join(r"Microsoft\Windows\Start Menu\Programs").join(NAME));
    }
    let update = update.iter().map(|p| p.to_string_lossy().into_owned()).collect::<Vec<_>>().join("|");

    // Paths travel in environment variables, so nothing in them is read as script.
    const SCRIPT: &str = r#"
$ErrorActionPreference = 'Stop'
$sh = New-Object -ComObject WScript.Shell
$exe = $env:COCOA_LNK_EXE
$ico = $env:COCOA_LNK_ICO + ',0'
if ($env:COCOA_LNK_CREATE) {
  $s = $sh.CreateShortcut($env:COCOA_LNK_CREATE)
  $s.TargetPath = $exe
  $s.WorkingDirectory = Split-Path -Parent $exe
  $s.IconLocation = $ico
  $s.Description = 'CocoaDesk'
  $s.Save()
}
foreach ($p in ($env:COCOA_LNK_UPDATE -split '\|')) {
  if ($p -and (Test-Path -LiteralPath $p)) {
    try { $s = $sh.CreateShortcut($p); if ($s.TargetPath -ieq $exe) { $s.IconLocation = $ico; $s.Save() } } catch { }
  }
}
"#;
    let out = std::process::Command::new("powershell.exe")
        .args(["-NoProfile", "-NonInteractive", "-ExecutionPolicy", "Bypass", "-Command", SCRIPT])
        .env("COCOA_LNK_EXE", &exe)
        .env("COCOA_LNK_ICO", &icon)
        .env("COCOA_LNK_CREATE", if create { desktop.as_os_str() } else { std::ffi::OsStr::new("") })
        .env("COCOA_LNK_UPDATE", update)
        .creation_flags(CREATE_NO_WINDOW)
        .output()
        .map_err(|e| format!("The shortcut could not be created: {e}"))?;
    if !out.status.success() {
        return Err("The shortcut could not be created.".into());
    }
    Ok(())
}

#[cfg(not(windows))]
pub fn shortcuts(_app: &AppHandle, _theme: &str, create: bool) -> Result<(), String> {
    if create {
        return Err("Shortcuts can be created on Windows only.".into());
    }
    Ok(())
}
