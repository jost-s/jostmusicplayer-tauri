use rusqlite::{params, Connection, Result};
use std::collections::HashSet;

pub struct TrackRow {
    pub path: String,
    pub filename: String,
    pub title: Option<String>,
    pub artist: Option<String>,
    pub album: Option<String>,
    pub year: Option<i32>,
    pub track_num: Option<u32>,
    pub duration: Option<u32>,
}

pub fn init_schema(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS tracks (
            id        INTEGER PRIMARY KEY,
            path      TEXT UNIQUE NOT NULL,
            filename  TEXT NOT NULL,
            title     TEXT,
            artist    TEXT,
            album     TEXT,
            year      INTEGER,
            track_num INTEGER,
            duration  INTEGER
        );",
    )
}

pub fn get_all_paths(conn: &Connection) -> Result<HashSet<String>> {
    let mut stmt = conn.prepare("SELECT path FROM tracks")?;
    let result = stmt
        .query_map([], |row| row.get(0))?
        .collect::<Result<HashSet<String>>>();
    result
}

pub fn upsert_track(conn: &Connection, track: &TrackRow) -> Result<()> {
    conn.execute(
        "INSERT INTO tracks (path, filename, title, artist, album, year, track_num, duration)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
         ON CONFLICT(path) DO UPDATE SET
             filename  = excluded.filename,
             title     = excluded.title,
             artist    = excluded.artist,
             album     = excluded.album,
             year      = excluded.year,
             track_num = excluded.track_num,
             duration  = excluded.duration",
        params![
            track.path,
            track.filename,
            track.title,
            track.artist,
            track.album,
            track.year,
            track.track_num,
            track.duration,
        ],
    )?;
    Ok(())
}

pub fn delete_paths(conn: &Connection, paths: &[String]) -> Result<()> {
    for path in paths {
        conn.execute("DELETE FROM tracks WHERE path = ?1", [path])?;
    }
    Ok(())
}

