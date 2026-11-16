//! News: topics checked on a timer, matched by the words a person chooses.
//!
//! There is no ranking or recommendation: a story is kept when it mentions one of the topic's
//! words, doesn't mention a word to leave out, and doesn't come from a blocked website.
//! Stories are listed newest first.

pub mod article;
pub mod feed;
pub mod store;

use std::collections::HashSet;
use std::sync::LazyLock;

use chrono::{DateTime, NaiveDateTime, NaiveTime, Utc};
use rust_stemmers::{Algorithm, Stemmer};
use serde::{Deserialize, Serialize};

use crate::{Error, Result};

pub use feed::{FeedEntry, Fetch, HttpFetcher};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SearchEngine {
    Google,
    Bing,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum MatchIn {
    Title,
    TitleAndSummary,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum NotifyMode {
    Off,
    All,
    Words,
}

/// Everything a person sets for a topic.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TopicConfig {
    /// A story must mention at least one of these. Word endings don't matter
    /// («автомобиль» finds «автомобили»); `word*` matches word starts; "quotes" match exactly.
    pub include: Vec<String>,
    /// Stories mentioning any of these are left out.
    #[serde(default)]
    pub exclude: Vec<String>,
    /// Websites left out for this topic only (domains, subdomains included).
    #[serde(default)]
    pub blocked_sites: Vec<String>,
    /// RSS or Atom feeds to read.
    #[serde(default)]
    pub feeds: Vec<String>,
    /// Also search news websites for the topic's words.
    #[serde(default)]
    pub search: Option<SearchEngine>,
    /// "ru" or "en": the edition of the news search.
    #[serde(default = "default_language")]
    pub language: String,
    #[serde(default = "default_match_in")]
    pub match_in: MatchIn,
    pub interval_minutes: u32,
    /// Only check between these times (both set), e.g. 08:00–23:00.
    #[serde(default, with = "opt_hm")]
    pub active_from: Option<NaiveTime>,
    #[serde(default, with = "opt_hm")]
    pub active_to: Option<NaiveTime>,
    #[serde(default = "default_notify")]
    pub notify: NotifyMode,
    #[serde(default)]
    pub notify_words: Vec<String>,
}

fn default_language() -> String {
    "ru".into()
}
fn default_match_in() -> MatchIn {
    MatchIn::TitleAndSummary
}
fn default_notify() -> NotifyMode {
    NotifyMode::Off
}

pub const MIN_INTERVAL: u32 = 5;
pub const MAX_INTERVAL: u32 = 7 * 24 * 60;

impl TopicConfig {
    /// Trims, drops empty and duplicate entries, normalises websites and feed addresses.
    pub fn cleaned(mut self) -> Self {
        let tidy = |v: Vec<String>| {
            let mut seen = HashSet::new();
            v.into_iter().map(|s| s.trim().to_string()).filter(|s| !s.is_empty() && seen.insert(normalize(s))).collect::<Vec<_>>()
        };
        self.include = tidy(self.include);
        self.exclude = tidy(self.exclude);
        self.notify_words = tidy(self.notify_words);
        let mut seen = HashSet::new();
        self.blocked_sites = self.blocked_sites.iter().filter_map(|s| site_domain(s)).filter(|d| seen.insert(d.clone())).collect();
        let mut seen = HashSet::new();
        self.feeds = self.feeds.into_iter().map(|f| f.trim().to_string()).filter(|f| !f.is_empty() && seen.insert(f.clone())).collect();
        self.language = if self.language == "en" { "en".into() } else { "ru".into() };
        self
    }

    pub fn validate(&self) -> Result<()> {
        if self.feeds.is_empty() && self.search.is_none() {
            return Err(Error::invalid("Add a feed or turn on news search, so the topic has somewhere to look."));
        }
        if self.search.is_some() && self.include.is_empty() {
            return Err(Error::invalid("News search needs at least one word to look for."));
        }
        if !(MIN_INTERVAL..=MAX_INTERVAL).contains(&self.interval_minutes) {
            return Err(Error::invalid(format!("Check every {MIN_INTERVAL} minutes at most often, and at least once a week.")));
        }
        if self.active_from.is_some() != self.active_to.is_some() {
            return Err(Error::invalid("Set both times for the checking hours, or neither."));
        }
        if self.notify == NotifyMode::Words && self.notify_words.is_empty() {
            return Err(Error::invalid("Add the words that should trigger a notification."));
        }
        for f in &self.feeds {
            let u = url::Url::parse(f).map_err(|_| Error::invalid(format!("«{f}» isn't a web address.")))?;
            if u.scheme() != "http" && u.scheme() != "https" {
                return Err(Error::invalid(format!("«{f}» isn't a web address.")));
            }
        }
        Ok(())
    }

    /// Whether `t` falls inside the checking hours (which may run past midnight).
    pub fn active_at(&self, t: NaiveTime) -> bool {
        match (self.active_from, self.active_to) {
            (Some(a), Some(b)) if a <= b => t >= a && t <= b,
            (Some(a), Some(b)) => t >= a || t <= b,
            _ => true,
        }
    }
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
        self.enabled
            && self.config.active_at(now.time())
            && self.last_checked.is_none_or(|t| t > now || (now - t).num_minutes() >= self.config.interval_minutes as i64)
    }
    pub fn next_check(&self) -> Option<NaiveDateTime> {
        self.last_checked.map(|t| t + chrono::Duration::minutes(self.config.interval_minutes as i64))
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

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NewsItem {
    pub id: i64,
    pub topic_id: i64,
    pub url: String,
    pub title: String,
    pub summary: String,
    pub source_name: String,
    pub domain: String,
    pub published_at: Option<DateTime<Utc>>,
    pub fetched_at: DateTime<Utc>,
    pub read: bool,
    pub saved: bool,
    pub matched: Vec<String>,
    pub story: String,
}

/// A story that passed the topic's rules, ready to be stored.
#[derive(Clone, Debug, PartialEq)]
pub struct Candidate {
    pub url: String,
    pub title: String,
    pub summary: String,
    pub source_name: String,
    pub domain: String,
    pub published_at: Option<DateTime<Utc>>,
    pub matched: Vec<String>,
}

// ---------- text matching ----------

/// Lower case, "ё" as "е", single spaces.
pub fn normalize(s: &str) -> String {
    let lower = s.to_lowercase().replace('ё', "е");
    lower.split_whitespace().collect::<Vec<_>>().join(" ")
}

static RU: LazyLock<Stemmer> = LazyLock::new(|| Stemmer::create(Algorithm::Russian));
static EN: LazyLock<Stemmer> = LazyLock::new(|| Stemmer::create(Algorithm::English));

/// The words of a (normalised) text.
pub fn words(text: &str) -> Vec<&str> {
    text.split(|c: char| !c.is_alphanumeric()).filter(|w| !w.is_empty()).collect()
}

/// A word without its ending (Snowball stemmers for Russian and English), so
/// «электромобиль», «электромобили» and «электромобилей» compare equal.
pub fn stem(word: &str) -> String {
    if word.chars().any(|c| ('а'..='я').contains(&c)) {
        RU.stem(word).into_owned()
    } else if word.chars().any(|c| c.is_ascii_alphabetic()) {
        EN.stem(word).into_owned()
    } else {
        word.to_string()
    }
}

/// Whether `term` occurs in `text` (normalised). Word endings don't matter by default;
/// `word*` matches the start of a word; `"quoted words"` must match exactly.
pub fn term_matches(text: &str, term: &str) -> bool {
    let t = term.trim();
    let tw_norm = normalize(t.trim_matches('"').trim_end_matches('*'));
    let tw: Vec<&str> = words(&tw_norm);
    if tw.is_empty() {
        return false;
    }
    let xw = words(text);
    if xw.len() < tw.len() {
        return false;
    }
    let exact = t.len() > 1 && t.starts_with('"') && t.ends_with('"');
    let prefix = !exact && t.ends_with('*');
    let last = tw.len() - 1;
    let want: Vec<String> = if exact || prefix { tw.iter().map(|w| w.to_string()).collect() } else { tw.iter().map(|w| stem(w)).collect() };
    let have: Vec<String> = if exact || prefix { xw.iter().map(|w| w.to_string()).collect() } else { xw.iter().map(|w| stem(w)).collect() };
    have.windows(tw.len()).any(|win| {
        win.iter().zip(&want).enumerate().all(|(i, (h, w))| {
            if prefix && i == last {
                h.starts_with(w.as_str())
            } else if exact {
                h == w
            } else {
                stems_match(h, w)
            }
        })
    })
}

pub(crate) fn stems_match_pub(a: &str, b: &str) -> bool {
    stems_match(a, b)
}

/// Snowball sometimes stems forms of one word a letter or two apart («автомобили» → «автомоб»,
/// «автомобиль» → «автомобил»), so stems also match when one extends the other slightly.
fn stems_match(a: &str, b: &str) -> bool {
    if a == b {
        return true;
    }
    let (short, long) = if a.chars().count() <= b.chars().count() { (a, b) } else { (b, a) };
    let (ns, nl) = (short.chars().count(), long.chars().count());
    ns >= 4 && nl - ns <= 2 && long.starts_with(short)
}

/// The host of a web address or a typed domain, lower case, without "www.".
pub fn site_domain(s: &str) -> Option<String> {
    let s = s.trim();
    if s.is_empty() {
        return None;
    }
    let with_scheme = if s.contains("://") { s.to_string() } else { format!("https://{s}") };
    let host = url::Url::parse(&with_scheme).ok()?.host_str()?.to_lowercase();
    let host = host.strip_prefix("www.").unwrap_or(&host).to_string();
    (host.contains('.') && !host.starts_with('.')).then_some(host)
}

/// Whether `domain` is one of `blocked` or a subdomain of one.
pub fn is_blocked(domain: &str, blocked: &[String]) -> bool {
    let d = domain.strip_prefix("www.").unwrap_or(domain);
    blocked.iter().any(|b| d == b || d.ends_with(&format!(".{b}")))
}

/// Applies a topic's rules to one entry. Search results are checked too: news searches also
/// return loosely related stories, and a topic should only show ones that mention its words.
pub fn screen(cfg: &TopicConfig, global_blocked: &[String], e: &FeedEntry) -> Option<Candidate> {
    let title = e.title.trim();
    if title.is_empty() || e.url.is_empty() {
        return None;
    }
    let domain = e.domain();
    if is_blocked(&domain, &cfg.blocked_sites) || is_blocked(&domain, global_blocked) {
        return None;
    }
    let haystack = match cfg.match_in {
        MatchIn::Title => normalize(title),
        MatchIn::TitleAndSummary => normalize(&format!("{title} {}", e.summary)),
    };
    let everything = normalize(&format!("{title} {}", e.summary));
    if cfg.exclude.iter().any(|w| term_matches(&everything, w)) {
        return None;
    }
    let matched: Vec<String> = cfg.include.iter().filter(|w| term_matches(&haystack, w)).cloned().collect();
    if !cfg.include.is_empty() && matched.is_empty() {
        return None;
    }
    Some(Candidate {
        url: canonical_url(&e.url),
        title: title.to_string(),
        summary: e.summary.clone(),
        source_name: e.source_name.clone(),
        domain,
        published_at: e.published,
        matched,
    })
}

/// Removes tracking parameters and fragments so the same article isn't stored twice.
pub fn canonical_url(u: &str) -> String {
    let Ok(mut parsed) = url::Url::parse(u.trim()) else { return u.trim().to_string() };
    parsed.set_fragment(None);
    let keep: Vec<(String, String)> = parsed
        .query_pairs()
        .filter(|(k, _)| {
            let k = k.to_lowercase();
            !(k.starts_with("utm_") || matches!(k.as_str(), "fbclid" | "gclid" | "yclid" | "ysclid" | "_openstat" | "from"))
        })
        .map(|(k, v)| (k.into_owned(), v.into_owned()))
        .collect();
    if keep.is_empty() {
        parsed.set_query(None);
    } else {
        parsed.query_pairs_mut().clear().extend_pairs(keep);
    }
    parsed.to_string()
}

/// Words of a headline used to spot the same story from several websites.
pub fn title_tokens(title: &str) -> HashSet<String> {
    normalize(title)
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| w.chars().count() >= 3)
        .map(|w| w.chars().take(6).collect::<String>())
        .collect()
}

/// Two headlines describe the same story when most of their words are shared.
pub fn same_story(a: &HashSet<String>, b: &HashSet<String>) -> bool {
    if a.len() < 3 || b.len() < 3 {
        return false;
    }
    let inter = a.intersection(b).count() as f64;
    let union = a.union(b).count() as f64;
    inter / union >= 0.5
}

mod opt_hm {
    use chrono::NaiveTime;
    use serde::{Deserialize, Deserializer, Serializer};

    pub fn serialize<S: Serializer>(t: &Option<NaiveTime>, s: S) -> Result<S::Ok, S::Error> {
        match t {
            Some(t) => s.serialize_str(&t.format("%H:%M").to_string()),
            None => s.serialize_none(),
        }
    }
    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Option<NaiveTime>, D::Error> {
        let s: Option<String> = Option::deserialize(d)?;
        match s.as_deref().map(str::trim) {
            None | Some("") => Ok(None),
            Some(v) => NaiveTime::parse_from_str(v, "%H:%M").map(Some).map_err(serde::de::Error::custom),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg() -> TopicConfig {
        TopicConfig {
            include: vec!["автомобиль".into(), "\"Tesla\"".into()],
            exclude: vec!["реклама".into()],
            blocked_sites: vec!["spam.ru".into()],
            feeds: vec!["https://example.com/rss".into()],
            search: None,
            language: "ru".into(),
            match_in: MatchIn::TitleAndSummary,
            interval_minutes: 30,
            active_from: None,
            active_to: None,
            notify: NotifyMode::Off,
            notify_words: vec![],
        }
    }
    fn entry(title: &str, url: &str) -> FeedEntry {
        FeedEntry {
            url: url.into(),
            title: title.into(),
            summary: String::new(),
            published: None,
            source_name: String::new(),
            source_url: None,
        }
    }

    #[test]
    fn endings_dont_matter_stars_match_starts_and_quotes_are_exact() {
        let t = normalize("Новые Автомобили и Teslas на выставке электромобилей");
        assert!(term_matches(&t, "автомобиль"));
        assert!(term_matches(&t, "электромобиль"));
        assert!(term_matches(&t, "автомоб*"));
        assert!(!term_matches(&t, "мобил*"));
        assert!(term_matches(&t, "Tesla"), "English endings too");
        assert!(!term_matches(&t, "\"Tesla\""));
        assert!(term_matches(&normalize("Tesla снизила цены"), "\"tesla\""));
        assert!(term_matches(&normalize("Ёлки зелёные"), "ёлка"));
        assert!(term_matches(&normalize("Открыты новые зарядные станции"), "зарядная станция"));
        assert!(!term_matches(&normalize("станции зарядные"), "зарядная станция"), "phrases keep their order");
        assert!(term_matches(&normalize("Сколько стоит iPhone 17?"), "iphone 17"));
    }

    #[test]
    fn screening_applies_words_exclusions_and_blocked_sites() {
        let c = cfg();
        assert!(screen(&c, &[], &entry("Продажи автомобилей выросли", "https://news.ru/a")).is_some());
        assert!(screen(&c, &[], &entry("Курс рубля", "https://news.ru/b")).is_none());
        assert!(screen(&c, &[], &entry("Автомобиль года — реклама", "https://news.ru/c")).is_none());
        assert!(screen(&c, &[], &entry("Автомобили", "https://www.spam.ru/x")).is_none());
        assert!(screen(&c, &[], &entry("Автомобили", "https://m.spam.ru/x")).is_none());
        assert!(screen(&c, &["news.ru".into()], &entry("Автомобили", "https://news.ru/x")).is_none());
        assert!(screen(&c, &["news.ru".into()], &entry("Автомобили", "https://fakenews.ru/x")).is_some());
        let got = screen(&c, &[], &entry("Автомобили Tesla", "https://news.ru/d?utm_source=x&id=5#top")).unwrap();
        assert_eq!(got.url, "https://news.ru/d?id=5");
        assert_eq!(got.matched, vec!["автомобиль".to_string(), "\"Tesla\"".to_string()]);
    }

    #[test]
    fn domains_are_normalised() {
        assert_eq!(site_domain("https://www.RBC.ru/politics/1"), Some("rbc.ru".into()));
        assert_eq!(site_domain("lenta.ru"), Some("lenta.ru".into()));
        assert_eq!(site_domain("  "), None);
        assert_eq!(site_domain("localhost"), None);
    }

    #[test]
    fn config_is_cleaned_and_validated() {
        let mut c = cfg();
        c.blocked_sites = vec!["https://www.rbc.ru/x".into(), "rbc.ru".into(), "".into()];
        c.include = vec![" автомобиль ".into(), "АВТОМОБИЛЬ".into(), "".into()];
        let c = c.cleaned();
        assert_eq!(c.blocked_sites, vec!["rbc.ru".to_string()]);
        assert_eq!(c.include, vec!["автомобиль".to_string()]);
        assert!(c.validate().is_ok());
        let mut bad = c.clone();
        bad.feeds.clear();
        assert!(bad.validate().is_err());
        bad.search = Some(SearchEngine::Google);
        assert!(bad.validate().is_ok());
        bad.interval_minutes = 1;
        assert!(bad.validate().is_err());
    }

    #[test]
    fn checking_hours_can_wrap_midnight() {
        let mut c = cfg();
        let t = |h| NaiveTime::from_hms_opt(h, 0, 0).unwrap();
        c.active_from = Some(t(8));
        c.active_to = Some(t(23));
        assert!(c.active_at(t(12)));
        assert!(!c.active_at(t(3)));
        c.active_from = Some(t(22));
        c.active_to = Some(t(6));
        assert!(c.active_at(t(23)));
        assert!(c.active_at(t(2)));
        assert!(!c.active_at(t(12)));
    }

    #[test]
    fn similar_headlines_are_one_story() {
        let a = title_tokens("Центробанк сохранил ключевую ставку на уровне 16%");
        let b = title_tokens("ЦБ сохранил ключевую ставку на уровне 16% годовых");
        let c = title_tokens("В Москве открылась новая станция метро");
        assert!(same_story(&a, &b));
        assert!(!same_story(&a, &c));
    }
}
