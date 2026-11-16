//! Local SQLite storage.

use std::path::Path;

use chrono::{NaiveDate, NaiveDateTime, NaiveTime};
use rusqlite::{params, Connection, OptionalExtension, Row};

use crate::model::*;
use crate::{Error, Result};

const DATE: &str = "%Y-%m-%d";
const TIME: &str = "%H:%M";
const STAMP: &str = "%Y-%m-%dT%H:%M:%S";

pub struct Store {
    pub(crate) conn: Connection,
}

/// What an import changed.
#[derive(Clone, Debug, Default, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportResult {
    pub added: usize,
    pub updated: usize,
    pub removed: usize,
}

#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SemesterInput {
    pub name: String,
    pub start: NaiveDate,
    pub end: NaiveDate,
    pub first_week: WeekKind,
}

impl SemesterInput {
    pub fn validate(&self) -> Result<()> {
        if self.name.trim().is_empty() {
            return Err(Error::invalid("Give the semester a name."));
        }
        if self.end < self.start {
            return Err(Error::invalid("The semester ends before it starts."));
        }
        if (self.end - self.start).num_days() > 400 {
            return Err(Error::invalid("A semester can't be longer than a year."));
        }
        Ok(())
    }
}

/// One class as an import produced it, identified by a key that stays the same across
/// re-imports of a newer version of the same file.
#[derive(Clone, Debug, PartialEq)]
pub struct ImportedClass {
    pub key: String,
    pub input: ClassInput,
}

fn date(s: String) -> rusqlite::Result<NaiveDate> {
    NaiveDate::parse_from_str(&s, DATE).map_err(|e| rusqlite::Error::FromSqlConversionFailure(0, rusqlite::types::Type::Text, Box::new(e)))
}
fn time(s: String) -> rusqlite::Result<NaiveTime> {
    NaiveTime::parse_from_str(&s, TIME).map_err(|e| rusqlite::Error::FromSqlConversionFailure(0, rusqlite::types::Type::Text, Box::new(e)))
}
fn stamp(s: String) -> rusqlite::Result<NaiveDateTime> {
    NaiveDateTime::parse_from_str(&s, STAMP)
        .map_err(|e| rusqlite::Error::FromSqlConversionFailure(0, rusqlite::types::Type::Text, Box::new(e)))
}
fn conv<T>(r: Result<T>) -> rusqlite::Result<T> {
    r.map_err(|e| rusqlite::Error::FromSqlConversionFailure(0, rusqlite::types::Type::Text, Box::new(e)))
}
fn ds(d: NaiveDate) -> String {
    d.format(DATE).to_string()
}
fn ts(t: NaiveDateTime) -> String {
    t.format(STAMP).to_string()
}

