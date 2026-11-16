//! News storage, checking and the views the News screen shows.

use std::collections::{HashMap, HashSet};

use chrono::{DateTime, Duration, NaiveDateTime, Utc};
use rusqlite::{params, OptionalExtension, Row};
use serde::{Deserialize, Serialize};

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

/// Topics whose interval has passed.
pub fn due_topics(store: &Store, now: NaiveDateTime) -> Result<Vec<i64>> {
    Ok(store.news_topics()?.into_iter().filter(|t| t.is_due(now)).map(|t| t.id).collect())
}

pub fn news_view(store: &Store, topic_id: Option<i64>, filter: Filter, now: NaiveDateTime) -> Result<NewsView> {
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
    if let Some(t) = &topic {
        let mut index: HashMap<String, usize> = HashMap::new();
        for it in store.news_items(t.topic.id)? {
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
    Ok(NewsView { topics, topic, stories, filter, blocked_sites: store.blocked_sites()?, now })
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
        let v = news_view(&s, Some(id), Filter::All, local(9, 31)).unwrap();
        // The two car-sales headlines are one story.
        assert_eq!(v.stories.len(), 2);
        let cars = v.stories.iter().find(|st| st.lead.domain == "rbc.ru" || st.lead.domain == "lenta.ru").unwrap();
        assert_eq!(cars.also.len(), 1);
        assert_eq!(v.topics[0].unread, 2);
        assert_eq!(v.topic.as_ref().unwrap().next_check, Some(local(10, 0)));
        s.set_story_read(cars.lead.id, true).unwrap();
        assert_eq!(news_view(&s, Some(id), Filter::Unread, local(9, 31)).unwrap().stories.len(), 1);
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
}
