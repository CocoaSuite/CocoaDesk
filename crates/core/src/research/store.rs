//! Research storage: papers, followed topics, the library (notes, collections, subjects,
//! task cards), blocked journals and websites, read texts, and the views the screen shows.

use std::collections::{HashMap, HashSet};
use std::sync::Mutex;

use chrono::{DateTime, Duration, NaiveDate, NaiveDateTime, NaiveTime, Utc};
use rusqlite::{params, OptionalExtension, Row};
use serde::{Deserialize, Serialize};

use super::cite::{self, Style};
use super::sources::{self, Explore, Keys, Overview};
use super::*;
use crate::news::article::{extract, Article};
use crate::news::feed::Fetch;
use crate::store::Store;
use crate::tasks::CardInput;
use crate::{Error, Result};

const STAMP: &str = "%Y-%m-%dT%H:%M:%S";
/// Papers found by a search but never kept are forgotten after this many days.
pub const KEEP_DAYS: i64 = 30;
/// Papers found for a topic and never saved are cleared after this many days.
pub const HIT_DAYS: i64 = 120;
pub const MIN_INTERVAL: u32 = 60;
pub const MAX_INTERVAL: u32 = 7 * 24 * 60;
pub const MAX_NOTIFICATIONS: usize = 3;

fn utc_str(t: DateTime<Utc>) -> String {
    t.to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
}
fn parse_utc(s: &str) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(s).ok().map(|d| d.with_timezone(&Utc))
}
fn local_str(t: NaiveDateTime) -> String {
    t.format(STAMP).to_string()
}
fn conv<T>(r: Result<T>) -> rusqlite::Result<T> {
    r.map_err(|e| rusqlite::Error::FromSqlConversionFailure(0, rusqlite::types::Type::Text, Box::new(e)))
}
fn lock(store: &Mutex<Store>) -> Result<std::sync::MutexGuard<'_, Store>> {
    store.lock().map_err(|_| Error::invalid("The data store is unavailable."))
}

// ---------- settings ----------

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Settings {
    #[serde(default)]
    pub keys: Keys,
    #[serde(default = "gost")]
    pub style: Style,
    /// The databases a new search starts with.
    #[serde(default = "all_sources")]
    pub sources: Vec<Source>,
}

fn gost() -> Style {
    Style::Gost
}

impl Default for Settings {
    fn default() -> Self {
        Settings { keys: Keys::default(), style: Style::Gost, sources: all_sources() }
    }
}

// ---------- topics ----------

/// A search CocoaDesk repeats on a schedule, keeping the papers it hasn't seen before.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TopicConfig {
    #[serde(flatten)]
    pub query: Query,
    pub interval_minutes: u32,
    #[serde(default)]
    pub notify: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Topic {
    pub id: i64,
    pub name: String,
    #[serde(flatten)]
    pub config: TopicConfig,
    pub enabled: bool,
    pub created_at: NaiveDateTime,
    pub last_checked: Option<NaiveDateTime>,
    pub last_error: Option<String>,
}

