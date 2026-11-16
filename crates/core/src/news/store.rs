//! News storage, checking and the views the News screen shows.

use std::collections::{HashMap, HashSet};

use chrono::{DateTime, Duration, NaiveDateTime, Utc};
use rusqlite::{params, OptionalExtension, Row};
use serde::{Deserialize, Serialize};

use super::article::{fetch_article, Article};
use super::feed::{parse_feed, search_url, Fetch};
use super::*;
use crate::store::Store;
use crate::{Error, Result};

const STAMP: &str = "%Y-%m-%dT%H:%M:%S";
/// Stories older than this are not fetched, and unsaved ones are deleted after it.
pub const RETENTION_DAYS: i64 = 30;
/// At most this many notifications per check; the rest are summarised.
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

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Filter {
    All,
    Unread,
    Saved,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TopicSummary {
    #[serde(flatten)]
    pub topic: Topic,
    pub unread: i64,
    pub next_check: Option<NaiveDateTime>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Story {
    /// The newest article of the story.
    pub lead: NewsItem,
    /// The same story from other websites.
    pub also: Vec<NewsItem>,
    pub unread: bool,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NewsView {
    pub topics: Vec<TopicSummary>,
    pub topic: Option<TopicSummary>,
    pub stories: Vec<Story>,
    pub filter: Filter,
    pub blocked_sites: Vec<String>,
    pub now: NaiveDateTime,
    pub digest: Option<Digest>,
}

/// The day so far for one topic, put together from counts — no text is generated.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Digest {
    /// New stories today (a story covered by several websites counts once).
    pub stories: usize,
    /// Different websites they came from.
    pub websites: usize,
    /// The most covered stories, most websites first.
    pub top: Vec<DigestStory>,
    /// Names that come up in several of today's headlines, most frequent first.
    pub names: Vec<String>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DigestStory {
    pub item_id: i64,
    pub title: String,
    pub websites: usize,
    /// The article's opening sentences, when CocoaDesk has read it.
    pub lead: Option<String>,
}

/// An article as stored for reading inside CocoaDesk.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Reading {
    pub item: NewsItem,
    pub article: Option<Article>,
    pub error: Option<String>,
    /// The same story from other websites.
    pub also: Vec<NewsItem>,
}

/// The result of checking one topic.
#[derive(Clone, Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CheckOutcome {
    pub topic_id: i64,
    pub topic_name: String,
    pub added: usize,
    pub error: Option<String>,
    /// New stories that should be announced, at most [`MAX_NOTIFICATIONS`].
    pub notify: Vec<NewsItem>,
    /// How many more new stories matched the notification rule.
    pub notify_more: usize,
}

const ITEM_COLS: &str = "id, topic_id, url, title, summary, source_name, domain, published_at, fetched_at, is_read, saved, matched, story";

fn item_row(r: &Row) -> rusqlite::Result<NewsItem> {
    let fetched: String = r.get(8)?;
    Ok(NewsItem {
        id: r.get(0)?,
        topic_id: r.get(1)?,
        url: r.get(2)?,
        title: r.get(3)?,
        summary: r.get(4)?,
        source_name: r.get(5)?,
        domain: r.get(6)?,
        published_at: r.get::<_, Option<String>>(7)?.as_deref().and_then(parse_utc),
        fetched_at: parse_utc(&fetched).unwrap_or_default(),
        read: r.get(9)?,
        saved: r.get(10)?,
        matched: serde_json::from_str(&r.get::<_, String>(11)?).unwrap_or_default(),
        story: r.get(12)?,
    })
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

fn checked_input(input: &TopicInput) -> Result<(String, TopicConfig)> {
    let name = input.name.trim().to_string();
    if name.is_empty() {
        return Err(Error::invalid("Give the topic a name."));
    }
    let cfg = input.config.clone().cleaned();
    cfg.validate()?;
    Ok((name, cfg))
}

impl Store {
    pub fn news_topics(&self) -> Result<Vec<Topic>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, name, config, enabled, created_at, last_checked, last_error FROM news_topics ORDER BY name COLLATE NOCASE",
        )?;
        let v = stmt.query_map([], topic_row)?.collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(v)
    }

    pub fn news_topic(&self, id: i64) -> Result<Topic> {
        self.conn
            .query_row(
                "SELECT id, name, config, enabled, created_at, last_checked, last_error FROM news_topics WHERE id = ?1",
                [id],
                topic_row,
            )
            .optional()?
            .ok_or_else(|| Error::NotFound("topic".into()))
    }

    pub fn create_topic(&self, input: &TopicInput, now: NaiveDateTime) -> Result<i64> {
        let (name, cfg) = checked_input(input)?;
        self.conn.execute(
            "INSERT INTO news_topics(name, config, enabled, created_at) VALUES (?1, ?2, ?3, ?4)",
            params![name, serde_json::to_string(&cfg).unwrap(), input.enabled, local_str(now)],
        )?;
        Ok(self.conn.last_insert_rowid())
    }

    /// Saves a topic. Changing what it looks for or where makes it due for a check right away.
    pub fn update_topic(&self, id: i64, input: &TopicInput) -> Result<()> {
        let (name, cfg) = checked_input(input)?;
        let old = self.news_topic(id)?;
        let sources_changed = old.config.include != cfg.include
            || old.config.feeds != cfg.feeds
            || old.config.search != cfg.search
            || old.config.language != cfg.language;
        self.conn.execute(
            "UPDATE news_topics SET name = ?2, config = ?3, enabled = ?4, last_checked = CASE WHEN ?5 THEN NULL ELSE last_checked END WHERE id = ?1",
            params![id, name, serde_json::to_string(&cfg).unwrap(), input.enabled, sources_changed],
        )?;
        // Apply new exclusions to what's already stored (saved stories stay).
        let items = self.news_items(id)?;
        let blocked = self.blocked_sites()?;
        for it in items.iter().filter(|i| !i.saved) {
            let text = normalize(&format!("{} {}", it.title, it.summary));
            if is_blocked(&it.domain, &cfg.blocked_sites)
                || is_blocked(&it.domain, &blocked)
                || cfg.exclude.iter().any(|w| term_matches(&text, w))
            {
                self.conn.execute("DELETE FROM news_items WHERE id = ?1", [it.id])?;
            }
        }
        Ok(())
    }

    pub fn delete_topic(&self, id: i64) -> Result<()> {
        self.conn.execute("DELETE FROM news_topics WHERE id = ?1", [id])?;
        Ok(())
    }

    // ---- blocked websites (all topics) ----

    pub fn blocked_sites(&self) -> Result<Vec<String>> {
        let mut stmt = self.conn.prepare("SELECT domain FROM news_blocked_sites ORDER BY domain")?;
        let v = stmt.query_map([], |r| r.get(0))?.collect::<rusqlite::Result<Vec<String>>>()?;
        Ok(v)
    }

    /// Blocks a website everywhere and removes its unsaved stories. Returns the domain.
    pub fn block_site(&self, input: &str) -> Result<String> {
        let domain = site_domain(input).ok_or_else(|| Error::invalid("Type a website, like lenta.ru."))?;
        self.conn.execute("INSERT OR IGNORE INTO news_blocked_sites(domain) VALUES (?1)", [&domain])?;
        self.conn.execute("DELETE FROM news_items WHERE saved = 0 AND (domain = ?1 OR domain LIKE '%.' || ?1)", [&domain])?;
        Ok(domain)
    }

    pub fn unblock_site(&self, domain: &str) -> Result<()> {
        self.conn.execute("DELETE FROM news_blocked_sites WHERE domain = ?1", [domain])?;
        Ok(())
    }

    // ---- items ----

    pub fn news_items(&self, topic_id: i64) -> Result<Vec<NewsItem>> {
        let sql = format!(
            "SELECT {ITEM_COLS} FROM news_items WHERE topic_id = ?1 ORDER BY COALESCE(published_at, fetched_at) DESC, id DESC LIMIT 600"
        );
        let mut stmt = self.conn.prepare(&sql)?;
        let v = stmt.query_map([topic_id], item_row)?.collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(v)
    }

    pub fn news_item(&self, id: i64) -> Result<NewsItem> {
        let sql = format!("SELECT {ITEM_COLS} FROM news_items WHERE id = ?1");
        self.conn.query_row(&sql, [id], item_row).optional()?.ok_or_else(|| Error::NotFound("story".into()))
    }

    pub fn unread_counts(&self) -> Result<HashMap<i64, i64>> {
        let mut stmt = self.conn.prepare("SELECT topic_id, COUNT(DISTINCT story) FROM news_items WHERE is_read = 0 GROUP BY topic_id")?;
        let v = stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?.collect::<rusqlite::Result<HashMap<_, _>>>()?;
        Ok(v)
    }

    /// Marks every article of the item's story as read or unread.
    pub fn set_story_read(&self, item_id: i64, read: bool) -> Result<()> {
        let it = self.news_item(item_id)?;
        self.conn.execute("UPDATE news_items SET is_read = ?3 WHERE topic_id = ?1 AND story = ?2", params![it.topic_id, it.story, read])?;
        Ok(())
    }

    pub fn mark_topic_read(&self, topic_id: i64) -> Result<()> {
        self.conn.execute("UPDATE news_items SET is_read = 1 WHERE topic_id = ?1", [topic_id])?;
        Ok(())
    }

    pub fn set_saved(&self, item_id: i64, saved: bool) -> Result<()> {
        self.conn.execute("UPDATE news_items SET saved = ?2 WHERE id = ?1", params![item_id, saved])?;
        Ok(())
    }

    pub fn cached_article(&self, item_id: i64) -> Result<Option<(Option<Article>, Option<String>)>> {
        let row: Option<(Option<String>, Option<String>)> = self
            .conn
            .query_row("SELECT article, error FROM news_articles WHERE item_id = ?1", [item_id], |r| Ok((r.get(0)?, r.get(1)?)))
            .optional()?;
        Ok(row.map(|(a, e)| (a.and_then(|a| serde_json::from_str::<Article>(&a).ok()), e)))
    }

    pub fn save_article(&self, item_id: i64, result: &Result<Article>, now: DateTime<Utc>) -> Result<()> {
        let (article, error) = match result {
            Ok(a) => (Some(serde_json::to_string(a).unwrap()), None),
            Err(e) => (None, Some(e.to_string())),
        };
        self.conn.execute(
            "INSERT INTO news_articles(item_id, article, error, fetched_at) VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT(item_id) DO UPDATE SET article = excluded.article, error = excluded.error, fetched_at = excluded.fetched_at",
            params![item_id, article, error, utc_str(now)],
        )?;
        Ok(())
    }

    /// The other articles of an item's story.
    pub fn story_siblings(&self, item: &NewsItem) -> Result<Vec<NewsItem>> {
        let sql = format!("SELECT {ITEM_COLS} FROM news_items WHERE topic_id = ?1 AND story = ?2 AND id != ?3 ORDER BY COALESCE(published_at, fetched_at) DESC");
        let mut stmt = self.conn.prepare(&sql)?;
        let v = stmt.query_map(params![item.topic_id, item.story, item.id], item_row)?.collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(v)
    }

    /// Deletes unsaved stories fetched more than [`RETENTION_DAYS`] ago.
    pub fn prune_news(&self, now: DateTime<Utc>) -> Result<usize> {
        let cutoff = utc_str(now - Duration::days(RETENTION_DAYS));
        Ok(self.conn.execute("DELETE FROM news_items WHERE saved = 0 AND fetched_at < ?1", [cutoff])?)
    }

    /// Stores what a check found and records when it ran. Returns the newly added stories.
    pub fn record_check(
        &mut self,
        topic_id: i64,
        found: &[Candidate],
        error: Option<&str>,
        now_utc: DateTime<Utc>,
        now_local: NaiveDateTime,
    ) -> Result<Vec<NewsItem>> {
        let recent_cutoff = utc_str(now_utc - Duration::days(3));
        let mut recent: Vec<(String, HashSet<String>)> = {
            let mut stmt = self.conn.prepare("SELECT story, title FROM news_items WHERE topic_id = ?1 AND fetched_at >= ?2")?;
            let v = stmt
                .query_map(params![topic_id, recent_cutoff], |r| Ok((r.get::<_, String>(0)?, title_tokens(&r.get::<_, String>(1)?))))?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            v
        };
        let tx = self.conn.transaction()?;
        let mut added_ids = Vec::new();
        for c in found {
            let exists: bool = tx
                .query_row("SELECT 1 FROM news_items WHERE topic_id = ?1 AND url = ?2", params![topic_id, c.url], |_| Ok(true))
                .optional()?
                .is_some();
            if exists {
                continue;
            }
            let tokens = title_tokens(&c.title);
            let story = recent.iter().find(|(_, t)| same_story(t, &tokens)).map(|(s, _)| s.clone()).unwrap_or_else(|| c.url.clone());
            tx.execute(
                "INSERT INTO news_items(topic_id, url, title, summary, source_name, domain, published_at, fetched_at, matched, story)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
                params![
                    topic_id,
                    c.url,
                    c.title,
                    c.summary,
                    c.source_name,
                    c.domain,
                    c.published_at.map(utc_str),
                    utc_str(now_utc),
                    serde_json::to_string(&c.matched).unwrap(),
                    story
                ],
            )?;
            added_ids.push(tx.last_insert_rowid());
            recent.push((story, tokens));
        }
        tx.execute(
            "UPDATE news_topics SET last_checked = ?2, last_error = ?3 WHERE id = ?1",
            params![topic_id, local_str(now_local), error],
        )?;
        tx.commit()?;
        added_ids.into_iter().map(|id| self.news_item(id)).collect()
    }
}

