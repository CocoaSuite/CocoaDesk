//! Research: looking up a query in scholarly databases, following topics on a timer, and a
//! library of saved papers with notes, collections and citations.
//!
//! Papers come from OpenAlex, arXiv, Semantic Scholar, Europe PMC and CyberLeninka. The same
//! paper found in several of them is shown once. As in News, nothing is ranked or summarised
//! by a model here: results keep each database's own order, or are sorted by date or citations.

pub mod cite;
pub mod sources;
pub mod store;

use std::collections::{HashMap, HashSet};

use chrono::NaiveDate;
use serde::{Deserialize, Serialize};

use crate::news::{normalize, site_domain, term_matches};
use crate::{Error, Result};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Source {
    OpenAlex,
    Arxiv,
    SemanticScholar,
    EuropePmc,
    CyberLeninka,
}

impl Source {
    pub const ALL: [Source; 5] = [Source::OpenAlex, Source::Arxiv, Source::SemanticScholar, Source::EuropePmc, Source::CyberLeninka];

    pub fn label(self) -> &'static str {
        match self {
            Source::OpenAlex => "OpenAlex",
            Source::Arxiv => "arXiv",
            Source::SemanticScholar => "Semantic Scholar",
            Source::EuropePmc => "Europe PMC",
            Source::CyberLeninka => "CyberLeninka",
        }
    }
    /// Whether the database tells how often its papers are cited.
    pub fn has_citations(self) -> bool {
        matches!(self, Source::OpenAlex | Source::SemanticScholar | Source::EuropePmc)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum PaperKind {
    Article,
    Preprint,
    Review,
    Conference,
    Book,
    Chapter,
    Thesis,
    Other,
}

impl PaperKind {
    pub fn label(self) -> &'static str {
        match self {
            PaperKind::Article => "Article",
            PaperKind::Preprint => "Preprint",
            PaperKind::Review => "Review",
            PaperKind::Conference => "Conference paper",
            PaperKind::Book => "Book",
            PaperKind::Chapter => "Book chapter",
            PaperKind::Thesis => "Thesis",
            PaperKind::Other => "Other",
        }
    }
}

/// The kinds a search can be limited to.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum KindFilter {
    Any,
    Article,
    Preprint,
    Review,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SortBy {
    /// Each database's own order, interleaved.
    Relevance,
    Newest,
    MostCited,
}

/// A person who wrote a paper. `family` and `initials` are worked out from however the
/// database writes the name ("Ivan I. Petrov", "Петров И. И.", "Petrov I").
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Author {
    pub name: String,
    pub family: String,
    /// Given names as written, if known ("Ivan Ivanovich").
    #[serde(default)]
    pub given: String,
    /// "I. I."
    pub initials: String,
}

fn is_initials(s: &str) -> bool {
    // "И.", "И.И.", "I", "IV", "A.-B."
    let letters: Vec<char> = s.chars().filter(|c| c.is_alphabetic()).collect();
    !letters.is_empty() && letters.len() <= 3 && (s.contains('.') || letters.iter().all(|c| c.is_uppercase()) && letters.len() <= 2)
}

fn initials_of(given: &str) -> String {
    given
        .split(|c: char| c.is_whitespace() || c == '.')
        .filter(|p| !p.is_empty())
        .map(|p| {
            // Hyphenated given names keep the hyphen: "Jean-Paul" → "J.-P."
            p.split('-').filter_map(|q| q.chars().next()).map(|c| format!("{}.", c.to_uppercase())).collect::<Vec<_>>().join("-")
        })
        .collect::<Vec<_>>()
        .join(" ")
}