impl Topic {
    pub fn is_due(&self, now: NaiveDateTime) -> bool {
        self.enabled && self.last_checked.is_none_or(|t| t > now || (now - t).num_minutes() >= self.config.interval_minutes as i64)
    }
    pub fn next_check(&self) -> Option<NaiveDateTime> {
        self.last_checked.map(|t| t + Duration::minutes(self.config.interval_minutes as i64))
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TopicInput {
    pub name: String,
    #[serde(flatten)]
    pub config: TopicConfig,
    pub enabled: bool,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TopicSummary {
    #[serde(flatten)]
    pub topic: Topic,
    pub unread: i64,
    pub next_check: Option<NaiveDateTime>,
}

fn checked_topic(input: &TopicInput) -> Result<(String, TopicConfig)> {
    let mut cfg = input.config.clone();
    cfg.query = cfg.query.cleaned();
    cfg.query.validate()?;
    let name = match input.name.trim() {
        "" => cfg.query.text.chars().take(60).collect(),
        n => n.to_string(),
    };
    if !(MIN_INTERVAL..=MAX_INTERVAL).contains(&cfg.interval_minutes) {
        return Err(Error::invalid("Check at most once an hour, and at least once a week."));
    }
    Ok((name, cfg))
}

fn topic_row(r: &Row) -> rusqlite::Result<Topic> {
    let config: TopicConfig = conv(serde_json::from_str(&r.get::<_, String>(2)?).map_err(|e| Error::invalid(e.to_string())))?;
    Ok(Topic {
        id: r.get(0)?,
        name: r.get(1)?,
        config,
        enabled: r.get(3)?,
        created_at: NaiveDateTime::parse_from_str(&r.get::<_, String>(4)?, STAMP).unwrap_or_default(),
        last_checked: r.get::<_, Option<String>>(5)?.and_then(|s| NaiveDateTime::parse_from_str(&s, STAMP).ok()),
        last_error: r.get(6)?,
    })
}

// ---------- papers as the screen sees them ----------

/// A paper with what the person has done with it.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PaperView {
    pub id: i64,
    #[serde(flatten)]
    pub paper: Paper,
    pub saved: bool,
    pub read: bool,
    /// Found by a followed topic and not opened yet.
    pub is_new: bool,
    pub note: String,
    pub subject: String,
    pub collections: Vec<i64>,
    pub task_card_id: Option<i64>,
    pub found_at: Option<DateTime<Utc>>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Collection {
    pub id: i64,
    pub name: String,
    pub papers: i64,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchResult {
    pub papers: Vec<PaperView>,
    /// Problems with single databases; the others still answered.
    pub errors: Vec<String>,
    pub asked: Vec<Source>,
    /// How many each database returned before merging and filtering.
    pub counts: Vec<(Source, usize)>,
}

/// One week of a topic, put together from counts — nothing is written by a model.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Digest {
    /// Papers found in the last 7 days.
    pub papers: usize,
    pub journals: usize,
    pub open_access: usize,
    /// The most cited of them.
    pub most_cited: Option<DigestPaper>,
    /// The newest by publication date.
    pub newest: Option<DigestPaper>,
    /// The journal or archive with the most of them, and how many.
    pub busiest: Option<(String, usize)>,
    /// Keywords several of them share.
    pub keywords: Vec<String>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DigestPaper {
    pub id: i64,
    pub title: String,
    pub citations: Option<i64>,
    pub date: Option<NaiveDate>,
    pub venue: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Filter {
    All,
    Unread,
    Saved,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TopicsView {
    pub topics: Vec<TopicSummary>,
    pub topic: Option<TopicSummary>,
    pub papers: Vec<PaperView>,
    pub filter: Filter,
    pub digest: Option<Digest>,
}

/// What the library shows: one collection, one subject, unread, or everything.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", tag = "kind")]
pub enum Shelf {
    All,
    Unread,
    Collection { id: i64 },
    Subject { name: String },
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LibraryView {
    pub shelf: Shelf,
    pub papers: Vec<PaperView>,
    pub collections: Vec<Collection>,
    /// Subjects papers are linked to, with counts.
    pub subjects: Vec<(String, i64)>,
    /// Subjects from the class timetable, newest semester first, to link papers to.
    pub class_subjects: Vec<String>,
    pub total: i64,
    pub unread: i64,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Citations {
    pub gost: String,
    pub apa: String,
    pub bibtex: String,
}

/// A paper opened in the reader.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Reading {
    pub paper: PaperView,
    /// The full text, when the paper has a readable web page.
    pub article: Option<Article>,
    pub text_error: Option<String>,
    pub citations: Citations,
}

#[derive(Clone, Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CheckOutcome {
    pub topic_id: i64,
    pub topic_name: String,
    pub added: usize,
    pub error: Option<String>,
    pub notify: Vec<PaperView>,
    pub notify_more: usize,
}

const VIEW_SQL: &str = "SELECT p.id, p.data, l.paper_id IS NOT NULL, COALESCE(l.is_read, 0), COALESCE(l.note, ''), COALESCE(l.subject, ''), l.task_card_id,
     (SELECT MIN(h.is_read) FROM research_hits h WHERE h.paper_id = p.id), (SELECT MAX(h.found_at) FROM research_hits h WHERE h.paper_id = p.id)
     FROM research_papers p LEFT JOIN research_library l ON l.paper_id = p.id";

impl Store {
    // ---- settings ----

    pub fn research_settings(&self) -> Result<Settings> {
        Ok(self.setting("research")?.and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default())
    }

    pub fn save_research_settings(&self, s: &Settings) -> Result<()> {
        let mut s = s.clone();
        s.keys.openalex = s.keys.openalex.trim().to_string();
        s.keys.semantic_scholar = s.keys.semantic_scholar.trim().to_string();
        s.keys.email = s.keys.email.trim().to_string();
        if !s.keys.email.is_empty() && !(s.keys.email.contains('@') && s.keys.email.contains('.')) {
            return Err(Error::invalid("That email address doesn't look right."));
        }
        let mut seen = HashSet::new();
        s.sources.retain(|x| seen.insert(*x));
        if s.sources.is_empty() {
            return Err(Error::invalid("Keep at least one database on."));
        }
        self.set_setting("research", &serde_json::to_string(&s).expect("serialisable"))
    }

    // ---- papers ----

    /// Stores a paper, merging it with what's already known about it. Returns its id.
    pub fn upsert_paper(&self, p: &Paper, now: DateTime<Utc>) -> Result<i64> {
        let mut keys = vec![p.key.clone()];
        let alt = format!("t:{}:{}", title_key(&p.title), p.year.map(|y| y.to_string()).unwrap_or_default());
        if alt != p.key {
            keys.push(alt);
        }
        if let Some(a) = &p.arxiv {
            keys.push(format!("arxiv:{a}"));
        }
        for k in &keys {
            let found: Option<(i64, String)> =
                self.conn.query_row("SELECT id, data FROM research_papers WHERE key = ?1", [k], |r| Ok((r.get(0)?, r.get(1)?))).optional()?;
            if let Some((id, data)) = found {
                let mut old: Paper = serde_json::from_str(&data).map_err(|e| Error::invalid(e.to_string()))?;
                old.merge(p);
                // Another row may already hold the merged key (rare); keep the old key then.
                let key_free: bool = old.key == *k
                    || self.conn.query_row("SELECT 1 FROM research_papers WHERE key = ?1", [&old.key], |_| Ok(())).optional()?.is_none();
                if !key_free {
                    old.key = k.clone();
                }
                self.conn.execute(
                    "UPDATE research_papers SET key = ?2, data = ?3, seen_at = ?4 WHERE id = ?1",
                    params![id, old.key, serde_json::to_string(&old).expect("serialisable"), utc_str(now)],
                )?;
                return Ok(id);
            }
        }
        self.conn.execute(
            "INSERT INTO research_papers(key, data, seen_at) VALUES (?1, ?2, ?3)",
            params![p.key, serde_json::to_string(p).expect("serialisable"), utc_str(now)],
        )?;
        Ok(self.conn.last_insert_rowid())
    }

    pub fn research_paper(&self, id: i64) -> Result<Paper> {
        let data: String = self
            .conn
            .query_row("SELECT data FROM research_papers WHERE id = ?1", [id], |r| r.get(0))
            .optional()?
            .ok_or_else(|| Error::NotFound("paper".into()))?;
        serde_json::from_str(&data).map_err(|e| Error::invalid(e.to_string()))
    }

    fn replace_paper(&self, id: i64, p: &Paper) -> Result<()> {
        self.conn.execute("UPDATE research_papers SET data = ?2 WHERE id = ?1", params![id, serde_json::to_string(p).expect("serialisable")])?;
        Ok(())
    }

    fn paper_collections(&self) -> Result<HashMap<i64, Vec<i64>>> {
        let mut stmt = self.conn.prepare("SELECT paper_id, collection_id FROM research_collection_papers ORDER BY collection_id")?;
        let mut m: HashMap<i64, Vec<i64>> = HashMap::new();
        for r in stmt.query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)?)))? {
            let (p, c) = r?;
            m.entry(p).or_default().push(c);
        }
        Ok(m)
    }

    fn views_where(&self, clause: &str, args: &[&dyn rusqlite::ToSql]) -> Result<Vec<PaperView>> {
        let cols = self.paper_collections()?;
        let sql = format!("{VIEW_SQL} {clause}");
        let mut stmt = self.conn.prepare(&sql)?;
        let rows = stmt
            .query_map(args, |r| {
                let paper: Paper = conv(serde_json::from_str(&r.get::<_, String>(1)?).map_err(|e| Error::invalid(e.to_string())))?;
                let id: i64 = r.get(0)?;
                let saved: bool = r.get(2)?;
                let hit_read: Option<bool> = r.get(7)?;
                Ok(PaperView {
                    id,
                    paper,
                    saved,
                    read: if saved { r.get(3)? } else { hit_read.unwrap_or(true) },
                    is_new: hit_read == Some(false),
                    note: r.get(4)?,
                    subject: r.get(5)?,
                    collections: vec![],
                    task_card_id: r.get(6)?,
                    found_at: r.get::<_, Option<String>>(8)?.as_deref().and_then(parse_utc),
                })
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows
            .into_iter()
            .map(|mut v| {
                v.collections = cols.get(&v.id).cloned().unwrap_or_default();
                v
            })
            .collect())
    }

    pub fn paper_view(&self, id: i64) -> Result<PaperView> {
        self.views_where("WHERE p.id = ?1", &[&id])?.pop().ok_or_else(|| Error::NotFound("paper".into()))
    }

    /// Views for ids in the given order.
    pub fn paper_views(&self, ids: &[i64]) -> Result<Vec<PaperView>> {
        if ids.is_empty() {
            return Ok(vec![]);
        }
        let list = ids.iter().map(|i| i.to_string()).collect::<Vec<_>>().join(",");
        let found = self.views_where(&format!("WHERE p.id IN ({list})"), &[])?;
        let mut by_id: HashMap<i64, PaperView> = found.into_iter().map(|v| (v.id, v)).collect();
        Ok(ids.iter().filter_map(|i| by_id.remove(i)).collect())
    }

    /// Forgets papers nobody kept: not saved, not found by a topic, and not seen for a while.
    pub fn prune_research(&self, now: DateTime<Utc>) -> Result<usize> {
        let hit_cutoff = utc_str(now - Duration::days(HIT_DAYS));
        self.conn.execute(
            "DELETE FROM research_hits WHERE found_at < ?1 AND paper_id NOT IN (SELECT paper_id FROM research_library)",
            [hit_cutoff],
        )?;
        let cutoff = utc_str(now - Duration::days(KEEP_DAYS));
        Ok(self.conn.execute(
            "DELETE FROM research_papers WHERE seen_at < ?1 AND id NOT IN (SELECT paper_id FROM research_library) AND id NOT IN (SELECT paper_id FROM research_hits)",
            [cutoff],
        )?)
    }

    // ---- topics ----

    pub fn research_topics(&self) -> Result<Vec<Topic>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, name, config, enabled, created_at, last_checked, last_error FROM research_topics ORDER BY name COLLATE NOCASE",
        )?;
        let v = stmt.query_map([], topic_row)?.collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(v)
    }

    pub fn research_topic(&self, id: i64) -> Result<Topic> {
        self.conn
            .query_row(
                "SELECT id, name, config, enabled, created_at, last_checked, last_error FROM research_topics WHERE id = ?1",
                [id],
                topic_row,
            )
            .optional()?
            .ok_or_else(|| Error::NotFound("topic".into()))
    }

    pub fn create_research_topic(&self, input: &TopicInput, now: NaiveDateTime) -> Result<i64> {
        let (name, cfg) = checked_topic(input)?;
        self.conn.execute(
            "INSERT INTO research_topics(name, config, enabled, created_at) VALUES (?1, ?2, ?3, ?4)",
            params![name, serde_json::to_string(&cfg).expect("serialisable"), input.enabled, local_str(now)],
        )?;
        Ok(self.conn.last_insert_rowid())
    }

    /// Saves a topic. Changing what it looks for makes it due right away; papers that the new
    /// filters leave out are removed (saved ones stay in the library either way).
    pub fn update_research_topic(&self, id: i64, input: &TopicInput) -> Result<()> {
        let (name, cfg) = checked_topic(input)?;
        let old = self.research_topic(id)?;
        let changed = old.config.query != cfg.query;
        self.conn.execute(
            "UPDATE research_topics SET name = ?2, config = ?3, enabled = ?4, last_checked = CASE WHEN ?5 THEN NULL ELSE last_checked END WHERE id = ?1",
            params![id, name, serde_json::to_string(&cfg).expect("serialisable"), input.enabled, changed],
        )?;
        if changed {
            let blocked = self.research_blocked()?;
            for v in self.views_where("JOIN research_hits h2 ON h2.paper_id = p.id WHERE h2.topic_id = ?1", &[&id])? {
                if !passes(&cfg.query, &blocked, &v.paper) {
                    self.conn.execute("DELETE FROM research_hits WHERE topic_id = ?1 AND paper_id = ?2", params![id, v.id])?;
                }
            }
        }
        Ok(())
    }

    pub fn delete_research_topic(&self, id: i64) -> Result<()> {
        self.conn.execute("DELETE FROM research_topics WHERE id = ?1", [id])?;
        Ok(())
    }

    pub fn research_unread(&self) -> Result<HashMap<i64, i64>> {
        let mut stmt = self.conn.prepare("SELECT topic_id, COUNT(*) FROM research_hits WHERE is_read = 0 GROUP BY topic_id")?;
        let v = stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?.collect::<rusqlite::Result<HashMap<_, _>>>()?;
        Ok(v)
    }

    pub fn mark_research_topic_read(&self, topic_id: i64) -> Result<()> {
        self.conn.execute("UPDATE research_hits SET is_read = 1 WHERE topic_id = ?1", [topic_id])?;
        Ok(())
    }

    /// Records a check: stores the papers, adds the ones the topic hasn't had before.
    /// Returns the ids of the new ones.
    pub fn record_research_check(
        &mut self,
        topic_id: i64,
        found: &[Paper],
        error: Option<&str>,
        now_utc: DateTime<Utc>,
        now_local: NaiveDateTime,
    ) -> Result<Vec<i64>> {
        let mut added = Vec::new();
        for p in found {
            let pid = self.upsert_paper(p, now_utc)?;
            let n = self.conn.execute(
                "INSERT OR IGNORE INTO research_hits(topic_id, paper_id, found_at) VALUES (?1, ?2, ?3)",
                params![topic_id, pid, utc_str(now_utc)],
            )?;
            if n > 0 {
                added.push(pid);
            }
        }
        self.conn.execute(
            "UPDATE research_topics SET last_checked = ?2, last_error = ?3 WHERE id = ?1",
            params![topic_id, local_str(now_local), error],
        )?;
        Ok(added)
    }

    // ---- reading state ----

    /// Marks a paper read (or unread) wherever it appears.
    pub fn set_paper_read(&self, paper_id: i64, read: bool) -> Result<()> {
        self.conn.execute("UPDATE research_hits SET is_read = ?2 WHERE paper_id = ?1", params![paper_id, read])?;
        self.conn.execute("UPDATE research_library SET is_read = ?2 WHERE paper_id = ?1", params![paper_id, read])?;
        Ok(())
    }

    // ---- library ----

    pub fn set_paper_saved(&self, paper_id: i64, saved: bool, now: DateTime<Utc>) -> Result<()> {
        if saved {
            self.research_paper(paper_id)?;
            let read: bool = self
                .conn
                .query_row("SELECT COALESCE(MIN(is_read), 0) FROM research_hits WHERE paper_id = ?1", [paper_id], |r| r.get(0))
                .unwrap_or(false);
            self.conn.execute(
                "INSERT OR IGNORE INTO research_library(paper_id, added_at, is_read) VALUES (?1, ?2, ?3)",
                params![paper_id, utc_str(now), read],
            )?;
        } else {
            self.conn.execute("DELETE FROM research_library WHERE paper_id = ?1", [paper_id])?;
        }
        Ok(())
    }

    pub fn set_paper_note(&self, paper_id: i64, note: &str, now: DateTime<Utc>) -> Result<()> {
        self.set_paper_saved(paper_id, true, now)?;
        let note: String = note.chars().take(20_000).collect();
        self.conn.execute("UPDATE research_library SET note = ?2 WHERE paper_id = ?1", params![paper_id, note])?;
        Ok(())
    }

    /// Links a paper to a class subject (or unlinks it with an empty subject).
    pub fn set_paper_subject(&self, paper_id: i64, subject: &str, now: DateTime<Utc>) -> Result<()> {
        let subject = subject.trim();
        if !subject.is_empty() {
            self.set_paper_saved(paper_id, true, now)?;
        }
        self.conn.execute("UPDATE research_library SET subject = ?2 WHERE paper_id = ?1", params![paper_id, subject])?;
        Ok(())
    }

    pub fn set_paper_collections(&self, paper_id: i64, collections: &[i64], now: DateTime<Utc>) -> Result<()> {
        if !collections.is_empty() {
            self.set_paper_saved(paper_id, true, now)?;
        }
        self.conn.execute("DELETE FROM research_collection_papers WHERE paper_id = ?1", [paper_id])?;
        for c in collections {
            self.conn.execute(
                "INSERT OR IGNORE INTO research_collection_papers(collection_id, paper_id) SELECT ?1, ?2 WHERE EXISTS (SELECT 1 FROM research_collections WHERE id = ?1)",
                params![c, paper_id],
            )?;
        }
        Ok(())
    }

    pub fn research_collections(&self) -> Result<Vec<Collection>> {
        let mut stmt = self.conn.prepare(
            "SELECT c.id, c.name, (SELECT COUNT(*) FROM research_collection_papers cp WHERE cp.collection_id = c.id)
             FROM research_collections c ORDER BY c.position, c.name COLLATE NOCASE",
        )?;
        let v = stmt.query_map([], |r| Ok(Collection { id: r.get(0)?, name: r.get(1)?, papers: r.get(2)? }))?.collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(v)
    }

    pub fn save_collection(&self, id: Option<i64>, name: &str) -> Result<i64> {
        let name = name.trim();
        if name.is_empty() {
            return Err(Error::invalid("Give the collection a name."));
        }
        let name: String = name.chars().take(80).collect();
        match id {
            Some(id) => {
                self.conn.execute("UPDATE research_collections SET name = ?2 WHERE id = ?1", params![id, name])?;
                Ok(id)
            }
            None => {
                let pos: i64 = self.conn.query_row("SELECT COALESCE(MAX(position), -1) + 1 FROM research_collections", [], |r| r.get(0))?;
                self.conn.execute("INSERT INTO research_collections(name, position) VALUES (?1, ?2)", params![name, pos])?;
                Ok(self.conn.last_insert_rowid())
            }
        }
    }

    pub fn delete_collection(&self, id: i64) -> Result<()> {
        self.conn.execute("DELETE FROM research_collections WHERE id = ?1", [id])?;
        Ok(())
    }

    /// Subjects in the class timetable, newest semester first.
    pub fn class_subjects(&self) -> Result<Vec<String>> {
        let mut stmt =
            self.conn.prepare("SELECT c.subject, MAX(s.start) AS st FROM classes c JOIN semesters s ON s.id = c.semester_id GROUP BY c.subject ORDER BY st DESC, c.subject")?;
        let v = stmt.query_map([], |r| r.get::<_, String>(0))?.collect::<rusqlite::Result<Vec<_>>>()?;
        let mut seen = HashSet::new();
        Ok(v.into_iter().map(|s| s.trim().to_string()).filter(|s| !s.is_empty() && seen.insert(s.to_lowercase())).collect())
    }

    pub fn link_task_card(&self, paper_id: i64, card_id: i64) -> Result<()> {
        self.conn.execute("UPDATE research_library SET task_card_id = ?2 WHERE paper_id = ?1", params![paper_id, card_id])?;
        Ok(())
    }

    // ---- blocked journals and websites ----

    pub fn research_blocked(&self) -> Result<Vec<String>> {
        let mut stmt = self.conn.prepare("SELECT value FROM research_blocked ORDER BY value COLLATE NOCASE")?;
        let v = stmt.query_map([], |r| r.get(0))?.collect::<rusqlite::Result<Vec<String>>>()?;
        Ok(v)
    }

    /// Leaves a journal or website out of every search and topic, and removes papers from
    /// it that topics found (saved ones stay). Returns what was blocked.
    pub fn block_research(&self, input: &str) -> Result<String> {
        let value = blocked_value(input);
        if value.chars().count() < 2 {
            return Err(Error::invalid("Type a journal name or a website, like mdpi.com."));
        }
        self.conn.execute("INSERT OR IGNORE INTO research_blocked(value) VALUES (?1)", [&value])?;
        let list = vec![value.clone()];
        for v in self.views_where("WHERE p.id IN (SELECT paper_id FROM research_hits) AND l.paper_id IS NULL", &[])? {
            if is_blocked(&v.paper, &list) {
                self.conn.execute("DELETE FROM research_hits WHERE paper_id = ?1", [v.id])?;
            }
        }
        Ok(value)
    }

    pub fn unblock_research(&self, value: &str) -> Result<()> {
        self.conn.execute("DELETE FROM research_blocked WHERE value = ?1", [value])?;
        Ok(())
    }

    // ---- read texts ----

    fn cached_text(&self, paper_id: i64) -> Result<Option<(Option<Article>, Option<String>)>> {
        let row: Option<(Option<String>, Option<String>)> = self
            .conn
            .query_row("SELECT article, error FROM research_texts WHERE paper_id = ?1", [paper_id], |r| Ok((r.get(0)?, r.get(1)?)))
            .optional()?;
        Ok(row.map(|(a, e)| (a.and_then(|a| serde_json::from_str::<Article>(&a).ok()), e)))
    }

    fn save_text(&self, paper_id: i64, result: &std::result::Result<Article, String>, now: DateTime<Utc>) -> Result<()> {
        let (a, e) = match result {
            Ok(a) => (Some(serde_json::to_string(a).expect("serialisable")), None),
            Err(e) => (None, Some(e.clone())),
        };
        self.conn.execute(
            "INSERT INTO research_texts(paper_id, article, error, fetched_at) VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT(paper_id) DO UPDATE SET article = excluded.article, error = excluded.error, fetched_at = excluded.fetched_at",
            params![paper_id, a, e, utc_str(now)],
        )?;
        Ok(())
    }
}

// ---------- searching ----------

/// Runs a search across the chosen databases and stores the papers so they can be opened,
/// saved and cited. The store is only locked for the database work.
pub fn search(store: &Mutex<Store>, fetcher: &dyn Fetch, query: &Query, now: DateTime<Utc>) -> Result<SearchResult> {
    let q = query.clone().cleaned();
    q.validate()?;
    let (keys, blocked) = {
        let s = lock(store)?;
        (s.research_settings()?.keys, s.research_blocked()?)
    };
    let (lists, errors) = sources::search_all(fetcher, &q, &keys, q.sort);
    let counts = lists.iter().map(|(s, v)| (*s, v.len())).collect();
    let asked = q.useful_sources();
    let merged: Vec<Paper> = merge_results(lists, q.sort).into_iter().filter(|p| passes(&q, &blocked, p)).collect();
    let s = lock(store)?;
    let ids = merged.iter().map(|p| s.upsert_paper(p, now)).collect::<Result<Vec<_>>>()?;
    let papers = s.paper_views(&ids)?;
    if papers.is_empty() && !errors.is_empty() && errors.len() >= asked.len() {
        return Err(Error::invalid(errors.join(" ")));
    }
    Ok(SearchResult { papers, errors, asked, counts })
}

/// Papers citing, cited by, or similar to a stored paper.
pub fn explore(store: &Mutex<Store>, fetcher: &dyn Fetch, paper_id: i64, kind: Explore, now: DateTime<Utc>) -> Result<Vec<PaperView>> {
    let (paper, keys, blocked) = {
        let s = lock(store)?;
        (s.research_paper(paper_id)?, s.research_settings()?.keys, s.research_blocked()?)
    };
    let found = sources::explore(fetcher, &paper, kind, &keys)?;
    let q = Query::simple(&paper.title);
    let found: Vec<Paper> = merge_results(vec![(Source::SemanticScholar, found)], if kind == Explore::Similar { SortBy::Relevance } else { SortBy::MostCited })
        .into_iter()
        .filter(|p| passes(&q, &blocked, p) && p.key != paper.key)
        .collect();
    let s = lock(store)?;
    let ids = found.iter().map(|p| s.upsert_paper(p, now)).collect::<Result<Vec<_>>>()?;
    s.paper_views(&ids)
}

pub fn overview(fetcher: &dyn Fetch, text: &str) -> Result<Option<Overview>> {
    if text.trim().chars().count() < 2 {
        return Ok(None);
    }
    sources::overview(fetcher, text.trim())
}

// ---------- topics ----------

/// Repeats a topic's search, newest first, and keeps what's new.
pub fn check_topic(store: &Mutex<Store>, topic_id: i64, fetcher: &dyn Fetch, now_utc: DateTime<Utc>, now_local: NaiveDateTime) -> Result<CheckOutcome> {
    let (topic, keys, blocked) = {
        let s = lock(store)?;
        (s.research_topic(topic_id)?, s.research_settings()?.keys, s.research_blocked()?)
    };
    let q = &topic.config.query;
    let (lists, errors) = sources::search_all(fetcher, q, &keys, SortBy::Newest);
    let asked = q.useful_sources().len();
    let found: Vec<Paper> = merge_results(lists, SortBy::Newest).into_iter().filter(|p| passes(q, &blocked, p)).collect();
    let error = (!errors.is_empty()).then(|| if errors.len() >= asked { errors.join(" ") } else { format!("Some databases didn't answer. {}", errors.join(" ")) });
    let first = topic.last_checked.is_none();
    let added = lock(store)?.record_research_check(topic_id, &found, error.as_deref(), now_utc, now_local)?;
    let s = lock(store)?;
    let mut notify = if first || !topic.config.notify { vec![] } else { s.paper_views(&added)? };
    let notify_more = notify.len().saturating_sub(MAX_NOTIFICATIONS);
    notify.truncate(MAX_NOTIFICATIONS);
    Ok(CheckOutcome { topic_id, topic_name: topic.name, added: added.len(), error, notify, notify_more })
}

pub fn due_topics(store: &Store, now: NaiveDateTime) -> Result<Vec<i64>> {
    Ok(store.research_topics()?.into_iter().filter(|t| t.is_due(now)).map(|t| t.id).collect())
}

fn digest(papers: &[PaperView], now: DateTime<Utc>) -> Option<Digest> {
    let week = now - Duration::days(7);
    let recent: Vec<&PaperView> = papers.iter().filter(|p| p.found_at.is_some_and(|f| f >= week)).collect();
    if recent.is_empty() {
        return None;
    }
    let dp = |v: &PaperView| DigestPaper { id: v.id, title: v.paper.title.clone(), citations: v.paper.citations, date: v.paper.date, venue: v.paper.venue.clone() };
    let mut venues: HashMap<String, (String, usize)> = HashMap::new();
    for v in &recent {
        if !v.paper.venue.trim().is_empty() {
            let e = venues.entry(crate::news::normalize(&v.paper.venue)).or_insert((v.paper.venue.clone(), 0));
            e.1 += 1;
        }
    }
    let busiest = venues.values().filter(|(_, n)| *n >= 2).max_by(|a, b| a.1.cmp(&b.1).then(b.0.cmp(&a.0))).cloned();
    let most_cited = recent.iter().filter(|v| v.paper.citations.unwrap_or(0) > 0).max_by_key(|v| v.paper.citations.unwrap_or(0)).map(|v| dp(v));
    let newest = recent.iter().filter(|v| v.paper.date.is_some()).max_by_key(|v| v.paper.date).map(|v| dp(v));
    let mut kw: HashMap<String, (String, usize)> = HashMap::new();
    for v in &recent {
        let mut seen = HashSet::new();
        for k in &v.paper.keywords {
            let n = crate::news::normalize(k);
            // arXiv category codes and catalogue marks aren't keywords.
            if n.contains('.') && !n.contains(' ') || k.chars().all(|c| c.is_uppercase() || !c.is_alphabetic()) || !seen.insert(n.clone()) {
                continue;
            }
            kw.entry(n).or_insert((k.clone(), 0)).1 += 1;
        }
    }
    let mut keywords: Vec<(String, usize)> = kw.into_values().filter(|(_, n)| *n >= 2).collect();
    keywords.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
    Some(Digest {
        papers: recent.len(),
        journals: venues.len(),
        open_access: recent.iter().filter(|v| v.paper.open_access).count(),
        most_cited,
        newest,
        busiest,
        keywords: keywords.into_iter().take(6).map(|k| k.0).collect(),
    })
}

pub fn topics_view(store: &Store, topic_id: Option<i64>, filter: Filter, now: DateTime<Utc>) -> Result<TopicsView> {
    let unread = store.research_unread()?;
    let topics: Vec<TopicSummary> = store
        .research_topics()?
        .into_iter()
        .map(|t| TopicSummary { unread: unread.get(&t.id).copied().unwrap_or(0), next_check: t.enabled.then(|| t.next_check()).flatten(), topic: t })
        .collect();
    let topic = topic_id
        .and_then(|id| topics.iter().find(|t| t.topic.id == id))
        .or_else(|| topics.iter().find(|t| t.unread > 0))
        .or_else(|| topics.iter().find(|t| t.topic.enabled))
        .or(topics.first())
        .cloned();
    let mut papers = Vec::new();
    let mut dg = None;
    if let Some(t) = &topic {
        let id = t.topic.id;
        // This topic's own read state, newest finds first, then newest publications.
        let mut list = store.views_where(
            "JOIN research_hits th ON th.paper_id = p.id AND th.topic_id = ?1 ORDER BY th.found_at DESC, p.id DESC LIMIT 500",
            &[&id],
        )?;
        let reads: HashMap<i64, bool> = {
            let mut stmt = store.conn.prepare("SELECT paper_id, is_read FROM research_hits WHERE topic_id = ?1")?;
            let v = stmt.query_map([id], |r| Ok((r.get(0)?, r.get(1)?)))?.collect::<rusqlite::Result<HashMap<_, _>>>()?;
            v
        };
        for v in &mut list {
            let r = reads.get(&v.id).copied().unwrap_or(true);
            v.is_new = !r;
            if !v.saved {
                v.read = r;
            }
        }
        // Within one check, the newest publication first.
        list.sort_by(|a, b| {
            let day = |v: &PaperView| v.found_at.map(|f| f.date_naive());
            day(b).cmp(&day(a)).then(b.paper.date.or_else(|| b.paper.year.and_then(|y| NaiveDate::from_ymd_opt(y, 1, 1))).cmp(&a.paper.date.or_else(|| a.paper.year.and_then(|y| NaiveDate::from_ymd_opt(y, 1, 1)))))
        });
        dg = digest(&list, now);
        list.retain(|v| match filter {
            Filter::All => true,
            Filter::Unread => v.is_new,
            Filter::Saved => v.saved,
        });
        papers = list;
    }
    Ok(TopicsView { topics, topic, papers, filter, digest: dg })
}

pub fn library_view(store: &Store, shelf: Shelf) -> Result<LibraryView> {
    let all = store.views_where("WHERE l.paper_id IS NOT NULL ORDER BY l.added_at DESC, p.id DESC", &[])?;
    let total = all.len() as i64;
    let unread = all.iter().filter(|v| !v.read).count() as i64;
    let mut subjects: Vec<(String, i64)> = Vec::new();
    for v in &all {
        if v.subject.is_empty() {
            continue;
        }
        match subjects.iter_mut().find(|(s, _)| s == &v.subject) {
            Some(e) => e.1 += 1,
            None => subjects.push((v.subject.clone(), 1)),
        }
    }
    subjects.sort_by(|a, b| a.0.to_lowercase().cmp(&b.0.to_lowercase()));
    let papers = all
        .into_iter()
        .filter(|v| match &shelf {
            Shelf::All => true,
            Shelf::Unread => !v.read,
            Shelf::Collection { id } => v.collections.contains(id),
            Shelf::Subject { name } => &v.subject == name,
        })
        .collect();
    Ok(LibraryView { shelf, papers, collections: store.research_collections()?, subjects, class_subjects: store.class_subjects()?, total, unread })
}

// ---------- reading ----------

pub fn citations(p: &Paper, today: NaiveDate) -> Citations {
    Citations { gost: cite::gost(p, today), apa: cite::apa(p), bibtex: cite::bibtex(p) }
}

/// Opens a paper: the full text from its web page when there is one, details filled in from
/// the page's tags, and a free PDF looked up by DOI. Marks it read.
pub fn read_paper(store: &Mutex<Store>, paper_id: i64, fetcher: &dyn Fetch, now: DateTime<Utc>, today: NaiveDate, refresh: bool) -> Result<Reading> {
    let (mut paper, cached, keys) = {
        let s = lock(store)?;
        (s.research_paper(paper_id)?, s.cached_text(paper_id)?, s.research_settings()?.keys)
    };
    let (article, text_error) = match cached {
        Some((a, e)) if !refresh && (a.is_some() || e.is_some()) => (a, e),
        _ => {
            let before = paper.clone();
            let page = paper.html_url.clone().or_else(|| paper.open_access.then(|| paper.url.clone()).filter(|u| !u.is_empty() && !u.contains("doi.org/")));
            let result: std::result::Result<Article, String> = match page {
                None => Err("There's no free full text of this paper on the web. The abstract and the publisher’s page are below.".into()),
                Some(u) => match fetcher.get_page(&u) {
                    Err(e) => Err(e.to_string()),
                    Ok(pg) => {
                        let ct = pg.content_type.to_lowercase();
                        if !ct.is_empty() && !ct.contains("html") {
                            Err("The paper's page isn't a web page (it may be a PDF).".into())
                        } else {
                            let html = crate::news::article::decode_page(&pg);
                            sources::enrich_from_meta(&mut paper, &sources::citation_meta(&html));
                            let base = if pg.final_url.is_empty() { u.clone() } else { pg.final_url.clone() };
                            match extract(&html, &base) {
                                // A page that only shows the abstract isn't the full text.
                                Ok(a) if a.words < 400 && !paper.abstract_text.is_empty() => Err("The paper's page only shows the abstract.".into()),
                                Ok(a) => Ok(a),
                                Err(e) => Err(e.to_string()),
                            }
                        }
                    }
                },
            };
            if paper.pdf_url.is_none() {
                if let Some(doi) = paper.doi.clone() {
                    if let Ok(Some(pdf)) = sources::unpaywall_pdf(fetcher, &doi, &keys.email) {
                        paper.pdf_url = Some(pdf);
                        paper.open_access = true;
                    }
                }
            }
            let s = lock(store)?;
            if paper != before {
                s.replace_paper(paper_id, &paper)?;
            }
            s.save_text(paper_id, &result, now)?;
            match result {
                Ok(a) => (Some(a), None),
                Err(e) => (None, Some(e)),
            }
        }
    };
    let s = lock(store)?;
    s.set_paper_read(paper_id, true)?;
    let view = s.paper_view(paper_id)?;
    let citations = citations(&view.paper, today);
    Ok(Reading { paper: view, article, text_error, citations })
}

/// Downloads a paper's free PDF. Some publishers answer a PDF link with a web page; that's
/// reported rather than shown as a broken document.
pub fn fetch_pdf(fetcher: &dyn Fetch, url: &str) -> Result<Vec<u8>> {
    let page = fetcher.get_file(url)?;
    if page.bytes.starts_with(b"%PDF") {
        Ok(page.bytes)
    } else {
        Err(Error::invalid("The publisher shows this PDF only on its own website. Open it in the browser instead."))
    }
}

// ---------- tasks ----------

/// Puts "Read: …" on a task board, in its first list that isn't a done list, with the
/// reference and link in the description. The paper is saved to the library too.
pub fn add_to_tasks(store: &Store, paper_id: i64, board_id: i64, now_utc: DateTime<Utc>, now_local: NaiveDateTime) -> Result<i64> {
    let p = store.research_paper(paper_id)?;
    let lists = store.task_lists(board_id)?;
    let list = lists.iter().find(|l| !l.is_done).or(lists.first()).ok_or_else(|| Error::invalid("That board has no lists yet."))?;
    let title: String = format!("Read: {}", p.title).chars().take(200).collect();
    let mut description = cite::gost(&p, now_local.date());
    if !p.url.is_empty() {
        description.push_str(&format!("\n\n{}", p.url));
    }
    let input = CardInput { title, description, labels: vec![], due: None, checklist: vec![], salary: None };
    let card = store.add_task_card(list.id, &input, now_local)?;
    store.set_paper_saved(paper_id, true, now_utc)?;
    store.link_task_card(paper_id, card)?;
    Ok(card)
}

/// Ends of day used by views that count "this week".
pub fn midnight(d: NaiveDate) -> NaiveDateTime {
    d.and_time(NaiveTime::MIN)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::news::feed::Page;

    const OPENALEX: &str = include_str!("testdata/openalex.json");
    const ARXIV: &str = include_str!("testdata/arxiv.xml");
    const S2: &str = include_str!("testdata/s2.json");
    const EPMC: &str = include_str!("testdata/europepmc.json");
    const CL: &str = include_str!("testdata/cyberleninka.json");

    struct Canned {
        fail_s2: bool,
    }
    impl Fetch for Canned {
        fn get(&self, url: &str) -> Result<Vec<u8>> {
            self.get_api(url, &[])
        }
        fn get_api(&self, url: &str, _h: &[(&str, &str)]) -> Result<Vec<u8>> {
            let body = if url.contains("api.openalex.org") {
                OPENALEX
            } else if url.contains("export.arxiv.org") {
                ARXIV
            } else if url.contains("semanticscholar") {
                if self.fail_s2 {
                    return Err(Error::invalid("api.semanticscholar.org asked us to slow down; trying again later."));
                }
                S2
            } else if url.contains("europepmc") {
                EPMC
            } else {
                return Err(Error::invalid("offline"));
            };
            Ok(body.as_bytes().to_vec())
        }
        fn post_json(&self, url: &str, _b: &str) -> Result<Vec<u8>> {
            assert_eq!(url, sources::CYBERLENINKA_SEARCH);
            Ok(CL.as_bytes().to_vec())
        }
        fn get_page(&self, url: &str) -> Result<Page> {
            if url.contains("cyberleninka.ru/article") {
                let body = format!(
                    r#"<html><head><meta name="citation_issue" content="2 (18)"><meta name="citation_firstpage" content="10"><meta name="citation_lastpage" content="17"><title>X</title></head><body><div class="ocr" itemprop="articleBody">{}</div></body></html>"#,
                    "<p>Бетон с добавкой графена становится прочнее, и это подтверждено испытаниями на сжатие и изгиб в течение двадцати восьми суток твердения образцов.</p>".repeat(30)
                );
                Ok(Page { bytes: body.into_bytes(), content_type: "text/html; charset=utf-8".into(), final_url: url.into() })
            } else {
                Err(Error::invalid("offline"))
            }
        }
    }

    fn now() -> DateTime<Utc> {
        DateTime::parse_from_rfc3339("2026-11-17T09:00:00Z").unwrap().with_timezone(&Utc)
    }
    fn local() -> NaiveDateTime {
        NaiveDate::from_ymd_opt(2026, 11, 17).unwrap().and_hms_opt(12, 0, 0).unwrap()
    }

    #[test]
    fn search_merges_every_database_and_survives_one_failing() {
        let store = Mutex::new(Store::open_in_memory().unwrap());
        let r = search(&store, &Canned { fail_s2: true }, &Query::simple("graphene"), now()).unwrap();
        assert_eq!(r.asked.len(), 5);
        assert_eq!(r.errors, vec!["Semantic Scholar is busy right now; try again in a minute.".to_string()]);
        // OpenAlex 2 + arXiv 2 + Europe PMC 2 + CyberLeninka 3, and the Russian paper is in
        // both OpenAlex and CyberLeninka.
        assert_eq!(r.papers.len(), 8, "{:?}", r.papers.iter().map(|p| &p.paper.title).collect::<Vec<_>>());
        let ru = r.papers.iter().find(|p| p.paper.title.starts_with("Модифицирование")).unwrap();
        assert!(ru.paper.sources.contains(&Source::OpenAlex) && ru.paper.sources.contains(&Source::CyberLeninka));
        // Searching again doesn't duplicate stored papers.
        let again = search(&store, &Canned { fail_s2: false }, &Query::simple("graphene"), now()).unwrap();
        let n: i64 = store.lock().unwrap().conn.query_row("SELECT COUNT(*) FROM research_papers", [], |r| r.get(0)).unwrap();
        assert_eq!(n as usize, again.papers.len());
        let carbon = again.papers.iter().find(|p| p.paper.doi.as_deref() == Some("10.1016/j.carbon.2020.01.002")).unwrap();
        assert_eq!(carbon.paper.citations, Some(45));
        assert!(carbon.paper.tldr.is_some());
    }

    #[test]
    fn blocked_journals_and_filters_apply() {
        let store = Mutex::new(Store::open_in_memory().unwrap());
        store.lock().unwrap().block_research("Carbon").unwrap();
        store.lock().unwrap().block_research("https://cyberleninka.ru/").unwrap();
        let mut q = Query::simple("graphene");
        q.from_year = Some(2026);
        let r = search(&store, &Canned { fail_s2: false }, &q, now()).unwrap();
        assert!(r.papers.iter().all(|p| p.paper.venue != "Carbon" && !p.paper.url.contains("cyberleninka")));
        assert!(r.papers.iter().all(|p| p.paper.year.unwrap_or(2026) >= 2026));
        assert_eq!(store.lock().unwrap().research_blocked().unwrap(), vec!["Carbon".to_string(), "cyberleninka.ru".to_string()]);
    }

    #[test]
    fn topics_keep_what_is_new_and_summarise_the_week() {
        let mut s = Store::open_in_memory().unwrap();
        let input = TopicInput { name: String::new(), config: TopicConfig { query: Query::simple("graphene"), interval_minutes: 1440, notify: true }, enabled: true };
        let id = s.create_research_topic(&input, local()).unwrap();
        assert_eq!(s.research_topic(id).unwrap().name, "graphene");
        s.save_research_settings(&Settings::default()).unwrap();
        let _ = &mut s;
        let store = Mutex::new(s);
        let out = check_topic(&store, id, &Canned { fail_s2: false }, now(), local()).unwrap();
        assert!(out.added >= 8);
        assert!(out.notify.is_empty(), "the first check doesn't notify");
        let out2 = check_topic(&store, id, &Canned { fail_s2: false }, now(), local()).unwrap();
        assert_eq!(out2.added, 0);
        let s = store.lock().unwrap();
        let v = topics_view(&s, Some(id), Filter::All, now()).unwrap();
        assert_eq!(v.topics[0].unread as usize, out.added);
        let d = v.digest.unwrap();
        assert_eq!(d.papers, out.added);
        assert_eq!(d.most_cited.unwrap().citations, Some(45));
        assert!(!s.research_topic(id).unwrap().is_due(local()));
        s.set_paper_read(v.papers[0].id, true).unwrap();
        assert_eq!(topics_view(&s, Some(id), Filter::Unread, now()).unwrap().papers.len(), out.added - 1);
        // A narrower query drops what no longer fits.
        let mut narrower = input.clone();
        narrower.config.query.language = Some("ru".into());
        s.update_research_topic(id, &narrower).unwrap();
        let v = topics_view(&s, Some(id), Filter::All, now()).unwrap();
        assert!(v.papers.iter().all(|p| p.paper.language.as_deref() == Some("ru")));
        assert!(s.research_topic(id).unwrap().is_due(local()));
    }

    #[test]
    fn library_notes_collections_subjects_and_tasks() {
        let store = Mutex::new(Store::open_in_memory().unwrap());
        let r = search(&store, &Canned { fail_s2: false }, &Query::simple("graphene"), now()).unwrap();
        let pid = r.papers.iter().find(|p| p.paper.url.contains("cyberleninka")).unwrap().id;
        let s = store.lock().unwrap();
        let c = s.save_collection(None, "Курсовая").unwrap();
        s.set_paper_collections(pid, &[c], now()).unwrap();
        s.set_paper_note(pid, "Таблица 2 — данные для главы 1", now()).unwrap();
        s.set_paper_subject(pid, "Строительные материалы", now()).unwrap();
        let v = library_view(&s, Shelf::Collection { id: c }).unwrap();
        assert_eq!(v.papers.len(), 1);
        assert_eq!(v.papers[0].note, "Таблица 2 — данные для главы 1");
        assert_eq!(v.subjects, vec![("Строительные материалы".to_string(), 1)]);
        assert_eq!(v.collections[0].papers, 1);
        assert_eq!(library_view(&s, Shelf::Unread).unwrap().papers.len(), 1);
        let board = s
            .create_task_board(&crate::tasks::BoardInput { name: "Учёба".into(), color: "#8AA7D9".into(), commission: 10.0, labels: vec![] }, local())
            .unwrap();
        let card = add_to_tasks(&s, pid, board, now(), local()).unwrap();
        let cd = s.task_card(card).unwrap();
        assert!(cd.title.starts_with("Read: Модифицирование"));
        assert!(cd.description.contains("// Journal of Advanced Materials and Technologies"));
        assert_eq!(s.paper_view(pid).unwrap().task_card_id, Some(card));
        s.set_paper_saved(pid, false, now()).unwrap();
        assert_eq!(library_view(&s, Shelf::All).unwrap().total, 0);
        assert_eq!(s.research_collections().unwrap()[0].papers, 0);
    }

    #[test]
    fn reading_fetches_full_text_and_fills_details() {
        let store = Mutex::new(Store::open_in_memory().unwrap());
        let r = search(&store, &Canned { fail_s2: false }, &Query::simple("графен"), now()).unwrap();
        let pid = r.papers.iter().find(|p| p.paper.url.contains("cyberleninka")).unwrap().id;
        let today = NaiveDate::from_ymd_opt(2026, 11, 17).unwrap();
        let rd = read_paper(&store, pid, &Canned { fail_s2: false }, now(), today, false).unwrap();
        assert!(rd.article.is_some(), "{:?}", rd.text_error);
        assert_eq!(rd.paper.paper.pages, "10–17");
        assert!(rd.citations.gost.contains("С. 10–17"));
        assert!(rd.paper.read);
        // Papers without a free web page say so instead of failing.
        let other = r.papers.iter().find(|p| p.paper.html_url.is_none() && !p.paper.open_access).map(|p| p.id);
        if let Some(o) = other {
            let rd = read_paper(&store, o, &Canned { fail_s2: false }, now(), today, false).unwrap();
            assert!(rd.article.is_none() && rd.text_error.is_some());
        }
    }

    #[test]
    fn pdfs_must_be_pdfs() {
        struct F(&'static [u8]);
        impl Fetch for F {
            fn get(&self, _: &str) -> Result<Vec<u8>> {
                Ok(self.0.to_vec())
            }
        }
        assert!(fetch_pdf(&F(b"%PDF-1.7 ..."), "https://x/a.pdf").is_ok());
        assert!(fetch_pdf(&F(b"<html>"), "https://x/a.pdf").is_err());
    }

    #[test]
    fn old_unkept_papers_are_forgotten() {
        let store = Mutex::new(Store::open_in_memory().unwrap());
        let r = search(&store, &Canned { fail_s2: false }, &Query::simple("graphene"), now()).unwrap();
        let s = store.lock().unwrap();
        s.set_paper_saved(r.papers[0].id, true, now()).unwrap();
        let removed = s.prune_research(now() + Duration::days(KEEP_DAYS + 1)).unwrap();
        assert_eq!(removed, r.papers.len() - 1);
        assert!(s.paper_view(r.papers[0].id).is_ok());
    }
}

#[cfg(test)]
mod live {
    use super::*;

    /// Asks the real databases. Run with `cargo test -p cocoadesk-core live_research -- --ignored --nocapture`.
    #[test]
    #[ignore]
    fn live_research() {
        let store = Mutex::new(Store::open_in_memory().unwrap());
        let f = crate::news::HttpFetcher::new();
        for text in ["графен бетон", "graphene concrete"] {
            match search(&store, &f, &Query::simple(text), Utc::now()) {
                Ok(r) => {
                    println!("== {text}: {} papers, counts {:?}, errors {:?}", r.papers.len(), r.counts, r.errors);
                    for p in r.papers.iter().take(5) {
                        println!("  [{:?}] {} ({:?}) {}", p.paper.sources, p.paper.title, p.paper.year, p.paper.venue);
                    }
                    if let Some(p) = r.papers.iter().find(|p| p.paper.html_url.is_some()) {
                        let rd = read_paper(&store, p.id, &f, Utc::now(), Utc::now().date_naive(), false).unwrap();
                        println!("  read {}: words={:?} err={:?}\n  {}", p.paper.title, rd.article.as_ref().map(|a| a.words), rd.text_error, rd.citations.gost);
                    }
                }
                Err(e) => println!("== {text}: {e}"),
            }
            println!("overview: {:?}", overview(&f, text).map(|o| o.map(|o| (o.title, o.extract.chars().take(80).collect::<String>()))));
        }
    }
}
