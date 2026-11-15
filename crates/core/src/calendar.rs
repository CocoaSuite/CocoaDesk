//! Calendar rules: alternating weeks, which classes happen on a date, and their status.

use chrono::{Datelike, Duration, NaiveDate, NaiveDateTime};
use serde::Serialize;

use crate::model::{Break, Class, Mark, MarkStatus, MarkedBy, Semester, WeekKind};

pub fn monday_of(date: NaiveDate) -> NaiveDate {
    date - Duration::days(date.weekday().num_days_from_monday() as i64)
}

/// Zero-based number of the week inside the semester, counted from the week that holds its
/// first day. Breaks do not interrupt the count. Works for dates outside the semester too, so
/// the grid can still say "top" or "bottom" while browsing.
pub fn week_offset(sem: &Semester, date: NaiveDate) -> i64 {
    (monday_of(date) - monday_of(sem.start)).num_days().div_euclid(7)
}

pub fn week_kind(sem: &Semester, date: NaiveDate) -> WeekKind {
    if week_offset(sem, date).rem_euclid(2) == 0 {
        sem.first_week
    } else {
        sem.first_week.other()
    }
}

pub fn break_on(sem: &Semester, date: NaiveDate) -> Option<&Break> {
    sem.breaks.iter().find(|b| date >= b.from && date <= b.to)
}

/// Whether `class` has a session on `date` (ignoring cancellations, which are marks).
pub fn occurs_on(sem: &Semester, class: &Class, date: NaiveDate) -> bool {
    sem.contains(date)
        && date.weekday().number_from_monday() as u8 == class.weekday
        && class.valid_from.is_none_or(|f| date >= f)
        && class.valid_to.is_none_or(|t| date <= t)
        && break_on(sem, date).is_none()
        && class.weeks.includes(week_kind(sem, date))
        && sem.period(class.period).is_some()
}

#[derive(Clone, Debug, PartialEq)]
pub struct Occurrence {
    pub class_id: i64,
    pub date: NaiveDate,
    pub start: NaiveDateTime,
    pub end: NaiveDateTime,
    pub created_at: NaiveDateTime,
}

pub fn occurrence(sem: &Semester, class: &Class, date: NaiveDate) -> Option<Occurrence> {
    if !occurs_on(sem, class, date) {
        return None;
    }
    let p = sem.period(class.period)?;
    Some(Occurrence { class_id: class.id, date, start: date.and_time(p.start), end: date.and_time(p.end), created_at: class.created_at })
}

pub fn occurrences_on(sem: &Semester, classes: &[Class], date: NaiveDate) -> Vec<Occurrence> {
    let mut v: Vec<_> = classes.iter().filter_map(|c| occurrence(sem, c, date)).collect();
    v.sort_by_key(|o| (o.start, o.class_id));
    v
}

