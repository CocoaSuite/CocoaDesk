//! Reading one timetable cell, e.g.
//! `Социология; сем; Шаронова С.А. ГК-109 / Литература; сем; Попова Е.О.; ГК-156`.
//!
//! The conventions handled here are the ones Russian university timetables commonly use:
//! `top / bottom` for alternating weeks, `;` between subject, type, teacher and room,
//! `*…*` for remarks and `(ноябрь-декабрь)` for courses that only run in some months.

use std::sync::LazyLock;

use regex::Regex;
use serde::Serialize;

use crate::model::{ClassKind, Weeks};

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ParsedClass {
    pub weeks: Weeks,
    pub subject: String,
    pub kind: ClassKind,
    pub teacher: String,
    pub room: String,
    pub note: String,
    /// First and last month (1–12) the class runs in, when the cell says so.
    pub months: Option<(u32, u32)>,
}

static REMARK: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\*([^*]*)\*?").unwrap());
static PAREN: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\(([^)]*)\)").unwrap());
static SPACES: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\s+").unwrap());
static KIND: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)^(лекция|лек|семинар|сем|лабораторная|лаб|практика|практ|пр)\.?\b\s*(.*)$").unwrap());
// "Сорокин Л.А.", "Александрова Н.А..", "Глинская И.Ю." — optionally followed by more text.
static TEACHER: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^([А-ЯЁ][а-яё]+(?:-[А-ЯЁ][а-яё]+)?)\s*(?:([А-ЯЁ])\.+\s*(?:([А-ЯЁ])\.+)?)?\s*(.*)$").unwrap());
// Rooms look like a building code, a hyphen and a number or name: "ГК-315", "ЕГФ-конф зал", "ФОК-".
static ROOM: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^[А-ЯЁA-Z]{1,5}-").unwrap());

const MONTHS: [(&str, u32); 12] = [
    ("январ", 1),
    ("феврал", 2),
    ("март", 3),
    ("апрел", 4),
    ("ма", 5),
    ("июн", 6),
    ("июл", 7),
    ("август", 8),
    ("сентябр", 9),
    ("октябр", 10),
    ("ноябр", 11),
    ("декабр", 12),
];

const PLACEHOLDER_TEACHERS: [&str; 2] = ["преподаватель", "преподаватели"];

fn clean(s: &str) -> String {
    SPACES.replace_all(s.trim(), " ").into_owned()
}

/// Splits a cell into the classes it describes.
pub fn parse_cell(text: &str) -> Vec<ParsedClass> {
    let text = clean(text);
    if text.is_empty() {
        return vec![];
    }
    match split_weeks(&text) {
        None => parse_part(&text, Weeks::Every).into_iter().collect(),
        Some((top, bottom)) => {
            if !top.is_empty() && clean(top) == clean(bottom) {
                return parse_part(top, Weeks::Every).into_iter().collect();
            }
            let mut v = Vec::new();
            v.extend(parse_part(top, Weeks::Top));
            v.extend(parse_part(bottom, Weeks::Bottom));
            v
        }
    }
}

/// Finds the `/` that separates the top-week class from the bottom-week class,
/// ignoring slashes inside `*remarks*` and `(brackets)`.
fn split_weeks(text: &str) -> Option<(&str, &str)> {
    let mut in_remark = false;
    let mut depth = 0i32;
    for (i, ch) in text.char_indices() {
        match ch {
            '*' => in_remark = !in_remark,
            '(' => depth += 1,
            ')' => depth -= 1,
            '/' if !in_remark && depth <= 0 => return Some((text[..i].trim(), text[i + 1..].trim())),
            _ => {}
        }
    }
    None
}

fn months_in(s: &str) -> Vec<u32> {
    let lower = s.to_lowercase();
    let mut found: Vec<(usize, u32)> = Vec::new();
    for word in lower.split(|c: char| !c.is_alphabetic()).filter(|w| !w.is_empty()) {
        let pos = lower.find(word).unwrap_or(0);
        if let Some((_, m)) = MONTHS.iter().find(|(stem, m)| {
            // "ма" is a short stem; only accept май/мая/маю etc. for May.
            if *m == 5 {
                word == "май" || word == "мая" || word == "маю" || word == "мае"
            } else {
                word.starts_with(stem)
            }
        }) {
            found.push((pos, *m));
        }
    }
    found.sort();
    found.into_iter().map(|(_, m)| m).collect()
}