impl Author {
    /// Reads a name in any of the common orders.
    pub fn parse(name: &str) -> Author {
        let name = name.split_whitespace().collect::<Vec<_>>().join(" ");
        let name = name.trim_matches(|c: char| c == ',' || c == ';').to_string();
        if let Some((family, given)) = name.split_once(',') {
            let given = given.trim().to_string();
            return Author { family: family.trim().to_string(), initials: initials_of(&given), given, name };
        }
        let parts: Vec<&str> = name.split(' ').collect();
        if parts.len() == 1 {
            return Author { family: name.clone(), given: String::new(), initials: String::new(), name };
        }
        // "Петров И. И." / "Petrov IA": the family name comes first, initials after.
        let trailing = parts.iter().skip(1).all(|p| is_initials(p));
        let leading = parts[..parts.len() - 1].iter().all(|p| is_initials(p));
        if trailing && !is_initials(parts[0]) {
            let raw = parts[1..].join(" ");
            // "IA" without dots means two initials.
            let spaced: String = if raw.contains('.') { raw.clone() } else { raw.chars().filter(|c| c.is_alphabetic()).map(|c| format!("{c}.")).collect::<Vec<_>>().join(" ") };
            return Author { family: parts[0].to_string(), given: String::new(), initials: initials_of(&spaced), name };
        }
        let family = parts[parts.len() - 1].to_string();
        let given = parts[..parts.len() - 1].join(" ");
        Author { initials: initials_of(&given), given: if leading { String::new() } else { given }, family, name }
    }

    /// From separate family and given names, as Europe PMC gives them.
    pub fn from_parts(family: &str, given: &str, full: &str) -> Author {
        let family = family.trim();
        if family.is_empty() {
            return Author::parse(full);
        }
        let given = given.trim();
        Author {
            name: if full.trim().is_empty() { format!("{given} {family}").trim().to_string() } else { full.trim().to_string() },
            family: family.to_string(),
            initials: initials_of(given),
            given: given.to_string(),
        }
    }
}

/// A paper as CocoaDesk keeps it, merged from every database that knows it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Paper {
    /// What identifies the paper across databases: its DOI, arXiv id, or title and year.
    pub key: String,
    pub title: String,
    pub authors: Vec<Author>,
    pub year: Option<i32>,
    pub date: Option<NaiveDate>,
    /// The journal, conference or archive.
    #[serde(default)]
    pub venue: String,
    #[serde(default)]
    pub publisher: String,
    #[serde(default)]
    pub volume: String,
    #[serde(default)]
    pub issue: String,
    #[serde(default)]
    pub pages: String,
    pub kind: PaperKind,
    /// "ru", "en", …
    #[serde(default)]
    pub language: Option<String>,
    #[serde(default)]
    pub abstract_text: String,
    /// A one-sentence summary written by Semantic Scholar's model, shown labelled as such.
    #[serde(default)]
    pub tldr: Option<String>,
    pub citations: Option<i64>,
    #[serde(default)]
    pub doi: Option<String>,
    #[serde(default)]
    pub arxiv: Option<String>,
    #[serde(default)]
    pub pmid: Option<String>,
    #[serde(default)]
    pub pmcid: Option<String>,
    #[serde(default)]
    pub openalex: Option<String>,
    #[serde(default)]
    pub s2: Option<String>,
    /// The paper's page at the publisher or archive.
    pub url: String,
    /// A free PDF, when one is known.
    #[serde(default)]
    pub pdf_url: Option<String>,
    /// A page with the full text that can be read inside CocoaDesk.
    #[serde(default)]
    pub html_url: Option<String>,
    pub open_access: bool,
    #[serde(default)]
    pub keywords: Vec<String>,
    pub sources: Vec<Source>,
}

impl Paper {
    /// An empty paper to be filled by a source parser.
    pub fn new(title: &str, source: Source) -> Paper {
        Paper {
            key: String::new(),
            title: tidy_title(title),
            authors: vec![],
            year: None,
            date: None,
            venue: String::new(),
            publisher: String::new(),
            volume: String::new(),
            issue: String::new(),
            pages: String::new(),
            kind: PaperKind::Article,
            language: None,
            abstract_text: String::new(),
            tldr: None,
            citations: None,
            doi: None,
            arxiv: None,
            pmid: None,
            pmcid: None,
            openalex: None,
            s2: None,
            url: String::new(),
            pdf_url: None,
            html_url: None,
            open_access: false,
            keywords: vec![],
            sources: vec![source],
        }
    }

