use lofty::config::WriteOptions;
use lofty::file::{AudioFile, TaggedFileExt};
use lofty::probe::Probe;
use lofty::tag::{Accessor, Tag, TagExt};
use rusqlite::Connection;
use std::path::Path;
use std::sync::Mutex;
use walkdir::WalkDir;

const AUDIO_EXTENSIONS: &[&str] = &["mp3", "opus", "m4a", "aac"];

pub fn is_audio_file(path: &Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .map(|extension| {
            let lower = extension.to_lowercase();
            AUDIO_EXTENSIONS.contains(&lower.as_str())
        })
        .unwrap_or(false)
}

struct TagData {
    title: Option<String>,
    artist: Option<String>,
    album: Option<String>,
    year: Option<i32>,
    track_num: Option<u32>,
    duration: Option<u32>,
    genre: Option<String>,
}

const EMPTY_TAGS: TagData = TagData {
    title: None,
    artist: None,
    album: None,
    year: None,
    track_num: None,
    duration: None,
    genre: None,
};

/// Read metadata from any format lofty understands (MP3 ID3, Opus/Ogg Vorbis
/// comments, etc.). Missing tags or unreadable files yield `None` fields rather
/// than failing the scan.
fn read_tags(path: &Path) -> TagData {
    let tagged_file = match lofty::read_from_path(path) {
        Ok(f) => f,
        Err(e) => {
            log::warn!("scanner: failed to read tags for {}: {e}", path.display());
            return EMPTY_TAGS;
        }
    };

    // Duration comes from the decoded audio properties, independent of any tag.
    let secs = tagged_file.properties().duration().as_secs();
    let duration = (secs > 0).then_some(secs as u32);

    // Prefer the file's primary tag, falling back to whatever tag exists.
    match tagged_file
        .primary_tag()
        .or_else(|| tagged_file.first_tag())
    {
        Some(tag) => TagData {
            title: tag.title().map(|c| c.into_owned()),
            artist: tag.artist().map(|c| c.into_owned()),
            album: tag.album().map(|c| c.into_owned()),
            year: tag.year().map(|y| y as i32),
            track_num: tag.track(),
            duration,
            genre: tag.genre().map(|c| c.into_owned()),
        },
        None => TagData {
            duration,
            ..EMPTY_TAGS
        },
    }
}

/// The user-editable subset of a track's tags. `None` means "clear this field";
/// `Some` means "set it to this value". Numeric fields use the same widths as
/// `TagData`/`TrackRow`; they're widened to `u32` for lofty's setters.
pub struct TagEdit {
    pub title: Option<String>,
    pub artist: Option<String>,
    pub album: Option<String>,
    pub year: Option<i32>,
    pub track_num: Option<u32>,
    pub genre: Option<String>,
}

/// A valid tag year must be exactly 4 digits. lofty happily *writes* a shorter
/// year (e.g. 0 or 20) but then can no longer *read* the file — it fails with
/// "invalid year length (should be 4 digits)", corrupting the whole tag. We only
/// ever write years in this range; anything else is treated as "no year".
const YEAR_RANGE: std::ops::RangeInclusive<i32> = 1000..=9999;

/// Apply `edit` to a tag: `Some(value)` sets a field, `None` removes it. Year and
/// track values outside their valid range (including 0) are removed rather than
/// written, so we never persist a value that would make the file unreadable.
fn apply_edit(tag: &mut Tag, edit: &TagEdit) {
    match &edit.title {
        Some(v) => tag.set_title(v.clone()),
        None => tag.remove_title(),
    }
    match &edit.artist {
        Some(v) => tag.set_artist(v.clone()),
        None => tag.remove_artist(),
    }
    match &edit.album {
        Some(v) => tag.set_album(v.clone()),
        None => tag.remove_album(),
    }
    match &edit.genre {
        Some(v) => tag.set_genre(v.clone()),
        None => tag.remove_genre(),
    }
    match edit.year {
        Some(y) if YEAR_RANGE.contains(&y) => tag.set_year(y as u32),
        _ => tag.remove_year(),
    }
    match edit.track_num {
        Some(n) if n > 0 => tag.set_track(n),
        _ => tag.remove_track(),
    }
}