fn parse_part(text: &str, weeks: Weeks) -> Option<ParsedClass> {
    let text = clean(text);
    if text.is_empty() {
        return None;
    }
    let mut notes: Vec<String> = Vec::new();
    for cap in REMARK.captures_iter(&text) {
        let n = clean(&cap[1]);
        if !n.is_empty() {
            notes.push(n);
        }
    }
    let without_remarks = REMARK.replace_all(&text, "").into_owned();
    let fields: Vec<String> = without_remarks.split(';').map(clean).filter(|f| !f.is_empty()).collect();
    let mut fields = fields.into_iter();
    let mut subject = fields.next().unwrap_or_default();

    let mut months = None;
    if let Some(cap) = PAREN.captures(&subject) {
        let inside = cap[1].to_string();
        let ms = months_in(&inside);
        if let (Some(first), Some(last)) = (ms.first(), ms.last()) {
            months = Some((*first, *last));
            subject = clean(&PAREN.replace(&subject, ""));
            notes.insert(0, inside.replace('-', "–"));
        }
    }

    let rest: Vec<String> = fields.collect();
    let mut kind = None;
    let mut rest_iter = rest.into_iter().peekable();
    if let Some(first) = rest_iter.peek() {
        if let Some(cap) = KIND.captures(first) {
            kind = Some(kind_from(&cap[1]));
            let extra = clean(&cap[2]);
            if !extra.is_empty() {
                notes.push(extra);
            }
            rest_iter.next();
        }
    }

    let mut teacher = String::new();
    let mut room = String::new();
    for field in rest_iter {
        let lower = field.to_lowercase();
        if PLACEHOLDER_TEACHERS.iter().any(|p| lower == *p) {
            continue;
        }
        if ROOM.is_match(&field) {
            if room.is_empty() {
                room = tidy_room(&field);
            } else {
                notes.push(field);
            }
            continue;
        }
        if teacher.is_empty() {
            if let Some(cap) = TEACHER.captures(&field) {
                let tail = clean(cap[4].trim_start_matches('.'));
                if tail.is_empty() || ROOM.is_match(&tail) {
                    teacher = format_teacher(&cap[1], cap.get(2).map(|m| m.as_str()), cap.get(3).map(|m| m.as_str()));
                    if !tail.is_empty() && room.is_empty() {
                        room = tidy_room(&tail);
                    }
                    continue;
                }
            }
        }
        notes.push(field);
    }

    let kind = kind.unwrap_or_else(|| {
        if subject.to_lowercase().contains("физическая культура") {
            ClassKind::Sport
        } else {
            ClassKind::Other
        }
    });
    let kind = if kind == ClassKind::Seminar && subject.to_lowercase().contains("физическая культура") {
        ClassKind::Sport
    } else {
        kind
    };

    if subject.is_empty() {
        return None;
    }
    Some(ParsedClass { weeks, subject, kind, teacher, room, note: notes.join(" · "), months })
}

fn kind_from(token: &str) -> ClassKind {
    let t = token.to_lowercase();
    if t.starts_with("лек") {
        ClassKind::Lecture
    } else if t.starts_with("сем") {
        ClassKind::Seminar
    } else if t.starts_with("лаб") {
        ClassKind::Lab
    } else {
        ClassKind::Practice
    }
}

fn format_teacher(surname: &str, i1: Option<&str>, i2: Option<&str>) -> String {
    match (i1, i2) {
        (Some(a), Some(b)) => format!("{surname} {a}.{b}."),
        (Some(a), None) => format!("{surname} {a}."),
        _ => surname.to_string(),
    }
}