    /// Sets the key and fills in what can be worked out (language, year from date).
    pub fn finish(mut self) -> Paper {
        if self.year.is_none() {
            self.year = self.date.map(|d| chrono::Datelike::year(&d));
        }
        if self.language.is_none() {
            self.language = Some(if has_cyrillic(&self.title) { "ru".into() } else { "en".into() });
        }
        if self.url.is_empty() {
            self.url = self.doi.as_ref().map(|d| format!("https://doi.org/{d}")).or_else(|| self.arxiv.as_ref().map(|a| format!("https://arxiv.org/abs/{a}"))).unwrap_or_default();
        }
        self.key = paper_key(&self);
        self
    }

    /// The website the paper's page is on.
    pub fn domain(&self) -> String {
        site_domain(&self.url).unwrap_or_default()
    }

    /// "Petrov, Sidorova" / "Petrov et al."
    pub fn short_authors(&self) -> String {
        match self.authors.len() {
            0 => String::new(),
            1 => self.authors[0].family.clone(),
            2 => format!("{}, {}", self.authors[0].family, self.authors[1].family),
            _ => format!("{} et al.", self.authors[0].family),
        }
    }

    /// Adds what `other` knows that this record doesn't. Both describe the same paper.
    pub fn merge(&mut self, other: &Paper) {
        let fill = |a: &mut String, b: &String| {
            if a.trim().is_empty() && !b.trim().is_empty() {
                *a = b.clone();
            }
        };
        let fill_opt = |a: &mut Option<String>, b: &Option<String>| {
            if a.is_none() && b.is_some() {
                *a = b.clone();
            }
        };
        fill(&mut self.venue, &other.venue);
        fill(&mut self.publisher, &other.publisher);
        fill(&mut self.volume, &other.volume);
        fill(&mut self.issue, &other.issue);
        fill(&mut self.pages, &other.pages);
        fill(&mut self.url, &other.url);
        if other.abstract_text.chars().count() > self.abstract_text.chars().count() {
            self.abstract_text = other.abstract_text.clone();
        }
        if self.authors.len() < other.authors.len() {
            self.authors = other.authors.clone();
        }
        fill_opt(&mut self.tldr, &other.tldr);
        fill_opt(&mut self.doi, &other.doi);
        fill_opt(&mut self.arxiv, &other.arxiv);
        fill_opt(&mut self.pmid, &other.pmid);
        fill_opt(&mut self.pmcid, &other.pmcid);
        fill_opt(&mut self.openalex, &other.openalex);
        fill_opt(&mut self.s2, &other.s2);
        fill_opt(&mut self.pdf_url, &other.pdf_url);
        fill_opt(&mut self.html_url, &other.html_url);
        self.citations = match (self.citations, other.citations) {
            (Some(a), Some(b)) => Some(a.max(b)),
            (a, b) => a.or(b),
        };
        if self.date.is_none() {
            self.date = other.date;
        }
        if self.year.is_none() {
            self.year = other.year;
        }
        // A journal article beats the preprint of the same work.
        if self.kind == PaperKind::Preprint && other.kind != PaperKind::Preprint || self.kind == PaperKind::Other {
            self.kind = other.kind;
        }
        self.open_access |= other.open_access;
        for k in &other.keywords {
            if !self.keywords.iter().any(|x| normalize(x) == normalize(k)) {
                self.keywords.push(k.clone());
            }
        }
        for s in &other.sources {
            if !self.sources.contains(s) {
                self.sources.push(*s);
            }
        }
        if self.language.is_none() {
            self.language = other.language.clone();
        }
        // The DOI is the strongest identity; adopt it if it arrived late.
        self.key = paper_key(self);
    }
}

pub fn has_cyrillic(s: &str) -> bool {
    s.chars().any(|c| ('а'..='я').contains(&c.to_lowercase().next().unwrap_or(c)) || c == 'ё' || c == 'Ё')
}

/// Removes markup and stray full stops that some databases leave in titles.
pub fn tidy_title(t: &str) -> String {
    let t = crate::news::feed::plain_text(t);
    let t = t.trim();
    let t = t.strip_suffix('.').filter(|s| !s.ends_with('.')).unwrap_or(t);
    t.to_string()
}

