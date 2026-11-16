//! Moodboards: infinite canvases of reference images, sections, text and sketches.
//!
//! A board's contents (its *scene*) are kept as one JSON document that the interface owns;
//! the core only checks that it is well formed and reads which images it uses. Images are
//! copied into CocoaDesk's data folder, named by a hash of their bytes, so the same picture
//! pasted twice is stored once and boards keep working if the original file moves.

use std::collections::HashSet;
use std::io::Write;
use std::path::{Path, PathBuf};

use chrono::NaiveDateTime;
use rusqlite::{params, OptionalExtension};
use serde::Serialize;
use serde_json::Value;
use sha2::{Digest, Sha256};

use crate::store::Store;
use crate::{Error, Result};

const STAMP: &str = "%Y-%m-%dT%H:%M:%S";
/// Largest scene document accepted, to catch runaway data rather than real boards.
pub const MAX_SCENE_BYTES: usize = 32 * 1024 * 1024;
/// Largest single image accepted.
pub const MAX_IMAGE_BYTES: usize = 64 * 1024 * 1024;
/// An empty board.
pub const EMPTY_SCENE: &str = r#"{"items":[]}"#;

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BoardSummary {
    pub id: i64,
    pub name: String,
    pub created_at: NaiveDateTime,
    pub updated_at: NaiveDateTime,
    pub items: usize,
    /// The first image on the board, shown next to its name.
    pub cover: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Board {
    pub id: i64,
    pub name: String,
    /// The scene document, as the interface saved it.
    pub scene: String,
}

fn stamp(s: String) -> rusqlite::Result<NaiveDateTime> {
    NaiveDateTime::parse_from_str(&s, STAMP)
        .map_err(|e| rusqlite::Error::FromSqlConversionFailure(0, rusqlite::types::Type::Text, Box::new(e)))
}
fn ts(t: NaiveDateTime) -> String {
    t.format(STAMP).to_string()
}

fn clean_name(name: &str) -> Result<String> {
    let name = name.split_whitespace().collect::<Vec<_>>().join(" ");
    if name.is_empty() {
        return Err(Error::invalid("Give the board a name."));
    }
    if name.chars().count() > 120 {
        return Err(Error::invalid("That name is too long."));
    }
    Ok(name)
}

/// Checks a scene document and returns it parsed.
pub fn parse_scene(scene: &str) -> Result<Value> {
    if scene.len() > MAX_SCENE_BYTES {
        return Err(Error::invalid("This board is too large to save."));
    }
    let v: Value = serde_json::from_str(scene).map_err(|_| Error::invalid("The board data is damaged."))?;
    if !v.get("items").is_some_and(Value::is_array) {
        return Err(Error::invalid("The board data is damaged."));
    }
    Ok(v)
}

fn items(scene: &Value) -> impl Iterator<Item = &Value> {
    scene.get("items").and_then(Value::as_array).into_iter().flatten()
}

/// The images a scene uses, in the order they appear.
pub fn scene_images(scene: &Value) -> Vec<String> {
    let mut seen = HashSet::new();
    items(scene)
        .filter(|it| it.get("type").and_then(Value::as_str) == Some("image"))
        .filter_map(|it| it.get("src").and_then(Value::as_str))
        .filter(|s| is_image_id(s) && seen.insert(s.to_string()))
        .map(String::from)
        .collect()
}

impl Store {
    pub fn boards(&self) -> Result<Vec<BoardSummary>> {
        let mut stmt = self.conn.prepare("SELECT id, name, created_at, updated_at, scene FROM boards ORDER BY position, id")?;
        let rows = stmt
            .query_map([], |r| {
                Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?, stamp(r.get(2)?)?, stamp(r.get(3)?)?, r.get::<_, String>(4)?))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows
            .into_iter()
            .map(|(id, name, created_at, updated_at, scene)| {
                let parsed = serde_json::from_str::<Value>(&scene).unwrap_or(Value::Null);
                BoardSummary {
                    id,
                    name,
                    created_at,
                    updated_at,
                    items: items(&parsed).count(),
                    cover: scene_images(&parsed).into_iter().next(),
                }
            })
            .collect())
    }

    pub fn board(&self, id: i64) -> Result<Board> {
        self.conn
            .query_row("SELECT id, name, scene FROM boards WHERE id = ?1", [id], |r| {
                Ok(Board { id: r.get(0)?, name: r.get(1)?, scene: r.get(2)? })
            })
            .optional()?
            .ok_or_else(|| Error::NotFound("board".into()))
    }

    pub fn create_board(&self, name: &str, now: NaiveDateTime) -> Result<i64> {
        let name = clean_name(name)?;
        let pos: i64 = self.conn.query_row("SELECT COALESCE(MAX(position), 0) + 1 FROM boards", [], |r| r.get(0))?;
        self.conn.execute(
            "INSERT INTO boards(name, scene, position, created_at, updated_at) VALUES (?1, ?2, ?3, ?4, ?4)",
            params![name, EMPTY_SCENE, pos, ts(now)],
        )?;
        Ok(self.conn.last_insert_rowid())
    }

    pub fn rename_board(&self, id: i64, name: &str) -> Result<()> {
        let name = clean_name(name)?;
        if self.conn.execute("UPDATE boards SET name = ?2 WHERE id = ?1", params![id, name])? == 0 {
            return Err(Error::NotFound("board".into()));
        }
        Ok(())
    }

    /// Makes a copy of a board, images included (they are shared, not duplicated on disk).
    pub fn duplicate_board(&self, id: i64, now: NaiveDateTime) -> Result<i64> {
        let b = self.board(id)?;
        let mut name = format!("{} copy", b.name);
        if name.chars().count() > 120 {
            name = b.name;
        }
        let new_id = self.create_board(&name, now)?;
        self.save_board(new_id, &b.scene, now)?;
        Ok(new_id)
    }

    pub fn delete_board(&self, id: i64) -> Result<()> {
        self.conn.execute("DELETE FROM boards WHERE id = ?1", [id])?;
        Ok(())
    }

    pub fn save_board(&self, id: i64, scene: &str, now: NaiveDateTime) -> Result<()> {
        parse_scene(scene)?;
        if self.conn.execute("UPDATE boards SET scene = ?2, updated_at = ?3 WHERE id = ?1", params![id, scene, ts(now)])? == 0 {
            return Err(Error::NotFound("board".into()));
        }
        Ok(())
    }

    /// Every image used by any board.
    pub fn board_images_in_use(&self) -> Result<HashSet<String>> {
        let mut stmt = self.conn.prepare("SELECT scene FROM boards")?;
        let scenes = stmt.query_map([], |r| r.get::<_, String>(0))?.collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(scenes.iter().filter_map(|s| serde_json::from_str::<Value>(s).ok()).flat_map(|v| scene_images(&v)).collect())
    }
}