/// Sessions that ended in `(since, now]` and are not yet marked. These are the ones the
/// background timer marks as done. Sessions that ended before a class was added are left alone.
pub fn due_for_auto_mark(
    sem: &Semester,
    classes: &[Class],
    since: NaiveDateTime,
    now: NaiveDateTime,
    is_marked: impl Fn(i64, NaiveDate) -> bool,
) -> Vec<Occurrence> {
    let mut out = Vec::new();
    if now <= since {
        return out;
    }
    let mut day = since.date().max(sem.start);
    let last = now.date().min(sem.end);
    while day <= last {
        for o in occurrences_on(sem, classes, day) {
            if o.end > since && o.end <= now && o.end > o.created_at && !is_marked(o.class_id, o.date) {
                out.push(o);
            }
        }
        day += Duration::days(1);
    }
    out
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Status {
    Upcoming,
    InProgress,
    /// Ended, recorded as attended.
    Done,
    Skipped,
    Cancelled,
    /// Ended and not yet recorded (automatic marking is off, or the timer has not run yet).
    Ended,
    /// Ended before the class was added to CocoaDesk, so nothing is known about it.
    Untracked,
}

pub fn resolve_status(o: &Occurrence, mark: Option<&Mark>, now: NaiveDateTime) -> (Status, Option<MarkedBy>) {
    if let Some(m) = mark {
        let s = match m.status {
            MarkStatus::Done => Status::Done,
            MarkStatus::Skipped => Status::Skipped,
            MarkStatus::Cancelled => Status::Cancelled,
        };
        return (s, Some(m.by));
    }
    if now < o.start {
        (Status::Upcoming, None)
    } else if now < o.end {
        (Status::InProgress, None)
    } else if o.end <= o.created_at {
        (Status::Untracked, None)
    } else {
        (Status::Ended, None)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::*;
    use chrono::NaiveTime;

    fn d(y: i32, m: u32, day: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, day).unwrap()
    }
    fn t(h: u32, m: u32) -> NaiveTime {
        NaiveTime::from_hms_opt(h, m, 0).unwrap()
    }
    fn sem() -> Semester {
        Semester {
            id: 1,
            name: "Autumn".into(),
            start: d(2026, 9, 1), // a Tuesday
            end: d(2026, 12, 31),
            first_week: WeekKind::Top,
            periods: vec![Period { number: 1, start: t(9, 0), end: t(10, 20) }, Period { number: 2, start: t(10, 30), end: t(11, 50) }],
            breaks: vec![Break { id: 1, from: d(2026, 11, 4), to: d(2026, 11, 4), label: "Holiday".into() }],
        }
    }
    fn class(id: i64, weekday: u8, period: u8, weeks: Weeks) -> Class {
        Class {
            id,
            semester_id: 1,
            subject: format!("C{id}"),
            kind: ClassKind::Seminar,
            teacher: String::new(),
            room: String::new(),
            note: String::new(),
            weekday,
            period,
            weeks,
            valid_from: None,
            valid_to: None,
            source: Source::Manual,
            import_key: None,
            created_at: d(2026, 8, 20).and_time(t(0, 0)),
        }
    }

    #[test]
    fn weeks_alternate_from_the_first_week() {
        let s = sem();
        // The week of 1 Sep (Mon 31 Aug – Sun 6 Sep) is top.
        assert_eq!(week_kind(&s, d(2026, 8, 31)), WeekKind::Top);
        assert_eq!(week_kind(&s, d(2026, 9, 6)), WeekKind::Top);
        assert_eq!(week_kind(&s, d(2026, 9, 7)), WeekKind::Bottom);
        assert_eq!(week_kind(&s, d(2026, 9, 14)), WeekKind::Top);
        // 16 Nov 2026 is 11 weeks after 31 Aug, so it's a bottom week.
        assert_eq!(week_offset(&s, d(2026, 11, 16)), 11);
        assert_eq!(week_kind(&s, d(2026, 11, 16)), WeekKind::Bottom);
        assert_eq!(week_kind(&s, d(2026, 11, 15)), WeekKind::Top);
    }

    #[test]
    fn breaks_do_not_shift_the_alternation() {
        let mut s = sem();
        s.breaks.push(Break { id: 2, from: d(2026, 10, 1), to: d(2026, 10, 31), label: "Long break".into() });
        assert_eq!(week_kind(&s, d(2026, 11, 16)), WeekKind::Bottom);
    }

    #[test]
    fn occurrences_respect_weeks_breaks_and_ranges() {
        let s = sem();
        let mut special = class(4, 4, 1, Weeks::Every);
        special.valid_from = Some(d(2026, 11, 1));
        special.valid_to = Some(d(2026, 12, 31));
        let classes = vec![
            class(1, 1, 1, Weeks::Every),
            class(2, 1, 2, Weeks::Top),
            class(3, 1, 2, Weeks::Bottom),
            special,
            class(5, 3, 1, Weeks::Every),
        ];
        // Monday 7 Sep is a bottom week.
        let ids: Vec<_> = occurrences_on(&s, &classes, d(2026, 9, 7)).iter().map(|o| o.class_id).collect();
        assert_eq!(ids, vec![1, 3]);
        // Monday 14 Sep is top.
        let ids: Vec<_> = occurrences_on(&s, &classes, d(2026, 9, 14)).iter().map(|o| o.class_id).collect();
        assert_eq!(ids, vec![1, 2]);
        // The November–December course: not in October, yes in November.
        assert!(occurrences_on(&s, &classes, d(2026, 10, 29)).is_empty());
        assert_eq!(occurrences_on(&s, &classes, d(2026, 11, 5)).len(), 1);
        // Wednesday 4 Nov is a holiday.
        assert!(occurrences_on(&s, &classes, d(2026, 11, 4)).is_empty());
        // Before the semester.
        assert!(occurrences_on(&s, &classes, d(2026, 8, 31)).is_empty());
    }

    #[test]
    fn status_follows_the_clock_and_marks() {
        let s = sem();
        let c = class(1, 1, 1, Weeks::Every);
        let o = occurrence(&s, &c, d(2026, 9, 7)).unwrap();
        let at = |h, m| d(2026, 9, 7).and_time(t(h, m));
        assert_eq!(resolve_status(&o, None, at(8, 59)).0, Status::Upcoming);
        assert_eq!(resolve_status(&o, None, at(9, 0)).0, Status::InProgress);
        assert_eq!(resolve_status(&o, None, at(10, 20)).0, Status::Ended);
        let mark = Mark { class_id: 1, date: d(2026, 9, 7), status: MarkStatus::Cancelled, by: MarkedBy::User, at: at(8, 0) };
        assert_eq!(resolve_status(&o, Some(&mark), at(9, 30)).0, Status::Cancelled);
    }

    #[test]
    fn classes_added_late_do_not_backfill() {
        let s = sem();
        let mut c = class(1, 1, 1, Weeks::Every);
        c.created_at = d(2026, 9, 10).and_time(t(12, 0));
        let o = occurrence(&s, &c, d(2026, 9, 7)).unwrap();
        assert_eq!(resolve_status(&o, None, d(2026, 9, 10).and_time(t(13, 0))).0, Status::Untracked);
        let due = due_for_auto_mark(&s, &[c], d(2026, 9, 1).and_time(t(0, 0)), d(2026, 9, 15).and_time(t(0, 0)), |_, _| false);
        let dates: Vec<_> = due.iter().map(|o| o.date).collect();
        assert_eq!(dates, vec![d(2026, 9, 14)]);
    }

    #[test]
    fn catch_up_after_sleep_marks_everything_missed_once() {
        let s = sem();
        let classes = vec![class(1, 1, 1, Weeks::Every), class(2, 1, 2, Weeks::Every)];
        let since = d(2026, 9, 7).and_time(t(8, 0));
        let now = d(2026, 9, 7).and_time(t(12, 0));
        assert_eq!(due_for_auto_mark(&s, &classes, since, now, |_, _| false).len(), 2);
        // Already marked: nothing to do.
        assert!(due_for_auto_mark(&s, &classes, since, now, |_, _| true).is_empty());
        // Halfway: only the first has ended.
        let half = due_for_auto_mark(&s, &classes, since, d(2026, 9, 7).and_time(t(11, 0)), |_, _| false);
        assert_eq!(half.len(), 1);
        assert_eq!(half[0].class_id, 1);
    }
}
