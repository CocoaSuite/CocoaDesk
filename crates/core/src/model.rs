use chrono::{NaiveDate, NaiveDateTime, NaiveTime};
use serde::{Deserialize, Serialize};

use crate::{Error, Result};

/// Which of the two alternating weeks a calendar week is.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum WeekKind {
    Top,
    Bottom,
}

impl WeekKind {
    pub fn other(self) -> Self {
        match self {
            WeekKind::Top => WeekKind::Bottom,
            WeekKind::Bottom => WeekKind::Top,
        }
    }
    pub fn as_str(self) -> &'static str {
        match self {
            WeekKind::Top => "top",
            WeekKind::Bottom => "bottom",
        }
    }
    pub fn parse(s: &str) -> Result<Self> {
        match s {
            "top" => Ok(WeekKind::Top),
            "bottom" => Ok(WeekKind::Bottom),
            _ => Err(Error::invalid(format!("unknown week kind '{s}'"))),
        }
    }
}

/// Which weeks a class runs in.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Weeks {
    Every,
    Top,
    Bottom,
}

impl Weeks {
    pub fn includes(self, kind: WeekKind) -> bool {
        match self {
            Weeks::Every => true,
            Weeks::Top => kind == WeekKind::Top,
            Weeks::Bottom => kind == WeekKind::Bottom,
        }
    }
    pub fn only(kind: WeekKind) -> Self {
        match kind {
            WeekKind::Top => Weeks::Top,
            WeekKind::Bottom => Weeks::Bottom,
        }
    }
    pub fn as_str(self) -> &'static str {
        match self {
            Weeks::Every => "every",
            Weeks::Top => "top",
            Weeks::Bottom => "bottom",
        }
    }
    pub fn parse(s: &str) -> Result<Self> {
        match s {
            "every" => Ok(Weeks::Every),
            "top" => Ok(Weeks::Top),
            "bottom" => Ok(Weeks::Bottom),
            _ => Err(Error::invalid(format!("unknown weeks value '{s}'"))),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ClassKind {
    Lecture,
    Seminar,
    Lab,
    Practice,
    Sport,
    Other,
}

impl ClassKind {
    pub fn as_str(self) -> &'static str {
        match self {
            ClassKind::Lecture => "lecture",
            ClassKind::Seminar => "seminar",
            ClassKind::Lab => "lab",
            ClassKind::Practice => "practice",
            ClassKind::Sport => "sport",
            ClassKind::Other => "other",
        }
    }
    pub fn parse(s: &str) -> Result<Self> {
        Ok(match s {
            "lecture" => ClassKind::Lecture,
            "seminar" => ClassKind::Seminar,
            "lab" => ClassKind::Lab,
            "practice" => ClassKind::Practice,
            "sport" => ClassKind::Sport,
            "other" => ClassKind::Other,
            _ => return Err(Error::invalid(format!("unknown class type '{s}'"))),
        })
    }
}

/// One slot of the bell schedule ("пара").
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Period {
    pub number: u8,
    #[serde(with = "hm")]
    pub start: NaiveTime,
    #[serde(with = "hm")]
    pub end: NaiveTime,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Break {
    pub id: i64,
    pub from: NaiveDate,
    pub to: NaiveDate,
    pub label: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Semester {
    pub id: i64,
    pub name: String,
    pub start: NaiveDate,
    pub end: NaiveDate,
    /// Kind of the week that contains `start`. Weeks alternate from there.
    pub first_week: WeekKind,
    pub periods: Vec<Period>,
    pub breaks: Vec<Break>,
}

impl Semester {
    pub fn period(&self, number: u8) -> Option<&Period> {
        self.periods.iter().find(|p| p.number == number)
    }
    pub fn contains(&self, date: NaiveDate) -> bool {
        date >= self.start && date <= self.end
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Source {
    Manual,
    Import,
}

impl Source {
    pub fn as_str(self) -> &'static str {
        match self {
            Source::Manual => "manual",
            Source::Import => "import",
        }
    }
    pub fn parse(s: &str) -> Result<Self> {
        match s {
            "manual" => Ok(Source::Manual),
            "import" => Ok(Source::Import),
            _ => Err(Error::invalid(format!("unknown source '{s}'"))),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Class {
    pub id: i64,
    pub semester_id: i64,
    pub subject: String,
    pub kind: ClassKind,
    pub teacher: String,
    pub room: String,
    pub note: String,
    /// ISO weekday, 1 = Monday … 7 = Sunday.
    pub weekday: u8,
    pub period: u8,
    pub weeks: Weeks,
    /// Optional limits inside the semester, e.g. a course that only runs in November–December.
    pub valid_from: Option<NaiveDate>,
    pub valid_to: Option<NaiveDate>,
    pub source: Source,
    pub import_key: Option<String>,
    /// Classes are only marked done automatically for sessions that end after this moment,
    /// so adding a schedule mid-semester does not invent attendance for past weeks.
    pub created_at: NaiveDateTime,
}

/// Fields a person (or an import) supplies for a class.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ClassInput {
    pub subject: String,
    pub kind: ClassKind,
    #[serde(default)]
    pub teacher: String,
    #[serde(default)]
    pub room: String,
    #[serde(default)]
    pub note: String,
    pub weekday: u8,
    pub period: u8,
    pub weeks: Weeks,
    #[serde(default)]
    pub valid_from: Option<NaiveDate>,
    #[serde(default)]
    pub valid_to: Option<NaiveDate>,
}

impl ClassInput {
    pub fn validate(&self) -> Result<()> {
        if self.subject.trim().is_empty() {
            return Err(Error::invalid("A class needs a subject."));
        }
        if !(1..=7).contains(&self.weekday) {
            return Err(Error::invalid("Day must be between Monday and Sunday."));
        }
        if self.period == 0 {
            return Err(Error::invalid("Pick a period."));
        }
        if let (Some(a), Some(b)) = (self.valid_from, self.valid_to) {
            if a > b {
                return Err(Error::invalid("The class's first date is after its last date."));
            }
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MarkStatus {
    Done,
    Skipped,
    Cancelled,
}

impl MarkStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            MarkStatus::Done => "done",
            MarkStatus::Skipped => "skipped",
            MarkStatus::Cancelled => "cancelled",
        }
    }
    pub fn parse(s: &str) -> Result<Self> {
        match s {
            "done" => Ok(MarkStatus::Done),
            "skipped" => Ok(MarkStatus::Skipped),
            "cancelled" => Ok(MarkStatus::Cancelled),
            _ => Err(Error::invalid(format!("unknown status '{s}'"))),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MarkedBy {
    Auto,
    User,
}

impl MarkedBy {
    pub fn as_str(self) -> &'static str {
        match self {
            MarkedBy::Auto => "auto",
            MarkedBy::User => "user",
        }
    }
    pub fn parse(s: &str) -> Result<Self> {
        match s {
            "auto" => Ok(MarkedBy::Auto),
            "user" => Ok(MarkedBy::User),
            _ => Err(Error::invalid(format!("unknown marker '{s}'"))),
        }
    }
}

/// A recorded outcome for one session of a class on one date.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Mark {
    pub class_id: i64,
    pub date: NaiveDate,
    pub status: MarkStatus,
    pub by: MarkedBy,
    pub at: NaiveDateTime,
}

/// Times as "HH:MM".
pub mod hm {
    use chrono::NaiveTime;
    use serde::{Deserialize, Deserializer, Serializer};

    pub fn serialize<S: Serializer>(t: &NaiveTime, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&t.format("%H:%M").to_string())
    }
    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<NaiveTime, D::Error> {
        let s = String::deserialize(d)?;
        NaiveTime::parse_from_str(&s, "%H:%M").or_else(|_| NaiveTime::parse_from_str(&s, "%H:%M:%S")).map_err(serde::de::Error::custom)
    }
}
