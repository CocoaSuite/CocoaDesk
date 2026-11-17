//! The scholarly databases: how to ask each one, and how to read its answers.
//!
//! - **OpenAlex** — every field, about 250 million works, citation counts. Its API asks for a
//!   free key; without one there is only a small shared daily allowance.
//! - **arXiv** — preprints in physics, mathematics, computer science and nearby fields.
//! - **Semantic Scholar** — citations, references and similar papers. Works without a key,
//!   but answers less often when it's busy.
//! - **Europe PMC** — biology and medicine, with many full texts.
//! - **CyberLeninka** — Russian journals, all open access, with full text on each page.
//!
//! Plus **Wikipedia** for the short overview above a search, and **Unpaywall** (with an email)
//! for finding a free PDF of a paper that has a DOI.

use std::collections::HashMap;
use std::sync::LazyLock;

use chrono::NaiveDate;
use regex::Regex;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::{clean_arxiv, clean_doi, Author, KindFilter, Paper, PaperKind, Query, SortBy, Source};
use crate::news::feed::{plain_text, Fetch};
use crate::news::{normalize, stem, words};
use crate::{Error, Result};

/// Keys and contact details some databases ask for. All optional.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Keys {
    #[serde(default)]
    pub openalex: String,
    #[serde(default)]
    pub semantic_scholar: String,
    /// Sent to Unpaywall and OpenAlex, which ask callers to say who they are.
    #[serde(default)]
    pub email: String,
}

/// How many papers to ask each database for.
pub const PER_SOURCE: usize = 20;

fn enc(s: &str) -> String {
    url::form_urlencoded::byte_serialize(s.as_bytes()).collect()
}

fn s(v: &Value) -> String {
    v.as_str().map(|x| x.trim().to_string()).unwrap_or_default()
}

fn opt_s(v: &Value) -> Option<String> {
    v.as_str().map(|x| x.trim().to_string()).filter(|x| !x.is_empty())
}

fn json(bytes: &[u8], who: &str) -> Result<Value> {
    serde_json::from_slice(bytes).map_err(|_| Error::invalid(format!("{who} sent an answer CocoaDesk couldn't read.")))
}

fn parse_date(s: &str) -> Option<NaiveDate> {
    let s = s.trim();
    NaiveDate::parse_from_str(s.get(..10).unwrap_or(s), "%Y-%m-%d").ok()
}

fn https(u: &str) -> String {
    match u.strip_prefix("http://") {
        Some(rest) => format!("https://{rest}"),
        None => u.to_string(),
    }
}

// ---------- OpenAlex ----------

fn openalex_filters(q: &Query) -> Vec<String> {
    let mut f = Vec::new();
    match (q.from_year, q.to_year) {
        (Some(a), Some(b)) => f.push(format!("publication_year:{a}-{b}")),
        (Some(a), None) => f.push(format!("publication_year:>{}", a - 1)),
        (None, Some(b)) => f.push(format!("publication_year:<{}", b + 1)),
        _ => {}
    }
    if q.open_access {
        f.push("is_oa:true".into());
    }
    if let Some(n) = q.min_citations {
        f.push(format!("cited_by_count:>{}", n - 1));
    }
    match q.kind {
        KindFilter::Any => {}
        KindFilter::Article => f.push("type:article|review".into()),
        KindFilter::Preprint => f.push("type:preprint".into()),
        KindFilter::Review => f.push("type:review".into()),
    }
    if let Some(l) = &q.language {
        f.push(format!("language:{l}"));
    }
    f
}

fn openalex_auth(keys: &Keys) -> String {
    let mut s = String::new();
    if !keys.openalex.trim().is_empty() {
        s.push_str(&format!("&api_key={}", enc(keys.openalex.trim())));
    }
    if !keys.email.trim().is_empty() {
        s.push_str(&format!("&mailto={}", enc(keys.email.trim())));
    }
    s
}

pub fn openalex_url(q: &Query, keys: &Keys, sort: SortBy) -> String {
    let mut u = format!("https://api.openalex.org/works?search={}&per_page={PER_SOURCE}", enc(&q.text));
    let f = openalex_filters(q);
    if !f.is_empty() {
        u.push_str(&format!("&filter={}", enc(&f.join(","))));
    }
    match sort {
        SortBy::Newest => u.push_str("&sort=publication_date:desc"),
        SortBy::MostCited => u.push_str("&sort=cited_by_count:desc"),
        SortBy::Relevance => {}
    }
    u + &openalex_auth(keys)
}

/// OpenAlex sends abstracts as a word → positions index; this puts the words back in order.
fn rebuild_abstract(v: &Value) -> String {
    let Some(map) = v.as_object() else { return String::new() };
    let mut slots: Vec<(u64, &str)> = Vec::new();
    for (word, pos) in map {
        for p in pos.as_array().into_iter().flatten() {
            if let Some(p) = p.as_u64() {
                slots.push((p, word.as_str()));
            }
        }
    }
    slots.sort_by_key(|x| x.0);
    slots.into_iter().map(|x| x.1).collect::<Vec<_>>().join(" ")
}

