//! Importing a timetable from an Excel workbook.
//!
//! The expected layout is the one university timetables usually use: a header block with
//! columns "День", "Пары" and "Время" followed by one column per group, then one row per
//! period, with days merged down the first column. Lectures shared by several groups are
//! cells merged across their columns.

pub mod cell;

use std::collections::{BTreeMap, HashMap};
use std::io::Cursor;
use std::sync::LazyLock;

use calamine::{Data, Reader, Xlsx};
use chrono::{Datelike, NaiveDate, NaiveTime};
use regex::Regex;
use serde::Serialize;

#[cfg(test)]
use crate::model::Weeks;
use crate::model::{ClassInput, Period};
use crate::store::ImportedClass;
use crate::{Error, Result};

pub use cell::{parse_cell, ParsedClass};

/// A sheet as plain text, with merged regions filled in.
#[derive(Clone, Debug)]
pub struct Grid {
    pub name: String,
    rows: Vec<Vec<String>>,
    /// For each cell inside a merged region: how many columns the region spans.
    span: HashMap<(usize, usize), usize>,
}

impl Grid {
    pub fn text(&self, r: usize, c: usize) -> &str {
        self.rows.get(r).and_then(|row| row.get(c)).map(|s| s.as_str()).unwrap_or("")
    }
    pub fn height(&self) -> usize {
        self.rows.len()
    }
    pub fn width(&self) -> usize {
        self.rows.iter().map(|r| r.len()).max().unwrap_or(0)
    }
    fn col_span(&self, r: usize, c: usize) -> usize {
        self.span.get(&(r, c)).copied().unwrap_or(1)
    }

    /// Builds a grid from rows of text. Merged regions are `(top, left, bottom, right)`, inclusive.
    pub fn from_rows(name: &str, mut rows: Vec<Vec<String>>, merges: &[(usize, usize, usize, usize)]) -> Self {
        let mut span = HashMap::new();
        for &(r0, c0, r1, c1) in merges {
            let value = rows.get(r0).and_then(|r| r.get(c0)).cloned().unwrap_or_default();
            for r in r0..=r1 {
                if rows.len() <= r {
                    rows.resize(r + 1, vec![]);
                }
                for c in c0..=c1 {
                    if rows[r].len() <= c {
                        rows[r].resize(c + 1, String::new());
                    }
                    rows[r][c] = value.clone();
                    span.insert((r, c), c1 - c0 + 1);
                }
            }
        }
        Grid { name: name.to_string(), rows, span }
    }
}

fn data_text(d: &Data) -> String {
    match d {
        Data::Empty => String::new(),
        Data::String(s) => s.clone(),
        Data::Float(f) if f.fract() == 0.0 && f.abs() < 1e12 => format!("{}", *f as i64),
        Data::Float(f) => f.to_string(),
        Data::Int(i) => i.to_string(),
        Data::Bool(b) => b.to_string(),
        Data::DateTime(dt) => dt.to_string(),
        Data::DateTimeIso(s) | Data::DurationIso(s) => s.clone(),
        Data::Error(_) => String::new(),
    }
}

/// Reads every sheet of an .xlsx file.
pub fn read_workbook(bytes: &[u8]) -> Result<Vec<Grid>> {
    let mut wb: Xlsx<_> = Xlsx::new(Cursor::new(bytes)).map_err(|e| Error::Spreadsheet(e.to_string()))?;
    let names = wb.sheet_names().to_vec();
    let mut grids = Vec::new();
    for name in names {
        let range = wb.worksheet_range(&name).map_err(|e| Error::Spreadsheet(e.to_string()))?;
        let mut rows: Vec<Vec<String>> = Vec::new();
        if let (Some((r0, c0)), Some((r1, c1))) = (range.start(), range.end()) {
            rows = vec![vec![String::new(); c1 as usize + 1]; r1 as usize + 1];
            for r in r0..=r1 {
                for c in c0..=c1 {
                    if let Some(v) = range.get_value((r, c)) {
                        rows[r as usize][c as usize] = data_text(v);
                    }
                }
            }
        }
        let merges: Vec<_> = match wb.worksheet_merge_cells(&name) {
            Some(Ok(dims)) => dims.iter().map(|d| (d.start.0 as usize, d.start.1 as usize, d.end.0 as usize, d.end.1 as usize)).collect(),
            Some(Err(e)) => return Err(Error::Spreadsheet(e.to_string())),
            None => vec![],
        };
        grids.push(Grid::from_rows(&name, rows, &merges));
    }
    Ok(grids)
}