/// A DOI without its "https://doi.org/" prefix, lower case.
pub fn clean_doi(s: &str) -> Option<String> {
    let s = s.trim();
    let lower = s.to_lowercase();
    let start = lower.find("10.")?;
    let d = &lower[start..];
    let d = d.trim_end_matches(['.', ',', ';', ')']);
    (d.contains('/') && d.len() > 6).then(|| d.to_string())
}

/// An arXiv id without its version or prefix: "2611.18318".
pub fn clean_arxiv(s: &str) -> Option<String> {
    let s = s.trim();
    let s = s.rsplit("/abs/").next().unwrap_or(s);
    let s = s.rsplit("/pdf/").next().unwrap_or(s);
    let s = s.trim_start_matches("arXiv:").trim_start_matches("arxiv:").trim_end_matches(".pdf");
    let base = match s.rfind('v') {
        Some(i) if i > 0 && s[i + 1..].chars().all(|c| c.is_ascii_digit()) && !s[i + 1..].is_empty() => &s[..i],
        _ => s,
    };
    (!base.is_empty()).then(|| base.to_string())
}

pub(crate) fn title_key(title: &str) -> String {
    normalize(title).chars().filter(|c| c.is_alphanumeric()).collect()
}

pub fn paper_key(p: &Paper) -> String {
    if let Some(d) = &p.doi {
        return format!("doi:{d}");
    }
    if let Some(a) = &p.arxiv {
        return format!("arxiv:{a}");
    }
    format!("t:{}:{}", title_key(&p.title), p.year.map(|y| y.to_string()).unwrap_or_default())
}

/// Whether two records describe the same paper: a shared DOI or arXiv id, or the same title
/// and year (a preprint and its journal version usually keep the title).
pub fn same_paper(a: &Paper, b: &Paper) -> bool {
    if let (Some(x), Some(y)) = (&a.doi, &b.doi) {
        if x == y {
            return true;
        }
    }
    if let (Some(x), Some(y)) = (&a.arxiv, &b.arxiv) {
        if x == y {
            return true;
        }
    }
    let (ta, tb) = (title_key(&a.title), title_key(&b.title));
    if ta.chars().count() < 12 || ta != tb {
        return false;
    }
    match (a.year, b.year) {
        (Some(x), Some(y)) => (x - y).abs() <= 1,
        _ => true,
    }
}

/// What to look for and how to narrow it down.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Query {
    pub text: String,
    #[serde(default)]
    pub from_year: Option<i32>,
    #[serde(default)]
    pub to_year: Option<i32>,
    #[serde(default)]
    pub open_access: bool,
    #[serde(default)]
    pub min_citations: Option<i64>,
    #[serde(default = "any_kind")]
    pub kind: KindFilter,
    /// "ru", "en" or nothing for any.
    #[serde(default)]
    pub language: Option<String>,
    #[serde(default = "all_sources")]
    pub sources: Vec<Source>,
    /// Papers mentioning these in the title or abstract are left out.
    #[serde(default)]
    pub exclude: Vec<String>,
    /// Journals or websites left out for this search only.
    #[serde(default)]
    pub blocked: Vec<String>,
    #[serde(default = "relevance")]
    pub sort: SortBy,
}

fn any_kind() -> KindFilter {
    KindFilter::Any
}
pub fn all_sources() -> Vec<Source> {
    Source::ALL.to_vec()
}
fn relevance() -> SortBy {
    SortBy::Relevance
}

impl Query {
    pub fn simple(text: &str) -> Query {
        Query {
            text: text.into(),
            from_year: None,
            to_year: None,
            open_access: false,
            min_citations: None,
            kind: KindFilter::Any,
            language: None,
            sources: all_sources(),
            exclude: vec![],
            blocked: vec![],
            sort: SortBy::Relevance,
        }
    }