// ---------- images ----------

/// Image formats a board can hold, recognised from the file's first bytes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ImageKind {
    Png,
    Jpeg,
    Gif,
    Webp,
    Bmp,
    Avif,
    Ico,
}

impl ImageKind {
    pub fn sniff(b: &[u8]) -> Option<Self> {
        if b.starts_with(b"\x89PNG\r\n\x1a\n") {
            Some(Self::Png)
        } else if b.starts_with(&[0xFF, 0xD8, 0xFF]) {
            Some(Self::Jpeg)
        } else if b.starts_with(b"GIF87a") || b.starts_with(b"GIF89a") {
            Some(Self::Gif)
        } else if b.len() >= 12 && &b[0..4] == b"RIFF" && &b[8..12] == b"WEBP" {
            Some(Self::Webp)
        } else if b.starts_with(b"BM") && b.len() > 26 {
            Some(Self::Bmp)
        } else if b.len() >= 12 && &b[4..8] == b"ftyp" && (&b[8..12] == b"avif" || &b[8..12] == b"avis") {
            Some(Self::Avif)
        } else if b.starts_with(&[0, 0, 1, 0]) && b.len() > 22 {
            Some(Self::Ico)
        } else {
            None
        }
    }

    pub fn mime(self) -> &'static str {
        match self {
            Self::Png => "image/png",
            Self::Jpeg => "image/jpeg",
            Self::Gif => "image/gif",
            Self::Webp => "image/webp",
            Self::Bmp => "image/bmp",
            Self::Avif => "image/avif",
            Self::Ico => "image/x-icon",
        }
    }
}

