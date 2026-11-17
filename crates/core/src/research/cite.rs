//! Reference-list entries: ГОСТ Р 7.0.100–2018 (Russian coursework), APA 7 and BibTeX.

use chrono::NaiveDate;
use serde::{Deserialize, Serialize};

use super::{has_cyrillic, Author, Paper, PaperKind};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Style {
    Gost,
    Apa,
    Bibtex,
}

impl Style {
    pub const ALL: [Style; 3] = [Style::Gost, Style::Apa, Style::Bibtex];
    pub fn parse(s: &str) -> Style {
        match s {
            "apa" => Style::Apa,
            "bibtex" => Style::Bibtex,
            _ => Style::Gost,
        }
    }
    pub fn as_str(self) -> &'static str {
        match self {
            Style::Gost => "gost",
            Style::Apa => "apa",
            Style::Bibtex => "bibtex",
        }
    }
}

pub fn cite(p: &Paper, style: Style, today: NaiveDate) -> String {
    match style {
        Style::Gost => gost(p, today),
        Style::Apa => apa(p),
        Style::Bibtex => bibtex(p),
    }
}

/// "И. И. Петров"
fn initials_first(a: &Author) -> String {
    if a.initials.is_empty() {
        a.family.clone()
    } else {
        format!("{} {}", a.initials, a.family)
    }
}

/// "Петров, И. И."
fn family_first(a: &Author) -> String {
    if a.initials.is_empty() {
        a.family.clone()
    } else {
        format!("{}, {}", a.family, a.initials)
    }
}

fn end_with_period(s: &mut String) {
    let t = s.trim_end();
    if !t.ends_with(['.', '!', '?', '…']) {
        s.truncate(t.len());
        s.push('.');
    }
}

/// ГОСТ Р 7.0.100–2018. One to three authors head the entry; with four, all are named after
/// the title; with five or more, the first three and «[и др.]».
pub fn gost(p: &Paper, today: NaiveDate) -> String {
    let ru = has_cyrillic(&p.title) || p.language.as_deref() == Some("ru");
    let n = p.authors.len();
    let mut out = String::new();
    if (1..=3).contains(&n) {
        out.push_str(&family_first(&p.authors[0]));
        end_with_period(&mut out);
        out.push(' ');
    }
    out.push_str(p.title.trim());
    if n > 0 {
        let shown: Vec<String> = p.authors.iter().take(if n == 4 { 4 } else { 3 }).map(initials_first).collect();
        out.push_str(" / ");
        out.push_str(&shown.join(", "));
        if n > 4 {
            out.push_str(if ru { " [и др.]" } else { " [et al.]" });
        }
    }
    let year = p.year.map(|y| y.to_string()).unwrap_or_else(|| if ru { "б. г.".into() } else { "n. d.".into() });
    let preprint = p.kind == PaperKind::Preprint && (p.venue.is_empty() || p.venue.eq_ignore_ascii_case("arxiv"));
    if !p.venue.is_empty() && !preprint && !matches!(p.kind, PaperKind::Book | PaperKind::Thesis) {
        out.push_str(" // ");
        out.push_str(p.venue.trim());
        out.push_str(&format!(". – {year}"));
        let mut vol = Vec::new();
        if !p.volume.is_empty() {
            vol.push(format!("{} {}", if ru { "Т." } else { "Vol." }, p.volume));
        }
        if !p.issue.is_empty() {
            vol.push(format!("{} {}", if ru { "№" } else { "no." }, p.issue));
        }
        if !vol.is_empty() {
            out.push_str(&format!(". – {}", vol.join(", ")));
        }
        if !p.pages.is_empty() {
            let pages = p.pages.replace('-', "–");
            out.push_str(&format!(". – {} {pages}", if ru { "С." } else { "P." }));
        }
    } else {
        if !p.publisher.is_empty() {
            out.push_str(&format!(". – {}", p.publisher.trim()));
            out.push_str(&format!(", {year}"));
        } else {
            out.push_str(&format!(". – {year}"));
        }
        if let Some(a) = p.arxiv.as_ref().filter(|_| preprint) {
            out.push_str(&format!(". – arXiv:{a}"));
        }
    }
    if let Some(d) = &p.doi {
        out.push_str(&format!(". – DOI {d}"));
    } else if !p.url.is_empty() {
        out.push_str(&format!(". – URL: {} ({} {})", p.url, if ru { "дата обращения:" } else { "date accessed:" }, today.format("%d.%m.%Y")));
    }
    end_with_period(&mut out);
    out
}