fn openalex_paper(w: &Value) -> Option<Paper> {
    let title = opt_s(&w["title"]).or_else(|| opt_s(&w["display_name"]))?;
    let mut p = Paper::new(&title, Source::OpenAlex);
    p.openalex = opt_s(&w["id"]).map(|id| id.rsplit('/').next().unwrap_or(&id).to_string());
    p.doi = opt_s(&w["doi"]).and_then(|d| clean_doi(&d));
    p.pmid = opt_s(&w["ids"]["pmid"]).map(|x| x.trim_end_matches('/').rsplit('/').next().unwrap_or(&x).to_string());
    p.year = w["publication_year"].as_i64().map(|y| y as i32);
    p.date = opt_s(&w["publication_date"]).and_then(|d| parse_date(&d));
    p.language = opt_s(&w["language"]);
    p.citations = w["cited_by_count"].as_i64();
    p.kind = match w["type"].as_str().unwrap_or("") {
        "article" | "letter" | "editorial" => PaperKind::Article,
        "preprint" => PaperKind::Preprint,
        "review" => PaperKind::Review,
        "book" => PaperKind::Book,
        "book-chapter" => PaperKind::Chapter,
        "dissertation" => PaperKind::Thesis,
        "proceedings-article" => PaperKind::Conference,
        _ => PaperKind::Other,
    };
    p.authors = w["authorships"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|a| opt_s(&a["author"]["display_name"]).or_else(|| opt_s(&a["raw_author_name"])))
        .map(|n| Author::parse(&n))
        .collect();
    let loc = &w["primary_location"];
    p.venue = s(&loc["source"]["display_name"]);
    p.publisher = s(&loc["source"]["host_organization_name"]);
    if loc["source"]["type"].as_str() == Some("conference") && p.kind == PaperKind::Article {
        p.kind = PaperKind::Conference;
    }
    p.url = opt_s(&loc["landing_page_url"]).map(|u| https(&u)).unwrap_or_default();
    let b = &w["biblio"];
    p.volume = s(&b["volume"]);
    p.issue = s(&b["issue"]);
    p.pages = match (opt_s(&b["first_page"]), opt_s(&b["last_page"])) {
        (Some(a), Some(z)) if a != z => format!("{a}–{z}"),
        (Some(a), _) => a,
        _ => String::new(),
    };
    p.open_access = w["open_access"]["is_oa"].as_bool().unwrap_or(false);
    p.pdf_url = opt_s(&w["best_oa_location"]["pdf_url"]).or_else(|| opt_s(&loc["pdf_url"])).map(|u| https(&u));
    p.abstract_text = rebuild_abstract(&w["abstract_inverted_index"]);
    p.keywords = w["keywords"].as_array().into_iter().flatten().filter_map(|k| opt_s(&k["display_name"])).take(8).collect();
    if p.url.contains("arxiv.org/abs/") {
        p.arxiv = clean_arxiv(&p.url);
    }
    if p.url.contains("cyberleninka.ru/article/") {
        p.html_url = Some(p.url.clone());
    }
    Some(p.finish())
}

pub fn parse_openalex(bytes: &[u8]) -> Result<Vec<Paper>> {
    let v = json(bytes, "OpenAlex")?;
    if let Some(e) = opt_s(&v["error"]) {
        return Err(Error::invalid(format!("OpenAlex: {e}")));
    }
    Ok(v["results"].as_array().into_iter().flatten().filter_map(openalex_paper).collect())
}

// ---------- arXiv ----------

/// arXiv's query language wants each word as `all:word`; quoted phrases stay together.
pub fn arxiv_query(text: &str) -> String {
    static TOKEN: LazyLock<Regex> = LazyLock::new(|| Regex::new(r#""([^"]+)"|(\S+)"#).unwrap());
    TOKEN
        .captures_iter(text)
        .filter_map(|c| {
            if let Some(ph) = c.get(1) {
                Some(format!("all:\"{}\"", ph.as_str().trim()))
            } else {
                let w: String = c[2].chars().filter(|ch| ch.is_alphanumeric() || *ch == '-').collect();
                (!w.is_empty() && !matches!(w.to_uppercase().as_str(), "AND" | "OR" | "NOT")).then(|| format!("all:{w}"))
            }
        })
        .collect::<Vec<_>>()
        .join(" AND ")
}

pub fn arxiv_url(q: &Query, sort: SortBy) -> String {
    let mut sq = arxiv_query(&q.text);
    if q.from_year.is_some() || q.to_year.is_some() {
        let a = q.from_year.unwrap_or(1991);
        let b = q.to_year.unwrap_or(2100);
        sq = format!("({sq}) AND submittedDate:[{a}01010000 TO {b}12312359]");
    }
    let sort = match sort {
        SortBy::Newest => "&sortBy=submittedDate&sortOrder=descending",
        _ => "&sortBy=relevance&sortOrder=descending",
    };
    format!("https://export.arxiv.org/api/query?search_query={}&max_results={PER_SOURCE}{sort}", enc(&sq))
}

static ENTRY: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?s)<entry>(.*?)</entry>").unwrap());
static NAME: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?s)<author>\s*<name>(.*?)</name>").unwrap());

fn tag<'a>(xml: &'a str, name: &str) -> Option<&'a str> {
    let open = format!("<{name}");
    let start = xml.find(&open)?;
    let after = &xml[start..];
    let gt = after.find('>')?;
    if after[..gt].ends_with('/') {
        return None;
    }
    let body = &after[gt + 1..];
    let end = body.find(&format!("</{name}>"))?;
    Some(&body[..end])
}

