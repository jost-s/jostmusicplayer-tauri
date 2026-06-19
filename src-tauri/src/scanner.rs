use lofty::file::{AudioFile, TaggedFileExt};
use lofty::tag::Accessor;
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
}

const EMPTY_TAGS: TagData = TagData {
    title: None,
    artist: None,
    album: None,
    year: None,
    track_num: None,
    duration: None,
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
        },
        None => TagData {
            duration,
            ..EMPTY_TAGS
        },
    }
}

/// Sync the database to the contents of `folder`: insert newly-seen audio files
/// (reading their tags) and remove rows whose files are gone.
///
/// Takes the connection's `Mutex` rather than a locked `&Connection` and locks
/// only for each individual DB operation. The slow work — walking the tree and
/// reading tags — happens without the lock held, so callers like `get_library`
/// can read the existing library while a scan is in progress.
pub fn scan_and_sync(db: &Mutex<Connection>, folder: &str) {
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

    for path_str in disk_paths.difference(&db_paths) {
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
        };
        let Ok(conn) = db.lock() else { return };
        let _ = crate::db::upsert_track(&conn, &row);
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
        scan_and_sync(&db, dir.path().to_str().unwrap());

        let paths = crate::db::get_all_paths(&db.lock().unwrap()).unwrap();
        assert_eq!(paths.len(), 2);
    }

    #[test]
    fn scan_and_sync_ignores_non_audio() {
        let dir = TempDir::new().unwrap();
        touch_mp3(&dir, "song.mp3");
        fs::write(dir.path().join("readme.txt"), b"hello").unwrap();

        let db = open_db();
        scan_and_sync(&db, dir.path().to_str().unwrap());

        let paths = crate::db::get_all_paths(&db.lock().unwrap()).unwrap();
        assert_eq!(paths.len(), 1);
    }

    #[test]
    fn scan_and_sync_removes_deleted_files() {
        let dir = TempDir::new().unwrap();
        let path = touch_mp3(&dir, "gone.mp3");

        let db = open_db();
        scan_and_sync(&db, dir.path().to_str().unwrap());
        assert_eq!(
            crate::db::get_all_paths(&db.lock().unwrap()).unwrap().len(),
            1
        );

        fs::remove_file(&path).unwrap();
        scan_and_sync(&db, dir.path().to_str().unwrap());
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
        scan_and_sync(&db, dir.path().to_str().unwrap());

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
        scan_and_sync(&db, dir.path().to_str().unwrap());

        let tracks = crate::db::get_tracks(&db.lock().unwrap(), "artist", "asc").unwrap();
        assert_eq!(tracks.len(), 1);
        assert_eq!(tracks[0].title.as_deref(), Some("My Song"));
        assert_eq!(tracks[0].artist.as_deref(), Some("My Artist"));
    }
}