/// Reads a topic's sources and applies its rules. Network errors are collected, not fatal,
/// so one broken feed doesn't stop the others.
pub fn gather(topic: &Topic, global_blocked: &[String], fetcher: &dyn Fetch, now: DateTime<Utc>) -> (Vec<Candidate>, Vec<String>) {
    let cfg = &topic.config;
    let mut found: Vec<Candidate> = Vec::new();
    let mut errors = Vec::new();
    let oldest = now - Duration::days(RETENTION_DAYS);
    let mut sources: Vec<(String, bool)> = cfg.feeds.iter().map(|f| (f.clone(), false)).collect();
    if let Some(engine) = cfg.search {
        sources.push((search_url(engine, cfg, global_blocked), true));
    }
    for (url, from_search) in sources {
        let label = if from_search {
            match cfg.search {
                Some(SearchEngine::Bing) => "Bing News".to_string(),
                _ => "Google News".to_string(),
            }
        } else {
            site_domain(&url).unwrap_or_else(|| url.clone())
        };
        match fetcher.get(&url).and_then(|b| parse_feed(&b)) {
            Ok(feed) => {
                for e in &feed.entries {
                    if e.published.is_some_and(|p| p < oldest) {
                        continue;
                    }
                    if let Some(c) = screen(cfg, global_blocked, e) {
                        if !found.iter().any(|f| f.url == c.url) {
                            found.push(c);
                        }
                    }
                }
            }
            Err(e) => errors.push(format!("{label}: {e}")),
        }
    }
    (found, errors)
}