pub fn parse_arxiv(bytes: &[u8]) -> Result<Vec<Paper>> {
    let text = String::from_utf8_lossy(bytes);
    if !text.contains("<feed") {
        return Err(Error::invalid("arXiv sent an answer CocoaDesk couldn't read."));
    }
    let mut out = Vec::new();
    for c in ENTRY.captures_iter(&text) {
        let e = &c[1];
        let Some(title) = tag(e, "title") else { continue };
        let title = plain_text(title);
        if title.is_empty() || title == "Error" {
            continue;
        }
        let mut p = Paper::new(&title, Source::Arxiv);
        let id = tag(e, "id").map(plain_text).unwrap_or_default();
        p.arxiv = clean_arxiv(&id);
        p.kind = PaperKind::Preprint;
        p.open_access = true;
        p.language = Some("en".into());
        p.venue = "arXiv".into();
        p.abstract_text = tag(e, "summary").map(plain_text).unwrap_or_default();
        p.date = tag(e, "published").and_then(parse_date);
        p.authors = NAME.captures_iter(e).map(|a| Author::parse(&plain_text(&a[1]))).collect();
        p.doi = tag(e, "arxiv:doi").and_then(clean_doi);
        if let Some(j) = tag(e, "arxiv:journal_ref").map(plain_text).filter(|j| !j.is_empty()) {
            // The published version; the DOI (if any) points there.
            p.venue = j;
            p.kind = PaperKind::Article;
        }
        p.keywords = Regex::new(r#"<category term="([^"]+)""#).unwrap().captures_iter(e).map(|c| c[1].to_string()).take(4).collect();
        if let Some(a) = &p.arxiv {
            p.url = format!("https://arxiv.org/abs/{a}");
            p.pdf_url = Some(format!("https://arxiv.org/pdf/{a}"));
            // arXiv's HTML version of the paper, for reading inside CocoaDesk.
            p.html_url = Some(format!("https://arxiv.org/html/{a}"));
        }
        out.push(p.finish());
    }
    Ok(out)
}

// ---------- Semantic Scholar ----------

const S2_FIELDS: &str = "title,year,authors,venue,citationCount,externalIds,abstract,tldr,openAccessPdf,publicationDate,url,publicationTypes,journal";
const S2_FIELDS_LIGHT: &str = "title,year,authors,venue,citationCount,externalIds,abstract,openAccessPdf,publicationDate,url,publicationTypes,journal";

pub fn s2_url(q: &Query) -> String {
    let mut u = format!("https://api.semanticscholar.org/graph/v1/paper/search?query={}&limit={PER_SOURCE}&fields={S2_FIELDS}", enc(&q.text));
    match (q.from_year, q.to_year) {
        (Some(a), Some(b)) => u.push_str(&format!("&year={a}-{b}")),
        (Some(a), None) => u.push_str(&format!("&year={a}-")),
        (None, Some(b)) => u.push_str(&format!("&year=-{b}")),
        _ => {}
    }
    if q.open_access {
        u.push_str("&openAccessPdf");
    }
    if let Some(n) = q.min_citations {
        u.push_str(&format!("&minCitationCount={n}"));
    }
    match q.kind {
        KindFilter::Review => u.push_str("&publicationTypes=Review"),
        KindFilter::Article => u.push_str("&publicationTypes=JournalArticle,Review,Conference"),
        _ => {}
    }
    u
}

fn s2_headers(keys: &Keys) -> Vec<(&'static str, String)> {
    if keys.semantic_scholar.trim().is_empty() {
        vec![]
    } else {
        vec![("x-api-key", keys.semantic_scholar.trim().to_string())]
    }
}

pub fn s2_paper(v: &Value) -> Option<Paper> {
    let title = opt_s(&v["title"])?;
    let mut p = Paper::new(&title, Source::SemanticScholar);
    p.s2 = opt_s(&v["paperId"]);
    let ids = &v["externalIds"];
    p.doi = opt_s(&ids["DOI"]).and_then(|d| clean_doi(&d));
    p.arxiv = opt_s(&ids["ArXiv"]).and_then(|a| clean_arxiv(&a));
    p.pmid = opt_s(&ids["PubMed"]);
    p.pmcid = opt_s(&ids["PubMedCentral"]).map(|x| if x.starts_with("PMC") { x } else { format!("PMC{x}") });
    p.year = v["year"].as_i64().map(|y| y as i32);
    p.date = opt_s(&v["publicationDate"]).and_then(|d| parse_date(&d));
    p.citations = v["citationCount"].as_i64();
    p.abstract_text = s(&v["abstract"]);
    p.tldr = opt_s(&v["tldr"]["text"]);
    p.venue = opt_s(&v["journal"]["name"]).or_else(|| opt_s(&v["venue"])).unwrap_or_default();
    p.volume = s(&v["journal"]["volume"]);
    p.pages = s(&v["journal"]["pages"]).replace('-', "–").replace(' ', "");
    p.authors = v["authors"].as_array().into_iter().flatten().filter_map(|a| opt_s(&a["name"])).map(|n| Author::parse(&n)).collect();
    let types: Vec<&str> = v["publicationTypes"].as_array().into_iter().flatten().filter_map(|t| t.as_str()).collect();
    p.kind = if types.contains(&"Review") {
        PaperKind::Review
    } else if types.contains(&"Conference") {
        PaperKind::Conference
    } else if types.contains(&"Book") {
        PaperKind::Book
    } else if types.contains(&"JournalArticle") {
        PaperKind::Article
    } else if p.arxiv.is_some() && p.doi.is_none() {
        PaperKind::Preprint
    } else {
        PaperKind::Article
    };
    p.pdf_url = opt_s(&v["openAccessPdf"]["url"]).map(|u| https(&u));
    p.open_access = p.pdf_url.is_some();
    // Semantic Scholar's own page; the publisher's comes from the DOI.
    p.url = p.doi.as_ref().map(|d| format!("https://doi.org/{d}")).or_else(|| opt_s(&v["url"])).unwrap_or_default();
    if let Some(a) = &p.arxiv {
        p.html_url = Some(format!("https://arxiv.org/html/{a}"));
        p.url = format!("https://arxiv.org/abs/{a}");
    }
    Some(p.finish())
}

pub fn parse_s2(bytes: &[u8]) -> Result<Vec<Paper>> {
    let v = json(bytes, "Semantic Scholar")?;
    if let Some(m) = opt_s(&v["message"]).or_else(|| opt_s(&v["error"])) {
        return Err(Error::invalid(format!("Semantic Scholar: {m}")));
    }
    Ok(v["data"].as_array().into_iter().flatten().filter_map(s2_paper).collect())
}

// ---------- Europe PMC ----------

pub fn epmc_url(q: &Query, sort: SortBy) -> String {
    let mut query = format!("({})", q.text);
    match (q.from_year, q.to_year) {
        (None, None) => {}
        (a, b) => query.push_str(&format!(" AND PUB_YEAR:[{} TO {}]", a.unwrap_or(1600), b.unwrap_or(2100))),
    }
    if q.open_access {
        query.push_str(" AND OPEN_ACCESS:y");
    }
    match q.kind {
        KindFilter::Preprint => query.push_str(" AND SRC:PPR"),
        KindFilter::Review => query.push_str(" AND PUB_TYPE:\"review\""),
        KindFilter::Article => query.push_str(" NOT SRC:PPR"),
        KindFilter::Any => {}
    }
    match q.language.as_deref() {
        Some("ru") => query.push_str(" AND LANG:\"rus\""),
        Some("en") => query.push_str(" AND LANG:\"eng\""),
        _ => {}
    }
    let sort = match sort {
        SortBy::Newest => "&sort=P_PDATE_D%20desc",
        SortBy::MostCited => "&sort=CITED%20desc",
        SortBy::Relevance => "",
    };
    format!("https://www.ebi.ac.uk/europepmc/webservices/rest/search?query={}&format=json&resultType=core&pageSize={PER_SOURCE}{sort}", enc(&query))
}

fn epmc_paper(r: &Value) -> Option<Paper> {
    let title = opt_s(&r["title"])?;
    let mut p = Paper::new(&title, Source::EuropePmc);
    let src = s(&r["source"]);
    let id = s(&r["id"]);
    p.doi = opt_s(&r["doi"]).and_then(|d| clean_doi(&d));
    p.pmid = opt_s(&r["pmid"]);
    p.pmcid = opt_s(&r["pmcid"]);
    p.year = s(&r["pubYear"]).parse().ok();
    p.date = opt_s(&r["firstPublicationDate"]).and_then(|d| parse_date(&d));
    p.citations = s(&r["citedByCount"]).parse().ok().or_else(|| r["citedByCount"].as_i64());
    p.abstract_text = plain_text(&s(&r["abstractText"]));
    p.authors = r["authorList"]["author"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|a| Author::from_parts(&s(&a["lastName"]), &s(&a["firstName"]), &s(&a["fullName"])))
        .filter(|a| !a.family.is_empty())
        .collect();
    let ji = &r["journalInfo"];
    p.venue = opt_s(&ji["journal"]["title"]).or_else(|| opt_s(&r["bookOrReportDetails"]["publisher"])).unwrap_or_default();
    p.volume = s(&ji["volume"]);
    p.issue = s(&ji["issue"]);
    p.pages = s(&r["pageInfo"]).replace('-', "–");
    p.language = match r["language"].as_str() {
        Some("rus") => Some("ru".into()),
        Some("eng") => Some("en".into()),
        _ => None,
    };
    let types: Vec<String> = r["pubTypeList"]["pubType"].as_array().into_iter().flatten().filter_map(|t| t.as_str()).map(|t| t.to_lowercase()).collect();
    p.kind = if src == "PPR" || types.iter().any(|t| t.contains("preprint")) {
        PaperKind::Preprint
    } else if types.iter().any(|t| t.contains("review")) {
        PaperKind::Review
    } else {
        PaperKind::Article
    };
    p.open_access = r["isOpenAccess"].as_str() == Some("Y");
    let urls: Vec<&Value> = r["fullTextUrlList"]["fullTextUrl"].as_array().into_iter().flatten().collect();
    let free = |u: &&&Value| matches!(u["availabilityCode"].as_str(), Some("OA") | Some("F"));
    p.pdf_url = urls.iter().filter(free).find(|u| u["documentStyle"].as_str() == Some("pdf")).and_then(|u| opt_s(&u["url"])).map(|u| https(&u));
    if p.pdf_url.is_none() && p.open_access {
        p.pdf_url = p.pmcid.as_ref().map(|id| format!("https://europepmc.org/articles/{id}?pdf=render"));
    }
    if let Some(pmc) = &p.pmcid {
        p.html_url = Some(format!("https://pmc.ncbi.nlm.nih.gov/articles/{pmc}/"));
    }
    p.keywords = r["keywordList"]["keyword"].as_array().into_iter().flatten().filter_map(opt_s).take(8).collect();
    p.url = format!("https://europepmc.org/article/{src}/{id}");
    Some(p.finish())
}

pub fn parse_epmc(bytes: &[u8]) -> Result<Vec<Paper>> {
    let v = json(bytes, "Europe PMC")?;
    Ok(v["resultList"]["result"].as_array().into_iter().flatten().filter_map(epmc_paper).collect())
}

// ---------- CyberLeninka ----------

pub const CYBERLENINKA_SEARCH: &str = "https://cyberleninka.ru/api/search";

pub fn cyberleninka_body(q: &Query) -> String {
    serde_json::json!({ "mode": "articles", "q": q.text, "size": PER_SOURCE, "from": 0 }).to_string()
}

pub fn parse_cyberleninka(bytes: &[u8]) -> Result<Vec<Paper>> {
    let v = json(bytes, "CyberLeninka")?;
    Ok(v["articles"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|a| {
            let title = opt_s(&a["name"])?;
            let mut p = Paper::new(&title, Source::CyberLeninka);
            p.abstract_text = plain_text(&s(&a["annotation"]));
            p.authors = a["authors"].as_array().into_iter().flatten().filter_map(opt_s).map(|n| Author::parse(&n)).collect();
            p.year = a["year"].as_i64().map(|y| y as i32);
            p.venue = plain_text(&s(&a["journal"]));
            let link = s(&a["link"]);
            if link.is_empty() {
                return None;
            }
            p.url = format!("https://cyberleninka.ru{link}");
            p.pdf_url = Some(format!("{}/pdf", p.url));
            p.html_url = Some(p.url.clone());
            p.open_access = true;
            p.keywords = a["catalogs"].as_array().into_iter().flatten().filter_map(|c| opt_s(&c["acronym"])).collect();
            Some(p.finish())
        })
        .collect())
}

// ---------- searching ----------

/// Asks one database. Errors are worded for people.
pub fn search_one(fetcher: &dyn Fetch, src: Source, q: &Query, keys: &Keys, sort: SortBy) -> Result<Vec<Paper>> {
    let label = src.label();
    let wrap = |e: Error| -> Error {
        let msg = e.to_string();
        if src == Source::OpenAlex && msg.contains("slow down") && keys.openalex.trim().is_empty() {
            Error::invalid("OpenAlex: today's free allowance without a key is used up. Add your free OpenAlex key in Research settings.".to_string())
        } else if src == Source::SemanticScholar && msg.contains("slow down") {
            Error::invalid("Semantic Scholar is busy right now; try again in a minute.".to_string())
        } else if msg.starts_with(label) {
            e
        } else {
            Error::invalid(format!("{label}: {msg}"))
        }
    };
    let result = match src {
        Source::OpenAlex => fetcher.get_api(&openalex_url(q, keys, sort), &[]).and_then(|b| parse_openalex(&b)),
        Source::Arxiv => fetcher.get_api(&arxiv_url(q, sort), &[]).and_then(|b| parse_arxiv(&b)),
        Source::SemanticScholar => {
            let h = s2_headers(keys);
            let hr: Vec<(&str, &str)> = h.iter().map(|(k, v)| (*k, v.as_str())).collect();
            fetcher.get_api(&s2_url(q), &hr).and_then(|b| parse_s2(&b))
        }
        Source::EuropePmc => fetcher.get_api(&epmc_url(q, sort), &[]).and_then(|b| parse_epmc(&b)),
        Source::CyberLeninka => fetcher.post_json(CYBERLENINKA_SEARCH, &cyberleninka_body(q)).and_then(|b| parse_cyberleninka(&b)),
    };
    result.map_err(wrap)
}

/// Asks every useful database at once. Returns each one's list (in its own order) and the
/// problems, which never stop the other databases.
pub fn search_all(fetcher: &dyn Fetch, q: &Query, keys: &Keys, sort: SortBy) -> (Vec<(Source, Vec<Paper>)>, Vec<String>) {
    let srcs = q.useful_sources();
    let results: Vec<(Source, Result<Vec<Paper>>)> = std::thread::scope(|scope| {
        let handles: Vec<_> = srcs.iter().map(|&src| (src, scope.spawn(move || search_one(fetcher, src, q, keys, sort)))).collect();
        handles.into_iter().map(|(src, h)| (src, h.join().unwrap_or_else(|_| Err(Error::invalid(format!("{}: something went wrong.", src.label())))))).collect()
    });
    let mut lists = Vec::new();
    let mut errors = Vec::new();
    for (src, r) in results {
        match r {
            Ok(v) => lists.push((src, v)),
            Err(e) => errors.push(e.to_string()),
        }
    }
    (lists, errors)
}

// ---------- following citations ----------

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Explore {
    /// Papers that cite this one.
    CitedBy,
    /// Papers this one cites.
    References,
    /// Papers on the same subject.
    Similar,
}

fn s2_id(p: &Paper) -> Option<String> {
    p.s2.clone()
        .or_else(|| p.doi.as_ref().map(|d| format!("DOI:{d}")))
        .or_else(|| p.arxiv.as_ref().map(|a| format!("ARXIV:{a}")))
        .or_else(|| p.pmid.as_ref().map(|m| format!("PMID:{m}")))
}

fn explore_s2(fetcher: &dyn Fetch, p: &Paper, kind: Explore, keys: &Keys) -> Result<Vec<Paper>> {
    let id = s2_id(p).ok_or_else(|| Error::invalid("Semantic Scholar doesn't know this paper."))?;
    let id = enc(&id);
    let url = match kind {
        Explore::CitedBy => format!("https://api.semanticscholar.org/graph/v1/paper/{id}/citations?fields={S2_FIELDS_LIGHT}&limit=60"),
        Explore::References => format!("https://api.semanticscholar.org/graph/v1/paper/{id}/references?fields={S2_FIELDS_LIGHT}&limit=100"),
        Explore::Similar => format!("https://api.semanticscholar.org/recommendations/v1/papers/forpaper/{id}?fields={S2_FIELDS_LIGHT}&limit=30"),
    };
    let h = s2_headers(keys);
    let hr: Vec<(&str, &str)> = h.iter().map(|(k, v)| (*k, v.as_str())).collect();
    let v = json(&fetcher.get_api(&url, &hr)?, "Semantic Scholar")?;
    if let Some(m) = opt_s(&v["message"]).or_else(|| opt_s(&v["error"])) {
        return Err(Error::invalid(format!("Semantic Scholar: {m}")));
    }
    let list: Vec<&Value> = match kind {
        Explore::CitedBy => v["data"].as_array().into_iter().flatten().map(|x| &x["citingPaper"]).collect(),
        Explore::References => v["data"].as_array().into_iter().flatten().map(|x| &x["citedPaper"]).collect(),
        Explore::Similar => v["recommendedPapers"].as_array().into_iter().flatten().collect(),
    };
    Ok(list.into_iter().filter_map(s2_paper).collect())
}

/// The OpenAlex id of a paper, looked up by DOI if needed (lookups by DOI are free).
fn openalex_id(fetcher: &dyn Fetch, p: &Paper, keys: &Keys) -> Result<String> {
    if let Some(id) = &p.openalex {
        return Ok(id.clone());
    }
    let doi = p.doi.as_ref().ok_or_else(|| Error::invalid("OpenAlex can only look this paper up by its DOI, and it has none."))?;
    let url = format!("https://api.openalex.org/works/doi:{}?select=id{}", doi, openalex_auth(keys));
    let v = json(&fetcher.get_api(&url, &[])?, "OpenAlex")?;
    opt_s(&v["id"]).map(|id| id.rsplit('/').next().unwrap_or(&id).to_string()).ok_or_else(|| Error::invalid("OpenAlex doesn't know this paper."))
}

fn explore_openalex(fetcher: &dyn Fetch, p: &Paper, kind: Explore, keys: &Keys) -> Result<Vec<Paper>> {
    let id = openalex_id(fetcher, p, keys)?;
    let filter = match kind {
        Explore::CitedBy => format!("cites:{id}"),
        Explore::References => format!("cited_by:{id}"),
        Explore::Similar => format!("related_to:{id}"),
    };
    let sort = if kind == Explore::CitedBy { "&sort=cited_by_count:desc" } else { "" };
    let url = format!("https://api.openalex.org/works?filter={}&per_page=60{sort}{}", enc(&filter), openalex_auth(keys));
    parse_openalex(&fetcher.get_api(&url, &[])?)
}

/// Papers citing, cited by or similar to `p`: Semantic Scholar first, OpenAlex when it's busy
/// or doesn't know the paper.
pub fn explore(fetcher: &dyn Fetch, p: &Paper, kind: Explore, keys: &Keys) -> Result<Vec<Paper>> {
    match explore_s2(fetcher, p, kind, keys) {
        Ok(v) if !v.is_empty() => Ok(v),
        first => match explore_openalex(fetcher, p, kind, keys) {
            Ok(v) => Ok(v),
            Err(e) => match first {
                Ok(v) => Ok(v),
                Err(f) => Err(Error::invalid(format!("{f} {e}"))),
            },
        },
    }
}

// ---------- Wikipedia overview ----------

/// A short description of the subject from Wikipedia, shown above the papers.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Overview {
    pub title: String,
    pub description: String,
    pub extract: String,
    pub image: Option<String>,
    pub url: String,
    pub lang: String,
}

