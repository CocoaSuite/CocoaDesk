//! Quick notes: plain Markdown texts. A note's title is its first line, so there is
//! nothing to name or file before writing.

use chrono::NaiveDateTime;
use rusqlite::{params, OptionalExtension};
use serde::Serialize;

use crate::store::Store;
use crate::{Error, Result};

const STAMP: &str = "%Y-%m-%dT%H:%M:%S";
/// Largest note accepted, to catch runaway data rather than real notes.
pub const MAX_NOTE_BYTES: usize = 4 * 1024 * 1024;

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Note {
    pub id: i64,
    pub body: String,
    pub pinned: bool,
    pub created_at: NaiveDateTime,
    pub updated_at: NaiveDateTime,
}

fn stamp(s: String) -> rusqlite::Result<NaiveDateTime> {
    NaiveDateTime::parse_from_str(&s, STAMP)
        .map_err(|e| rusqlite::Error::FromSqlConversionFailure(0, rusqlite::types::Type::Text, Box::new(e)))
}
fn ts(t: NaiveDateTime) -> String {
    t.format(STAMP).to_string()
}

/// The title shown for a note: its first non-empty line without Markdown heading or list marks.
pub fn title(body: &str) -> String {
    let line = body.lines().map(str::trim).find(|l| !l.is_empty()).unwrap_or("");
    let t = line.trim_start_matches('#').trim_start_matches(['-', '*', '>', ' ']).trim();
    t.chars().take(120).collect()
}

fn check(body: &str) -> Result<()> {
    if body.len() > MAX_NOTE_BYTES {
        return Err(Error::invalid("This note is too long to save."));
    }
    Ok(())
}

impl Store {
    /// Every note, pinned first, then most recently changed.
    pub fn notes(&self) -> Result<Vec<Note>> {
        let mut stmt = self
            .conn
            .prepare("SELECT id, body, pinned, created_at, updated_at FROM notes ORDER BY pinned DESC, updated_at DESC, id DESC")?;
        let rows = stmt
            .query_map([], |r| {
                Ok(Note { id: r.get(0)?, body: r.get(1)?, pinned: r.get(2)?, created_at: stamp(r.get(3)?)?, updated_at: stamp(r.get(4)?)? })
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    }

    pub fn note(&self, id: i64) -> Result<Note> {
        self.conn
            .query_row("SELECT id, body, pinned, created_at, updated_at FROM notes WHERE id = ?1", [id], |r| {
                Ok(Note { id: r.get(0)?, body: r.get(1)?, pinned: r.get(2)?, created_at: stamp(r.get(3)?)?, updated_at: stamp(r.get(4)?)? })
            })
            .optional()?
            .ok_or_else(|| Error::NotFound("note".into()))
    }

    pub fn create_note(&self, body: &str, now: NaiveDateTime) -> Result<Note> {
        check(body)?;
        self.conn.execute("INSERT INTO notes(body, pinned, created_at, updated_at) VALUES (?1, 0, ?2, ?2)", params![body, ts(now)])?;
        self.note(self.conn.last_insert_rowid())
    }

    /// Saves a note's text. Unchanged text keeps its time, so opening a note doesn't reorder the list.
    pub fn save_note(&self, id: i64, body: &str, now: NaiveDateTime) -> Result<Note> {
        check(body)?;
        let n = self.conn.execute(
            "UPDATE notes SET body = ?2, updated_at = CASE WHEN body = ?2 THEN updated_at ELSE ?3 END WHERE id = ?1",
            params![id, body, ts(now)],
        )?;
        if n == 0 {
            return Err(Error::NotFound("note".into()));
        }
        self.note(id)
    }

    pub fn set_note_pinned(&self, id: i64, pinned: bool) -> Result<()> {
        if self.conn.execute("UPDATE notes SET pinned = ?2 WHERE id = ?1", params![id, pinned])? == 0 {
            return Err(Error::NotFound("note".into()));
        }
        Ok(())
    }

    pub fn delete_note(&self, id: i64) -> Result<()> {
        self.conn.execute("DELETE FROM notes WHERE id = ?1", [id])?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::NaiveDate;

    fn at(h: u32) -> NaiveDateTime {
        NaiveDate::from_ymd_opt(2026, 11, 17).unwrap().and_hms_opt(h, 0, 0).unwrap()
    }

    #[test]
    fn titles_come_from_the_first_line() {
        assert_eq!(title("\n\n# Lab 3  \nbody"), "Lab 3");
        assert_eq!(title("- [ ] milk"), "[ ] milk");
        assert_eq!(title("   "), "");
    }

    #[test]
    fn notes_are_saved_ordered_and_deleted() {
        let s = Store::open_in_memory().unwrap();
        let a = s.create_note("first", at(9)).unwrap();
        let b = s.create_note("second", at(10)).unwrap();
        assert_eq!(s.notes().unwrap().iter().map(|n| n.id).collect::<Vec<_>>(), vec![b.id, a.id]);

        // Saving the same text keeps the time; a change moves the note to the top.
        assert_eq!(s.save_note(a.id, "first", at(11)).unwrap().updated_at, at(9));
        assert_eq!(s.save_note(a.id, "first!", at(11)).unwrap().updated_at, at(11));
        assert_eq!(s.notes().unwrap()[0].id, a.id);

        s.set_note_pinned(b.id, true).unwrap();
        assert_eq!(s.notes().unwrap()[0].id, b.id);

        s.delete_note(a.id).unwrap();
        assert!(s.save_note(a.id, "x", at(12)).is_err());
        assert_eq!(s.notes().unwrap().len(), 1);
    }
}