/// Fetches and stores one topic. The store is only locked around the database work, never
/// while waiting for the network.
pub fn check_topic(
    store: &std::sync::Mutex<Store>,
    topic_id: i64,
    fetcher: &dyn Fetch,
    now_utc: DateTime<Utc>,
    now_local: NaiveDateTime,
) -> Result<CheckOutcome> {
    let lock = || store.lock().map_err(|_| Error::invalid("The data store is unavailable."));
    let (topic, blocked) = {
        let s = lock()?;
        (s.news_topic(topic_id)?, s.blocked_sites()?)
    };
    let (found, errors) = gather(&topic, &blocked, fetcher, now_utc);
    let total_sources = topic.config.feeds.len() + usize::from(topic.config.search.is_some());
    let error = (!errors.is_empty()).then(|| {
        if errors.len() == total_sources {
            errors.join(" ")
        } else {
            format!("Some sources failed. {}", errors.join(" "))
        }
    });
    let first_check = topic.last_checked.is_none();
    let added = lock()?.record_check(topic_id, &found, error.as_deref(), now_utc, now_local)?;

    let mut notify: Vec<NewsItem> = if first_check {
        // The first check fills the topic; announcing all of it would be noise.
        vec![]
    } else {
        match topic.config.notify {
            NotifyMode::Off => vec![],
            NotifyMode::All => added.clone(),
            NotifyMode::Words => {
                added.iter().filter(|i| topic.config.notify_words.iter().any(|w| term_matches(&normalize(&i.title), w))).cloned().collect()
            }
        }
    };
    // One notification per story.
    let mut seen = HashSet::new();
    notify.retain(|i| seen.insert(i.story.clone()));
    let notify_more = notify.len().saturating_sub(MAX_NOTIFICATIONS);
    notify.truncate(MAX_NOTIFICATIONS);
    Ok(CheckOutcome { topic_id, topic_name: topic.name, added: added.len(), error, notify, notify_more })
}

