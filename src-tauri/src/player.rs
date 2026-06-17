use std::fs::File;
use std::io::BufReader;
use std::panic::{self, AssertUnwindSafe};
use std::sync::mpsc::{self, Sender};
use std::thread;
use std::time::Duration;

use rodio::{Decoder, OutputStream, OutputStreamHandle, Sink, Source};
use tauri::{AppHandle, Emitter};

/// How often the audio thread wakes up (when idle on commands) to check whether
/// the current track has finished playing on its own.
const POLL_INTERVAL: Duration = Duration::from_millis(200);

enum AudioCommand {
    Play {
        path: String,
        /// Replies with the track's total duration in seconds, when the decoder
        /// can determine it.
        resp: Sender<Result<Option<f64>, String>>,
    },
    Toggle {
        /// Replies with `true` if playback is now running, `false` if paused or idle.
        resp: Sender<bool>,
    },
    Position {
        /// Replies with the current playback position in seconds (0.0 if idle).
        resp: Sender<f64>,
    },
    Seek {
        secs: f64,
        resp: Sender<Result<(), String>>,
    },
}

/// Handle to the dedicated audio thread.
///
/// rodio's `OutputStream` wraps a `cpal::Stream` that is `!Send`, so it cannot
/// live in Tauri's `State`. Instead the stream is owned by a background thread
/// and driven through this channel; only the `Sender` (which is `Send + Sync`)
/// is stored in app state.
pub struct AudioPlayer {
    tx: Sender<AudioCommand>,
}

impl AudioPlayer {
    pub fn new(app: AppHandle) -> Self {
        let (tx, rx) = mpsc::channel::<AudioCommand>();

        thread::spawn(move || {
            // Kept alive for the lifetime of the thread; dropping it stops audio.
            let (_stream, handle) = match OutputStream::try_default() {
                Ok(pair) => pair,
                Err(e) => {
                    eprintln!("audio: failed to open output stream: {e}");
                    return;
                }
            };

            let mut sink: Option<Sink> = None;
            // True while a track is playing/paused and hasn't ended or been replaced.
            // Distinguishes a natural finish from a manual stop so we only emit
            // `playback-ended` for the former.
            let mut active = false;

            loop {
                match rx.recv_timeout(POLL_INTERVAL) {
                    Ok(cmd) => {
                        // Isolate command handling: a panic (e.g. a malformed file
                        // tripping a decoder) drops the current track but keeps the
                        // thread alive, so playback recovers on the next command
                        // instead of every later command failing with a closed channel.
                        let outcome = panic::catch_unwind(AssertUnwindSafe(|| {
                            handle_command(cmd, &handle, &mut sink)
                        }));
                        match outcome {
                            Ok(Some(new_active)) => active = new_active,
                            Ok(None) => {}
                            Err(_) => {
                                eprintln!("audio: command handler panicked; dropping track");
                                sink = None;
                                active = false;
                            }
                        }
                    }
                    Err(mpsc::RecvTimeoutError::Timeout) => {
                        if active && sink.as_ref().is_some_and(|s| s.empty()) {
                            let _ = app.emit("playback-ended", ());
                            active = false;
                            sink = None;
                        }
                    }
                    Err(mpsc::RecvTimeoutError::Disconnected) => break,
                }
            }
        });

        Self { tx }
    }

    pub fn play(&self, path: String) -> Result<Option<f64>, String> {
        let (resp, rx) = mpsc::channel();
        self.tx
            .send(AudioCommand::Play { path, resp })
            .map_err(|err| format!("audio thread is not running: {err}"))?;
        rx.recv()
            .map_err(|err| format!("audio thread did not respond: {err}"))?
    }

    pub fn toggle(&self) -> bool {
        let (resp, rx) = mpsc::channel();
        if self.tx.send(AudioCommand::Toggle { resp }).is_err() {
            return false;
        }
        rx.recv().unwrap_or(false)
    }

    pub fn position(&self) -> f64 {
        let (resp, rx) = mpsc::channel();
        if self.tx.send(AudioCommand::Position { resp }).is_err() {
            return 0.0;
        }
        rx.recv().unwrap_or(0.0)
    }

