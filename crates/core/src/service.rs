//! Operations the app exposes, built from the calendar rules and the store.

use std::collections::HashMap;

use chrono::{Datelike, Duration, NaiveDate, NaiveDateTime};
use serde::{Deserialize, Serialize};

use crate::calendar::{self, Occurrence, Status};
use crate::import::{self, WorkbookPreview};
use crate::model::*;
use crate::store::{ImportResult, SemesterInput, Store};
use crate::{Error, Result};

// ---------- settings ----------

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Settings {
    /// Mark a class as done when it ends.
    pub auto_mark: bool,
    /// Minutes before a class to show a reminder; 0 turns reminders off.
    pub remind_minutes: u32,
    /// Keep running in the notification area when the window is closed.
    pub close_to_tray: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Settings { auto_mark: true, remind_minutes: 10, close_to_tray: true }
    }
}

impl Settings {
    pub fn load(store: &Store) -> Result<Self> {
        let d = Settings::default();
        let flag = |k: &str, def: bool| -> Result<bool> { Ok(store.setting(k)?.map(|v| v == "1").unwrap_or(def)) };
        Ok(Settings {
            auto_mark: flag("auto_mark", d.auto_mark)?,
            remind_minutes: store.setting("remind_minutes")?.and_then(|v| v.parse().ok()).unwrap_or(d.remind_minutes),
            close_to_tray: flag("close_to_tray", d.close_to_tray)?,
        })
    }
    pub fn save(&self, store: &Store) -> Result<()> {
        if self.remind_minutes > 24 * 60 {
            return Err(Error::invalid("Reminders can be at most a day ahead."));
        }
        store.set_setting("auto_mark", if self.auto_mark { "1" } else { "0" })?;
        store.set_setting("remind_minutes", &self.remind_minutes.to_string())?;
        store.set_setting("close_to_tray", if self.close_to_tray { "1" } else { "0" })?;
        Ok(())
    }
}