/// Opens a story for reading: the stored text, or the article fetched now. The store isn't
/// locked while downloading.
pub fn read_story(
    store: &std::sync::Mutex<Store>,
    item_id: i64,
    fetcher: &dyn Fetch,
    now: DateTime<Utc>,
    refresh: bool,
) -> Result<Reading> {
    let lock = || store.lock().map_err(|_| Error::invalid("The data store is unavailable."));
    let (item, cached) = {
        let s = lock()?;
        (s.news_item(item_id)?, s.cached_article(item_id)?)
    };
    let (article, error) = match cached {
        Some((a, e)) if !refresh && (a.is_some() || e.is_some()) => (a, e),
        _ => {
            let result = fetch_article(fetcher, &item.url);
            lock()?.save_article(item_id, &result, now)?;
            match result {
                Ok(a) => (Some(a), None),
                Err(e) => (None, Some(e.to_string())),
            }
        }
    };
    let s = lock()?;
    s.set_story_read(item_id, true)?;
    let item = s.news_item(item_id)?;
    let also = s.story_siblings(&item)?;
    Ok(Reading { item, article, error, also })
}

/// Fetches and stores an article's text without marking it read (for the day's summary).
pub fn prefetch_article(store: &std::sync::Mutex<Store>, item_id: i64, fetcher: &dyn Fetch, now: DateTime<Utc>) -> Result<()> {
    let lock = || store.lock().map_err(|_| Error::invalid("The data store is unavailable."));
    let url = lock()?.news_item(item_id)?.url;
    let result = fetch_article(fetcher, &url);
    lock()?.save_article(item_id, &result, now)
}