    pub fn cleaned(mut self) -> Query {
        self.text = self.text.split_whitespace().collect::<Vec<_>>().join(" ");
        let tidy = |v: Vec<String>| {
            let mut seen = HashSet::new();
            v.into_iter().map(|s| s.trim().to_string()).filter(|s| !s.is_empty() && seen.insert(normalize(s))).collect::<Vec<_>>()
        };
        self.exclude = tidy(self.exclude);
        self.blocked = tidy(self.blocked).into_iter().map(|b| blocked_value(&b)).collect();
        self.language = self.language.filter(|l| l == "ru" || l == "en");
        self.min_citations = self.min_citations.filter(|n| *n > 0);
        let mut seen = HashSet::new();
        self.sources.retain(|s| seen.insert(*s));
        if let (Some(a), Some(b)) = (self.from_year, self.to_year) {
            if a > b {
                self.from_year = Some(b);
                self.to_year = Some(a);
            }
        }
        self
    }

    pub fn validate(&self) -> Result<()> {
        if self.text.chars().count() < 2 {
            return Err(Error::invalid("Type what to look for."));
        }
        if self.sources.is_empty() {
            return Err(Error::invalid("Choose at least one database to search."));
        }
        for y in [self.from_year, self.to_year].into_iter().flatten() {
            if !(1600..=2100).contains(&y) {
                return Err(Error::invalid(format!("{y} doesn't look like a year.")));
            }
        }
        Ok(())
    }

    /// The databases worth asking, given the filters: arXiv only has English preprints,
    /// CyberLeninka only articles that are mostly Russian, and neither counts citations.
    pub fn useful_sources(&self) -> Vec<Source> {
        self.sources
            .iter()
            .copied()
            .filter(|s| match s {
                Source::Arxiv => self.language.as_deref() != Some("ru") && !matches!(self.kind, KindFilter::Article | KindFilter::Review),
                Source::CyberLeninka => self.language.as_deref() != Some("en") && self.kind != KindFilter::Preprint,
                _ => true,
            })
            .filter(|s| self.min_citations.is_none() || s.has_citations())
            .collect()
    }
}

/// A journal name as typed, or a website's domain.
pub fn blocked_value(s: &str) -> String {
    let t = s.trim();
    if !t.contains(' ') && t.contains('.') {
        if let Some(d) = site_domain(t) {
            return d;
        }
    }
    t.to_string()
}

/// Whether a paper's journal or website is on a block list. Websites match with their
/// subdomains; journals match by name, ignoring case.
pub fn is_blocked(p: &Paper, blocked: &[String]) -> bool {
    if blocked.is_empty() {
        return false;
    }
    let domain = p.domain();
    let pdf_domain = p.pdf_url.as_deref().and_then(site_domain).unwrap_or_default();
    let venue = normalize(&p.venue);
    let publisher = normalize(&p.publisher);
    blocked.iter().any(|b| {
        let nb = normalize(b);
        (!venue.is_empty() && venue == nb)
            || (!publisher.is_empty() && publisher == nb)
            || [&domain, &pdf_domain].iter().any(|d| !d.is_empty() && (*d == b || d.ends_with(&format!(".{b}"))))
    })
}

/// Applies the filters a database couldn't apply itself.
pub fn passes(q: &Query, global_blocked: &[String], p: &Paper) -> bool {
    if p.title.trim().is_empty() {
        return false;
    }
    if let (Some(from), Some(y)) = (q.from_year, p.year) {
        if y < from {
            return false;
        }
    }
    if let (Some(to), Some(y)) = (q.to_year, p.year) {
        if y > to {
            return false;
        }
    }
    if q.open_access && !p.open_access {
        return false;
    }
    if let Some(min) = q.min_citations {
        if p.citations.unwrap_or(0) < min {
            return false;
        }
    }
    let kind_ok = match q.kind {
        KindFilter::Any => true,
        KindFilter::Article => matches!(p.kind, PaperKind::Article | PaperKind::Review | PaperKind::Conference),
        KindFilter::Preprint => p.kind == PaperKind::Preprint,
        KindFilter::Review => p.kind == PaperKind::Review,
    };
    if !kind_ok {
        return false;
    }
    if let (Some(l), Some(pl)) = (&q.language, &p.language) {
        if l != pl {
            return false;
        }
    }
    if !q.exclude.is_empty() {
        let text = normalize(&format!("{} {}", p.title, p.abstract_text));
        if q.exclude.iter().any(|w| term_matches(&text, w)) {
            return false;
        }
    }
    !is_blocked(p, &q.blocked) && !is_blocked(p, global_blocked)
}