    pub fn seek(&self, secs: f64) -> Result<(), String> {
        let (resp, rx) = mpsc::channel();
        self.tx
            .send(AudioCommand::Seek { secs, resp })
            .map_err(|err| format!("audio thread is not running: {err}"))?;
        rx.recv()
            .map_err(|err| format!("audio thread did not respond: {err}"))?
    }
}

/// Process one command, replying on its response channel. Returns `Some(active)`
/// when the command changes whether a track is loaded (i.e. `Play`), else `None`.
/// Runs inside `catch_unwind` on the audio thread, so it must not hold state that
/// a panic would corrupt beyond `sink` (which the caller resets on panic).
fn handle_command(
    cmd: AudioCommand,
    handle: &OutputStreamHandle,
    sink: &mut Option<Sink>,
) -> Option<bool> {
    match cmd {
        AudioCommand::Play { path, resp } => {
            let _ = resp.send(start_playback(handle, &path, sink));
            Some(sink.is_some())
        }
        AudioCommand::Toggle { resp } => {
            let playing = match sink.as_ref() {
                Some(s) => {
                    if s.is_paused() {
                        s.play();
                        true
                    } else {
                        s.pause();
                        false
                    }
                }
                None => false,
            };
            let _ = resp.send(playing);
            None
        }
        AudioCommand::Position { resp } => {
            let secs = sink.as_ref().map_or(0.0, |s| s.get_pos().as_secs_f64());
            let _ = resp.send(secs);
            None
        }
        AudioCommand::Seek { secs, resp } => {
            let result = match sink.as_ref() {
                Some(s) => s
                    .try_seek(Duration::from_secs_f64(secs.max(0.0)))
                    .map_err(|e| format!("failed to seek: {e}")),
                None => Err("nothing is playing".to_string()),
            };
            let _ = resp.send(result);
            None
        }
    }
}

/// Decode `path` and start playing it on a fresh sink, replacing any current one.
/// Returns the track's total duration in seconds when it can be determined.
fn start_playback(
    handle: &rodio::OutputStreamHandle,
    path: &str,
    sink: &mut Option<Sink>,
) -> Result<Option<f64>, String> {
    let new_sink = Sink::try_new(handle).map_err(|e| format!("failed to create sink: {e}"))?;

    // rodio's symphonia decoders don't cover Opus, so route .opus through our
    // own libopus-backed source; everything else goes through rodio's decoder.
    let total = if has_extension(path, "opus") {
        // The Opus source can't report its own length, so probe it with lofty.
        let total = probe_duration(path);
        let source = crate::opus_source::OpusSource::new(path)?.with_total_duration(total);
        new_sink.append(source);
        total
    } else if has_extension(path, "m4a") || has_extension(path, "aac") {
        // rodio's Decoder panics on these (its ReadSeekSource reports no length,
        // which trips symphonia's MP4 demuxer during init), so use our own
        // symphonia-backed source instead.
        let source = crate::symphonia_source::SymphoniaSource::new(path)?;
        let total = source
            .total_duration()
            .map(|d| d.as_secs_f64())
            .filter(|&s| s > 0.0)
            .or_else(|| probe_duration(path));
        new_sink.append(source);
        total
    } else {
        let file = File::open(path).map_err(|e| format!("failed to open file: {e}"))?;
        let source =
            Decoder::new(BufReader::new(file)).map_err(|e| format!("failed to decode: {e}"))?;
        // rodio reports `None` for files it can't size up front (e.g. VBR MP3 without a
        // Xing header); fall back to probing the file's properties with lofty.
        let total = source
            .total_duration()
            .map(|d| d.as_secs_f64())
            .filter(|&s| s > 0.0)
            .or_else(|| probe_duration(path));
        new_sink.append(source);
        total
    };

    *sink = Some(new_sink); // dropping the old sink stops the previous track
    Ok(total)
}

/// Case-insensitive check of a path's file extension.
fn has_extension(path: &str, ext: &str) -> bool {
    std::path::Path::new(path)
        .extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| e.eq_ignore_ascii_case(ext))
}

/// Determine a track's duration by reading its audio properties. Works for VBR
/// MP3s that rodio's decoder can't measure up front.
fn probe_duration(path: &str) -> Option<f64> {
    use lofty::file::AudioFile;
    let secs = lofty::read_from_path(path)
        .ok()?
        .properties()
        .duration()
        .as_secs_f64();
    (secs > 0.0).then_some(secs)
}
