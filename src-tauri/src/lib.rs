mod db;
mod player;
mod scanner;

use rusqlite::Connection;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::Mutex;
use tauri::{Manager, State};

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
fn scan_library(state: State<AppState>) -> Result<(), String> {
    let config = read_config(&state.config_path);
    let folder = config.library_folder.ok_or("no library folder set")?;
    let conn = state.db.lock().map_err(|e| e.to_string())?;
    scanner::scan_and_sync(&conn, &folder);
    Ok(())
}

#[tauri::command]
fn get_library(sort_by: String, sort_dir: String, state: State<AppState>) -> Result<Vec<Track>, String> {
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
            let config = read_config(&config_path);

            if let Some(ref folder) = config.library_folder {
                scanner::scan_and_sync(&conn, folder);
            }

            app.manage(AppState {
                db: Mutex::new(conn),
                config_path,
            });

            app.manage(player::AudioPlayer::new(app.handle().clone()));

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            get_library_folder,
            set_library_folder,
            scan_library,
            get_library,
            play_track,
            toggle_playback,
            playback_position,
            seek,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
