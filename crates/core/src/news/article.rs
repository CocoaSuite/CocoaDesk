//! Reading a story inside CocoaDesk: finding the real article behind a news-search link,
//! extracting its text (the readability.js method) and cleaning the HTML for display.

use std::collections::{HashMap, HashSet};
use std::sync::LazyLock;

use regex::Regex;
use serde::{Deserialize, Serialize};

use super::feed::{plain_text, Fetch, Page};
use crate::{Error, Result};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Article {
    /// The article's own address (after unwrapping news-search links).
    pub url: String,
    pub title: String,
    pub byline: Option<String>,
    pub site_name: Option<String>,
    pub image: Option<String>,
    /// Cleaned HTML, safe to display.
    pub html: String,
    /// The opening sentences, used for the day's summary.
    pub lead: String,
    pub words: usize,
}

static NEWS_ATTR: LazyLock<Regex> = LazyLock::new(|| Regex::new(r#"data-n-a-(id|ts|sg)="([^"]+)""#).unwrap());
static GARTURL: LazyLock<Regex> = LazyLock::new(|| Regex::new(r#"\\"garturlres\\",\\"([^\\]+)\\""#).unwrap());
static META_CHARSET: LazyLock<Regex> = LazyLock::new(|| Regex::new(r#"(?i)<meta[^>]+charset\s*=\s*["']?([A-Za-z0-9_\-]+)"#).unwrap());
static SENTENCE_END: LazyLock<Regex> = LazyLock::new(|| Regex::new(r#"[.!?…]["»”)]?\s+["«“(]?[A-ZА-ЯЁ0-9]"#).unwrap());

/// Google News search results link to a Google page that forwards to the article with
/// JavaScript. This asks Google for the real address the same way that page does.
pub fn resolve_google_news(fetcher: &dyn Fetch, url: &str) -> Result<String> {
    let page = fetcher.get_page(url)?;
    let html = String::from_utf8_lossy(&page.bytes);
    let attrs: HashMap<String, String> = NEWS_ATTR.captures_iter(&html).map(|c| (c[1].to_string(), c[2].to_string())).collect();
    let (Some(id), Some(ts), Some(sg)) = (attrs.get("id"), attrs.get("ts"), attrs.get("sg")) else {
        return Err(Error::invalid("Google News didn't say where this story is."));
    };
    let inner = format!(
        r#"["garturlreq",[["X","X",["X","X"],null,null,1,1,"US:en",null,1,null,null,null,null,null,0,1],"X","X",1,[1,1,1],1,1,null,0,0,null,0],"{id}",{ts},"{sg}"]"#
    );
    let req = serde_json::json!([[["Fbv4je", inner, null, "generic"]]]).to_string();
    let body = format!("f.req={}", url::form_urlencoded::byte_serialize(req.as_bytes()).collect::<String>());
    let resp = fetcher.post_form("https://news.google.com/_/DotsSplashUi/data/batchexecute", &body)?;
    let text = String::from_utf8_lossy(&resp);
    GARTURL
        .captures(&text)
        .map(|c| c[1].replace("\\u003d", "=").replace("\\u0026", "&"))
        .filter(|u| u.starts_with("http"))
        .ok_or_else(|| Error::invalid("Google News didn't say where this story is."))
}

pub fn is_google_news(url: &str) -> bool {
    url.starts_with("https://news.google.com/") && url.contains("/articles/")
}

/// Turns page bytes into text using the charset from the headers or the page itself
/// (many Russian websites still use windows-1251).
pub fn decode_page(page: &Page) -> String {
    let from_header = page.content_type.to_lowercase().split("charset=").nth(1).map(|s| s.trim().trim_matches('"').to_string());
    let head = String::from_utf8_lossy(&page.bytes[..page.bytes.len().min(4096)]).into_owned();
    let from_meta = META_CHARSET.captures(&head).map(|c| c[1].to_lowercase());
    let label = from_header.or(from_meta).unwrap_or_else(|| "utf-8".into());
    let enc = encoding_rs::Encoding::for_label(label.as_bytes()).unwrap_or(encoding_rs::UTF_8);
    enc.decode(&page.bytes).0.into_owned()
}

/// Keeps text, structure, links and pictures; removes scripts, styles, forms and anything
/// that could run or track.
pub fn sanitize(html: &str) -> String {
    let tags: HashSet<&str> = [
        "p",
        "br",
        "h2",
        "h3",
        "h4",
        "h5",
        "ul",
        "ol",
        "li",
        "blockquote",
        "a",
        "img",
        "figure",
        "figcaption",
        "strong",
        "b",
        "em",
        "i",
        "u",
        "s",
        "sub",
        "sup",
        "pre",
        "code",
        "table",
        "thead",
        "tbody",
        "tr",
        "td",
        "th",
        "hr",
        "span",
        "div",
    ]
    .into_iter()
    .collect();
    let mut attrs: HashMap<&str, HashSet<&str>> = HashMap::new();
    attrs.insert("a", ["href"].into_iter().collect());
    attrs.insert("img", ["src", "alt"].into_iter().collect());
    attrs.insert("td", ["colspan", "rowspan"].into_iter().collect());
    attrs.insert("th", ["colspan", "rowspan"].into_iter().collect());
    ammonia::Builder::default()
        .tags(tags)
        .tag_attributes(attrs)
        .generic_attributes(HashSet::new())
        .url_schemes(["https", "http"].into_iter().collect())
        .link_rel(Some("noopener noreferrer"))
        .attribute_filter(|el, attr, value| {
            // Only load pictures over https.
            if el == "img" && attr == "src" && !value.starts_with("https://") {
                None
            } else {
                Some(value.into())
            }
        })
        .clean(html)
        .to_string()
}

/// The opening one or two sentences of the article, at most about 300 characters.
pub fn lead_of(text: &str) -> String {
    let para = text
        .split(['\n', '\r'])
        .map(|p| p.split_whitespace().collect::<Vec<_>>().join(" "))
        .find(|p| p.chars().count() >= 60)
        .unwrap_or_default();
    if para.is_empty() {
        return String::new();
    }
    let mut ends: Vec<usize> =
        SENTENCE_END.find_iter(&para).map(|m| m.start() + m.as_str().find(char::is_whitespace).unwrap_or(1)).collect();
    ends.push(para.len());
    let mut out = String::new();
    let mut start = 0;
    for e in ends {
        let sentence = para[start..e].trim();
        if !out.is_empty() && out.chars().count() + sentence.chars().count() > 300 {
            break;
        }
        if !out.is_empty() {
            out.push(' ');
        }
        out.push_str(sentence);
        start = e;
        if out.chars().count() >= 100 {
            break;
        }
    }
    if out.chars().count() > 320 {
        let cut: String = out.chars().take(300).collect();
        out = format!("{}…", cut.rsplit_once(' ').map(|(a, _)| a).unwrap_or(&cut));
    }
    out
}

/// Extracts the readable part of an article page.
pub fn extract(html: &str, url: &str) -> Result<Article> {
    let cfg = dom_smoothie::Config { max_elements_to_parse: 20_000, ..Default::default() };
    let mut r =
        dom_smoothie::Readability::new(html, Some(url), Some(cfg)).map_err(|e| Error::invalid(format!("Couldn't read the page ({e}).")))?;
    let a = r.parse().map_err(|_| Error::invalid("Couldn't find the article text on this page."))?;
    let text = a.text_content.to_string();
    let words = text.split_whitespace().count();
    if words < 40 {
        return Err(Error::invalid("This page shows very little text — it may need a subscription or only work in a browser."));
    }
    let lead = lead_from_html(&a.content);
    Ok(Article {
        url: url.to_string(),
        title: plain_text(&a.title),
        byline: a.byline.map(|b| plain_text(&b)).filter(|b| !b.is_empty()),
        site_name: a.site_name.map(|s| plain_text(&s)).filter(|s| !s.is_empty()),
        image: a.image.filter(|i| i.starts_with("https://")),
        html: sanitize(&a.content),
        lead,
        words,
    })
}

static PARAGRAPH: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?is)<p\b[^>]*>(.*?)</p>").unwrap());
static BLOCK_END: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)</(p|div|li|h[1-6]|blockquote|figure|section|article)>|<br\s*/?>").unwrap());

/// The lead from the article's first real paragraph: a `<p>` with a full sentence in it, so
/// breadcrumbs, captions and bylines are passed over.
pub fn lead_from_html(html: &str) -> String {
    let sentence = Regex::new(r"[.!?…]").unwrap();
    for c in PARAGRAPH.captures_iter(html) {
        let text = plain_text(&c[1]);
        if text.chars().count() >= 60 && sentence.is_match(&text) {
            return lead_of(&text);
        }
    }
    let blocks = BLOCK_END.replace_all(html, "\n");
    lead_of(&blocks.lines().map(plain_text).collect::<Vec<_>>().join("\n"))
}

/// Downloads and extracts an article, unwrapping news-search links first.
pub fn fetch_article(fetcher: &dyn Fetch, url: &str) -> Result<Article> {
    let real = if is_google_news(url) { resolve_google_news(fetcher, url)? } else { url.to_string() };
    let page = fetcher.get_page(&real)?;
    let ct = page.content_type.to_lowercase();
    if !ct.is_empty() && !ct.contains("html") && !ct.contains("xml") {
        return Err(Error::invalid("This link isn't a web page (it may be a PDF or a video)."));
    }
    let html = decode_page(&page);
    let base = if page.final_url.is_empty() { real.clone() } else { page.final_url.clone() };
    extract(&html, &base)
}

#[cfg(test)]
mod tests {
    use super::*;

    const PAGE: &str = r#"<!doctype html><html><head><meta charset="utf-8"><title>Продажи электромобилей выросли — Пример</title>
<meta property="og:site_name" content="Пример Новости"><meta property="og:image" content="https://example.ru/pic.jpg"></head>
<body><nav><a href="/">Главная</a> <a href="/news">Новости</a></nav>
<article><h1>Продажи электромобилей выросли</h1><p class="byline">Иван Петров</p>
<p>Продажи новых электромобилей в России в октябре выросли на 12% по сравнению с сентябрём, сообщили аналитики агентства. Больше всего покупали кроссоверы.</p>
<p>Эксперты связывают рост с расширением сети зарядных станций в крупных городах и снижением цен на ряд моделей. <a href="https://example.ru/stations" onclick="steal()">Подробнее о станциях</a>.</p>
<p>По прогнозу аналитиков, к концу года доля электромобилей на рынке может достигнуть трёх процентов, если тенденция сохранится.</p>
<script>alert(1)</script><img src="http://insecure.example/x.png"><img src="https://example.ru/chart.png" onerror="x()" alt="График">
</article><footer>© Пример</footer></body></html>"#;

    #[test]
    fn extracts_and_cleans_an_article() {
        let a = extract(PAGE, "https://example.ru/news/1").unwrap();
        assert!(a.html.contains("Продажи новых электромобилей"));
        assert!(!a.html.contains("script") && !a.html.contains("onclick") && !a.html.contains("onerror"));
        assert!(!a.html.contains("http://insecure.example"));
        assert!(a.html.contains("https://example.ru/chart.png"));
        assert!(!a.html.contains("Главная"), "navigation is dropped");
        assert_eq!(a.site_name.as_deref(), Some("Пример Новости"));
        assert!(a.lead.starts_with("Продажи новых электромобилей в России в октябре выросли на 12%"), "{}", a.lead);
        assert!(a.lead.ends_with("агентства."), "{}", a.lead);
    }

    #[test]
    fn lead_takes_whole_sentences() {
        let t = "Коротко.\nВ понедельник Банк России сохранил ключевую ставку на уровне 16% годовых. Решение совпало с ожиданиями. Следующее заседание пройдёт в декабре, сообщили в регуляторе.";
        assert_eq!(lead_of(t), "В понедельник Банк России сохранил ключевую ставку на уровне 16% годовых. Решение совпало с ожиданиями.");
    }

    #[test]
    fn decodes_windows_1251() {
        let (bytes, _, _) =
            encoding_rs::WINDOWS_1251.encode("<html><head><meta charset=\"windows-1251\"></head><body>Привет</body></html>");
        let page = Page { bytes: bytes.into_owned(), content_type: "text/html".into(), final_url: String::new() };
        assert!(decode_page(&page).contains("Привет"));
    }

    struct Google;
    impl Fetch for Google {
        fn get(&self, url: &str) -> Result<Vec<u8>> {
            if url.contains("news.google.com/rss/articles/") {
                Ok(br#"<c-wiz data-n-a-id="CBMiXYZ" data-n-a-ts="1794810939" data-n-a-sg="AdwtSYN2"></c-wiz>"#.to_vec())
            } else if url == "https://example.ru/news/1" {
                Ok(PAGE.as_bytes().to_vec())
            } else {
                Err(Error::invalid("offline"))
            }
        }
        fn post_form(&self, url: &str, body: &str) -> Result<Vec<u8>> {
            assert!(url.ends_with("/batchexecute"));
            assert!(body.contains("CBMiXYZ") && body.contains("AdwtSYN2"));
            Ok(br#")]}'

[["wrb.fr","Fbv4je","[\"garturlres\",\"https://example.ru/news/1\",1]",null,null,null,"generic"]]"#
                .to_vec())
        }
    }

    #[test]
    fn follows_google_news_links_to_the_article() {
        let a = fetch_article(&Google, "https://news.google.com/rss/articles/CBMiXYZ?oc=5").unwrap();
        assert_eq!(a.url, "https://example.ru/news/1");
        assert!(a.words > 40);
    }
}
