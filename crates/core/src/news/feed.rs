//! Reading RSS and Atom feeds, building news-search addresses, and the HTTP client.

use std::collections::HashMap;
use std::sync::LazyLock;
use std::time::Duration;

use chrono::{DateTime, Utc};
use regex::Regex;

use super::{normalize, site_domain, SearchEngine, TopicConfig};
use crate::{Error, Result};

/// One article as a feed describes it.
#[derive(Clone, Debug, PartialEq)]
pub struct FeedEntry {
    pub url: String,
    pub title: String,
    pub summary: String,
    pub published: Option<DateTime<Utc>>,
    /// The publishing website's name, when the feed gives it (news searches do).
    pub source_name: String,
    pub source_url: Option<String>,
}

impl FeedEntry {
    /// The website the article is on: the stated source for aggregators, else the link's host.
    pub fn domain(&self) -> String {
        self.source_url.as_deref().and_then(site_domain).or_else(|| site_domain(&self.url)).unwrap_or_default()
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct ParsedFeed {
    pub title: String,
    pub entries: Vec<FeedEntry>,
}

static ITEM: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?is)<item\b.*?</item>").unwrap());
static ITEM_LINK: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?is)<link>\s*(?:<!\[CDATA\[)?\s*(.*?)\s*(?:\]\]>)?\s*</link>").unwrap());
static ITEM_SOURCE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"(?is)<source\b[^>]*\burl\s*=\s*["']([^"']+)["'][^>]*>(.*?)</source>"#).unwrap());
static TAG: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?is)<script.*?</script>|<style.*?</style>|<[^>]+>").unwrap());
static ENTITY: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"&(#x[0-9a-fA-F]+|#[0-9]+|[a-zA-Z]+);").unwrap());
static FEED_LINK: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"(?is)<link\b[^>]*type\s*=\s*["']application/(?:rss|atom)\+xml["'][^>]*>"#).unwrap());
static HREF: LazyLock<Regex> = LazyLock::new(|| Regex::new(r#"(?is)\bhref\s*=\s*["']([^"']+)["']"#).unwrap());

/// HTML to plain text: tags removed, common entities decoded, spaces collapsed.
pub fn plain_text(html: &str) -> String {
    let no_tags = TAG.replace_all(html, " ");
    let decoded = ENTITY.replace_all(&no_tags, |c: &regex::Captures| {
        let e = &c[1];
        let ch = if let Some(hex) = e.strip_prefix("#x").or_else(|| e.strip_prefix("#X")) {
            u32::from_str_radix(hex, 16).ok().and_then(char::from_u32)
        } else if let Some(dec) = e.strip_prefix('#') {
            dec.parse::<u32>().ok().and_then(char::from_u32)
        } else {
            match e {
                "amp" => Some('&'),
                "lt" => Some('<'),
                "gt" => Some('>'),
                "quot" => Some('"'),
                "apos" => Some('\''),
                "nbsp" => Some(' '),
                "laquo" => Some('«'),
                "raquo" => Some('»'),
                "mdash" => Some('—'),
                "ndash" => Some('–'),
                "hellip" => Some('…'),
                _ => None,
            }
        };
        ch.map(String::from).unwrap_or_else(|| c[0].to_string())
    });
    decoded.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn shorten(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    let cut: String = s.chars().take(max).collect();
    let cut = cut.rsplit_once(' ').map(|(a, _)| a.to_string()).unwrap_or(cut);
    format!("{cut}…")
}

/// Bing wraps article links in a click-tracking address; this returns the real one.
fn unwrap_link(link: &str) -> String {
    if let Ok(u) = url::Url::parse(link) {
        if u.host_str().is_some_and(|h| h.ends_with("bing.com")) {
            if let Some((_, real)) = u.query_pairs().find(|(k, _)| k == "url") {
                return real.into_owned();
            }
        }
    }
    link.to_string()
}

/// Parses an RSS, Atom or JSON feed.
pub fn parse_feed(bytes: &[u8]) -> Result<ParsedFeed> {
    let feed = feed_rs::parser::parse(bytes).map_err(|e| Error::invalid(format!("Not a news feed ({e}).")))?;

    // feed-rs drops RSS <source url="…">Name</source>, which news searches use to name the
    // publishing website, so read it from the XML directly, keyed by the item's link.
    let mut sources: HashMap<String, (String, String)> = HashMap::new();
    if let Ok(text) = std::str::from_utf8(bytes) {
        for item in ITEM.find_iter(text) {
            let item = item.as_str();
            if let (Some(link), Some(src)) = (ITEM_LINK.captures(item), ITEM_SOURCE.captures(item)) {
                sources.insert(plain_text(&link[1]), (src[1].to_string(), plain_text(&src[2])));
            }
        }
    }

    let entries = feed
        .entries
        .into_iter()
        .filter_map(|e| {
            let raw_link = e
                .links
                .iter()
                .find(|l| l.rel.as_deref().is_none_or(|r| r == "alternate"))
                .or(e.links.first())
                .map(|l| l.href.trim().to_string())
                .or_else(|| e.id.starts_with("http").then(|| e.id.clone()))?;
            let (source_url, source_name) = match sources.get(&raw_link) {
                Some((u, n)) => (Some(u.clone()), n.clone()),
                None => (None, String::new()),
            };
            let link = unwrap_link(&raw_link);
            let mut title = e.title.map(|t| plain_text(&t.content)).unwrap_or_default();
            // News searches append " - Source" to every headline.
            if !source_name.is_empty() {
                if let Some(stripped) = title.strip_suffix(&format!(" - {source_name}")) {
                    title = stripped.trim().to_string();
                }
            }
            let body = e.summary.map(|s| s.content).or_else(|| e.content.and_then(|c| c.body)).unwrap_or_default();
            let mut summary = plain_text(&body);
            // Aggregators often repeat the headline (and source) as the summary.
            let (ns, nt) = (normalize(&summary), normalize(&title));
            if ns.starts_with(&nt) {
                let rest = summary.chars().skip(title.chars().count()).collect::<String>();
                summary = rest.trim().to_string();
                if !source_name.is_empty() && normalize(&summary) == normalize(&source_name) {
                    summary.clear();
                }
            }
            Some(FeedEntry {
                url: link,
                title,
                summary: shorten(&summary, 400),
                published: e.published.or(e.updated),
                source_name,
                source_url,
            })
        })
        .collect();
    Ok(ParsedFeed { title: feed.title.map(|t| plain_text(&t.content)).unwrap_or_default(), entries })
}

fn quote(term: &str) -> String {
    // Search engines have no "starts with": send the start as a word.
    let t = term.trim().trim_matches('"').trim_end_matches('*');
    if t.contains(' ') || term.trim().starts_with('"') {
        format!("\"{t}\"")
    } else {
        t.to_string()
    }
}

/// The search query for a topic: any of its words, minus the words and websites to leave out.
pub fn search_query(cfg: &TopicConfig, global_blocked: &[String]) -> String {
    let mut q = cfg.include.iter().map(|w| quote(w)).collect::<Vec<_>>().join(" OR ");
    if cfg.include.len() > 1 && (!cfg.exclude.is_empty() || !cfg.blocked_sites.is_empty() || !global_blocked.is_empty()) {
        q = format!("({q})");
    }
    for w in &cfg.exclude {
        q.push_str(&format!(" -{}", quote(w)));
    }
    let mut sites: Vec<&String> = cfg.blocked_sites.iter().chain(global_blocked.iter()).collect();
    sites.dedup();
    // Search engines cap query length; anything past this is still removed locally.
    for s in sites.into_iter().take(25) {
        q.push_str(&format!(" -site:{s}"));
    }
    q
}

/// The RSS address of a news search for the topic.
pub fn search_url(engine: SearchEngine, cfg: &TopicConfig, global_blocked: &[String]) -> String {
    let q = search_query(cfg, global_blocked);
    let en = cfg.language == "en";
    let mut u = match engine {
        SearchEngine::Google => url::Url::parse("https://news.google.com/rss/search").unwrap(),
        SearchEngine::Bing => url::Url::parse("https://www.bing.com/news/search").unwrap(),
    };
    {
        let mut p = u.query_pairs_mut();
        match engine {
            SearchEngine::Google => {
                p.append_pair("q", &format!("{q} when:7d"));
                if en {
                    p.append_pair("hl", "en-US").append_pair("gl", "US").append_pair("ceid", "US:en");
                } else {
                    p.append_pair("hl", "ru").append_pair("gl", "RU").append_pair("ceid", "RU:ru");
                }
            }
            SearchEngine::Bing => {
                p.append_pair("q", &q).append_pair("format", "rss").append_pair("setlang", if en { "en" } else { "ru" });
            }
        }
    }
    u.to_string()
}

// ---------- fetching ----------

/// A downloaded page.
#[derive(Clone, Debug, Default)]
pub struct Page {
    pub bytes: Vec<u8>,
    pub content_type: String,
    /// Where the page ended up after redirects.
    pub final_url: String,
}

/// Downloads a web address. The app uses [`HttpFetcher`]; tests use canned responses.
pub trait Fetch: Send + Sync {
    fn get(&self, url: &str) -> Result<Vec<u8>>;
    fn get_page(&self, url: &str) -> Result<Page> {
        Ok(Page { bytes: self.get(url)?, content_type: String::new(), final_url: url.to_string() })
    }
    /// Downloads an image, allowing larger files than pages.
    fn get_image(&self, url: &str) -> Result<Vec<u8>> {
        self.get(url)
    }
    /// Posts a URL-encoded form.
    fn post_form(&self, _url: &str, _body: &str) -> Result<Vec<u8>> {
        Err(Error::invalid("Not supported."))
    }
    /// Calls a web API (JSON or XML), saying honestly that it's CocoaDesk asking.
    fn get_api(&self, url: &str, _headers: &[(&str, &str)]) -> Result<Vec<u8>> {
        self.get(url)
    }
    /// Posts JSON to a web API.
    fn post_json(&self, _url: &str, _body: &str) -> Result<Vec<u8>> {
        Err(Error::invalid("Not supported."))
    }
    /// Downloads a document such as a PDF, allowing larger files than pages.
    fn get_file(&self, url: &str) -> Result<Page> {
        self.get_page(url)
    }
}

/// How CocoaDesk names itself to web APIs, as Wikipedia and others ask.
pub const API_USER_AGENT: &str = concat!("CocoaDesk/", env!("CARGO_PKG_VERSION"), " (https://github.com/CocoaSuite/cocoadesk)");

pub struct HttpFetcher {
    agent: ureq::Agent,
}

const MAX_BYTES: u64 = 8 * 1024 * 1024;
const MAX_IMAGE_BYTES: u64 = 64 * 1024 * 1024;

impl HttpFetcher {
    pub fn new() -> Self {
        let config = ureq::Agent::config_builder()
            .timeout_global(Some(Duration::from_secs(25)))
            // Many news websites turn away anything that doesn't look like a browser.
            .user_agent(concat!(
                "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/131.0 Safari/537.36 CocoaDesk/",
                env!("CARGO_PKG_VERSION")
            ))
            .build();
        HttpFetcher { agent: config.into() }
    }
}

impl Default for HttpFetcher {
    fn default() -> Self {
        Self::new()
    }
}

fn host_of(url: &str) -> String {
    url::Url::parse(url).ok().and_then(|u| u.host_str().map(String::from)).unwrap_or_else(|| url.to_string())
}

fn net_error(host: &str, e: ureq::Error) -> Error {
    Error::invalid(match e {
        ureq::Error::StatusCode(404) => format!("{host} says the page doesn't exist (404)."),
        ureq::Error::StatusCode(403) | ureq::Error::StatusCode(401) => format!("{host} refused the request."),
        ureq::Error::StatusCode(429) => format!("{host} asked us to slow down; trying again later."),
        ureq::Error::StatusCode(c) => format!("{host} answered with an error ({c})."),
        ureq::Error::Timeout(_) => format!("{host} took too long to answer."),
        ureq::Error::HostNotFound => format!("Couldn't find {host}. Are you online?"),
        ureq::Error::Io(_) | ureq::Error::ConnectionFailed => format!("Couldn't connect to {host}."),
        other => format!("Couldn't load {host}: {other}"),
    })
}

impl Fetch for HttpFetcher {
    fn get(&self, url: &str) -> Result<Vec<u8>> {
        Ok(self.get_page(url)?.bytes)
    }

    fn get_page(&self, url: &str) -> Result<Page> {
        use ureq::ResponseExt;
        let host = host_of(url);
        let mut resp = self
            .agent
            .get(url)
            .header(
                "Accept",
                "application/rss+xml, application/atom+xml, application/xml;q=0.9, text/xml;q=0.9, text/html;q=0.8, */*;q=0.5",
            )
            .header("Accept-Language", "ru,en;q=0.8")
            .call()
            .map_err(|e| net_error(&host, e))?;
        let final_url = resp.get_uri().to_string();
        let content_type = resp.headers().get("content-type").and_then(|v| v.to_str().ok()).unwrap_or_default().to_string();
        let bytes = resp
            .body_mut()
            .with_config()
            .limit(MAX_BYTES)
            .read_to_vec()
            .map_err(|e| Error::invalid(format!("Couldn't read {host}: {e}")))?;
        Ok(Page { bytes, content_type, final_url })
    }

    fn get_image(&self, url: &str) -> Result<Vec<u8>> {
        let host = host_of(url);
        // Some websites only hand out their pictures to pages of their own.
        let referer = url::Url::parse(url).ok().map(|u| format!("{}/", u.origin().ascii_serialization()));
        let mut req = self.agent.get(url).header("Accept", "image/avif,image/webp,image/png,image/*;q=0.8,*/*;q=0.5");
        if let Some(r) = &referer {
            req = req.header("Referer", r);
        }
        let mut resp = req.call().map_err(|e| net_error(&host, e))?;
        resp.body_mut()
            .with_config()
            .limit(MAX_IMAGE_BYTES)
            .read_to_vec()
            .map_err(|e| Error::invalid(format!("Couldn't download the image from {host}: {e}")))
    }

    fn post_form(&self, url: &str, body: &str) -> Result<Vec<u8>> {
        let host = host_of(url);
        let mut resp = self
            .agent
            .post(url)
            .header("Content-Type", "application/x-www-form-urlencoded;charset=UTF-8")
            .send(body)
            .map_err(|e| net_error(&host, e))?;
        resp.body_mut().with_config().limit(MAX_BYTES).read_to_vec().map_err(|e| Error::invalid(format!("Couldn't read {host}: {e}")))
    }

    fn get_api(&self, url: &str, headers: &[(&str, &str)]) -> Result<Vec<u8>> {
        let host = host_of(url);
        let mut req = self
            .agent
            .get(url)
            .header("User-Agent", API_USER_AGENT)
            .header("Accept", "application/json, application/atom+xml;q=0.9, */*;q=0.5");
        for (k, v) in headers {
            req = req.header(*k, *v);
        }
        let mut resp = req.call().map_err(|e| net_error(&host, e))?;
        resp.body_mut().with_config().limit(MAX_BYTES).read_to_vec().map_err(|e| Error::invalid(format!("Couldn't read {host}: {e}")))
    }

    fn post_json(&self, url: &str, body: &str) -> Result<Vec<u8>> {
        let host = host_of(url);
        let mut resp = self
            .agent
            .post(url)
            .header("Content-Type", "application/json")
            .header("Accept", "application/json")
            .send(body)
            .map_err(|e| net_error(&host, e))?;
        resp.body_mut().with_config().limit(MAX_BYTES).read_to_vec().map_err(|e| Error::invalid(format!("Couldn't read {host}: {e}")))
    }

    fn get_file(&self, url: &str) -> Result<Page> {
        use ureq::ResponseExt;
        let host = host_of(url);
        let mut resp = self
            .agent
            .get(url)
            .header("Accept", "application/pdf, text/html;q=0.8, */*;q=0.5")
            .header("Accept-Language", "ru,en;q=0.8")
            .call()
            .map_err(|e| net_error(&host, e))?;
        let final_url = resp.get_uri().to_string();
        let content_type = resp.headers().get("content-type").and_then(|v| v.to_str().ok()).unwrap_or_default().to_string();
        let bytes = resp
            .body_mut()
            .with_config()
            .limit(MAX_IMAGE_BYTES)
            .read_to_vec()
            .map_err(|e| Error::invalid(format!("Couldn't download the file from {host}: {e}")))?;
        Ok(Page { bytes, content_type, final_url })
    }
}

/// Turns what a person pastes — a feed address or a website — into a feed address and title.
pub fn discover_feed(fetcher: &dyn Fetch, input: &str) -> Result<(String, String)> {
    let mut addr = input.trim().to_string();
    if !addr.contains("://") {
        addr = format!("https://{addr}");
    }
    let base = url::Url::parse(&addr).map_err(|_| Error::invalid("That isn't a web address."))?;
    if base.scheme() != "http" && base.scheme() != "https" {
        return Err(Error::invalid("That isn't a web address."));
    }
    let bytes = fetcher.get(base.as_str())?;
    if let Ok(f) = parse_feed(&bytes) {
        let title = if f.title.is_empty() { base.host_str().unwrap_or_default().to_string() } else { f.title };
        return Ok((base.to_string(), title));
    }
    let html = String::from_utf8_lossy(&bytes);
    for tag in FEED_LINK.find_iter(&html) {
        if let Some(href) = HREF.captures(tag.as_str()) {
            let href = plain_text(&href[1]);
            if let Ok(feed_url) = base.join(&href) {
                if let Ok(bytes) = fetcher.get(feed_url.as_str()) {
                    if let Ok(f) = parse_feed(&bytes) {
                        let title = if f.title.is_empty() { base.host_str().unwrap_or_default().to_string() } else { f.title };
                        return Ok((feed_url.to_string(), title));
                    }
                }
            }
        }
    }
    Err(Error::invalid("Couldn't find a news feed on that page. Look for an RSS link on the website."))
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::news::{MatchIn, NotifyMode};

    pub const GOOGLE: &str = r##"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<rss version="2.0" xmlns:media="http://search.yahoo.com/mrss/"><channel><title>"автомоб" - Google Новости</title>
<item><title>Продажи автомобилей выросли на 12% - РБК</title><link>https://news.google.com/rss/articles/AAA?oc=5</link>
<guid isPermaLink="false">AAA</guid><pubDate>Mon, 16 Nov 2026 05:10:00 GMT</pubDate>
<description>&lt;a href="https://news.google.com/rss/articles/AAA?oc=5" target="_blank"&gt;Продажи автомобилей выросли на 12%&lt;/a&gt;&amp;nbsp;&amp;nbsp;&lt;font color="#6f6f6f"&gt;РБК&lt;/font&gt;</description>
<source url="https://www.rbc.ru">РБК</source></item>
<item><title>Продажи автомобилей в октябре выросли на 12% - Лента.ру</title><link>https://news.google.com/rss/articles/BBB?oc=5</link>
<guid isPermaLink="false">BBB</guid><pubDate>Mon, 16 Nov 2026 04:00:00 GMT</pubDate>
<description>&lt;a href="x"&gt;Продажи автомобилей в октябре выросли на 12%&lt;/a&gt;</description>
<source url="https://lenta.ru">Лента.ру</source></item>
</channel></rss>"##;

    pub const ATOM: &str = r##"<?xml version="1.0" encoding="utf-8"?>
<feed xmlns="http://www.w3.org/2005/Atom"><title>Dev blog</title>
<entry><title>Release 2.0 adds tray icons</title><link href="https://blog.example.com/2-0?utm_source=rss"/><id>tag:1</id>
<updated>2026-11-15T10:00:00Z</updated><summary type="html">&lt;p&gt;We &amp;amp; you: &lt;b&gt;tray&lt;/b&gt; support.&lt;/p&gt;</summary></entry>
</feed>"##;

    #[test]
    fn reads_news_search_results_with_their_sources() {
        let f = parse_feed(GOOGLE.as_bytes()).unwrap();
        assert_eq!(f.entries.len(), 2);
        let e = &f.entries[0];
        assert_eq!(e.title, "Продажи автомобилей выросли на 12%");
        assert_eq!(e.source_name, "РБК");
        assert_eq!(e.domain(), "rbc.ru");
        assert_eq!(e.summary, "");
        assert!(e.published.is_some());
        assert_eq!(f.entries[1].domain(), "lenta.ru");
    }

    #[test]
    fn reads_atom_and_cleans_html() {
        let f = parse_feed(ATOM.as_bytes()).unwrap();
        assert_eq!(f.title, "Dev blog");
        assert_eq!(f.entries[0].summary, "We & you: tray support.");
        assert_eq!(f.entries[0].domain(), "blog.example.com");
    }

    #[test]
    fn unwraps_bing_links() {
        assert_eq!(
            unwrap_link("http://www.bing.com/news/apiclick.aspx?ref=FexRss&url=https%3a%2f%2flenta.ru%2fnews%2f1&c=1"),
            "https://lenta.ru/news/1"
        );
    }

    #[test]
    fn rejects_non_feeds() {
        assert!(parse_feed(b"<html><body>hello</body></html>").is_err());
    }

    #[test]
    fn builds_search_queries() {
        let cfg = TopicConfig {
            include: vec!["автомоб*".into(), "\"электрокар\"".into(), "Tesla Model".into()],
            exclude: vec!["реклама".into()],
            blocked_sites: vec!["rbc.ru".into()],
            feeds: vec![],
            search: Some(SearchEngine::Google),
            language: "ru".into(),
            match_in: MatchIn::TitleAndSummary,
            interval_minutes: 30,
            active_from: None,
            active_to: None,
            notify: NotifyMode::Off,
            notify_words: vec![],
        };
        assert_eq!(
            search_query(&cfg, &["lenta.ru".into()]),
            "(автомоб OR \"электрокар\" OR \"Tesla Model\") -реклама -site:rbc.ru -site:lenta.ru"
        );
        let u = search_url(SearchEngine::Google, &cfg, &[]);
        assert!(u.starts_with("https://news.google.com/rss/search?q="));
        assert!(u.contains("ceid=RU%3Aru"));
        assert!(search_url(SearchEngine::Bing, &cfg, &[]).contains("format=rss"));
    }

    struct Canned(HashMap<String, String>);
    impl Fetch for Canned {
        fn get(&self, url: &str) -> Result<Vec<u8>> {
            self.0.get(url).map(|s| s.clone().into_bytes()).ok_or_else(|| Error::invalid("not found"))
        }
    }

    #[test]
    fn finds_the_feed_behind_a_website() {
        let mut m = HashMap::new();
        m.insert(
            "https://blog.example.com/".to_string(),
            r#"<html><head><link rel="alternate" type="application/atom+xml" href="/feed.xml"></head></html>"#.to_string(),
        );
        m.insert("https://blog.example.com/feed.xml".to_string(), ATOM.to_string());
        let (u, t) = discover_feed(&Canned(m), "blog.example.com").unwrap();
        assert_eq!(u, "https://blog.example.com/feed.xml");
        assert_eq!(t, "Dev blog");
    }
}