/// APA 7th edition, as plain text.
pub fn apa(p: &Paper) -> String {
    let names: Vec<String> = p.authors.iter().map(family_first).collect();
    let authors = match names.len() {
        0 => String::new(),
        1 => names[0].clone(),
        2 => format!("{}, & {}", names[0], names[1]),
        n if n <= 20 => format!("{}, & {}", names[..n - 1].join(", "), names[n - 1]),
        n => format!("{}, . . . {}", names[..19].join(", "), names[n - 1]),
    };
    let year = p.year.map(|y| y.to_string()).unwrap_or_else(|| "n.d.".into());
    let mut out = String::new();
    if authors.is_empty() {
        out.push_str(p.title.trim());
        end_with_period(&mut out);
        out.push_str(&format!(" ({year})."));
    } else {
        out.push_str(&authors);
        if !out.ends_with('.') {
            out.push('.');
        }
        out.push_str(&format!(" ({year}). {}", p.title.trim()));
        end_with_period(&mut out);
    }
    let preprint = p.kind == PaperKind::Preprint && (p.venue.is_empty() || p.venue.eq_ignore_ascii_case("arxiv"));
    if preprint {
        out.push_str(if p.arxiv.is_some() { " arXiv." } else { "" });
    } else if !p.venue.is_empty() {
        out.push(' ');
        out.push_str(p.venue.trim());
        if !p.volume.is_empty() {
            out.push_str(&format!(", {}", p.volume));
            if !p.issue.is_empty() {
                out.push_str(&format!("({})", p.issue));
            }
        }
        if !p.pages.is_empty() {
            out.push_str(&format!(", {}", p.pages.replace('-', "–")));
        }
        out.push('.');
    } else if !p.publisher.is_empty() {
        out.push_str(&format!(" {}.", p.publisher.trim()));
    }
    if let Some(d) = &p.doi {
        out.push_str(&format!(" https://doi.org/{d}"));
    } else if !p.url.is_empty() {
        out.push_str(&format!(" {}", p.url));
    }
    out
}

/// Russian letters in Latin, for BibTeX keys (ГОСТ 7.79 system B, simplified).
fn translit(s: &str) -> String {
    s.chars()
        .map(|c| {
            let l = c.to_lowercase().next().unwrap_or(c);
            match l {
                'а' => "a", 'б' => "b", 'в' => "v", 'г' => "g", 'д' => "d", 'е' | 'ё' | 'э' => "e", 'ж' => "zh", 'з' => "z",
                'и' | 'й' => "i", 'к' => "k", 'л' => "l", 'м' => "m", 'н' => "n", 'о' => "o", 'п' => "p", 'р' => "r", 'с' => "s",
                'т' => "t", 'у' => "u", 'ф' => "f", 'х' => "kh", 'ц' => "ts", 'ч' => "ch", 'ш' => "sh", 'щ' => "shch",
                'ы' => "y", 'ю' => "yu", 'я' => "ya", 'ъ' | 'ь' => "",
                _ => return if l.is_ascii_alphanumeric() { l.to_string() } else { String::new() },
            }
            .to_string()
        })
        .collect()
}

pub fn bibtex_key(p: &Paper) -> String {
    let family = p.authors.first().map(|a| translit(&a.family)).unwrap_or_else(|| "anon".into());
    let word = p
        .title
        .split_whitespace()
        .map(translit)
        .find(|w| w.len() > 3 && !matches!(w.as_str(), "with" | "from" | "this" | "that" | "using"))
        .unwrap_or_default();
    format!("{}{}{}", if family.is_empty() { "anon".into() } else { family }, p.year.map(|y| y.to_string()).unwrap_or_default(), word)
}

fn esc(s: &str) -> String {
    s.replace('\\', "\\textbackslash{}").replace('{', "\\{").replace('}', "\\}").replace('&', "\\&").replace('%', "\\%").replace('$', "\\$").replace('#', "\\#").replace('_', "\\_")
}