const MIGRATIONS: &[&str] = &[
    r#"
CREATE TABLE semesters (
  id INTEGER PRIMARY KEY,
  name TEXT NOT NULL,
  start TEXT NOT NULL,
  end TEXT NOT NULL,
  first_week TEXT NOT NULL CHECK (first_week IN ('top','bottom'))
);
CREATE TABLE periods (
  semester_id INTEGER NOT NULL REFERENCES semesters(id) ON DELETE CASCADE,
  number INTEGER NOT NULL,
  start TEXT NOT NULL,
  end TEXT NOT NULL,
  PRIMARY KEY (semester_id, number)
);
CREATE TABLE breaks (
  id INTEGER PRIMARY KEY,
  semester_id INTEGER NOT NULL REFERENCES semesters(id) ON DELETE CASCADE,
  date_from TEXT NOT NULL,
  date_to TEXT NOT NULL,
  label TEXT NOT NULL DEFAULT ''
);
CREATE TABLE classes (
  id INTEGER PRIMARY KEY,
  semester_id INTEGER NOT NULL REFERENCES semesters(id) ON DELETE CASCADE,
  subject TEXT NOT NULL,
  kind TEXT NOT NULL,
  teacher TEXT NOT NULL DEFAULT '',
  room TEXT NOT NULL DEFAULT '',
  note TEXT NOT NULL DEFAULT '',
  weekday INTEGER NOT NULL CHECK (weekday BETWEEN 1 AND 7),
  period INTEGER NOT NULL,
  weeks TEXT NOT NULL CHECK (weeks IN ('every','top','bottom')),
  valid_from TEXT,
  valid_to TEXT,
  source TEXT NOT NULL,
  import_key TEXT,
  created_at TEXT NOT NULL
);
CREATE INDEX classes_semester ON classes(semester_id);
CREATE UNIQUE INDEX classes_import_key ON classes(semester_id, import_key) WHERE import_key IS NOT NULL;
CREATE TABLE marks (
  class_id INTEGER NOT NULL REFERENCES classes(id) ON DELETE CASCADE,
  date TEXT NOT NULL,
  status TEXT NOT NULL,
  marked_by TEXT NOT NULL,
  at TEXT NOT NULL,
  PRIMARY KEY (class_id, date)
);
CREATE TABLE reminders_sent (
  class_id INTEGER NOT NULL REFERENCES classes(id) ON DELETE CASCADE,
  date TEXT NOT NULL,
  PRIMARY KEY (class_id, date)
);
CREATE TABLE import_exclusions (
  semester_id INTEGER NOT NULL REFERENCES semesters(id) ON DELETE CASCADE,
  import_key TEXT NOT NULL,
  PRIMARY KEY (semester_id, import_key)
);
CREATE TABLE settings (
  key TEXT PRIMARY KEY,
  value TEXT NOT NULL
);
"#,
    r#"
CREATE TABLE news_topics (
  id INTEGER PRIMARY KEY,
  name TEXT NOT NULL,
  config TEXT NOT NULL,
  enabled INTEGER NOT NULL DEFAULT 1,
  created_at TEXT NOT NULL,
  last_checked TEXT,
  last_error TEXT
);
CREATE TABLE news_items (
  id INTEGER PRIMARY KEY,
  topic_id INTEGER NOT NULL REFERENCES news_topics(id) ON DELETE CASCADE,
  url TEXT NOT NULL,
  title TEXT NOT NULL,
  summary TEXT NOT NULL DEFAULT '',
  source_name TEXT NOT NULL DEFAULT '',
  domain TEXT NOT NULL DEFAULT '',
  published_at TEXT,
  fetched_at TEXT NOT NULL,
  is_read INTEGER NOT NULL DEFAULT 0,
  saved INTEGER NOT NULL DEFAULT 0,
  matched TEXT NOT NULL DEFAULT '[]',
  story TEXT NOT NULL,
  UNIQUE (topic_id, url)
);
CREATE INDEX news_items_topic ON news_items(topic_id, fetched_at);
CREATE TABLE news_blocked_sites (
  domain TEXT PRIMARY KEY
);
"#,
];