static TIME_RANGE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(\d{1,2})\s*[.:]\s*(\d{2})\s*[-–—]\s*(\d{1,2})\s*[.:]\s*(\d{2})").unwrap());
static NUMBER: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^\s*(\d{1,2})").unwrap());

pub fn weekday_from(text: &str) -> Option<u8> {
    let t = text.trim().to_lowercase();
    let t = t.trim_end_matches('.');
    let table: [(&[&str], u8); 7] = [
        (&["пн", "пон", "понедельник", "mon", "monday"], 1),
        (&["вт", "вто", "вторник", "tue", "tuesday"], 2),
        (&["ср", "сре", "среда", "wed", "wednesday"], 3),
        (&["чт", "чет", "четверг", "thu", "thursday"], 4),
        (&["пт", "пят", "пятница", "fri", "friday"], 5),
        (&["сб", "суб", "суббота", "sat", "saturday"], 6),
        (&["вс", "вос", "воскресенье", "sun", "sunday"], 7),
    ];
    table.iter().find(|(names, _)| names.contains(&t)).map(|(_, n)| *n)
}

fn time_range(text: &str) -> Option<(NaiveTime, NaiveTime)> {
    let c = TIME_RANGE.captures(text)?;
    let n = |i: usize| c[i].parse::<u32>().ok();
    let a = NaiveTime::from_hms_opt(n(1)?, n(2)?, 0)?;
    let b = NaiveTime::from_hms_opt(n(3)?, n(4)?, 0)?;
    (b > a).then_some((a, b))
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GroupColumn {
    pub column: usize,
    /// The group's code, e.g. "ШРОбд-04-26".
    pub label: String,
    /// Other header text above it (programme, faculty), for telling groups apart.
    pub detail: String,
}

#[derive(Clone, Debug)]
struct Layout {
    day_col: usize,
    period_col: usize,
    time_col: usize,
    first_data_row: usize,
    groups: Vec<GroupColumn>,
}

fn find_layout(g: &Grid) -> Result<Layout> {
    let mut header = None;
    'rows: for r in 0..g.height().min(60) {
        for c in 0..g.width().min(10) {
            let t = g.text(r, c).trim().to_lowercase();
            if t.starts_with("день") || t == "day" {
                header = Some((r, c));
                break 'rows;
            }
        }
    }
    let (header_row, day_col) = header.ok_or_else(|| {
        Error::invalid("Couldn't find the timetable header. The sheet should have columns named «День», «Пары» and «Время».")
    })?;
    let find = |pred: &dyn Fn(&str) -> bool, fallback: usize| {
        (0..g.width()).find(|&c| pred(&g.text(header_row, c).trim().to_lowercase())).unwrap_or(fallback)
    };
    let period_col = find(&|t: &str| t.starts_with("пар") || t.starts_with("№") || t == "period" || t == "periods", day_col + 1);
    let time_col = find(&|t: &str| t.starts_with("врем") || t == "time", period_col + 1);

    let first_data_row = (header_row + 1..g.height())
        .find(|&r| weekday_from(g.text(r, day_col)).is_some())
        .ok_or_else(|| Error::invalid("Couldn't find any days (ПН, ВТ, …) under the «День» column."))?;

    let first_group_col = day_col.max(period_col).max(time_col) + 1;
    let width = g.width();
    // The group row is the header row whose cells are all different from each other.
    let mut best: Option<(usize, usize)> = None;
    for r in header_row..first_data_row {
        let cells: Vec<&str> = (first_group_col..width).map(|c| g.text(r, c).trim()).filter(|t| !t.is_empty()).collect();
        if cells.is_empty() {
            continue;
        }
        let mut distinct = cells.clone();
        distinct.sort();
        distinct.dedup();
        if distinct.len() == cells.len() && best.is_none_or(|(_, n)| cells.len() >= n) {
            best = Some((r, cells.len()));
        }
    }
    let group_row = best.map(|(r, _)| r).unwrap_or(first_data_row.saturating_sub(1));
    let groups = (first_group_col..width)
        .filter_map(|c| {
            let label = g.text(group_row, c).trim().to_string();
            if label.is_empty() {
                return None;
            }
            let mut detail: Vec<String> = Vec::new();
            for r in header_row..first_data_row {
                if r == group_row {
                    continue;
                }
                let t = g.text(r, c).trim();
                if !t.is_empty() && !detail.iter().any(|d| d == t) {
                    detail.push(t.to_string());
                }
            }
            Some(GroupColumn { column: c, label, detail: detail.join(", ") })
        })
        .collect::<Vec<_>>();
    if groups.is_empty() {
        return Err(Error::invalid("Couldn't find any group columns to the right of «Время»."));
    }
    Ok(Layout { day_col, period_col, time_col, first_data_row, groups })
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportItem {
    /// Stays the same when a newer version of the file is imported, so updates replace in place.
    pub key: String,
    pub weekday: u8,
    pub period: u8,
    #[serde(flatten)]
    pub class: ParsedClass,
    /// Whether the cell was shared with other groups (a merged cell, usually a lecture).
    pub shared: bool,
    /// The original cell text.
    pub raw: String,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SheetPreview {
    pub sheet: String,
    pub groups: Vec<GroupColumn>,
    pub group: String,
    pub periods: Vec<Period>,
    pub items: Vec<ImportItem>,
    pub problems: Vec<String>,
}

/// Reads the classes for one group. `group` is matched by label; when it is `None` or not
/// found, the first group is used.
pub fn preview_sheet(g: &Grid, group: Option<&str>) -> Result<SheetPreview> {
    let layout = find_layout(g)?;
    let col = group.and_then(|label| layout.groups.iter().find(|gc| gc.label == label)).unwrap_or(&layout.groups[0]);
    let mut periods: BTreeMap<u8, (NaiveTime, NaiveTime)> = BTreeMap::new();
    let mut problems = Vec::new();
    let mut items: Vec<ImportItem> = Vec::new();
    let mut day: Option<u8> = None;

    for r in layout.first_data_row..g.height() {
        if let Some(d) = weekday_from(g.text(r, layout.day_col)) {
            day = Some(d);
        }
        let Some(weekday) = day else { continue };
        let Some(number) = NUMBER.captures(g.text(r, layout.period_col)).and_then(|c| c[1].parse::<u8>().ok()) else {
            continue;
        };
        if number == 0 {
            continue;
        }
        let time_text = g.text(r, layout.time_col);
        match time_range(time_text) {
            Some(t) => match periods.get(&number) {
                None => {
                    periods.insert(number, t);
                }
                Some(existing) if *existing != t => {
                    let msg = format!(
                        "Period {number} has different times on different days; using {}–{}.",
                        existing.0.format("%H:%M"),
                        existing.1.format("%H:%M")
                    );
                    if !problems.contains(&msg) {
                        problems.push(msg);
                    }
                }
                _ => {}
            },
            None if !time_text.trim().is_empty() => {
                problems.push(format!("Couldn't read the time «{}» for period {number}.", time_text.trim()))
            }
            None => {}
        }
        let raw = g.text(r, col.column).trim();
        if raw.is_empty() {
            continue;
        }
        let shared = g.col_span(r, col.column) > 1;
        let parsed = parse_cell(raw);
        if parsed.is_empty() {
            problems.push(format!("Couldn't read «{raw}»."));
        }
        for class in parsed {
            let base = format!("{}|{}|{}|{}|{}", weekday, number, class.weeks.as_str(), class.subject.to_lowercase(), class.kind.as_str());
            let mut key = base.clone();
            let mut n = 2;
            while items.iter().any(|i| i.key == key) {
                key = format!("{base}#{n}");
                n += 1;
            }
            items.push(ImportItem { key, weekday, period: number, class, shared, raw: raw.to_string() });
        }
    }
    for i in &items {
        if !periods.contains_key(&i.period) {
            let msg = format!("Period {} has no time in the file; add it to the bell schedule.", i.period);
            if !problems.contains(&msg) {
                problems.push(msg);
            }
        }
    }
    let periods = periods.into_iter().map(|(number, (start, end))| Period { number, start, end }).collect();
    Ok(SheetPreview { sheet: g.name.clone(), groups: layout.groups.clone(), group: col.label.clone(), periods, items, problems })
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkbookPreview {
    pub sheets: Vec<String>,
    #[serde(flatten)]
    pub preview: SheetPreview,
}

pub fn preview_workbook(bytes: &[u8], sheet: Option<&str>, group: Option<&str>) -> Result<WorkbookPreview> {
    let grids = read_workbook(bytes)?;
    if grids.is_empty() {
        return Err(Error::invalid("The workbook has no sheets."));
    }
    let sheets: Vec<String> = grids.iter().map(|g| g.name.clone()).collect();
    let grid = match sheet {
        Some(name) => grids.iter().find(|g| g.name == name).ok_or_else(|| Error::NotFound(format!("sheet «{name}»")))?,
        None => grids.iter().find(|g| find_layout(g).is_ok()).unwrap_or(&grids[0]),
    };
    Ok(WorkbookPreview { sheets, preview: preview_sheet(grid, group)? })
}

/// Turns a month range into dates inside the semester (months before the start month are next year).
pub fn month_range_dates(months: (u32, u32), sem_start: NaiveDate, sem_end: NaiveDate) -> Option<(NaiveDate, NaiveDate)> {
    let year_for = |m: u32| if m < sem_start.month() { sem_start.year() + 1 } else { sem_start.year() };
    let from = NaiveDate::from_ymd_opt(year_for(months.0), months.0, 1)?;
    let (ny, nm) = if months.1 == 12 { (year_for(12) + 1, 1) } else { (year_for(months.1), months.1 + 1) };
    let to = NaiveDate::from_ymd_opt(ny, nm, 1)?.pred_opt()?;
    Some((from.max(sem_start), to.min(sem_end)))
}

/// Converts preview items into classes for a semester.
pub fn to_imported(items: &[ImportItem], sem_start: NaiveDate, sem_end: NaiveDate) -> Vec<ImportedClass> {
    items
        .iter()
        .map(|i| {
            let range = i.class.months.and_then(|m| month_range_dates(m, sem_start, sem_end));
            ImportedClass {
                key: i.key.clone(),
                input: ClassInput {
                    subject: i.class.subject.clone(),
                    kind: i.class.kind,
                    teacher: i.class.teacher.clone(),
                    room: i.class.room.clone(),
                    note: i.class.note.clone(),
                    weekday: i.weekday,
                    period: i.period,
                    weeks: i.class.weeks,
                    valid_from: range.map(|r| r.0),
                    valid_to: range.map(|r| r.1),
                },
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(cells: &[&str]) -> Vec<String> {
        cells.iter().map(|s| s.to_string()).collect()
    }

    /// A small timetable laid out like a real one: header block, two groups, merged lecture.
    fn sample() -> Grid {
        let mut rows = vec![
            row(&["", "", "", "\"УТВЕРЖДАЮ\""]),
            row(&["", "", "", "1 КУРС"]),
            row(&[]),
            row(&["День", "Пары", "Время", "Факультет", "Факультет"]),
            row(&["", "", "", "Бакалавриат", "Бакалавриат"]),
            row(&["", "", "", "ГР-01-26", "ГР-02-26"]),
            row(&["ПН", "1", "9.00-10.20", "Математика; лек; Сорокин Л.А. ЕГФ-конф зал / Социология; лек; Авдеева Е.В.; ЕГФ-конф зал", ""]),
            row(&["", "2", "10.30- 11.50", "Политология; сем; Глинская И.Ю.; ГК-152 / ", " / Политология; сем; Глинская И.Ю.; ГК-152"]),
            row(&["ВТ", "1", "9.00-10.20", "", "Литература; сем; Попова Е.О.; ГК-156"]),
            row(&["", "2", "10.30-11.50", "Спецкурсы (ноябрь-декабрь)", ""]),
        ];
        rows[6].resize(5, String::new());
        // Monday is merged down two rows; the Monday lecture is merged across both groups,
        // and the Tuesday special course too.
        Grid::from_rows("1 курс", rows, &[(6, 0, 7, 0), (8, 0, 9, 0), (6, 3, 6, 4), (9, 3, 9, 4)])
    }

    #[test]
    fn finds_groups_and_periods() {
        let p = preview_sheet(&sample(), None).unwrap();
        assert_eq!(p.groups.iter().map(|g| g.label.as_str()).collect::<Vec<_>>(), vec!["ГР-01-26", "ГР-02-26"]);
        assert_eq!(p.groups[0].detail, "Факультет, Бакалавриат");
        assert_eq!(p.periods.len(), 2);
        assert_eq!(p.periods[1].start, NaiveTime::from_hms_opt(10, 30, 0).unwrap());
        assert!(p.problems.is_empty(), "{:?}", p.problems);
    }

    #[test]
    fn reads_one_groups_week_including_shared_lectures() {
        let p = preview_sheet(&sample(), Some("ГР-02-26")).unwrap();
        let summary: Vec<_> = p.items.iter().map(|i| (i.weekday, i.period, i.class.weeks, i.class.subject.as_str(), i.shared)).collect();
        assert_eq!(
            summary,
            vec![
                (1, 1, Weeks::Top, "Математика", true),
                (1, 1, Weeks::Bottom, "Социология", true),
                (1, 2, Weeks::Bottom, "Политология", false),
                (2, 1, Weeks::Every, "Литература", false),
                (2, 2, Weeks::Every, "Спецкурсы", true),
            ]
        );
    }

    #[test]
    fn keys_are_stable_and_unique() {
        let a = preview_sheet(&sample(), Some("ГР-01-26")).unwrap();
        let b = preview_sheet(&sample(), Some("ГР-01-26")).unwrap();
        assert_eq!(a.items.iter().map(|i| &i.key).collect::<Vec<_>>(), b.items.iter().map(|i| &i.key).collect::<Vec<_>>());
        let mut keys: Vec<_> = a.items.iter().map(|i| i.key.clone()).collect();
        keys.dedup();
        assert_eq!(keys.len(), a.items.len());
    }

    #[test]
    fn month_ranges_become_dates() {
        let s = NaiveDate::from_ymd_opt(2026, 9, 1).unwrap();
        let e = NaiveDate::from_ymd_opt(2026, 12, 31).unwrap();
        assert_eq!(
            month_range_dates((11, 12), s, e),
            Some((NaiveDate::from_ymd_opt(2026, 11, 1).unwrap(), NaiveDate::from_ymd_opt(2026, 12, 31).unwrap()))
        );
        let s2 = NaiveDate::from_ymd_opt(2027, 2, 9).unwrap();
        let e2 = NaiveDate::from_ymd_opt(2027, 6, 30).unwrap();
        assert_eq!(month_range_dates((2, 3), s2, e2), Some((s2, NaiveDate::from_ymd_opt(2027, 3, 31).unwrap())));
    }

    #[test]
    fn rejects_sheets_without_a_header() {
        let g = Grid::from_rows("x", vec![row(&["hello"])], &[]);
        assert!(preview_sheet(&g, None).is_err());
    }

    /// Checks a real timetable when one is available locally:
    /// `COCOADESK_SAMPLE_XLSX=/path/file.xlsx cargo test -p cocoadesk-core -- --ignored`
    #[test]
    #[ignore]
    fn reads_a_real_file() {
        let path = std::env::var("COCOADESK_SAMPLE_XLSX").expect("set COCOADESK_SAMPLE_XLSX");
        let bytes = std::fs::read(path).unwrap();
        let group = std::env::var("COCOADESK_SAMPLE_GROUP").ok();
        let p = preview_workbook(&bytes, None, group.as_deref()).unwrap();
        println!(
            "sheets: {:?}\ngroup: {}\nperiods: {:?}\nproblems: {:?}",
            p.sheets, p.preview.group, p.preview.periods, p.preview.problems
        );
        for i in &p.preview.items {
            println!(
                "{} {} {:?} | {} | {:?} | {} | {} | {} | {:?} | shared={}",
                i.weekday,
                i.period,
                i.class.weeks,
                i.class.subject,
                i.class.kind,
                i.class.teacher,
                i.class.room,
                i.class.note,
                i.class.months,
                i.shared
            );
        }
        assert!(!p.preview.items.is_empty());
    }
}