const NAME_STOPWORDS: &[&str] = &[
    "как",
    "что",
    "это",
    "его",
    "для",
    "при",
    "после",
    "почему",
    "когда",
    "где",
    "кто",
    "или",
    "the",
    "and",
    "for",
    "with",
    "from",
    "how",
    "why",
    "what",
];

/// Today's stories grouped, counted and ranked by how many websites covered them.
fn compute_digest(store: &Store, cfg: &TopicConfig, items: &[NewsItem], day_start: DateTime<Utc>) -> Result<Option<Digest>> {
    let today: Vec<&NewsItem> = items.iter().filter(|i| i.published_at.unwrap_or(i.fetched_at) >= day_start).collect();
    if today.is_empty() {
        return Ok(None);
    }
    let mut order: Vec<String> = Vec::new();
    let mut groups: HashMap<String, Vec<&NewsItem>> = HashMap::new();
    for it in &today {
        if !groups.contains_key(&it.story) {
            order.push(it.story.clone());
        }
        groups.entry(it.story.clone()).or_default().push(it);
    }
    let websites: HashSet<&str> = today.iter().map(|i| i.domain.as_str()).collect();
    let mut ranked: Vec<(usize, usize, &Vec<&NewsItem>)> = order
        .iter()
        .enumerate()
        .map(|(pos, k)| {
            let g = &groups[k];
            (g.iter().map(|i| i.domain.as_str()).collect::<HashSet<_>>().len(), pos, g)
        })
        .collect();
    // Most websites first; among equals, the newest (lists are newest first).
    ranked.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
    let mut top = Vec::new();
    for (sites, _, g) in ranked.iter().take(3) {
        let lead_item = g[0];
        let mut lead = None;
        for it in g.iter() {
            if let Some((Some(a), _)) = store.cached_article(it.id)? {
                if !a.lead.is_empty() {
                    lead = Some(a.lead);
                    break;
                }
            }
        }
        if lead.is_none() && lead_item.summary.chars().count() >= 60 {
            let l = super::article::lead_of(&lead_item.summary);
            if !l.is_empty() {
                lead = Some(l);
            }
        }
        top.push(DigestStory { item_id: lead_item.id, title: lead_item.title.clone(), websites: *sites, lead });
    }
    let names = frequent_names(cfg, order.iter().map(|k| groups[k][0].title.as_str()));
    Ok(Some(Digest { stories: order.len(), websites: websites.len(), top, names }))
}