/// Merges the lists from several databases into one, the same paper shown once. Each list is
/// in its database's own order; the merged list takes turns between them, so no database
/// crowds out the others, and papers several databases agree on rise a little.
pub fn merge_results(lists: Vec<(Source, Vec<Paper>)>, sort: SortBy) -> Vec<Paper> {
    let mut merged: Vec<Paper> = Vec::new();
    // Best (lowest) rank seen for each merged paper, and in how many lists it appeared.
    let mut rank: Vec<(usize, usize)> = Vec::new();
    let mut by_doi: HashMap<String, usize> = HashMap::new();
    for (_, list) in &lists {
        for (i, p) in list.iter().enumerate() {
            let found = p.doi.as_ref().and_then(|d| by_doi.get(d).copied()).or_else(|| merged.iter().position(|m| same_paper(m, p)));
            match found {
                Some(j) => {
                    merged[j].merge(p);
                    rank[j] = (rank[j].0.min(i), rank[j].1 + 1);
                    if let Some(d) = &merged[j].doi {
                        by_doi.insert(d.clone(), j);
                    }
                }
                None => {
                    if let Some(d) = &p.doi {
                        by_doi.insert(d.clone(), merged.len());
                    }
                    merged.push(p.clone());
                    rank.push((i, 1));
                }
            }
        }
    }
    let mut order: Vec<usize> = (0..merged.len()).collect();
    match sort {
        SortBy::Relevance => order.sort_by_key(|&i| (rank[i].0.saturating_sub(rank[i].1 - 1), usize::MAX - rank[i].1, i)),
        SortBy::Newest => order.sort_by(|&a, &b| {
            let k = |p: &Paper| p.date.or_else(|| p.year.and_then(|y| NaiveDate::from_ymd_opt(y, 1, 1)));
            k(&merged[b]).cmp(&k(&merged[a])).then(a.cmp(&b))
        }),
        SortBy::MostCited => order.sort_by(|&a, &b| merged[b].citations.unwrap_or(-1).cmp(&merged[a].citations.unwrap_or(-1)).then(a.cmp(&b))),
    }
    order.into_iter().map(|i| merged[i].clone()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_names_in_any_order() {
        let a = Author::parse("Ivan I. Petrov");
        assert_eq!((a.family.as_str(), a.initials.as_str()), ("Petrov", "I. I."));
        let a = Author::parse("Петров И. И.");
        assert_eq!((a.family.as_str(), a.initials.as_str()), ("Петров", "И. И."));
        let a = Author::parse("Машхадани А.");
        assert_eq!((a.family.as_str(), a.initials.as_str()), ("Машхадани", "А."));
        let a = Author::parse("В. Ф. Першин");
        assert_eq!((a.family.as_str(), a.initials.as_str()), ("Першин", "В. Ф."));
        let a = Author::parse("Petrov IA");
        assert_eq!((a.family.as_str(), a.initials.as_str()), ("Petrov", "I. A."));
        let a = Author::parse("Petrov, Ivan");
        assert_eq!((a.family.as_str(), a.initials.as_str(), a.given.as_str()), ("Petrov", "I.", "Ivan"));
        let a = Author::parse("Jean-Paul Sartre");
        assert_eq!(a.initials, "J.-P.");
        assert_eq!(Author::from_parts("Xu", "Ximei", "Xu X").initials, "X.");
    }

    #[test]
    fn identifiers_are_normalised() {
        assert_eq!(clean_doi("https://doi.org/10.1016/J.Carbon.2020.01.002"), Some("10.1016/j.carbon.2020.01.002".into()));
        assert_eq!(clean_doi("not a doi"), None);
        assert_eq!(clean_arxiv("http://arxiv.org/abs/2611.18490v1"), Some("2611.18490".into()));
        assert_eq!(clean_arxiv("https://arxiv.org/pdf/2611.18490v12"), Some("2611.18490".into()));
        assert_eq!(clean_arxiv("cond-mat/0101001v2"), Some("cond-mat/0101001".into()));
        assert_eq!(tidy_title("Field <i>Emission</i> Enhancement."), "Field Emission Enhancement");
    }

    fn paper(title: &str, src: Source, doi: Option<&str>, year: i32) -> Paper {
        let mut p = Paper::new(title, src);
        p.doi = doi.map(String::from);
        p.year = Some(year);
        p.finish()
    }

    #[test]
    fn the_same_paper_is_merged() {
        let mut a = paper("Graphene oxide in cement composites", Source::OpenAlex, Some("10.1/x"), 2020);
        a.citations = Some(42);
        let mut b = paper("Graphene Oxide in Cement Composites", Source::SemanticScholar, Some("10.1/x"), 2020);
        b.citations = Some(45);
        b.abstract_text = "A longer abstract.".into();
        let mut c = paper("Graphene oxide in cement composites.", Source::Arxiv, None, 2019);
        c.kind = PaperKind::Preprint;
        c.arxiv = Some("1901.00001".into());
        c.pdf_url = Some("https://arxiv.org/pdf/1901.00001".into());
        let d = paper("Something else entirely", Source::Arxiv, None, 2020);
        let out = merge_results(vec![(Source::OpenAlex, vec![a]), (Source::SemanticScholar, vec![b]), (Source::Arxiv, vec![c, d])], SortBy::Relevance);
        assert_eq!(out.len(), 2);
        let m = &out[0];
        assert_eq!(m.citations, Some(45));
        assert_eq!(m.abstract_text, "A longer abstract.");
        assert_eq!(m.kind, PaperKind::Article);
        assert_eq!(m.arxiv.as_deref(), Some("1901.00001"));
        assert!(m.pdf_url.is_some());
        assert_eq!(m.sources.len(), 3);
        assert_eq!(m.key, "doi:10.1/x");
    }

    #[test]
    fn filters_and_blocks() {
        let mut q = Query::simple("graphene");
        q.from_year = Some(2021);
        let mut p = paper("Graphene oxide", Source::OpenAlex, None, 2020);
        assert!(!passes(&q, &[], &p));
        p.year = Some(2022);
        assert!(passes(&q, &[], &p));
        p.venue = "Predatory Journal of Things".into();
        assert!(!passes(&q, &["predatory journal of things".into()], &p));
        p.url = "https://www.mdpi.com/1/2".into();
        assert!(!passes(&q, &["mdpi.com".into()], &p));
        q.exclude = vec!["оксид".into(), "oxide".into()];
        assert!(!passes(&q, &[], &p));
        q.exclude.clear();
        q.min_citations = Some(5);
        assert!(!passes(&q, &[], &p));
        p.citations = Some(5);
        assert!(passes(&q, &[], &p));
        q.language = Some("ru".into());
        assert!(!passes(&q, &[], &p));
    }

    #[test]
    fn filters_decide_which_databases_to_ask() {
        let mut q = Query::simple("графен");
        q.language = Some("ru".into());
        assert!(!q.useful_sources().contains(&Source::Arxiv));
        q.language = None;
        q.min_citations = Some(3);
        assert_eq!(q.useful_sources(), vec![Source::OpenAlex, Source::SemanticScholar, Source::EuropePmc]);
        assert_eq!(blocked_value("https://www.MDPI.com/x"), "mdpi.com");
        assert_eq!(blocked_value("Journal of Things"), "Journal of Things");
    }

    #[test]
    fn sorts_by_date_and_citations() {
        let mut a = paper("Paper number one here", Source::OpenAlex, Some("10.1/a"), 2018);
        a.citations = Some(100);
        let mut b = paper("Paper number two here", Source::OpenAlex, Some("10.1/b"), 2024);
        b.citations = Some(3);
        let newest = merge_results(vec![(Source::OpenAlex, vec![a.clone(), b.clone()])], SortBy::Newest);
        assert_eq!(newest[0].doi.as_deref(), Some("10.1/b"));
        let cited = merge_results(vec![(Source::OpenAlex, vec![b, a])], SortBy::MostCited);
        assert_eq!(cited[0].doi.as_deref(), Some("10.1/a"));
    }
}
