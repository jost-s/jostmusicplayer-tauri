//! Cross-platform OS media control integration via the `souvlaki` crate.
//!
//! Two directions of flow:
//! - **OS → app:** hardware media keys and the system now-playing UI (macOS
//!   Control Center, Windows SMTC, Linux MPRIS) deliver `MediaControlEvent`s to
//!   the handler attached in [`init`], which re-emits them as Tauri events. The
//!   frontend decides what they mean, because the play queue (sort/filter order)
//!   lives there, not in the backend.
//! - **app → OS:** the frontend pushes now-playing text and play state down
//!   through the [`MediaController`] methods so the OS UI stays in sync.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::Duration;

use base64::Engine;
use souvlaki::{
    MediaControlEvent, MediaControls, MediaMetadata, MediaPlayback, MediaPosition, PlatformConfig,
    SeekDirection,
};
use tauri::{AppHandle, Emitter};
use url::Url;

/// Sidecar cover-art filenames (matched case-insensitively), most-preferred first.
const COVER_BASENAMES: &[&str] = &["cover", "folder", "front", "album", "albumart"];
/// Image extensions the OS now-playing UIs reliably render.
const COVER_EXTENSIONS: &[&str] = &["jpg", "jpeg", "png"];

/// Look for an album-cover image sitting next to the track. We only handle
/// sidecar files in the track's folder (the common `cover.jpg` / `folder.jpg`
/// case); artwork embedded in the file's tags is intentionally not handled here.
fn find_cover_path(track_path: &str) -> Option<PathBuf> {
    let dir = Path::new(track_path).parent()?;
    // Index the folder once by lowercased name so matching is case-insensitive.
    let by_name: HashMap<String, PathBuf> = std::fs::read_dir(dir)
        .ok()?
        .flatten()
        .filter_map(|e| Some((e.file_name().to_str()?.to_lowercase(), e.path())))
        .collect();
    COVER_BASENAMES.iter().find_map(|base| {
        COVER_EXTENSIONS
            .iter()
            .find_map(|ext| by_name.get(&format!("{base}.{ext}")).cloned())
    })
}

/// Sidecar cover for the OS now-playing UI, as a percent-encoded `file://` URL.
fn find_cover_url(track_path: &str) -> Option<String> {
    let path = find_cover_path(track_path)?;
    Url::from_file_path(path).ok().map(|u| u.to_string())
}

/// Sidecar cover for the in-app UI, as a base64 `data:` URI the webview can load
/// directly (the asset sandbox blocks `file://`). Returns `None` if there's no
/// cover or it can't be read.
pub fn find_cover_data_uri(track_path: &str) -> Option<String> {
    let path = find_cover_path(track_path)?;
    let mime = match path.extension().and_then(|e| e.to_str()) {
        Some(e) if e.eq_ignore_ascii_case("png") => "image/png",
        _ => "image/jpeg",
    };
    let bytes = std::fs::read(&path).ok()?;
    let encoded = base64::engine::general_purpose::STANDARD.encode(bytes);
    Some(format!("data:{mime};base64,{encoded}"))
}

/// Handle to the OS media controls, stored in Tauri state.
///
/// `souvlaki::MediaControls` is `Send + Sync`, but its mutating methods take
/// `&mut self`, so it lives behind a `Mutex`.
pub struct MediaController(Mutex<MediaControls>);

impl MediaController {
    /// Update the now-playing text shown in the OS media UI, plus a sidecar cover
    /// image from the track's folder when one exists (`track_path` is the audio
    /// file's path; embedded artwork is not handled).
    pub fn update_metadata(
        &self,
        title: Option<String>,
        artist: Option<String>,
        album: Option<String>,
        duration: Option<f64>,
        track_path: Option<String>,
    ) -> Result<(), String> {
        let cover_url = track_path.as_deref().and_then(find_cover_url);
        let mut controls = self.0.lock().map_err(|e| e.to_string())?;
        controls
            .set_metadata(MediaMetadata {
                title: title.as_deref(),
                artist: artist.as_deref(),
                album: album.as_deref(),
                cover_url: cover_url.as_deref(),
                duration: duration.map(Duration::from_secs_f64),
            })
            .map_err(|e| format!("set_metadata failed: {e:?}"))
    }