/// Proper names that recur across headlines: capitalised words that don't start the
/// headline, plus acronyms. Headlines written in Title Case are skipped, since every word
/// there is capitalised. The topic's own words are left out.
pub fn frequent_names<'a>(cfg: &TopicConfig, titles: impl Iterator<Item = &'a str>) -> Vec<String> {
    let mut counts: HashMap<String, (usize, HashMap<String, usize>)> = HashMap::new();
    let topic_words: Vec<String> = cfg
        .include
        .iter()
        .flat_map(|w| words(&normalize(w.trim_matches('"').trim_end_matches('*'))).into_iter().map(stem).collect::<Vec<_>>())
        .collect();
    for title in titles {
        let raw: Vec<&str> = title.split_whitespace().collect();
        let tokens: Vec<String> =
            raw.iter().map(|w| w.trim_matches(|c: char| !c.is_alphanumeric() && c != '-').to_string()).filter(|w| !w.is_empty()).collect();
        let caps = tokens.iter().filter(|w| w.chars().next().is_some_and(char::is_uppercase)).count();
        let title_case = tokens.len() >= 4 && caps * 10 > tokens.len() * 6;
        let mut seen = HashSet::new();
        for (i, w) in tokens.iter().enumerate() {
            let n = w.chars().count();
            let acronym =
                (2..=6).contains(&n) && w.chars().all(|c| c.is_uppercase() || c.is_ascii_digit()) && w.chars().any(char::is_alphabetic);
            let capital = n >= 3 && w.chars().next().is_some_and(char::is_uppercase) && w.chars().skip(1).any(char::is_lowercase);
            if !(acronym || (capital && i > 0 && !title_case)) {
                continue;
            }
            let lower = normalize(w);
            if NAME_STOPWORDS.contains(&lower.as_str()) {
                continue;
            }
            let key = stem(&lower);
            if topic_words.iter().any(|t| super::stems_match_pub(t, &key)) || !seen.insert(key.clone()) {
                continue;
            }
            let e = counts.entry(key).or_default();
            e.0 += 1;
            *e.1.entry(w.clone()).or_default() += 1;
        }
    }
    let mut v: Vec<(usize, String)> = counts
        .into_values()
        .filter(|(n, _)| *n >= 2)
        .map(|(n, forms)| {
            (
                n,
                forms
                    .into_iter()
                    .max_by(|a, b| a.1.cmp(&b.1).then(b.0.chars().count().cmp(&a.0.chars().count())))
                    .map(|f| f.0)
                    .unwrap_or_default(),
            )
        })
        .collect();
    v.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
    v.into_iter().take(6).map(|(_, s)| s).collect()
}

/// Items whose article text would complete today's digest (the most covered stories that
/// haven't been read yet). The app fetches them in the background after each check.
pub fn digest_prefetch(store: &Store, topic_id: i64, day_start: DateTime<Utc>) -> Result<Vec<i64>> {
    let topic = store.news_topic(topic_id)?;
    let items = store.news_items(topic_id)?;
    let Some(d) = compute_digest(store, &topic.config, &items, day_start)? else { return Ok(vec![]) };
    let mut out = Vec::new();
    for s in d.top {
        if store.cached_article(s.item_id)?.is_none() {
            out.push(s.item_id);
        }
    }
    Ok(out)
}

/// Topics whose interval has passed.
pub fn due_topics(store: &Store, now: NaiveDateTime) -> Result<Vec<i64>> {
    Ok(store.news_topics()?.into_iter().filter(|t| t.is_due(now)).map(|t| t.id).collect())
}