pub fn wikipedia_search_url(lang: &str, text: &str) -> String {
    format!("https://{lang}.wikipedia.org/w/api.php?action=query&list=search&srsearch={}&srlimit=3&srprop=&format=json&utf8=1", enc(text))
}

pub fn wikipedia_summary_url(lang: &str, title: &str) -> String {
    let t: String = url::form_urlencoded::byte_serialize(title.replace(' ', "_").as_bytes()).collect();
    format!("https://{lang}.wikipedia.org/api/rest_v1/page/summary/{t}")
}

pub fn parse_wikipedia_search(bytes: &[u8]) -> Result<Vec<String>> {
    let v = json(bytes, "Wikipedia")?;
    Ok(v["query"]["search"].as_array().into_iter().flatten().filter_map(|x| opt_s(&x["title"])).collect())
}

pub fn parse_wikipedia_summary(bytes: &[u8], lang: &str) -> Result<Option<Overview>> {
    let v = json(bytes, "Wikipedia")?;
    if v["type"].as_str() == Some("disambiguation") {
        return Ok(None);
    }
    let extract = s(&v["extract"]);
    if extract.chars().count() < 40 {
        return Ok(None);
    }
    Ok(Some(Overview {
        title: s(&v["title"]),
        description: s(&v["description"]),
        extract,
        image: opt_s(&v["thumbnail"]["source"]).filter(|u| u.starts_with("https://")),
        url: opt_s(&v["content_urls"]["desktop"]["page"]).unwrap_or_else(|| format!("https://{lang}.wikipedia.org/wiki/{}", enc(&s(&v["title"])))),
        lang: lang.into(),
    }))
}