pub fn bibtex(p: &Paper) -> String {
    let kind = match p.kind {
        PaperKind::Article | PaperKind::Review => "article",
        PaperKind::Conference => "inproceedings",
        PaperKind::Book => "book",
        PaperKind::Chapter => "incollection",
        PaperKind::Thesis => "phdthesis",
        PaperKind::Preprint | PaperKind::Other => "misc",
    };
    let mut fields: Vec<(&str, String)> = Vec::new();
    if !p.authors.is_empty() {
        let names: Vec<String> =
            p.authors.iter().map(|a| if a.given.is_empty() { family_first(a) } else { format!("{}, {}", a.family, a.given) }).collect();
        fields.push(("author", names.join(" and ")));
    }
    fields.push(("title", format!("{{{}}}", esc(&p.title))));
    if !p.venue.is_empty() {
        let f = match kind {
            "article" => "journal",
            "inproceedings" | "incollection" => "booktitle",
            _ => "howpublished",
        };
        fields.push((f, esc(&p.venue)));
    }
    if let Some(y) = p.year {
        fields.push(("year", y.to_string()));
    }
    for (k, v) in [("volume", &p.volume), ("number", &p.issue), ("publisher", &p.publisher)] {
        if !v.is_empty() {
            fields.push((k, esc(v)));
        }
    }
    if !p.pages.is_empty() {
        fields.push(("pages", p.pages.replace('–', "--")));
    }
    if let Some(d) = &p.doi {
        fields.push(("doi", d.clone()));
    }
    if let Some(a) = &p.arxiv {
        fields.push(("eprint", a.clone()));
        fields.push(("archivePrefix", "arXiv".into()));
    }
    if !p.url.is_empty() {
        fields.push(("url", p.url.clone()));
    }
    if let Some(l) = &p.language {
        fields.push(("language", if l == "ru" { "russian".into() } else { "english".into() }));
    }
    let body = fields.iter().map(|(k, v)| format!("  {k} = {{{v}}}")).collect::<Vec<_>>().join(",\n");
    format!("@{kind}{{{},\n{body}\n}}", bibtex_key(p))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::research::Source;

    fn today() -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, 11, 17).unwrap()
    }

    fn ru_paper(authors: &[&str]) -> Paper {
        let mut p = Paper::new("Модифицирование бетона графеном и оксидом графена", Source::CyberLeninka);
        p.authors = authors.iter().map(|a| Author::parse(a)).collect();
        p.year = Some(2020);
        p.venue = "Journal of Advanced Materials and Technologies".into();
        p.issue = "2 (18)".into();
        p.pages = "10-17".into();
        p.url = "https://cyberleninka.ru/article/n/x".into();
        p.finish()
    }

    #[test]
    fn gost_with_up_to_three_authors_has_a_heading() {
        let p = ru_paper(&["Машхадани А.", "Першин В. Ф."]);
        assert_eq!(
            gost(&p, today()),
            "Машхадани, А. Модифицирование бетона графеном и оксидом графена / А. Машхадани, В. Ф. Першин // Journal of Advanced Materials and Technologies. – 2020. – № 2 (18). – С. 10–17. – URL: https://cyberleninka.ru/article/n/x (дата обращения: 17.11.2026)."
        );
    }

    #[test]
    fn gost_with_many_authors_and_a_doi() {
        let mut p = ru_paper(&["Петров И. И.", "Сидоров С. С.", "Иванов А. А.", "Кузнецов К. К.", "Смирнов П. П."]);
        p.doi = Some("10.17277/jamt.2020.02".into());
        p.volume = "5".into();
        let g = gost(&p, today());
        assert!(g.starts_with("Модифицирование бетона графеном и оксидом графена / И. И. Петров, С. С. Сидоров, А. А. Иванов [и др.] //"), "{g}");
        assert!(g.contains("– Т. 5, № 2 (18). – С. 10–17. – DOI 10.17277/jamt.2020.02."), "{g}");
        let mut four = ru_paper(&["Петров И. И.", "Сидоров С. С.", "Иванов А. А.", "Кузнецов К. К."]);
        four.doi = Some("10.1/x".into());
        assert!(gost(&four, today()).contains("Иванов, К. К. Кузнецов //"));
    }

    #[test]
    fn gost_in_english_and_for_preprints() {
        let mut p = Paper::new("Graphene oxide in cement composites", Source::OpenAlex);
        p.authors = vec![Author::parse("Ivan I. Petrov")];
        p.year = Some(2020);
        p.venue = "Carbon".into();
        p.volume = "160".into();
        p.issue = "2".into();
        p.pages = "101–112".into();
        p.doi = Some("10.1016/j.carbon.2020.01.002".into());
        let p = p.finish();
        assert_eq!(
            gost(&p, today()),
            "Petrov, I. I. Graphene oxide in cement composites / I. I. Petrov // Carbon. – 2020. – Vol. 160, no. 2. – P. 101–112. – DOI 10.1016/j.carbon.2020.01.002."
        );
        let mut pre = Paper::new("Quantized resistance limitations", Source::Arxiv);
        pre.authors = vec![Author::parse("Tamar M. Padwa")];
        pre.kind = PaperKind::Preprint;
        pre.venue = "arXiv".into();
        pre.arxiv = Some("2611.18318".into());
        pre.year = Some(2026);
        let pre = pre.finish();
        assert_eq!(
            gost(&pre, today()),
            "Padwa, T. M. Quantized resistance limitations / T. M. Padwa. – 2026. – arXiv:2611.18318. – URL: https://arxiv.org/abs/2611.18318 (date accessed: 17.11.2026)."
        );
    }

    #[test]
    fn apa_and_bibtex() {
        let mut p = Paper::new("Graphene oxide in cement composites", Source::OpenAlex);
        p.authors = vec![Author::parse("Ivan I. Petrov"), Author::parse("Anna Sidorova")];
        p.year = Some(2020);
        p.venue = "Carbon".into();
        p.volume = "160".into();
        p.issue = "2".into();
        p.pages = "101–112".into();
        p.doi = Some("10.1016/j.carbon.2020.01.002".into());
        let p = p.finish();
        assert_eq!(apa(&p), "Petrov, I. I., & Sidorova, A. (2020). Graphene oxide in cement composites. Carbon, 160(2), 101–112. https://doi.org/10.1016/j.carbon.2020.01.002");
        let b = bibtex(&p);
        assert!(b.starts_with("@article{petrov2020graphene,\n"), "{b}");
        assert!(b.contains("author = {Petrov, Ivan I. and Sidorova, Anna}"));
        assert!(b.contains("pages = {101--112}"));
        let r = ru_paper(&["Машхадани А."]);
        assert!(bibtex(&r).starts_with("@article{mashkhadani2020modifitsirovanie,"), "{}", bibtex(&r));
    }
}