pub fn news_view(store: &Store, topic_id: Option<i64>, filter: Filter, now: NaiveDateTime, day_start: DateTime<Utc>) -> Result<NewsView> {
    let unread = store.unread_counts()?;
    let topics: Vec<TopicSummary> = store
        .news_topics()?
        .into_iter()
        .map(|t| TopicSummary {
            unread: unread.get(&t.id).copied().unwrap_or(0),
            next_check: t.enabled.then(|| t.next_check()).flatten(),
            topic: t,
        })
        .collect();
    // Without a choice, open the first topic with something unread, else the first active one.
    let topic = topic_id
        .and_then(|id| topics.iter().find(|t| t.topic.id == id))
        .or_else(|| topics.iter().find(|t| t.unread > 0))
        .or_else(|| topics.iter().find(|t| t.topic.enabled))
        .or(topics.first())
        .cloned();
    let mut stories: Vec<Story> = Vec::new();
    let mut digest = None;
    if let Some(t) = &topic {
        let items = store.news_items(t.topic.id)?;
        digest = compute_digest(store, &t.topic.config, &items, day_start)?;
        let mut index: HashMap<String, usize> = HashMap::new();
        for it in items {
            match index.get(&it.story) {
                Some(&i) => {
                    stories[i].unread |= !it.read;
                    stories[i].also.push(it);
                }
                None => {
                    index.insert(it.story.clone(), stories.len());
                    stories.push(Story { unread: !it.read, lead: it, also: vec![] });
                }
            }
        }
        stories.retain(|s| match filter {
            Filter::All => true,
            Filter::Unread => s.unread,
            Filter::Saved => s.lead.saved || s.also.iter().any(|i| i.saved),
        });
    }
    Ok(NewsView { topics, topic, stories, filter, blocked_sites: store.blocked_sites()?, now, digest })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::news::feed::tests::{ATOM, GOOGLE};
    use std::sync::Mutex;

    struct Canned(HashMap<String, Result<String>>);
    impl Fetch for Canned {
        fn get(&self, url: &str) -> Result<Vec<u8>> {
            for (k, v) in &self.0 {
                if url.starts_with(k) {
                    return match v {
                        Ok(s) => Ok(s.clone().into_bytes()),
                        Err(e) => Err(Error::invalid(e.to_string())),
                    };
                }
            }
            Err(Error::invalid("offline"))
        }
    }

    fn input(search: bool, feeds: Vec<&str>, notify: NotifyMode) -> TopicInput {
        TopicInput {
            name: "Автомобили".into(),
            enabled: true,
            config: TopicConfig {
                include: vec!["автомобиль".into(), "tray".into()],
                exclude: vec![],
                blocked_sites: vec![],
                feeds: feeds.into_iter().map(String::from).collect(),
                search: search.then_some(SearchEngine::Google),
                language: "ru".into(),
                match_in: MatchIn::TitleAndSummary,
                interval_minutes: 30,
                active_from: None,
                active_to: None,
                notify,
                notify_words: vec![],
            },
        }
    }
    fn now_utc() -> DateTime<Utc> {
        DateTime::parse_from_rfc3339("2026-11-16T06:00:00Z").unwrap().with_timezone(&Utc)
    }
    fn day() -> DateTime<Utc> {
        DateTime::parse_from_rfc3339("2026-11-15T21:00:00Z").unwrap().with_timezone(&Utc)
    }
    fn local(h: u32, m: u32) -> NaiveDateTime {
        chrono::NaiveDate::from_ymd_opt(2026, 11, 16).unwrap().and_hms_opt(h, m, 0).unwrap()
    }
    fn fetcher() -> Canned {
        let mut m = HashMap::new();
        m.insert("https://news.google.com/".to_string(), Ok(GOOGLE.to_string()));
        m.insert("https://blog.example.com/feed".to_string(), Ok(ATOM.to_string()));
        m.insert("https://broken.example.com/".to_string(), Err(Error::invalid("broken.example.com took too long to answer.")));
        Canned(m)
    }

    #[test]
    fn checks_store_group_and_dedupe() {
        let s = Store::open_in_memory().unwrap();
        let id = s.create_topic(&input(true, vec!["https://blog.example.com/feed"], NotifyMode::All), local(8, 0)).unwrap();
        let store = Mutex::new(s);
        let out = check_topic(&store, id, &fetcher(), now_utc(), local(9, 0)).unwrap();
        assert_eq!(out.added, 3);
        assert!(out.error.is_none());
        assert!(out.notify.is_empty(), "first check doesn't notify");
        // Same results again: nothing new.
        let out = check_topic(&store, id, &fetcher(), now_utc(), local(9, 30)).unwrap();
        assert_eq!(out.added, 0);

        let s = store.lock().unwrap();
        let v = news_view(&s, Some(id), Filter::All, local(9, 31), day()).unwrap();
        // The two car-sales headlines are one story.
        assert_eq!(v.stories.len(), 2);
        let cars = v.stories.iter().find(|st| st.lead.domain == "rbc.ru" || st.lead.domain == "lenta.ru").unwrap();
        assert_eq!(cars.also.len(), 1);
        assert_eq!(v.topics[0].unread, 2);
        assert_eq!(v.topic.as_ref().unwrap().next_check, Some(local(10, 0)));
        s.set_story_read(cars.lead.id, true).unwrap();
        assert_eq!(news_view(&s, Some(id), Filter::Unread, local(9, 31), day()).unwrap().stories.len(), 1);
    }

    #[test]
    fn blocking_a_site_removes_and_filters_it() {
        let s = Store::open_in_memory().unwrap();
        let id = s.create_topic(&input(true, vec![], NotifyMode::Off), local(8, 0)).unwrap();
        let store = Mutex::new(s);
        check_topic(&store, id, &fetcher(), now_utc(), local(9, 0)).unwrap();
        {
            let s = store.lock().unwrap();
            assert_eq!(s.block_site("https://www.rbc.ru/news").unwrap(), "rbc.ru");
            let items = s.news_items(id).unwrap();
            assert_eq!(items.len(), 1);
            assert_eq!(items[0].domain, "lenta.ru");
            // Make it due again.
            s.conn.execute("UPDATE news_topics SET last_checked = NULL", []).unwrap();
            s.conn.execute("DELETE FROM news_items", []).unwrap();
        }
        check_topic(&store, id, &fetcher(), now_utc(), local(9, 30)).unwrap();
        let s = store.lock().unwrap();
        assert!(s.news_items(id).unwrap().iter().all(|i| i.domain != "rbc.ru"));
        s.unblock_site("rbc.ru").unwrap();
        assert!(s.blocked_sites().unwrap().is_empty());
    }

    #[test]
    fn per_topic_blocks_and_exclusions_apply_to_stored_stories() {
        let s = Store::open_in_memory().unwrap();
        let id = s.create_topic(&input(true, vec![], NotifyMode::Off), local(8, 0)).unwrap();
        let store = Mutex::new(s);
        check_topic(&store, id, &fetcher(), now_utc(), local(9, 0)).unwrap();
        let s = store.lock().unwrap();
        let mut i = input(true, vec![], NotifyMode::Off);
        i.config.blocked_sites = vec!["lenta.ru".into()];
        s.update_topic(id, &i).unwrap();
        let items = s.news_items(id).unwrap();
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].domain, "rbc.ru");
    }

    #[test]
    fn later_checks_notify_and_errors_are_kept() {
        let s = Store::open_in_memory().unwrap();
        let id = s
            .create_topic(
                &input(false, vec!["https://broken.example.com/rss", "https://blog.example.com/feed"], NotifyMode::All),
                local(8, 0),
            )
            .unwrap();
        s.conn.execute("UPDATE news_topics SET last_checked = '2026-11-16T07:00:00'", []).unwrap();
        let store = Mutex::new(s);
        let out = check_topic(&store, id, &fetcher(), now_utc(), local(9, 0)).unwrap();
        assert_eq!(out.added, 1);
        assert_eq!(out.notify.len(), 1);
        assert!(out.error.as_deref().unwrap().starts_with("Some sources failed. broken.example.com:"));
        let t = store.lock().unwrap().news_topic(id).unwrap();
        assert!(t.last_error.is_some());
        assert!(!t.is_due(local(9, 10)));
        assert!(t.is_due(local(9, 30)));
    }

    #[test]
    fn digest_ranks_by_coverage_and_finds_names() {
        let s = Store::open_in_memory().unwrap();
        let id = s.create_topic(&input(true, vec!["https://blog.example.com/feed"], NotifyMode::Off), local(8, 0)).unwrap();
        let store = Mutex::new(s);
        check_topic(&store, id, &fetcher(), now_utc(), local(9, 0)).unwrap();
        let s = store.lock().unwrap();
        let d = news_view(&s, Some(id), Filter::All, local(9, 1), day()).unwrap().digest.unwrap();
        // Today: the car-sales story (2 websites). The blog post was published yesterday.
        assert_eq!(d.stories, 1);
        assert_eq!(d.websites, 2);
        assert_eq!(d.top[0].websites, 2);
        assert!(d.top[0].lead.is_none());
        assert_eq!(digest_prefetch(&s, id, day()).unwrap(), vec![d.top[0].item_id]);
    }

    #[test]
    fn names_are_recurring_proper_nouns() {
        let mut cfg = input(false, vec!["https://x.ru/rss"], NotifyMode::Off).config;
        cfg.include = vec!["электромобиль".into()];
        let titles = [
            "Продажи электромобилей Tesla в России выросли",
            "В Москве открылся салон Tesla",
            "Электромобили подешевели в Москве и Петербурге",
            "ЦБ сохранил ставку",
            "Аналитики ЦБ ждут роста рынка",
            "How Tesla And BYD Are Changing The Market",
        ];
        assert_eq!(frequent_names(&cfg, titles.into_iter()), vec!["Tesla".to_string(), "Москве".to_string(), "ЦБ".to_string()]);
    }
}