/// Whether an article is about the query: most of the query's words appear in its title or
/// opening, so a loosely related top hit isn't passed off as the subject.
pub fn overview_matches(text: &str, o: &Overview) -> bool {
    let q = normalize(text);
    let qw: Vec<String> = words(&q).into_iter().filter(|w| w.chars().count() >= 3).map(stem).collect();
    if qw.is_empty() {
        return false;
    }
    let first_sentence: String = o.extract.chars().take(300).collect();
    let hay = normalize(&format!("{} {} {}", o.title, o.description, first_sentence));
    let hw: Vec<String> = words(&hay).into_iter().map(stem).collect();
    let hits = qw.iter().filter(|w| hw.iter().any(|h| h == *w || (w.chars().count() >= 4 && h.starts_with(w.as_str())))).count();
    hits * 2 >= qw.len().max(1) && hits >= 1
}

/// Finds the Wikipedia article for a query, in Russian for Russian queries and in English
/// otherwise. `None` when there's no article clearly about it.
pub fn overview(fetcher: &dyn Fetch, text: &str) -> Result<Option<Overview>> {
    let lang = if super::has_cyrillic(text) { "ru" } else { "en" };
    let titles = parse_wikipedia_search(&fetcher.get_api(&wikipedia_search_url(lang, text), &[])?)?;
    for t in titles.iter().take(2) {
        if let Some(o) = parse_wikipedia_summary(&fetcher.get_api(&wikipedia_summary_url(lang, t), &[])?, lang)? {
            if overview_matches(text, &o) {
                return Ok(Some(o));
            }
        }
    }
    Ok(None)
}