/// Write `edit` into `path`'s tags and save to disk. Reuses the file's primary
/// tag when it has one, creating a tag of the file's native type (e.g. ID3v2 for
/// MP3, Vorbis comments for Opus) otherwise. Returns the error string on failure
/// so the Tauri command can surface it to the frontend.
///
/// If the file's existing tags can't be parsed — e.g. it was corrupted by a
/// previously-written invalid year — we fall back to writing a fresh tag over it,
/// so the edit dialog can repair the file rather than being permanently stuck.
pub fn write_tags(path: &Path, edit: &TagEdit) -> Result<(), String> {
    // Reject an out-of-range year with a clear message instead of silently
    // dropping it, so a mistyped year (e.g. "20") is corrected rather than lost.
    if let Some(y) = edit.year {
        if y != 0 && !YEAR_RANGE.contains(&y) {
            return Err("Year must be a 4-digit number (1000–9999).".to_string());
        }
    }

    match lofty::read_from_path(path) {
        Ok(mut tagged_file) => {
            let tag = match tagged_file.primary_tag_mut() {
                Some(_) => tagged_file.primary_tag_mut().unwrap(),
                None => {
                    let tag_type = tagged_file.primary_tag_type();
                    tagged_file.insert_tag(Tag::new(tag_type));
                    tagged_file.primary_tag_mut().unwrap()
                }
            };
            apply_edit(tag, edit);
            tagged_file
                .save_to_path(path, WriteOptions::default())
                .map_err(|e| e.to_string())
        }
        Err(_) => {
            // Determine the format without parsing (possibly corrupt) tags, then
            // overwrite with a fresh tag of the file's native type.
            let file_type = Probe::open(path)
                .map_err(|e| e.to_string())?
                .guess_file_type()
                .map_err(|e| e.to_string())?
                .file_type()
                .ok_or_else(|| "unrecognized audio format".to_string())?;
            let mut tag = Tag::new(file_type.primary_tag_type());
            apply_edit(&mut tag, edit);
            tag.save_to_path(path, WriteOptions::default())
                .map_err(|e| e.to_string())
        }
    }
}

/// Re-read `path`'s tags from disk and upsert them into the DB, keeping the
/// cache consistent with the file after an edit without a full rescan. The file
/// is the source of truth: we index whatever `write_tags` actually persisted.
pub fn reindex_path(db: &Mutex<Connection>, path: &Path) -> Result<(), String> {
    let filename = path
        .file_name()
        .and_then(|n| n.to_str())
        .ok_or_else(|| format!("invalid filename: {}", path.display()))?
        .to_owned();
    let tags = read_tags(path);
    let row = crate::db::TrackRow {
        path: path.to_string_lossy().into_owned(),
        filename,
        title: tags.title,
        artist: tags.artist,
        album: tags.album,
        year: tags.year,
        track_num: tags.track_num,
        duration: tags.duration,
        genre: tags.genre,
    };
    let conn = db.lock().map_err(|e| e.to_string())?;
    crate::db::upsert_track(&conn, &row).map_err(|e| e.to_string())
}

/// How many inserts to make between `on_progress` callbacks. The frontend
/// re-fetches the whole library on each one, so we batch to avoid emitting an
/// event per file while still surfacing new tracks while the scan runs.
const PROGRESS_BATCH: usize = 25;