impl Store {
    pub fn open(path: &Path) -> Result<Self> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).map_err(|e| Error::invalid(format!("Can't create the data folder: {e}")))?;
        }
        Self::init(Connection::open(path)?)
    }

    pub fn open_in_memory() -> Result<Self> {
        Self::init(Connection::open_in_memory()?)
    }

    fn init(conn: Connection) -> Result<Self> {
        conn.pragma_update(None, "foreign_keys", "ON")?;
        conn.pragma_update(None, "journal_mode", "WAL")?;
        let mut store = Store { conn };
        store.migrate()?;
        Ok(store)
    }

    fn migrate(&mut self) -> Result<()> {
        let version: usize = self.conn.pragma_query_value(None, "user_version", |r| r.get(0))?;
        if version > MIGRATIONS.len() {
            return Err(Error::invalid("This data was made by a newer version of CocoaDesk."));
        }
        for (i, sql) in MIGRATIONS.iter().enumerate().skip(version) {
            let tx = self.conn.transaction()?;
            tx.execute_batch(sql)?;
            tx.pragma_update(None, "user_version", i + 1)?;
            tx.commit()?;
        }
        Ok(())
    }

    // ---- settings ----

    pub fn setting(&self, key: &str) -> Result<Option<String>> {
        Ok(self.conn.query_row("SELECT value FROM settings WHERE key = ?1", [key], |r| r.get(0)).optional()?)
    }

    pub fn set_setting(&self, key: &str, value: &str) -> Result<()> {
        self.conn.execute(
            "INSERT INTO settings(key, value) VALUES (?1, ?2) ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            params![key, value],
        )?;
        Ok(())
    }

    // ---- semesters ----

    pub fn semesters(&self) -> Result<Vec<Semester>> {
        let mut stmt = self.conn.prepare("SELECT id, name, start, end, first_week FROM semesters ORDER BY start")?;
        let rows = stmt
            .query_map([], |r| {
                Ok(Semester {
                    id: r.get(0)?,
                    name: r.get(1)?,
                    start: date(r.get(2)?)?,
                    end: date(r.get(3)?)?,
                    first_week: conv(WeekKind::parse(&r.get::<_, String>(4)?))?,
                    periods: vec![],
                    breaks: vec![],
                })
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        let mut out = Vec::with_capacity(rows.len());
        for mut s in rows {
            s.periods = self.periods(s.id)?;
            s.breaks = self.breaks(s.id)?;
            out.push(s);
        }
        Ok(out)
    }

    pub fn semester(&self, id: i64) -> Result<Semester> {
        self.semesters()?.into_iter().find(|s| s.id == id).ok_or_else(|| Error::NotFound("semester".into()))
    }

    pub fn create_semester(&self, input: &SemesterInput) -> Result<i64> {
        input.validate()?;
        self.conn.execute(
            "INSERT INTO semesters(name, start, end, first_week) VALUES (?1, ?2, ?3, ?4)",
            params![input.name.trim(), ds(input.start), ds(input.end), input.first_week.as_str()],
        )?;
        Ok(self.conn.last_insert_rowid())
    }

    pub fn update_semester(&self, id: i64, input: &SemesterInput) -> Result<()> {
        input.validate()?;
        let n = self.conn.execute(
            "UPDATE semesters SET name = ?2, start = ?3, end = ?4, first_week = ?5 WHERE id = ?1",
            params![id, input.name.trim(), ds(input.start), ds(input.end), input.first_week.as_str()],
        )?;
        if n == 0 {
            return Err(Error::NotFound("semester".into()));
        }
        Ok(())
    }

    pub fn delete_semester(&self, id: i64) -> Result<()> {
        self.conn.execute("DELETE FROM semesters WHERE id = ?1", [id])?;
        Ok(())
    }

    fn periods(&self, semester_id: i64) -> Result<Vec<Period>> {
        let mut stmt = self.conn.prepare("SELECT number, start, end FROM periods WHERE semester_id = ?1 ORDER BY number")?;
        let v = stmt
            .query_map([semester_id], |r| Ok(Period { number: r.get(0)?, start: time(r.get(1)?)?, end: time(r.get(2)?)? }))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(v)
    }

    pub fn set_periods(&mut self, semester_id: i64, periods: &[Period]) -> Result<()> {
        validate_periods(periods)?;
        let tx = self.conn.transaction()?;
        tx.execute("DELETE FROM periods WHERE semester_id = ?1", [semester_id])?;
        for p in periods {
            tx.execute(
                "INSERT INTO periods(semester_id, number, start, end) VALUES (?1, ?2, ?3, ?4)",
                params![semester_id, p.number, p.start.format(TIME).to_string(), p.end.format(TIME).to_string()],
            )?;
        }
        tx.commit()?;
        Ok(())
    }

    fn breaks(&self, semester_id: i64) -> Result<Vec<Break>> {
        let mut stmt = self.conn.prepare("SELECT id, date_from, date_to, label FROM breaks WHERE semester_id = ?1 ORDER BY date_from")?;
        let v = stmt
            .query_map([semester_id], |r| Ok(Break { id: r.get(0)?, from: date(r.get(1)?)?, to: date(r.get(2)?)?, label: r.get(3)? }))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(v)
    }

    pub fn add_break(&self, semester_id: i64, from: NaiveDate, to: NaiveDate, label: &str) -> Result<i64> {
        if to < from {
            return Err(Error::invalid("The break ends before it starts."));
        }
        self.conn.execute(
            "INSERT INTO breaks(semester_id, date_from, date_to, label) VALUES (?1, ?2, ?3, ?4)",
            params![semester_id, ds(from), ds(to), label.trim()],
        )?;
        Ok(self.conn.last_insert_rowid())
    }

    pub fn delete_break(&self, id: i64) -> Result<()> {
        self.conn.execute("DELETE FROM breaks WHERE id = ?1", [id])?;
        Ok(())
    }

    // ---- classes ----

    fn class_row(r: &Row) -> rusqlite::Result<Class> {
        Ok(Class {
            id: r.get(0)?,
            semester_id: r.get(1)?,
            subject: r.get(2)?,
            kind: conv(ClassKind::parse(&r.get::<_, String>(3)?))?,
            teacher: r.get(4)?,
            room: r.get(5)?,
            note: r.get(6)?,
            weekday: r.get(7)?,
            period: r.get(8)?,
            weeks: conv(Weeks::parse(&r.get::<_, String>(9)?))?,
            valid_from: r.get::<_, Option<String>>(10)?.map(date).transpose()?,
            valid_to: r.get::<_, Option<String>>(11)?.map(date).transpose()?,
            source: conv(Source::parse(&r.get::<_, String>(12)?))?,
            import_key: r.get(13)?,
            created_at: stamp(r.get(14)?)?,
        })
    }

    const CLASS_COLS: &'static str =
        "id, semester_id, subject, kind, teacher, room, note, weekday, period, weeks, valid_from, valid_to, source, import_key, created_at";

    pub fn classes(&self, semester_id: i64) -> Result<Vec<Class>> {
        let sql = format!("SELECT {} FROM classes WHERE semester_id = ?1 ORDER BY weekday, period, subject", Self::CLASS_COLS);
        let mut stmt = self.conn.prepare(&sql)?;
        let v = stmt.query_map([semester_id], Self::class_row)?.collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(v)
    }

    pub fn class(&self, id: i64) -> Result<Class> {
        let sql = format!("SELECT {} FROM classes WHERE id = ?1", Self::CLASS_COLS);
        self.conn.query_row(&sql, [id], Self::class_row).optional()?.ok_or_else(|| Error::NotFound("class".into()))
    }

    pub fn add_class(&self, semester_id: i64, input: &ClassInput, now: NaiveDateTime) -> Result<i64> {
        input.validate()?;
        insert_class(&self.conn, semester_id, input, Source::Manual, None, now)
    }
}

fn insert_class(conn: &Connection, semester_id: i64, c: &ClassInput, source: Source, key: Option<&str>, now: NaiveDateTime) -> Result<i64> {
    conn.execute(
            "INSERT INTO classes(semester_id, subject, kind, teacher, room, note, weekday, period, weeks, valid_from, valid_to, source, import_key, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14)",
            params![
                semester_id,
                c.subject.trim(),
                c.kind.as_str(),
                c.teacher.trim(),
                c.room.trim(),
                c.note.trim(),
                c.weekday,
                c.period,
                c.weeks.as_str(),
                c.valid_from.map(ds),
                c.valid_to.map(ds),
                source.as_str(),
                key,
                ts(now)
            ],
        )?;
    Ok(conn.last_insert_rowid())
}

impl Store {
    pub fn update_class(&self, id: i64, c: &ClassInput) -> Result<()> {
        c.validate()?;
        let n = self.conn.execute(
            "UPDATE classes SET subject = ?2, kind = ?3, teacher = ?4, room = ?5, note = ?6, weekday = ?7, period = ?8, weeks = ?9, valid_from = ?10, valid_to = ?11 WHERE id = ?1",
            params![
                id,
                c.subject.trim(),
                c.kind.as_str(),
                c.teacher.trim(),
                c.room.trim(),
                c.note.trim(),
                c.weekday,
                c.period,
                c.weeks.as_str(),
                c.valid_from.map(ds),
                c.valid_to.map(ds)
            ],
        )?;
        if n == 0 {
            return Err(Error::NotFound("class".into()));
        }
        Ok(())
    }

    pub fn delete_class(&self, id: i64) -> Result<()> {
        self.conn.execute("DELETE FROM classes WHERE id = ?1", [id])?;
        Ok(())
    }

    // ---- marks ----

    pub fn marks_between(&self, semester_id: i64, from: NaiveDate, to: NaiveDate) -> Result<Vec<Mark>> {
        let mut stmt = self.conn.prepare(
            "SELECT m.class_id, m.date, m.status, m.marked_by, m.at FROM marks m JOIN classes c ON c.id = m.class_id
             WHERE c.semester_id = ?1 AND m.date BETWEEN ?2 AND ?3",
        )?;
        let v = stmt
            .query_map(params![semester_id, ds(from), ds(to)], |r| {
                Ok(Mark {
                    class_id: r.get(0)?,
                    date: date(r.get(1)?)?,
                    status: conv(MarkStatus::parse(&r.get::<_, String>(2)?))?,
                    by: conv(MarkedBy::parse(&r.get::<_, String>(3)?))?,
                    at: stamp(r.get(4)?)?,
                })
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(v)
    }

    pub fn set_mark(&self, m: &Mark) -> Result<()> {
        self.conn.execute(
            "INSERT INTO marks(class_id, date, status, marked_by, at) VALUES (?1, ?2, ?3, ?4, ?5)
             ON CONFLICT(class_id, date) DO UPDATE SET status = excluded.status, marked_by = excluded.marked_by, at = excluded.at",
            params![m.class_id, ds(m.date), m.status.as_str(), m.by.as_str(), ts(m.at)],
        )?;
        Ok(())
    }

    /// Records an automatic mark only if nothing is recorded yet, so it never overrides a person.
    pub fn set_auto_mark_if_absent(&self, class_id: i64, d: NaiveDate, at: NaiveDateTime) -> Result<bool> {
        let n = self.conn.execute(
            "INSERT OR IGNORE INTO marks(class_id, date, status, marked_by, at) VALUES (?1, ?2, 'done', 'auto', ?3)",
            params![class_id, ds(d), ts(at)],
        )?;
        Ok(n > 0)
    }

    pub fn clear_mark(&self, class_id: i64, d: NaiveDate) -> Result<()> {
        self.conn.execute("DELETE FROM marks WHERE class_id = ?1 AND date = ?2", params![class_id, ds(d)])?;
        Ok(())
    }

    // ---- reminders ----

    /// Returns true the first time it is called for a session.
    pub fn claim_reminder(&self, class_id: i64, d: NaiveDate) -> Result<bool> {
        let n = self.conn.execute("INSERT OR IGNORE INTO reminders_sent(class_id, date) VALUES (?1, ?2)", params![class_id, ds(d)])?;
        Ok(n > 0)
    }

    // ---- imports ----

    pub fn import_exclusions(&self, semester_id: i64) -> Result<Vec<String>> {
        let mut stmt = self.conn.prepare("SELECT import_key FROM import_exclusions WHERE semester_id = ?1")?;
        let v = stmt.query_map([semester_id], |r| r.get(0))?.collect::<rusqlite::Result<Vec<String>>>()?;
        Ok(v)
    }

    /// Applies an import to a semester in one transaction. Imported classes are matched by key:
    /// matches are updated in place (keeping their history), new ones are added, and imported
    /// classes that are no longer in the file — or that were unticked — are removed. Classes
    /// added by hand are never touched.
    pub fn apply_import(
        &mut self,
        semester_id: i64,
        periods: Option<&[Period]>,
        included: &[ImportedClass],
        excluded_keys: &[String],
        now: NaiveDateTime,
    ) -> Result<ImportResult> {
        for c in included {
            c.input.validate()?;
        }
        if let Some(p) = periods {
            validate_periods(p)?;
        }
        let existing: Vec<(i64, String)> = {
            let mut stmt = self.conn.prepare("SELECT id, import_key FROM classes WHERE semester_id = ?1 AND import_key IS NOT NULL")?;
            let v = stmt.query_map([semester_id], |r| Ok((r.get(0)?, r.get(1)?)))?.collect::<rusqlite::Result<Vec<_>>>()?;
            v
        };
        let tx = self.conn.transaction()?;
        let mut result = ImportResult::default();
        if let Some(periods) = periods {
            tx.execute("DELETE FROM periods WHERE semester_id = ?1", [semester_id])?;
            for p in periods {
                tx.execute(
                    "INSERT INTO periods(semester_id, number, start, end) VALUES (?1, ?2, ?3, ?4)",
                    params![semester_id, p.number, p.start.format(TIME).to_string(), p.end.format(TIME).to_string()],
                )?;
            }
        }
        for c in included {
            let i = &c.input;
            if let Some((id, _)) = existing.iter().find(|(_, k)| *k == c.key) {
                tx.execute(
                    "UPDATE classes SET subject = ?2, kind = ?3, teacher = ?4, room = ?5, note = ?6, weekday = ?7, period = ?8, weeks = ?9, valid_from = ?10, valid_to = ?11 WHERE id = ?1",
                    params![
                        id,
                        i.subject.trim(),
                        i.kind.as_str(),
                        i.teacher.trim(),
                        i.room.trim(),
                        i.note.trim(),
                        i.weekday,
                        i.period,
                        i.weeks.as_str(),
                        i.valid_from.map(ds),
                        i.valid_to.map(ds)
                    ],
                )?;
                result.updated += 1;
            } else {
                insert_class(&tx, semester_id, &c.input, Source::Import, Some(&c.key), now)?;
                result.added += 1;
            }
        }
        for (id, key) in &existing {
            if !included.iter().any(|c| &c.key == key) {
                tx.execute("DELETE FROM classes WHERE id = ?1", [id])?;
                result.removed += 1;
            }
        }
        tx.execute("DELETE FROM import_exclusions WHERE semester_id = ?1", [semester_id])?;
        for k in excluded_keys {
            tx.execute("INSERT OR IGNORE INTO import_exclusions(semester_id, import_key) VALUES (?1, ?2)", params![semester_id, k])?;
        }
        tx.commit()?;
        Ok(result)
    }
}

pub fn validate_periods(periods: &[Period]) -> Result<()> {
    let mut seen = std::collections::HashSet::new();
    for p in periods {
        if p.number == 0 {
            return Err(Error::invalid("Periods are numbered from 1."));
        }
        if !seen.insert(p.number) {
            return Err(Error::invalid(format!("Period {} is listed twice.", p.number)));
        }
        if p.end <= p.start {
            return Err(Error::invalid(format!("Period {} ends before it starts.", p.number)));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn d(y: i32, m: u32, day: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, day).unwrap()
    }
    fn now() -> NaiveDateTime {
        d(2026, 11, 15).and_hms_opt(20, 0, 0).unwrap()
    }
    fn sem_input() -> SemesterInput {
        SemesterInput { name: "Autumn 2026".into(), start: d(2026, 9, 1), end: d(2026, 12, 31), first_week: WeekKind::Top }
    }
    fn class(subject: &str, weekday: u8, period: u8) -> ClassInput {
        ClassInput {
            subject: subject.into(),
            kind: ClassKind::Seminar,
            teacher: String::new(),
            room: String::new(),
            note: String::new(),
            weekday,
            period,
            weeks: Weeks::Every,
            valid_from: None,
            valid_to: None,
        }
    }
    fn periods() -> Vec<Period> {
        vec![Period { number: 1, start: NaiveTime::from_hms_opt(9, 0, 0).unwrap(), end: NaiveTime::from_hms_opt(10, 20, 0).unwrap() }]
    }

    #[test]
    fn round_trips_semesters_and_classes() {
        let mut s = Store::open_in_memory().unwrap();
        let id = s.create_semester(&sem_input()).unwrap();
        s.set_periods(id, &periods()).unwrap();
        s.add_break(id, d(2026, 11, 4), d(2026, 11, 4), "Holiday").unwrap();
        let cid = s.add_class(id, &class("Maths", 1, 1), now()).unwrap();
        let sem = s.semester(id).unwrap();
        assert_eq!(sem.periods.len(), 1);
        assert_eq!(sem.breaks[0].label, "Holiday");
        let c = s.class(cid).unwrap();
        assert_eq!(c.subject, "Maths");
        assert_eq!(c.created_at, now());
    }

    #[test]
    fn auto_marks_never_override_people() {
        let s = Store::open_in_memory().unwrap();
        let id = s.create_semester(&sem_input()).unwrap();
        let cid = s.add_class(id, &class("Maths", 1, 1), now()).unwrap();
        let day = d(2026, 11, 16);
        s.set_mark(&Mark { class_id: cid, date: day, status: MarkStatus::Skipped, by: MarkedBy::User, at: now() }).unwrap();
        assert!(!s.set_auto_mark_if_absent(cid, day, now()).unwrap());
        assert_eq!(s.marks_between(id, day, day).unwrap()[0].status, MarkStatus::Skipped);
    }

    #[test]
    fn reimport_updates_in_place_and_keeps_manual_classes() {
        let mut s = Store::open_in_memory().unwrap();
        let id = s.create_semester(&sem_input()).unwrap();
        let manual = s.add_class(id, &class("Own study group", 6, 1), now()).unwrap();
        let first = vec![
            ImportedClass { key: "a".into(), input: class("Maths", 1, 1) },
            ImportedClass { key: "b".into(), input: class("History", 2, 1) },
        ];
        let r = s.apply_import(id, Some(&periods()), &first, &[], now()).unwrap();
        assert_eq!(r, ImportResult { added: 2, updated: 0, removed: 0 });
        let maths_id = s.classes(id).unwrap().iter().find(|c| c.subject == "Maths").unwrap().id;
        s.set_mark(&Mark { class_id: maths_id, date: d(2026, 11, 16), status: MarkStatus::Done, by: MarkedBy::Auto, at: now() }).unwrap();

        let mut changed = class("Maths", 1, 1);
        changed.room = "GK-315".into();
        let second = vec![ImportedClass { key: "a".into(), input: changed }];
        let r = s.apply_import(id, None, &second, &["b".into()], now()).unwrap();
        assert_eq!(r, ImportResult { added: 0, updated: 1, removed: 1 });
        let classes = s.classes(id).unwrap();
        assert_eq!(classes.len(), 2);
        let maths = classes.iter().find(|c| c.subject == "Maths").unwrap();
        assert_eq!(maths.id, maths_id);
        assert_eq!(maths.room, "GK-315");
        assert!(classes.iter().any(|c| c.id == manual));
        assert_eq!(s.marks_between(id, d(2026, 11, 16), d(2026, 11, 16)).unwrap().len(), 1);
        assert_eq!(s.import_exclusions(id).unwrap(), vec!["b".to_string()]);
    }
}