// ---------- free PDFs and page details ----------

/// A free PDF for a DOI, from Unpaywall. Needs an email address.
pub fn unpaywall_pdf(fetcher: &dyn Fetch, doi: &str, email: &str) -> Result<Option<String>> {
    if email.trim().is_empty() {
        return Ok(None);
    }
    let url = format!("https://api.unpaywall.org/v2/{}?email={}", doi, enc(email.trim()));
    let v = json(&fetcher.get_api(&url, &[])?, "Unpaywall")?;
    Ok(opt_s(&v["best_oa_location"]["url_for_pdf"]).map(|u| https(&u)))
}

static META: LazyLock<Regex> = LazyLock::new(|| Regex::new(r#"(?is)<meta\s[^>]*>"#).unwrap());
static META_NAME: LazyLock<Regex> = LazyLock::new(|| Regex::new(r#"(?is)\bname\s*=\s*["'](citation_[a-z_]+)["']"#).unwrap());
static META_CONTENT: LazyLock<Regex> = LazyLock::new(|| Regex::new(r#"(?is)\bcontent\s*=\s*"([^"]*)"|\bcontent\s*=\s*'([^']*)'"#).unwrap());

/// The `citation_*` tags publishers put in their pages for Google Scholar.
pub fn citation_meta(html: &str) -> HashMap<String, Vec<String>> {
    let mut out: HashMap<String, Vec<String>> = HashMap::new();
    for m in META.find_iter(html) {
        let tag = m.as_str();
        let (Some(n), Some(c)) = (META_NAME.captures(tag), META_CONTENT.captures(tag)) else { continue };
        let content = plain_text(c.get(1).or(c.get(2)).map(|x| x.as_str()).unwrap_or(""));
        if !content.is_empty() {
            out.entry(n[1].to_lowercase()).or_default().push(content);
        }
    }
    out
}

/// Fills the details a database left out (issue, pages, DOI…) from the paper's own page.
pub fn enrich_from_meta(p: &mut Paper, meta: &HashMap<String, Vec<String>>) {
    let one = |k: &str| meta.get(k).and_then(|v| v.first()).cloned().unwrap_or_default();
    if p.doi.is_none() {
        p.doi = clean_doi(&one("citation_doi"));
    }
    if p.volume.is_empty() {
        p.volume = one("citation_volume");
    }
    if p.issue.is_empty() {
        p.issue = one("citation_issue");
    }
    if p.pages.is_empty() {
        let (a, b) = (one("citation_firstpage"), one("citation_lastpage"));
        p.pages = match (a.is_empty(), b.is_empty()) {
            (false, false) if a != b => format!("{a}–{b}"),
            (false, _) => a,
            _ => String::new(),
        };
    }
    if p.publisher.is_empty() {
        p.publisher = one("citation_publisher");
    }
    if p.venue.is_empty() {
        p.venue = one("citation_journal_title");
    }
    if p.pdf_url.is_none() {
        p.pdf_url = Some(one("citation_pdf_url")).filter(|u| u.starts_with("http")).map(|u| https(&u));
    }
    if p.authors.is_empty() {
        p.authors = meta.get("citation_author").into_iter().flatten().map(|n| Author::parse(n)).collect();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const OPENALEX: &str = include_str!("testdata/openalex.json");
    const ARXIV: &str = include_str!("testdata/arxiv.xml");
    const S2: &str = include_str!("testdata/s2.json");
    const EPMC: &str = include_str!("testdata/europepmc.json");
    const CL: &str = include_str!("testdata/cyberleninka.json");

    #[test]
    fn reads_openalex() {
        let v = parse_openalex(OPENALEX.as_bytes()).unwrap();
        assert_eq!(v.len(), 2);
        let p = &v[0];
        assert_eq!(p.doi.as_deref(), Some("10.1016/j.carbon.2020.01.002"));
        assert_eq!(p.openalex.as_deref(), Some("W3001"));
        assert_eq!(p.pmid.as_deref(), Some("31234567"));
        assert_eq!(p.venue, "Carbon");
        assert_eq!(p.publisher, "Elsevier BV");
        assert_eq!(p.pages, "101–112");
        assert_eq!(p.abstract_text, "Graphene oxide strengthens cement.");
        assert_eq!(p.authors[0].family, "Petrov");
        assert_eq!(p.citations, Some(42));
        assert!(p.open_access);
        assert_eq!(p.pdf_url.as_deref(), Some("https://europepmc.org/articles/PMC7000001/pdf"));
        let r = &v[1];
        assert_eq!(r.language.as_deref(), Some("ru"));
        assert_eq!(r.authors[1].initials, "В. Ф.");
        assert_eq!(r.html_url.as_deref(), Some("https://cyberleninka.ru/article/n/modifitsirovanie-betona-grafenom-i-oksidom-grafena"));
        assert!(r.key.starts_with("t:"));
    }

    #[test]
    fn reads_arxiv() {
        let v = parse_arxiv(ARXIV.as_bytes()).unwrap();
        assert_eq!(v.len(), 2);
        let p = &v[0];
        assert_eq!(p.arxiv.as_deref(), Some("2611.18490"));
        assert_eq!(p.kind, PaperKind::Preprint);
        assert_eq!(p.pdf_url.as_deref(), Some("https://arxiv.org/pdf/2611.18490"));
        assert_eq!(p.authors[0].family, "Usuga");
        assert_eq!(p.year, Some(2026));
        assert!(p.abstract_text.starts_with("Atomistic foundation models"));
        assert_eq!(p.key, "arxiv:2611.18490");
    }

    #[test]
    fn reads_semantic_scholar() {
        let v = parse_s2(S2.as_bytes()).unwrap();
        assert_eq!(v[0].doi.as_deref(), Some("10.1016/j.carbon.2020.01.002"));
        assert_eq!(v[0].tldr.as_deref(), Some("Adding graphene oxide makes cement stronger."));
        assert_eq!(v[0].pages, "101–112");
        assert!(!v[0].open_access, "an empty PDF address isn't a free copy");
        assert_eq!(v[1].arxiv.as_deref(), Some("2611.18318"));
        assert!(v[1].open_access);
        assert!(parse_s2(br#"{"message":"Too Many Requests","code":"429"}"#).is_err());
    }

    #[test]
    fn reads_europe_pmc() {
        let v = parse_epmc(EPMC.as_bytes()).unwrap();
        let p = &v[0];
        assert_eq!(p.pmcid.as_deref(), Some("PMC13750013"));
        assert_eq!(p.authors[0].family, "Xu");
        assert_eq!(p.authors[0].initials, "X.");
        assert!(p.venue.starts_with("Advanced science"));
        assert!(p.open_access);
        assert_eq!(p.html_url.as_deref(), Some("https://pmc.ncbi.nlm.nih.gov/articles/PMC13750013/"));
        assert!(!p.title.ends_with('.'));
    }

    #[test]
    fn reads_cyberleninka() {
        let v = parse_cyberleninka(CL.as_bytes()).unwrap();
        let p = &v[0];
        assert_eq!(p.title, "Модифицирование бетона графеном и оксидом графена");
        assert_eq!(p.authors[0].family, "Машхадани");
        assert_eq!(p.year, Some(2020));
        assert!(p.url.starts_with("https://cyberleninka.ru/article/n/"));
        assert!(p.pdf_url.as_deref().unwrap().ends_with("/pdf"));
        assert_eq!(p.language.as_deref(), Some("ru"));
    }

    #[test]
    fn builds_requests() {
        let mut q = Query::simple("graphene \"oxide layer\" AND cement");
        q.from_year = Some(2020);
        q.open_access = true;
        q.min_citations = Some(5);
        q.kind = KindFilter::Review;
        q.language = Some("en".into());
        let u = openalex_url(&q, &Keys { openalex: "k1".into(), ..Default::default() }, SortBy::Newest);
        assert!(u.contains("filter=publication_year%3A%3E2019%2Cis_oa%3Atrue%2Ccited_by_count%3A%3E4%2Ctype%3Areview%2Clanguage%3Aen"), "{u}");
        assert!(u.contains("sort=publication_date:desc") && u.contains("api_key=k1"));
        assert_eq!(arxiv_query(&q.text), "all:graphene AND all:\"oxide layer\" AND all:cement");
        assert!(arxiv_url(&q, SortBy::Newest).contains("submittedDate%3A%5B202001010000"));
        let s = s2_url(&q);
        assert!(s.contains("&year=2020-") && s.contains("&openAccessPdf") && s.contains("minCitationCount=5") && s.contains("publicationTypes=Review"));
        let e = epmc_url(&q, SortBy::MostCited);
        assert!(e.contains("OPEN_ACCESS%3Ay") && e.contains("sort=CITED"));
        assert!(cyberleninka_body(&q).contains("\"mode\":\"articles\""));
    }

    #[test]
    fn wikipedia_overview_is_only_kept_when_on_topic() {
        let body = r#"{"type":"standard","title":"Графен","description":"двумерная модификация углерода","extract":"Графе́н — двумерная аллотропная модификация углерода, образованная слоем атомов углерода толщиной в один атом.","thumbnail":{"source":"https://upload.wikimedia.org/x.jpg"},"content_urls":{"desktop":{"page":"https://ru.wikipedia.org/wiki/Графен"}}}"#;
        let o = parse_wikipedia_summary(body.as_bytes(), "ru").unwrap().unwrap();
        assert!(overview_matches("графен", &o));
        assert!(overview_matches("графена свойства", &o));
        assert!(!overview_matches("нейронные сети", &o));
        assert!(parse_wikipedia_summary(r#"{"type":"disambiguation","extract":"Графен may refer to many different things and more text here."}"#.as_bytes(), "ru").unwrap().is_none());
        assert_eq!(parse_wikipedia_search(r#"{"query":{"search":[{"ns":0,"title":"Графен"}]}}"#.as_bytes()).unwrap(), vec!["Графен".to_string()]);
        assert!(wikipedia_summary_url("ru", "Оксид графена").ends_with("%D0%9E%D0%BA%D1%81%D0%B8%D0%B4_%D0%B3%D1%80%D0%B0%D1%84%D0%B5%D0%BD%D0%B0"));
    }

    #[test]
    fn page_tags_fill_in_details() {
        let html = r#"<meta name="citation_title" content="X"><meta content="2 (18)" name="citation_issue"><meta name="citation_firstpage" content="10"><meta name="citation_lastpage" content="17"><meta name="citation_doi" content="10.17277/jamt.2020.02.pp.1"><meta name="citation_pdf_url" content="https://cyberleninka.ru/a/pdf">"#;
        let meta = citation_meta(html);
        let mut p = Paper::new("X", Source::CyberLeninka);
        enrich_from_meta(&mut p, &meta);
        assert_eq!(p.issue, "2 (18)");
        assert_eq!(p.pages, "10–17");
        assert_eq!(p.doi.as_deref(), Some("10.17277/jamt.2020.02.pp.1"));
        assert_eq!(p.pdf_url.as_deref(), Some("https://cyberleninka.ru/a/pdf"));
    }
}