    /// Reflect the current play/pause state and position in the OS UI so its
    /// button and timeline scrubber match the app.
    pub fn update_playback(&self, playing: bool, position: f64) -> Result<(), String> {
        let mut controls = self.0.lock().map_err(|e| e.to_string())?;
        let progress = Some(MediaPosition(Duration::from_secs_f64(position.max(0.0))));
        let playback = if playing {
            MediaPlayback::Playing { progress }
        } else {
            MediaPlayback::Paused { progress }
        };
        controls
            .set_playback(playback)
            .map_err(|e| format!("set_playback failed: {e:?}"))
    }

    /// Mark playback as fully stopped in the OS UI.
    pub fn stop(&self) -> Result<(), String> {
        let mut controls = self.0.lock().map_err(|e| e.to_string())?;
        controls
            .set_playback(MediaPlayback::Stopped)
            .map_err(|e| format!("set_playback failed: {e:?}"))
    }
}

/// Create the OS media controls and wire remote commands to the frontend.
///
/// Must be called on the main thread: souvlaki's macOS backend hooks into the
/// app's run loop (which Tauri runs on the main thread). Returns an error rather
/// than panicking so a missing backend (e.g. no DBus on a headless Linux box)
/// degrades gracefully instead of taking the app down.
pub fn init(app: &AppHandle) -> Result<MediaController, String> {
    // SMTC needs the window handle on Windows; macOS and Linux ignore it.
    #[cfg(target_os = "windows")]
    let hwnd = {
        use tauri::Manager;
        let window = app
            .get_webview_window("main")
            .ok_or("no main window for media controls")?;
        let handle = window.hwnd().map_err(|e| e.to_string())?;
        Some(handle.0 as *mut std::ffi::c_void)
    };
    #[cfg(not(target_os = "windows"))]
    let hwnd = None;

    let config = PlatformConfig {
        dbus_name: "jostmusicplayer",
        display_name: "Jost Music Player",
        hwnd,
    };

    let mut controls =
        MediaControls::new(config).map_err(|e| format!("media controls init failed: {e:?}"))?;

    let handle = app.clone();
    controls
        .attach(move |event| handle_event(&handle, event))
        .map_err(|e| format!("media controls attach failed: {e:?}"))?;

    Ok(MediaController(Mutex::new(controls)))
}

/// Translate an OS media command into a Tauri event for the frontend to act on.
/// Most map to a `media-control` string action; an absolute seek carries its
/// target seconds on `media-control-seek`.
fn handle_event(app: &AppHandle, event: MediaControlEvent) {
    let action = match event {
        MediaControlEvent::Play => "play",
        MediaControlEvent::Pause => "pause",
        MediaControlEvent::Toggle => "toggle",
        MediaControlEvent::Next => "next",
        MediaControlEvent::Previous => "previous",
        MediaControlEvent::Stop => "stop",
        MediaControlEvent::Seek(SeekDirection::Forward)
        | MediaControlEvent::SeekBy(SeekDirection::Forward, _) => "seek_forward",
        MediaControlEvent::Seek(SeekDirection::Backward)
        | MediaControlEvent::SeekBy(SeekDirection::Backward, _) => "seek_backward",
        MediaControlEvent::SetPosition(MediaPosition(pos)) => {
            let _ = app.emit("media-control-seek", pos.as_secs_f64());
            return;
        }
        // OpenUri / Raise / Quit and any future variants aren't relevant here.
        _ => return,
    };
    let _ = app.emit("media-control", action);
}