/// "ГК-" (building only) becomes "ГК"; anything else is kept as written.
fn tidy_room(s: &str) -> String {
    clean(s).trim_end_matches('-').to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn one(s: &str) -> ParsedClass {
        let v = parse_cell(s);
        assert_eq!(v.len(), 1, "{v:?}");
        v.into_iter().next().unwrap()
    }

    #[test]
    fn plain_seminar() {
        let c = one("Математика и статистика; сем; Сорокин Л.В.; ГК-315");
        assert_eq!(c.weeks, Weeks::Every);
        assert_eq!(c.subject, "Математика и статистика");
        assert_eq!(c.kind, ClassKind::Seminar);
        assert_eq!(c.teacher, "Сорокин Л.В.");
        assert_eq!(c.room, "ГК-315");
        assert_eq!(c.note, "");
    }

    #[test]
    fn top_and_bottom_week() {
        let v = parse_cell("Социология; сем; Шаронова С.А. ГК-109 / Литература; сем; Попова Е.О.; ГК-156");
        assert_eq!(v.len(), 2);
        assert_eq!(v[0].weeks, Weeks::Top);
        assert_eq!(v[0].subject, "Социология");
        assert_eq!(v[0].teacher, "Шаронова С.А.");
        assert_eq!(v[0].room, "ГК-109");
        assert_eq!(v[1].weeks, Weeks::Bottom);
        assert_eq!(v[1].subject, "Литература");
        assert_eq!(v[1].room, "ГК-156");
    }

    #[test]
    fn one_side_only() {
        let v = parse_cell(" / Политология; сем; Глинская И.Ю.; ГК-152");
        assert_eq!(v.len(), 1);
        assert_eq!(v[0].weeks, Weeks::Bottom);
        let v = parse_cell("Политология; сем; Базгиев М.М.; ГК-315 /");
        assert_eq!(v[0].weeks, Weeks::Top);
        let v = parse_cell("/ Цифровая грамотность; лек; Ревинова С.Ю.; ФРЯ-349");
        assert_eq!(v[0].weeks, Weeks::Bottom);
        assert_eq!(v[0].kind, ClassKind::Lecture);
    }

    #[test]
    fn lecture_with_teacher_and_room_in_one_field() {
        let v = parse_cell("Математика и статистика; лек; Сорокин Л.А. ЕГФ-конф зал / Социология; лек; Авдеева Е.В.;  ЕГФ-конф зал");
        assert_eq!(v[0].teacher, "Сорокин Л.А.");
        assert_eq!(v[0].room, "ЕГФ-конф зал");
        assert_eq!(v[1].teacher, "Авдеева Е.В.");
        assert_eq!(v[1].room, "ЕГФ-конф зал");
    }

    #[test]
    fn remarks_placeholders_and_subgroups() {
        let c = one("Иностранный язык; сем; Преподаватель; ГК-; *см. расписание ГК-*");
        assert_eq!(c.teacher, "");
        assert_eq!(c.room, "ГК");
        assert_eq!(c.note, "см. расписание ГК-");
        let c = one("Цифровая грамотность; сем пг4; Ковалевич ; ГК-21");
        assert_eq!(c.teacher, "Ковалевич");
        assert_eq!(c.room, "ГК-21");
        assert_eq!(c.note, "пг4");
        let c = one("Русский язык и культура речи; сем; Александрова Н.А..; ГК-154");
        assert_eq!(c.teacher, "Александрова Н.А.");
    }

    #[test]
    fn sport_and_month_ranges() {
        let c = one("Физическая культура; сем; Преподаватель; ФОК-; *см.расписание ФОК*");
        assert_eq!(c.kind, ClassKind::Sport);
        assert_eq!(c.room, "ФОК");
        let c = one("Спецкурсы (ноябрь-декабрь)");
        assert_eq!(c.subject, "Спецкурсы");
        assert_eq!(c.months, Some((11, 12)));
        assert_eq!(c.kind, ClassKind::Other);
        assert_eq!(c.note, "ноябрь–декабрь");
    }

    #[test]
    fn same_class_both_weeks_is_every_week() {
        let c = one("Политология; сем; Глинская И.Ю.; ГК-152 / Политология; сем; Глинская И.Ю.; ГК-152");
        assert_eq!(c.weeks, Weeks::Every);
    }
}