pub fn get_tracks(conn: &Connection, sort_by: &str, sort_dir: &str) -> Result<Vec<crate::Track>> {
    let col = match sort_by {
        "title" | "artist" | "album" | "year" | "track_num" | "filename" => sort_by,
        _ => "artist",
    };
    let direction = if sort_dir == "desc" { "DESC" } else { "ASC" };

    let sql = format!(
        "SELECT id, path, filename, title, artist, album, year, track_num, duration
         FROM tracks
         ORDER BY CASE WHEN {col} IS NULL THEN 1 ELSE 0 END, {col} {direction}"
    );

    let mut stmt = conn.prepare(&sql)?;
    let result = stmt
        .query_map([], |row| {
            Ok(crate::Track {
                id: row.get(0)?,
                path: row.get(1)?,
                filename: row.get(2)?,
                title: row.get(3)?,
                artist: row.get(4)?,
                album: row.get(5)?,
                year: row.get(6)?,
                track_num: row.get(7)?,
                duration: row.get(8)?,
            })
        })?
        .collect::<Result<Vec<_>>>();
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    fn open_db() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        init_schema(&conn).unwrap();
        conn
    }

    fn sample_row(path: &str) -> TrackRow {
        TrackRow {
            path: path.to_owned(),
            filename: "track.mp3".to_owned(),
            title: Some("My Title".to_owned()),
            artist: Some("Artist".to_owned()),
            album: Some("Album".to_owned()),
            year: Some(2024),
            track_num: Some(1),
            duration: Some(240),
        }
    }

    #[test]
    fn init_schema_is_idempotent() {
        let conn = Connection::open_in_memory().unwrap();
        init_schema(&conn).unwrap();
        init_schema(&conn).unwrap(); // second call must not fail
    }

    #[test]
    fn get_all_paths_empty_initially() {
        let conn = open_db();
        assert!(get_all_paths(&conn).unwrap().is_empty());
    }

    #[test]
    fn get_all_paths_after_insert() {
        let conn = open_db();
        upsert_track(&conn, &sample_row("/a/b.mp3")).unwrap();
        upsert_track(&conn, &sample_row("/a/c.mp3")).unwrap();
        let paths = get_all_paths(&conn).unwrap();
        assert_eq!(paths.len(), 2);
        assert!(paths.contains("/a/b.mp3"));
        assert!(paths.contains("/a/c.mp3"));
    }

    #[test]
    fn upsert_inserts_new_row() {
        let conn = open_db();
        upsert_track(&conn, &sample_row("/music/a.mp3")).unwrap();
        let tracks = get_tracks(&conn, "artist", "asc").unwrap();
        assert_eq!(tracks.len(), 1);
        assert_eq!(tracks[0].path, "/music/a.mp3");
        assert_eq!(tracks[0].title.as_deref(), Some("My Title"));
    }

    #[test]
    fn upsert_updates_existing_row() {
        let conn = open_db();
        upsert_track(&conn, &sample_row("/music/a.mp3")).unwrap();
        upsert_track(
            &conn,
            &TrackRow {
                path: "/music/a.mp3".to_owned(),
                filename: "a.mp3".to_owned(),
                title: Some("Updated".to_owned()),
                artist: None,
                album: None,
                year: None,
                track_num: None,
                duration: None,
            },
        )
        .unwrap();
        let tracks = get_tracks(&conn, "artist", "asc").unwrap();
        assert_eq!(tracks.len(), 1);
        assert_eq!(tracks[0].title.as_deref(), Some("Updated"));
    }

    #[test]
    fn delete_paths_removes_correct_rows() {
        let conn = open_db();
        upsert_track(&conn, &sample_row("/a.mp3")).unwrap();
        upsert_track(&conn, &sample_row("/b.mp3")).unwrap();
        delete_paths(&conn, &["/a.mp3".to_owned()]).unwrap();
        let paths = get_all_paths(&conn).unwrap();
        assert!(!paths.contains("/a.mp3"));
        assert!(paths.contains("/b.mp3"));
    }

    #[test]
    fn get_tracks_sorts_asc_desc() {
        let conn = open_db();
        upsert_track(
            &conn,
            &TrackRow {
                path: "/z.mp3".to_owned(),
                filename: "z.mp3".to_owned(),
                title: None,
                artist: Some("Zzz".to_owned()),
                album: None,
                year: None,
                track_num: None,
                duration: None,
            },
        )
        .unwrap();
        upsert_track(
            &conn,
            &TrackRow {
                path: "/a.mp3".to_owned(),
                filename: "a.mp3".to_owned(),
                title: None,
                artist: Some("Aaa".to_owned()),
                album: None,
                year: None,
                track_num: None,
                duration: None,
            },
        )
        .unwrap();

        let asc = get_tracks(&conn, "artist", "asc").unwrap();
        assert_eq!(asc[0].artist.as_deref(), Some("Aaa"));

        let desc = get_tracks(&conn, "artist", "desc").unwrap();
        assert_eq!(desc[0].artist.as_deref(), Some("Zzz"));
    }

    #[test]
    fn get_tracks_nones_sort_last() {
        let conn = open_db();
        upsert_track(
            &conn,
            &TrackRow {
                path: "/null.mp3".to_owned(),
                filename: "null.mp3".to_owned(),
                title: None,
                artist: None,
                album: None,
                year: None,
                track_num: None,
                duration: None,
            },
        )
        .unwrap();
        upsert_track(
            &conn,
            &TrackRow {
                path: "/aaa.mp3".to_owned(),
                filename: "aaa.mp3".to_owned(),
                title: None,
                artist: Some("Aaa".to_owned()),
                album: None,
                year: None,
                track_num: None,
                duration: None,
            },
        )
        .unwrap();

        let tracks = get_tracks(&conn, "artist", "asc").unwrap();
        assert_eq!(tracks[0].artist.as_deref(), Some("Aaa"));
        assert!(tracks[1].artist.is_none());
    }

    #[test]
    fn get_tracks_invalid_sort_col_falls_back() {
        let conn = open_db();
        upsert_track(&conn, &sample_row("/x.mp3")).unwrap();
        // must not panic or return error with unknown col
        let tracks = get_tracks(&conn, "unknown_col", "asc").unwrap();
        assert_eq!(tracks.len(), 1);
    }
}