/// An image id is the lowercase hex SHA-256 of its bytes.
pub fn is_image_id(s: &str) -> bool {
    s.len() == 64 && s.bytes().all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c))
}

/// Image files in a folder, named by their id.
pub struct ImageStore {
    dir: PathBuf,
}

impl ImageStore {
    pub fn new(dir: impl Into<PathBuf>) -> Self {
        ImageStore { dir: dir.into() }
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// Stores an image and returns its id. Storing the same bytes again is free.
    pub fn put(&self, bytes: &[u8]) -> Result<String> {
        if bytes.len() > MAX_IMAGE_BYTES {
            return Err(Error::invalid("That image is too large (over 64 MB)."));
        }
        if ImageKind::sniff(bytes).is_none() {
            return Err(Error::invalid("That isn't an image CocoaDesk can show (PNG, JPEG, GIF, WebP, BMP, AVIF or ICO)."));
        }
        let id: String = Sha256::digest(bytes).iter().map(|b| format!("{b:02x}")).collect();
        let path = self.dir.join(&id);
        if !path.exists() {
            std::fs::create_dir_all(&self.dir).map_err(|e| Error::invalid(format!("Can't create the image folder: {e}")))?;
            // Written under a temporary name first so a crash never leaves half an image behind.
            let tmp = self.dir.join(format!("{id}.part"));
            let mut f = std::fs::File::create(&tmp).map_err(|e| Error::invalid(format!("Can't save the image: {e}")))?;
            f.write_all(bytes).and_then(|_| f.sync_all()).map_err(|e| Error::invalid(format!("Can't save the image: {e}")))?;
            drop(f);
            std::fs::rename(&tmp, &path).map_err(|e| Error::invalid(format!("Can't save the image: {e}")))?;
        }
        Ok(id)
    }

    /// Reads an image and its type.
    pub fn get(&self, id: &str) -> Result<(Vec<u8>, ImageKind)> {
        if !is_image_id(id) {
            return Err(Error::NotFound("image".into()));
        }
        let bytes = std::fs::read(self.dir.join(id)).map_err(|_| Error::NotFound("image".into()))?;
        let kind = ImageKind::sniff(&bytes).ok_or_else(|| Error::NotFound("image".into()))?;
        Ok((bytes, kind))
    }

    /// Deletes images no board uses any more, and leftovers of interrupted saves.
    /// Returns how many files were removed.
    pub fn collect_garbage(&self, in_use: &HashSet<String>) -> usize {
        let Ok(entries) = std::fs::read_dir(&self.dir) else { return 0 };
        let mut removed = 0;
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            let stale = if is_image_id(&name) { !in_use.contains(&name) } else { name.ends_with(".part") };
            if stale && std::fs::remove_file(entry.path()).is_ok() {
                removed += 1;
            }
        }
        removed
    }
}