// ---------- views ----------

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SemesterSummary {
    #[serde(flatten)]
    pub semester: Semester,
    pub class_count: usize,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EntryView {
    pub class: Class,
    pub date: NaiveDate,
    #[serde(with = "hm_dt")]
    pub start: NaiveDateTime,
    #[serde(with = "hm_dt")]
    pub end: NaiveDateTime,
    pub status: Status,
    pub marked_by: Option<MarkedBy>,
    /// 0–1 while the class is running.
    pub progress: Option<f64>,
    pub minutes_left: Option<i64>,
    pub minutes_until: Option<i64>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DayView {
    pub date: NaiveDate,
    pub weekday: u8,
    pub is_today: bool,
    pub in_semester: bool,
    pub break_label: Option<String>,
    pub week_kind: Option<WeekKind>,
    pub entries: Vec<EntryView>,
    /// Classes in this slot that run in the other week, shown faintly.
    pub other_week: Vec<Class>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WeekView {
    pub semesters: Vec<SemesterSummary>,
    pub semester: Option<SemesterSummary>,
    pub monday: NaiveDate,
    pub week_kind: Option<WeekKind>,
    /// 1-based week of the semester, when the week overlaps it.
    pub week_number: Option<i64>,
    pub week_count: Option<i64>,
    pub days: Vec<DayView>,
    /// Today in the semester that contains today, for the side panel.
    pub today: Option<DayView>,
    pub now: NaiveDateTime,
    pub settings: Settings,
}

mod hm_dt {
    use chrono::NaiveDateTime;
    use serde::Serializer;
    pub fn serialize<S: Serializer>(t: &NaiveDateTime, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&t.format("%H:%M").to_string())
    }
}

/// The semester to show for a date: the one containing it, else the next one, else the last.
pub fn semester_for(semesters: &[Semester], date: NaiveDate) -> Option<&Semester> {
    semesters
        .iter()
        .find(|s| s.contains(date))
        .or_else(|| semesters.iter().filter(|s| s.start > date).min_by_key(|s| s.start))
        .or_else(|| semesters.iter().max_by_key(|s| s.end))
}

fn day_view(sem: &Semester, classes: &[Class], marks: &HashMap<(i64, NaiveDate), Mark>, date: NaiveDate, now: NaiveDateTime) -> DayView {
    let in_semester = sem.contains(date);
    let break_label = calendar::break_on(sem, date).map(|b| b.label.clone());
    let entries = calendar::occurrences_on(sem, classes, date)
        .into_iter()
        .filter_map(|o| {
            let class = classes.iter().find(|c| c.id == o.class_id)?.clone();
            Some(entry(class, &o, marks.get(&(o.class_id, o.date)), now))
        })
        .collect();
    let kind = calendar::week_kind(sem, date);
    let weekday = date.weekday().number_from_monday() as u8;
    let other_week = if in_semester && break_label.is_none() {
        classes
            .iter()
            .filter(|c| {
                c.weekday == weekday
                    && !c.weeks.includes(kind)
                    && c.valid_from.is_none_or(|f| date >= f)
                    && c.valid_to.is_none_or(|t| date <= t)
            })
            .cloned()
            .collect()
    } else {
        vec![]
    };
    DayView {
        date,
        weekday,
        is_today: date == now.date(),
        in_semester,
        break_label,
        week_kind: in_semester.then_some(kind),
        entries,
        other_week,
    }
}

fn entry(class: Class, o: &Occurrence, mark: Option<&Mark>, now: NaiveDateTime) -> EntryView {
    let (status, marked_by) = calendar::resolve_status(o, mark, now);
    let total = (o.end - o.start).num_seconds().max(1) as f64;
    let running = status == Status::InProgress;
    EntryView {
        class,
        date: o.date,
        start: o.start,
        end: o.end,
        status,
        marked_by,
        progress: running.then(|| ((now - o.start).num_seconds() as f64 / total).clamp(0.0, 1.0)),
        minutes_left: running.then(|| (o.end - now).num_minutes() + 1),
        minutes_until: (now < o.start).then(|| (o.start - now).num_minutes() + 1),
    }
}

fn marks_map(store: &Store, sem: &Semester, from: NaiveDate, to: NaiveDate) -> Result<HashMap<(i64, NaiveDate), Mark>> {
    Ok(store.marks_between(sem.id, from, to)?.into_iter().map(|m| ((m.class_id, m.date), m)).collect())
}

fn summaries(store: &Store, semesters: &[Semester]) -> Result<Vec<SemesterSummary>> {
    semesters.iter().map(|s| Ok(SemesterSummary { semester: s.clone(), class_count: store.classes(s.id)?.len() })).collect()
}

/// Everything the Classes screen needs for the week containing `date`.
pub fn week_view(store: &Store, semester_id: Option<i64>, date: NaiveDate, now: NaiveDateTime) -> Result<WeekView> {
    let semesters = store.semesters()?;
    let monday = calendar::monday_of(date);
    let settings = Settings::load(store)?;
    let sem = match semester_id {
        Some(id) => semesters.iter().find(|s| s.id == id),
        None => semester_for(&semesters, date),
    };
    let all = summaries(store, &semesters)?;
    let Some(sem) = sem else {
        return Ok(WeekView {
            semesters: all,
            semester: None,
            monday,
            week_kind: None,
            week_number: None,
            week_count: None,
            days: vec![],
            today: None,
            now,
            settings,
        });
    };
    let classes = store.classes(sem.id)?;
    let sunday = monday + Duration::days(6);
    let marks = marks_map(store, sem, monday, sunday)?;
    let days: Vec<DayView> = (0..7).map(|i| day_view(sem, &classes, &marks, monday + Duration::days(i), now)).collect();
    let overlaps = monday <= sem.end && sunday >= sem.start;
    let week_number = overlaps.then(|| calendar::week_offset(sem, monday) + 1);
    let week_count = Some(calendar::week_offset(sem, sem.end) + 1);

    let today = match semesters.iter().find(|s| s.contains(now.date())) {
        Some(ts) => {
            let tclasses = if ts.id == sem.id { classes.clone() } else { store.classes(ts.id)? };
            let tmarks = marks_map(store, ts, now.date(), now.date())?;
            Some(day_view(ts, &tclasses, &tmarks, now.date(), now))
        }
        None => None,
    };
    Ok(WeekView {
        semester: all.iter().find(|s| s.semester.id == sem.id).cloned(),
        semesters: all,
        monday,
        week_kind: overlaps.then(|| calendar::week_kind(sem, monday)),
        week_number,
        week_count,
        days,
        today,
        now,
        settings,
    })
}

/// Records what happened to one session, or clears it when `status` is `None`.
pub fn set_status(store: &Store, class_id: i64, date: NaiveDate, status: Option<MarkStatus>, now: NaiveDateTime) -> Result<()> {
    let class = store.class(class_id)?;
    let sem = store.semester(class.semester_id)?;
    if !calendar::occurs_on(&sem, &class, date) {
        return Err(Error::invalid("That class doesn't take place on that day."));
    }
    match status {
        Some(s) => store.set_mark(&Mark { class_id, date, status: s, by: MarkedBy::User, at: now }),
        None => store.clear_mark(class_id, date),
    }
}

// ---------- background timer ----------

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Reminder {
    pub class_id: i64,
    pub date: NaiveDate,
    pub subject: String,
    pub kind: ClassKind,
    pub room: String,
    #[serde(with = "hm_dt")]
    pub start: NaiveDateTime,
    pub minutes_until: i64,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TickOutcome {
    pub marked: usize,
    pub reminders: Vec<Reminder>,
}

const LAST_TICK: &str = "last_tick";
const STAMP: &str = "%Y-%m-%dT%H:%M:%S";
/// How far back the timer catches up after the computer was off.
const MAX_CATCH_UP_DAYS: i64 = 120;

/// Runs once a minute (and on wake): marks ended classes as done and collects due reminders.
/// Everything is driven by the stored time of the previous run, so sleep, shutdown or a crash
/// only delay the work; they never lose or repeat it.
pub fn tick(store: &Store, now: NaiveDateTime) -> Result<TickOutcome> {
    let settings = Settings::load(store)?;
    let last = store.setting(LAST_TICK)?.and_then(|s| NaiveDateTime::parse_from_str(&s, STAMP).ok());
    let since = match last {
        Some(t) if t <= now => t.max(now - Duration::days(MAX_CATCH_UP_DAYS)),
        // First run, or the clock moved backwards: start from now.
        _ => now,
    };
    let mut out = TickOutcome::default();
    let semesters = store.semesters()?;

    if settings.auto_mark && since < now {
        for sem in semesters.iter().filter(|s| s.start <= now.date() && s.end >= since.date()) {
            let classes = store.classes(sem.id)?;
            let marks = marks_map(store, sem, since.date(), now.date())?;
            let due = calendar::due_for_auto_mark(sem, &classes, since, now, |id, d| marks.contains_key(&(id, d)));
            for o in due {
                if store.set_auto_mark_if_absent(o.class_id, o.date, o.end)? {
                    out.marked += 1;
                }
            }
        }
    }

    if settings.remind_minutes > 0 {
        let lead = Duration::minutes(settings.remind_minutes as i64);
        let horizon = now + lead;
        let mut day = now.date();
        while day <= horizon.date() {
            if let Some(sem) = semesters.iter().find(|s| s.contains(day)) {
                let classes = store.classes(sem.id)?;
                let marks = marks_map(store, sem, day, day)?;
                for o in calendar::occurrences_on(sem, &classes, day) {
                    if o.start <= now || o.start > horizon || marks.contains_key(&(o.class_id, o.date)) {
                        continue;
                    }
                    if store.claim_reminder(o.class_id, o.date)? {
                        let c = classes.iter().find(|c| c.id == o.class_id).expect("occurrence of a listed class");
                        out.reminders.push(Reminder {
                            class_id: c.id,
                            date: o.date,
                            subject: c.subject.clone(),
                            kind: c.kind,
                            room: c.room.clone(),
                            start: o.start,
                            minutes_until: (o.start - now).num_minutes().max(0) + 1,
                        });
                    }
                }
            }
            day += Duration::days(1);
        }
    }

    store.set_setting(LAST_TICK, &now.format(STAMP).to_string())?;
    Ok(out)
}

// ---------- import ----------

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase", tag = "kind")]
pub enum ImportTarget {
    Existing { id: i64 },
    New { semester: SemesterInput },
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportRequest {
    pub sheet: String,
    pub group: String,
    pub target: ImportTarget,
    pub included_keys: Vec<String>,
    /// Replace the semester's bell schedule with the times from the file.
    pub use_file_periods: bool,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportDone {
    pub semester_id: i64,
    #[serde(flatten)]
    pub result: ImportResult,
}

pub fn preview_import(store: &Store, bytes: &[u8], sheet: Option<&str>, group: Option<&str>) -> Result<WorkbookPreview> {
    let remembered = store.setting("import_group")?;
    import::preview_workbook(bytes, sheet, group.or(remembered.as_deref()))
}

pub fn commit_import(store: &mut Store, bytes: &[u8], req: &ImportRequest, now: NaiveDateTime) -> Result<ImportDone> {
    let preview = import::preview_workbook(bytes, Some(&req.sheet), Some(&req.group))?.preview;
    if preview.group != req.group {
        return Err(Error::invalid(format!("The group «{}» isn't in this file any more.", req.group)));
    }
    let semester_id = match &req.target {
        ImportTarget::Existing { id } => store.semester(*id)?.id,
        ImportTarget::New { semester } => store.create_semester(semester)?,
    };
    let sem = store.semester(semester_id)?;
    let (included, excluded): (Vec<_>, Vec<_>) = preview.items.iter().cloned().partition(|i| req.included_keys.contains(&i.key));
    let use_file_periods = req.use_file_periods || sem.periods.is_empty();
    let periods = if use_file_periods { preview.periods.clone() } else { sem.periods.clone() };
    for i in &included {
        if !periods.iter().any(|p| p.number == i.period) {
            return Err(Error::invalid(format!("Period {} has no time. Add it to the bell schedule first.", i.period)));
        }
    }
    let imported = import::to_imported(&included, sem.start, sem.end);
    let excluded_keys: Vec<String> = excluded.into_iter().map(|i| i.key).collect();
    let result = store.apply_import(semester_id, use_file_periods.then_some(&preview.periods[..]), &imported, &excluded_keys, now)?;
    store.set_setting("import_group", &req.group)?;
    Ok(ImportDone { semester_id, result })
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::NaiveTime;

    fn d(y: i32, m: u32, day: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, day).unwrap()
    }
    fn at(date: NaiveDate, h: u32, m: u32) -> NaiveDateTime {
        date.and_time(NaiveTime::from_hms_opt(h, m, 0).unwrap())
    }
    fn setup() -> (Store, i64, i64) {
        let mut s = Store::open_in_memory().unwrap();
        let id = s
            .create_semester(&SemesterInput {
                name: "Autumn 2026".into(),
                start: d(2026, 9, 1),
                end: d(2026, 12, 31),
                first_week: WeekKind::Top,
            })
            .unwrap();
        s.set_periods(
            id,
            &[
                Period { number: 1, start: NaiveTime::from_hms_opt(9, 0, 0).unwrap(), end: NaiveTime::from_hms_opt(10, 20, 0).unwrap() },
                Period { number: 2, start: NaiveTime::from_hms_opt(10, 30, 0).unwrap(), end: NaiveTime::from_hms_opt(11, 50, 0).unwrap() },
            ],
        )
        .unwrap();
        let input = ClassInput {
            subject: "Математика".into(),
            kind: ClassKind::Seminar,
            teacher: String::new(),
            room: "ГК-315".into(),
            note: String::new(),
            weekday: 1,
            period: 1,
            weeks: Weeks::Every,
            valid_from: None,
            valid_to: None,
        };
        let cid = s.add_class(id, &input, at(d(2026, 11, 1), 12, 0)).unwrap();
        (s, id, cid)
    }

    #[test]
    fn tick_marks_ended_classes_and_catches_up_after_sleep() {
        let (s, id, cid) = setup();
        // First run just records the time.
        assert_eq!(tick(&s, at(d(2026, 11, 2), 8, 0)).unwrap().marked, 0);
        // Computer sleeps until Tuesday the 10th: two Monday sessions ended meanwhile.
        let out = tick(&s, at(d(2026, 11, 10), 7, 0)).unwrap();
        assert_eq!(out.marked, 2);
        // Running again marks nothing new.
        assert_eq!(tick(&s, at(d(2026, 11, 10), 7, 1)).unwrap().marked, 0);
        let marks = s.marks_between(id, d(2026, 11, 1), d(2026, 11, 30)).unwrap();
        assert!(marks.iter().all(|m| m.class_id == cid && m.by == MarkedBy::Auto && m.status == MarkStatus::Done));
    }

    #[test]
    fn cancelled_classes_are_not_marked_or_reminded() {
        let (s, _, cid) = setup();
        let monday = d(2026, 11, 16);
        tick(&s, at(monday, 7, 0)).unwrap();
        set_status(&s, cid, monday, Some(MarkStatus::Cancelled), at(monday, 7, 5)).unwrap();
        let out = tick(&s, at(monday, 8, 55)).unwrap();
        assert!(out.reminders.is_empty());
        let out = tick(&s, at(monday, 12, 0)).unwrap();
        assert_eq!(out.marked, 0);
    }

    #[test]
    fn reminders_fire_once_inside_the_lead_time() {
        let (s, _, cid) = setup();
        let monday = d(2026, 11, 16);
        tick(&s, at(monday, 8, 0)).unwrap();
        assert!(tick(&s, at(monday, 8, 45)).unwrap().reminders.is_empty());
        let r = tick(&s, at(monday, 8, 51)).unwrap().reminders;
        assert_eq!(r.len(), 1);
        assert_eq!(r[0].class_id, cid);
        assert_eq!(r[0].minutes_until, 10);
        assert!(tick(&s, at(monday, 8, 52)).unwrap().reminders.is_empty());
    }

    #[test]
    fn week_view_shows_status_progress_and_other_week() {
        let (s, id, cid) = setup();
        let bottom_only = ClassInput {
            subject: "Политология".into(),
            kind: ClassKind::Seminar,
            teacher: String::new(),
            room: String::new(),
            note: String::new(),
            weekday: 1,
            period: 2,
            weeks: Weeks::Top,
            valid_from: None,
            valid_to: None,
        };
        s.add_class(id, &bottom_only, at(d(2026, 11, 1), 12, 0)).unwrap();
        let monday = d(2026, 11, 16); // a bottom week
        let v = week_view(&s, None, monday, at(monday, 9, 40)).unwrap();
        assert_eq!(v.week_kind, Some(WeekKind::Bottom));
        assert_eq!(v.week_number, Some(12));
        assert_eq!(v.days.len(), 7);
        let mon = &v.days[0];
        assert_eq!(mon.entries.len(), 1);
        assert_eq!(mon.entries[0].class.id, cid);
        assert_eq!(mon.entries[0].status, Status::InProgress);
        assert_eq!(mon.entries[0].minutes_left, Some(41));
        assert_eq!(mon.other_week.len(), 1);
        assert!(v.today.is_some());
    }

    #[test]
    fn set_status_rejects_days_without_the_class() {
        let (s, _, cid) = setup();
        assert!(set_status(&s, cid, d(2026, 11, 17), Some(MarkStatus::Skipped), at(d(2026, 11, 17), 9, 0)).is_err());
    }
}