/// Sync the database to the contents of `folder`: insert newly-seen audio files
/// (reading their tags) and remove rows whose files are gone.
///
/// Takes the connection's `Mutex` rather than a locked `&Connection` and locks
/// only for each individual DB operation. The slow work — walking the tree and
/// reading tags — happens without the lock held, so callers like `get_library`
/// can read the existing library while a scan is in progress.
///
/// `on_progress` is invoked every `PROGRESS_BATCH` inserts so callers can refresh
/// the UI progressively rather than waiting for the whole scan to finish.
///
/// `should_cancel` is polled between files; when it returns true the scan stops
/// early, leaving whatever has been indexed so far in place.
pub fn scan_and_sync(
    db: &Mutex<Connection>,
    folder: &str,
    on_progress: impl Fn(),
    should_cancel: impl Fn() -> bool,
) {
    let mut disk_paths = std::collections::HashSet::new();

    for entry in WalkDir::new(folder)
        .follow_links(true)
        .into_iter()
        .filter_map(|e| e.ok())
    {
        let path = entry.path();
        if path.is_file() && is_audio_file(path) {
            if let Some(s) = path.to_str() {
                disk_paths.insert(s.to_owned());
            }
        }
    }

    let db_paths = {
        let Ok(conn) = db.lock() else { return };
        crate::db::get_all_paths(&conn).unwrap_or_default()
    };

    let mut inserted = 0usize;
    for path_str in disk_paths.difference(&db_paths) {
        if should_cancel() {
            return;
        }
        let path = Path::new(path_str);
        let Some(filename) = path.file_name().and_then(|n| n.to_str()) else {
            log::warn!("scanner: skipping path with invalid filename: {path_str}");
            continue;
        };
        let filename = filename.to_owned();
        // Read tags before taking the lock — this is the expensive step.
        let tags = read_tags(path);
        let row = crate::db::TrackRow {
            path: path_str.clone(),
            filename,
            title: tags.title,
            artist: tags.artist,
            album: tags.album,
            year: tags.year,
            track_num: tags.track_num,
            duration: tags.duration,
            genre: tags.genre,
        };
        {
            let Ok(conn) = db.lock() else { return };
            let _ = crate::db::upsert_track(&conn, &row);
        }
        inserted += 1;
        if inserted.is_multiple_of(PROGRESS_BATCH) {
            on_progress();
        }
    }

    let to_delete: Vec<String> = db_paths.difference(&disk_paths).cloned().collect();
    if !to_delete.is_empty() {
        let Ok(conn) = db.lock() else { return };
        let _ = crate::db::delete_paths(&conn, &to_delete);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::Connection;
    use std::fs;
    use tempfile::TempDir;

    fn open_db() -> Mutex<Connection> {
        let conn = Connection::open_in_memory().unwrap();
        crate::db::init_schema(&conn).unwrap();
        Mutex::new(conn)
    }

    fn touch_mp3(dir: &TempDir, name: &str) -> std::path::PathBuf {
        let path = dir.path().join(name);
        fs::write(&path, b"").unwrap();
        path
    }

    /// Copy one of the real fixtures (a 1-second silent file tagged
    /// title="My Song", artist="My Artist") into `dir` under `name`.
    /// `name`'s extension selects the fixture format (mp3 or opus).
    fn copy_tagged_fixture(dir: &TempDir, name: &str) -> std::path::PathBuf {
        let ext = Path::new(name)
            .extension()
            .and_then(|e| e.to_str())
            .unwrap();
        let fixture = format!("{}/tests/fixtures/tagged.{ext}", env!("CARGO_MANIFEST_DIR"));
        let path = dir.path().join(name);
        fs::copy(&fixture, &path).unwrap();
        path
    }

    #[test]
    fn is_audio_file_mp3() {
        assert!(is_audio_file(Path::new("song.mp3")));
    }

    #[test]
    fn is_audio_file_uppercase_extension() {
        assert!(is_audio_file(Path::new("song.MP3")));
    }

    #[test]
    fn is_audio_file_non_audio() {
        assert!(!is_audio_file(Path::new("notes.txt")));
        assert!(!is_audio_file(Path::new("image.png")));
        assert!(!is_audio_file(Path::new("noextension")));
    }

    #[test]
    fn is_audio_file_opus() {
        assert!(is_audio_file(Path::new("song.opus")));
        assert!(is_audio_file(Path::new("song.OPUS")));
    }

    #[test]
    fn is_audio_file_aac_m4a() {
        assert!(is_audio_file(Path::new("song.m4a")));
        assert!(is_audio_file(Path::new("song.M4A")));
        assert!(is_audio_file(Path::new("song.aac")));
    }

    #[test]
    fn read_tags_with_tags() {
        let dir = TempDir::new().unwrap();
        copy_tagged_fixture(&dir, "tagged.mp3");
        let tags = read_tags(&dir.path().join("tagged.mp3"));
        assert_eq!(tags.title.as_deref(), Some("My Song"));
        assert_eq!(tags.artist.as_deref(), Some("My Artist"));
        // Duration comes from decoded audio properties, not a tag frame.
        assert_eq!(tags.duration, Some(1));
    }

    #[test]
    fn write_tags_rejects_non_four_digit_year() {
        let dir = TempDir::new().unwrap();
        let path = copy_tagged_fixture(&dir, "tagged.mp3");
        let edit = TagEdit {
            title: Some("Keep".into()),
            artist: None,
            album: None,
            year: Some(20), // not 4 digits — would corrupt the file if written
            track_num: None,
            genre: None,
        };
        let err = write_tags(&path, &edit).unwrap_err();
        assert!(err.contains("4-digit"), "unexpected error: {err}");
        // The file must be untouched and still readable (fixture title intact).
        assert_eq!(read_tags(&path).title.as_deref(), Some("My Song"));
    }

    #[test]
    fn write_tags_recovers_from_corrupt_file() {
        let dir = TempDir::new().unwrap();
        let path = copy_tagged_fixture(&dir, "tagged.mp3");

        // Corrupt the file the way the old invalid-year write did: a 2-digit year
        // that lofty writes but can no longer read back.
        {
            let mut tf = lofty::read_from_path(&path).unwrap();
            tf.primary_tag_mut().unwrap().set_year(20);
            tf.save_to_path(&path, WriteOptions::default()).unwrap();
        }
        assert!(
            lofty::read_from_path(&path).is_err(),
            "expected the file to be unreadable after corruption"
        );

        // The edit dialog should still be able to repair it by writing fresh tags.
        write_tags(
            &path,
            &TagEdit {
                title: Some("Repaired".into()),
                artist: Some("Artist".into()),
                album: None,
                year: Some(2000),
                track_num: None,
                genre: None,
            },
        )
        .unwrap();

        let tags = read_tags(&path);
        assert_eq!(tags.title.as_deref(), Some("Repaired"));
        assert_eq!(tags.artist.as_deref(), Some("Artist"));
        assert_eq!(tags.year, Some(2000));
    }

    #[test]
    fn write_tags_year_zero_clears_year_without_wiping_tag() {
        // Regression: setting year to 0 must not destroy the other frames. lofty
        // encodes year 0 as an invalid ID3v2 timestamp that corrupts the whole
        // tag, so `write_tags` treats 0 (and negatives) as "remove the year".
        let dir = TempDir::new().unwrap();
        let path = copy_tagged_fixture(&dir, "tagged.mp3");
        let edit = TagEdit {
            title: Some("My Song".into()),
            artist: Some("My Artist".into()),
            album: Some("My Album".into()),
            year: Some(0),
            track_num: Some(0),
            genre: Some("Rock".into()),
        };
        write_tags(&path, &edit).unwrap();

        let tags = read_tags(&path);
        assert_eq!(tags.title.as_deref(), Some("My Song"));
        assert_eq!(tags.artist.as_deref(), Some("My Artist"));
        assert_eq!(tags.album.as_deref(), Some("My Album"));
        assert_eq!(tags.genre.as_deref(), Some("Rock"));
        // 0 is treated as "unset", so these round-trip as absent.
        assert_eq!(tags.year, None);
        assert_eq!(tags.track_num, None);
    }

    #[test]
    fn read_tags_from_opus() {
        let dir = TempDir::new().unwrap();
        copy_tagged_fixture(&dir, "tagged.opus");
        let tags = read_tags(&dir.path().join("tagged.opus"));
        assert_eq!(tags.title.as_deref(), Some("My Song"));
        assert_eq!(tags.artist.as_deref(), Some("My Artist"));
    }

    #[test]
    fn read_tags_from_m4a() {
        let dir = TempDir::new().unwrap();
        copy_tagged_fixture(&dir, "tagged.m4a");
        let tags = read_tags(&dir.path().join("tagged.m4a"));
        assert_eq!(tags.title.as_deref(), Some("My Song"));
        assert_eq!(tags.artist.as_deref(), Some("My Artist"));
        assert_eq!(tags.duration, Some(1));
    }

    #[test]
    fn read_tags_without_tags() {
        let dir = TempDir::new().unwrap();
        touch_mp3(&dir, "bare.mp3");
        let tags = read_tags(&dir.path().join("bare.mp3"));
        assert!(tags.title.is_none());
        assert!(tags.artist.is_none());
    }

    #[test]
    fn scan_and_sync_inserts_new_files() {
        let dir = TempDir::new().unwrap();
        touch_mp3(&dir, "a.mp3");
        touch_mp3(&dir, "b.mp3");

        let db = open_db();
        scan_and_sync(&db, dir.path().to_str().unwrap(), || {}, || false);

        let paths = crate::db::get_all_paths(&db.lock().unwrap()).unwrap();
        assert_eq!(paths.len(), 2);
    }

    #[test]
    fn scan_and_sync_stops_when_cancelled() {
        let dir = TempDir::new().unwrap();
        touch_mp3(&dir, "a.mp3");
        touch_mp3(&dir, "b.mp3");

        let db = open_db();
        // Cancelling before the first insert leaves the library untouched.
        scan_and_sync(&db, dir.path().to_str().unwrap(), || {}, || true);

        let paths = crate::db::get_all_paths(&db.lock().unwrap()).unwrap();
        assert!(paths.is_empty());
    }

    #[test]
    fn scan_and_sync_ignores_non_audio() {
        let dir = TempDir::new().unwrap();
        touch_mp3(&dir, "song.mp3");
        fs::write(dir.path().join("readme.txt"), b"hello").unwrap();

        let db = open_db();
        scan_and_sync(&db, dir.path().to_str().unwrap(), || {}, || false);

        let paths = crate::db::get_all_paths(&db.lock().unwrap()).unwrap();
        assert_eq!(paths.len(), 1);
    }

    #[test]
    fn scan_and_sync_removes_deleted_files() {
        let dir = TempDir::new().unwrap();
        let path = touch_mp3(&dir, "gone.mp3");

        let db = open_db();
        scan_and_sync(&db, dir.path().to_str().unwrap(), || {}, || false);
        assert_eq!(
            crate::db::get_all_paths(&db.lock().unwrap()).unwrap().len(),
            1
        );

        fs::remove_file(&path).unwrap();
        scan_and_sync(&db, dir.path().to_str().unwrap(), || {}, || false);
        assert!(crate::db::get_all_paths(&db.lock().unwrap())
            .unwrap()
            .is_empty());
    }

    #[test]
    fn scan_and_sync_scans_subfolders() {
        let dir = TempDir::new().unwrap();
        let sub = dir.path().join("artist").join("album");
        fs::create_dir_all(&sub).unwrap();
        fs::write(sub.join("track.mp3"), b"").unwrap();

        let db = open_db();
        scan_and_sync(&db, dir.path().to_str().unwrap(), || {}, || false);

        assert_eq!(
            crate::db::get_all_paths(&db.lock().unwrap()).unwrap().len(),
            1
        );
    }

    #[test]
    fn scan_and_sync_reads_tags_on_insert() {
        let dir = TempDir::new().unwrap();
        copy_tagged_fixture(&dir, "tagged.mp3");

        let db = open_db();
        scan_and_sync(&db, dir.path().to_str().unwrap(), || {}, || false);

        let tracks = crate::db::get_tracks(&db.lock().unwrap(), "artist", "asc").unwrap();
        assert_eq!(tracks.len(), 1);
        assert_eq!(tracks[0].title.as_deref(), Some("My Song"));
        assert_eq!(tracks[0].artist.as_deref(), Some("My Artist"));
    }

    fn full_edit() -> TagEdit {
        TagEdit {
            title: Some("New Title".to_owned()),
            artist: Some("New Artist".to_owned()),
            album: Some("New Album".to_owned()),
            year: Some(1999),
            track_num: Some(7),
            genre: Some("Jazz".to_owned()),
        }
    }

    #[test]
    fn write_tags_updates_fields() {
        let dir = TempDir::new().unwrap();
        let path = copy_tagged_fixture(&dir, "tagged.mp3");

        write_tags(&path, &full_edit()).unwrap();

        let tags = read_tags(&path);
        assert_eq!(tags.title.as_deref(), Some("New Title"));
        assert_eq!(tags.artist.as_deref(), Some("New Artist"));
        assert_eq!(tags.album.as_deref(), Some("New Album"));
        assert_eq!(tags.year, Some(1999));
        assert_eq!(tags.track_num, Some(7));
        assert_eq!(tags.genre.as_deref(), Some("Jazz"));
    }

    #[test]
    fn write_tags_clears_fields_when_none() {
        let dir = TempDir::new().unwrap();
        let path = copy_tagged_fixture(&dir, "tagged.mp3");

        let cleared = TagEdit {
            title: None,
            artist: None,
            album: None,
            year: None,
            track_num: None,
            genre: None,
        };
        write_tags(&path, &cleared).unwrap();

        let tags = read_tags(&path);
        assert!(tags.title.is_none());
        assert!(tags.artist.is_none());
    }

    #[test]
    fn write_tags_on_opus() {
        let dir = TempDir::new().unwrap();
        let path = copy_tagged_fixture(&dir, "tagged.opus");

        write_tags(&path, &full_edit()).unwrap();

        let tags = read_tags(&path);
        assert_eq!(tags.title.as_deref(), Some("New Title"));
        assert_eq!(tags.artist.as_deref(), Some("New Artist"));
    }

    #[test]
    fn reindex_path_syncs_db_after_edit() {
        let dir = TempDir::new().unwrap();
        let path = copy_tagged_fixture(&dir, "tagged.mp3");

        let db = open_db();
        scan_and_sync(&db, dir.path().to_str().unwrap(), || {}, || false);

        write_tags(&path, &full_edit()).unwrap();
        reindex_path(&db, &path).unwrap();

        let tracks = crate::db::get_tracks(&db.lock().unwrap(), "artist", "asc").unwrap();
        assert_eq!(tracks.len(), 1);
        assert_eq!(tracks[0].title.as_deref(), Some("New Title"));
        assert_eq!(tracks[0].artist.as_deref(), Some("New Artist"));
        assert_eq!(tracks[0].year, Some(1999));
    }
}