/// Downloads a picture dropped or pasted from a web page and stores it.
pub fn download_image(fetcher: &dyn crate::news::Fetch, images: &ImageStore, address: &str) -> Result<String> {
    let url = url::Url::parse(address.trim()).map_err(|_| Error::invalid("That isn't a web address."))?;
    if !matches!(url.scheme(), "http" | "https") {
        return Err(Error::invalid("Only web images can be downloaded."));
    }
    let bytes = fetcher.get_image(url.as_str())?;
    if ImageKind::sniff(&bytes).is_none() {
        return Err(Error::invalid("That link isn't an image CocoaDesk can show."));
    }
    images.put(&bytes)
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::NaiveDate;

    const PNG: &[u8] = b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR\0\0\0\x01\0\0\0\x01\x08\x06\0\0\0\x1f\x15\xc4\x89";

    fn now() -> NaiveDateTime {
        NaiveDate::from_ymd_opt(2026, 11, 16).unwrap().and_hms_opt(12, 0, 0).unwrap()
    }

    fn tmpdir(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("cocoadesk-test-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        d
    }

    #[test]
    fn boards_are_created_saved_and_listed() {
        let s = Store::open_in_memory().unwrap();
        assert!(s.create_board("   ", now()).is_err());
        let a = s.create_board("  Character   refs ", now()).unwrap();
        let b = s.create_board("Rooms", now()).unwrap();
        let id = "a".repeat(64);
        let scene = format!(
            r#"{{"items":[{{"type":"text","text":"hi"}},{{"type":"image","src":"{id}"}},{{"type":"image","src":"../etc"}}],"view":{{"x":0}}}}"#
        );
        s.save_board(a, &scene, now()).unwrap();
        assert!(s.save_board(a, "{\"nope\":1}", now()).is_err());
        assert!(s.save_board(a, "not json", now()).is_err());

        let list = s.boards().unwrap();
        assert_eq!(list.iter().map(|b| b.name.as_str()).collect::<Vec<_>>(), ["Character refs", "Rooms"]);
        assert_eq!(list[0].items, 3);
        assert_eq!(list[0].cover.as_deref(), Some(id.as_str()));
        assert_eq!(list[1].cover, None);
        assert_eq!(s.board(a).unwrap().scene, scene);

        let c = s.duplicate_board(a, now()).unwrap();
        assert_eq!(s.board(c).unwrap().name, "Character refs copy");
        s.rename_board(b, "Interiors").unwrap();
        s.delete_board(a).unwrap();
        assert_eq!(s.boards().unwrap().len(), 2);
        assert_eq!(s.board_images_in_use().unwrap(), HashSet::from([id]));
        assert!(s.board(a).is_err());
    }

    #[test]
    fn web_images_are_downloaded_and_checked() {
        struct Canned;
        impl crate::news::Fetch for Canned {
            fn get(&self, url: &str) -> Result<Vec<u8>> {
                if url.ends_with(".png") {
                    Ok(PNG.to_vec())
                } else {
                    Ok(b"<html>".to_vec())
                }
            }
        }
        let dir = tmpdir("download");
        let st = ImageStore::new(&dir);
        assert!(is_image_id(&download_image(&Canned, &st, "https://example.com/a.png").unwrap()));
        assert!(download_image(&Canned, &st, "https://example.com/page").is_err());
        assert!(download_image(&Canned, &st, "file:///c:/a.png").is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn images_are_recognised_by_their_bytes() {
        assert_eq!(ImageKind::sniff(PNG), Some(ImageKind::Png));
        assert_eq!(ImageKind::sniff(&[0xFF, 0xD8, 0xFF, 0xE0]), Some(ImageKind::Jpeg));
        assert_eq!(ImageKind::sniff(b"GIF89a....."), Some(ImageKind::Gif));
        assert_eq!(ImageKind::sniff(b"RIFF\0\0\0\0WEBPVP8 "), Some(ImageKind::Webp));
        assert_eq!(ImageKind::sniff(b"\0\0\0\x1cftypavif\0\0"), Some(ImageKind::Avif));
        assert_eq!(ImageKind::sniff(b"<svg xmlns="), None);
        assert_eq!(ImageKind::sniff(b"<html>"), None);
    }

    #[test]
    fn image_store_dedupes_and_collects_garbage() {
        let dir = tmpdir("images");
        let st = ImageStore::new(&dir);
        assert!(st.put(b"plain text").is_err());
        let a = st.put(PNG).unwrap();
        assert!(is_image_id(&a));
        assert_eq!(st.put(PNG).unwrap(), a);
        let mut other = PNG.to_vec();
        other.push(0);
        let b = st.put(&other).unwrap();
        assert_ne!(a, b);
        let (bytes, kind) = st.get(&a).unwrap();
        assert_eq!((bytes.as_slice(), kind), (PNG, ImageKind::Png));
        assert!(st.get("../secret").is_err());

        std::fs::write(dir.join("x.part"), b"half").unwrap();
        assert_eq!(st.collect_garbage(&HashSet::from([a.clone()])), 2);
        assert!(st.get(&a).is_ok());
        assert!(st.get(&b).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
