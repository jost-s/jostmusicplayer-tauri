mod db;
mod opus_source;
mod player;
mod scanner;
mod symphonia_source;

use rusqlite::Connection;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use tauri::{AppHandle, Emitter, Manager, State};

#[derive(Serialize)]
pub struct Track {
    pub id: i64,
    pub path: String,
    pub filename: String,
    pub title: Option<String>,
    pub artist: Option<String>,
    pub album: Option<String>,
    pub year: Option<i32>,
    pub track_num: Option<i64>,
    pub duration: Option<i64>,
}

#[derive(Serialize, Deserialize, Default)]
struct Config {
    library_folder: Option<String>,
}

pub struct AppState {
    pub db: Mutex<Connection>,
    pub config_path: PathBuf,
    /// True while a background library scan is running. Lets the frontend show a
    /// scanning indicator even if it mounts after a startup scan has begun.
    pub scanning: AtomicBool,
}

/// Run a library scan on a background thread, emitting `scan-started` and
/// `scan-finished` so the frontend can show progress and refresh when done.
/// No-ops if no library folder is configured or a scan is already running.
fn spawn_scan(app: AppHandle) {
    std::thread::spawn(move || {
        let state = app.state::<AppState>();

        // Guard against overlapping scans clobbering each other's DB sync.
        if state.scanning.swap(true, Ordering::SeqCst) {
            return;
        }

        if let Some(folder) = read_config(&state.config_path).library_folder {
            let _ = app.emit("scan-started", ());
            scanner::scan_and_sync(&state.db, &folder);
            state.scanning.store(false, Ordering::SeqCst);
            let _ = app.emit("scan-finished", ());
        } else {
            state.scanning.store(false, Ordering::SeqCst);
        }
    });
}

fn read_config(path: &PathBuf) -> Config {
    std::fs::read_to_string(path)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

fn write_config(path: &PathBuf, config: &Config) {
    if let Ok(json) = serde_json::to_string_pretty(config) {
        let _ = std::fs::write(path, json);
    }
}

#[tauri::command]
fn get_library_folder(state: State<AppState>) -> Option<String> {
    read_config(&state.config_path).library_folder
}

#[tauri::command]
fn set_library_folder(folder: String, state: State<AppState>) {
    let config = Config {
        library_folder: Some(folder),
    };
    write_config(&state.config_path, &config);
}

#[tauri::command]
fn scan_library(app: AppHandle) {
    spawn_scan(app);
}

#[tauri::command]
fn is_scanning(state: State<AppState>) -> bool {
    state.scanning.load(Ordering::SeqCst)
}

#[tauri::command]
fn get_library(
    sort_by: String,
    sort_dir: String,
    state: State<AppState>,
) -> Result<Vec<Track>, String> {
    let conn = state.db.lock().map_err(|e| e.to_string())?;
    db::get_tracks(&conn, &sort_by, &sort_dir).map_err(|e| e.to_string())
}

#[tauri::command]
fn play_track(path: String, player: State<player::AudioPlayer>) -> Result<Option<f64>, String> {
    player.play(path)
}

#[tauri::command]
fn toggle_playback(player: State<player::AudioPlayer>) -> bool {
    player.toggle()
}

#[tauri::command]
fn playback_position(player: State<player::AudioPlayer>) -> f64 {
    player.position()
}

#[tauri::command]
fn seek(seconds: f64, player: State<player::AudioPlayer>) -> Result<(), String> {
    player.seek(seconds)
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .setup(|app| {
            let app_data_dir = app
                .path()
                .app_data_dir()
                .expect("failed to resolve app data dir");
            std::fs::create_dir_all(&app_data_dir)?;

            let db_path = app_data_dir.join("library.db");
            let conn = Connection::open(&db_path).expect("failed to open database");
            db::init_schema(&conn).expect("failed to init db schema");

            let config_path = app_data_dir.join("config.json");
            let has_folder = read_config(&config_path).library_folder.is_some();

            app.manage(AppState {
                db: Mutex::new(conn),
                config_path,
                scanning: AtomicBool::new(false),
            });

            app.manage(player::AudioPlayer::new(app.handle().clone()));

            // Re-scan the saved library in the background so the window appears
            // instantly; the frontend listens for `scan-finished` to refresh.
            if has_folder {
                spawn_scan(app.handle().clone());
            }

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            get_library_folder,
            set_library_folder,
            scan_library,
            is_scanning,
            get_library,
            play_track,
            toggle_playback,
            playback_position,
            seek,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
