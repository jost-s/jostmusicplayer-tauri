use id3::{Tag, TagLike};
use rusqlite::Connection;
use std::path::Path;
use walkdir::WalkDir;

const AUDIO_EXTENSIONS: &[&str] = &["mp3"];

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

fn read_mp3_tags(path: &Path) -> TagData {
    match Tag::read_from_path(path) {
        Ok(tag) => TagData {
            title: tag.title().map(str::to_owned),
            artist: tag.artist().map(str::to_owned),
            album: tag.album().map(str::to_owned),
            year: tag.year(),
            track_num: tag.track(),
            // TLEN frame stores duration in milliseconds.
            duration: tag.duration().map(|ms| ms / 1000),
        },
        Err(e) => {
            eprintln!("scanner: failed to read tags for {}: {e}", path.display());
            TagData {
                title: None,
                artist: None,
                album: None,
                year: None,
                track_num: None,
                duration: None,
            }
        }
    }
}

pub fn scan_and_sync(conn: &Connection, folder: &str) {
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

    let db_paths = crate::db::get_all_paths(conn).unwrap_or_default();

    for path_str in disk_paths.difference(&db_paths) {
        let path = Path::new(path_str);
        let Some(filename) = path.file_name().and_then(|n| n.to_str()) else {
            eprintln!("scanner: skipping path with invalid filename: {path_str}");
            continue;
        };
        let filename = filename.to_owned();
        let tags = read_mp3_tags(path);
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
        let _ = crate::db::upsert_track(conn, &row);
    }

    let to_delete: Vec<String> = db_paths.difference(&disk_paths).cloned().collect();
    if !to_delete.is_empty() {
        let _ = crate::db::delete_paths(conn, &to_delete);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use id3::{Tag, Version};
    use rusqlite::Connection;
    use std::fs;
    use tempfile::TempDir;

    fn open_db() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        crate::db::init_schema(&conn).unwrap();
        conn
    }

    fn touch_mp3(dir: &TempDir, name: &str) -> std::path::PathBuf {
        let path = dir.path().join(name);
        fs::write(&path, b"").unwrap();
        path
    }

    fn create_mp3_with_tags(
        dir: &TempDir,
        name: &str,
        title: &str,
        artist: &str,
    ) -> std::path::PathBuf {
        let path = dir.path().join(name);
        fs::write(&path, b"").unwrap();
        let mut tag = Tag::new();
        tag.set_title(title);
        tag.set_artist(artist);
        tag.write_to_path(&path, Version::Id3v24).unwrap();
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
    fn read_mp3_tags_with_tags() {
        let dir = TempDir::new().unwrap();
        create_mp3_with_tags(&dir, "tagged.mp3", "My Song", "My Artist");
        let tags = read_mp3_tags(&dir.path().join("tagged.mp3"));
        assert_eq!(tags.title.as_deref(), Some("My Song"));
        assert_eq!(tags.artist.as_deref(), Some("My Artist"));
    }

    #[test]
    fn read_mp3_tags_without_tags() {
        let dir = TempDir::new().unwrap();
        touch_mp3(&dir, "bare.mp3");
        let tags = read_mp3_tags(&dir.path().join("bare.mp3"));
        assert!(tags.title.is_none());
        assert!(tags.artist.is_none());
    }

    #[test]
    fn scan_and_sync_inserts_new_files() {
        let dir = TempDir::new().unwrap();
        touch_mp3(&dir, "a.mp3");
        touch_mp3(&dir, "b.mp3");

        let conn = open_db();
        scan_and_sync(&conn, dir.path().to_str().unwrap());

        let paths = crate::db::get_all_paths(&conn).unwrap();
        assert_eq!(paths.len(), 2);
    }

    #[test]
    fn scan_and_sync_ignores_non_audio() {
        let dir = TempDir::new().unwrap();
        touch_mp3(&dir, "song.mp3");
        fs::write(dir.path().join("readme.txt"), b"hello").unwrap();

        let conn = open_db();
        scan_and_sync(&conn, dir.path().to_str().unwrap());

        let paths = crate::db::get_all_paths(&conn).unwrap();
        assert_eq!(paths.len(), 1);
    }

    #[test]
    fn scan_and_sync_removes_deleted_files() {
        let dir = TempDir::new().unwrap();
        let path = touch_mp3(&dir, "gone.mp3");

        let conn = open_db();
        scan_and_sync(&conn, dir.path().to_str().unwrap());
        assert_eq!(crate::db::get_all_paths(&conn).unwrap().len(), 1);

        fs::remove_file(&path).unwrap();
        scan_and_sync(&conn, dir.path().to_str().unwrap());
        assert!(crate::db::get_all_paths(&conn).unwrap().is_empty());
    }

    #[test]
    fn scan_and_sync_scans_subfolders() {
        let dir = TempDir::new().unwrap();
        let sub = dir.path().join("artist").join("album");
        fs::create_dir_all(&sub).unwrap();
        fs::write(sub.join("track.mp3"), b"").unwrap();

        let conn = open_db();
        scan_and_sync(&conn, dir.path().to_str().unwrap());

        assert_eq!(crate::db::get_all_paths(&conn).unwrap().len(), 1);
    }

    #[test]
    fn scan_and_sync_reads_tags_on_insert() {
        let dir = TempDir::new().unwrap();
        create_mp3_with_tags(&dir, "tagged.mp3", "Great Song", "Great Artist");

        let conn = open_db();
        scan_and_sync(&conn, dir.path().to_str().unwrap());

        let tracks = crate::db::get_tracks(&conn, "artist", "asc").unwrap();
        assert_eq!(tracks.len(), 1);
        assert_eq!(tracks[0].title.as_deref(), Some("Great Song"));
        assert_eq!(tracks[0].artist.as_deref(), Some("Great Artist"));
    }
}
