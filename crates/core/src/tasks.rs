//! Task boards in the spirit of Trello: boards hold lists, lists hold cards.
//!
//! A card can carry a salary — what a client pays for the work, in dollars or rubles. The
//! service's commission (10% unless the board or the card says otherwise) is taken off, and
//! the other currency is worked out from the Central Bank of Russia's dollar rate. When a card
//! is marked paid, the rate of that day is kept with it, so the ruble amount you actually got
//! doesn't drift afterwards.
//!
//! Lists can count as *done*. A card moved into such a list is stamped with the time, and the
//! monthly earnings add up the salaries of cards finished in each month.

use chrono::{Datelike, NaiveDate, NaiveDateTime};
use rusqlite::{params, OptionalExtension, Row};
use serde::{Deserialize, Serialize};

use crate::news::Fetch;
use crate::store::Store;
use crate::{Error, Result};

const STAMP: &str = "%Y-%m-%dT%H:%M:%S";
const DATE: &str = "%Y-%m-%d";

/// The commission a new board starts with, in percent.
pub const DEFAULT_COMMISSION: f64 = 10.0;
/// How many months the earnings history covers, this one included.
pub const MONTHS_SHOWN: usize = 6;
const MAX_AMOUNT: f64 = 1_000_000_000.0;

const RATE_KEY: &str = "tasks.rate";
const MANUAL_RATE_KEY: &str = "tasks.rate_manual";

/// The Central Bank of Russia's daily rates, and a mirror of the same file for when the bank's
/// own site doesn't answer.
pub const RATE_SOURCES: &[&str] = &["https://www.cbr.ru/scripts/XML_daily.asp", "https://www.cbr-xml-daily.ru/daily_utf8.xml"];

// ---------- types ----------

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Currency {
    Usd,
    Rub,
}

impl Currency {
    fn as_str(self) -> &'static str {
        match self {
            Currency::Usd => "usd",
            Currency::Rub => "rub",
        }
    }
    fn parse(s: &str) -> Option<Self> {
        match s {
            "usd" => Some(Currency::Usd),
            "rub" => Some(Currency::Rub),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Label {
    pub id: String,
    pub name: String,
    pub color: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BoardInput {
    pub name: String,
    pub color: String,
    /// Percent taken by the service, 0–100.
    pub commission: f64,
    pub labels: Vec<Label>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TaskBoard {
    pub id: i64,
    pub name: String,
    pub color: String,
    pub commission: f64,
    pub labels: Vec<Label>,
    pub cards: usize,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TaskList {
    pub id: i64,
    pub board_id: i64,
    pub name: String,
    pub is_done: bool,
    pub position: i64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChecklistItem {
    pub text: String,
    pub done: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Salary {
    /// What the client pays, before the commission.
    pub amount: f64,
    pub currency: Currency,
    /// This card's own commission in percent; `None` uses the board's.
    pub commission: Option<f64>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CardInput {
    pub title: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub labels: Vec<String>,
    #[serde(default)]
    pub due: Option<NaiveDate>,
    #[serde(default)]
    pub checklist: Vec<ChecklistItem>,
    #[serde(default)]
    pub salary: Option<Salary>,
}

/// A salary worked out in both currencies. A side is `None` when no rate is known yet.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Money {
    pub commission: f64,
    pub gross_usd: Option<f64>,
    pub gross_rub: Option<f64>,
    pub net_usd: Option<f64>,
    pub net_rub: Option<f64>,
    /// The rate used: the one kept when the card was paid, otherwise today's.
    pub rate: Option<f64>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Card {
    pub id: i64,
    pub list_id: i64,
    pub position: i64,
    pub title: String,
    pub description: String,
    pub labels: Vec<String>,
    pub due: Option<NaiveDate>,
    pub checklist: Vec<ChecklistItem>,
    pub salary: Option<Salary>,
    pub paid_at: Option<NaiveDateTime>,
    pub locked_rate: Option<f64>,
    pub done_at: Option<NaiveDateTime>,
    pub created_at: NaiveDateTime,
    pub money: Option<Money>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ListView {
    #[serde(flatten)]
    pub list: TaskList,
    pub cards: Vec<Card>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RateInfo {
    /// Rubles for one dollar.
    pub value: f64,
    /// The day the bank set this rate for; `None` for a rate typed in by hand.
    pub date: Option<NaiveDate>,
    pub manual: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MonthEarnings {
    /// "2026-11".
    pub month: String,
    pub net_usd: f64,
    pub net_rub: f64,
    pub cards: usize,
    /// The part of `net_usd` not marked paid yet.
    pub unpaid_usd: f64,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TasksView {
    pub boards: Vec<TaskBoard>,
    pub board: Option<TaskBoard>,
    pub lists: Vec<ListView>,
    pub rate: Option<RateInfo>,
    /// Earnings across every board, this month first.
    pub months: Vec<MonthEarnings>,
}

// ---------- money ----------

/// Works out a salary in both currencies after the commission.
pub fn money(salary: &Salary, board_commission: f64, rate: Option<f64>) -> Money {
    let commission = salary.commission.unwrap_or(board_commission).clamp(0.0, 100.0);
    let keep = 1.0 - commission / 100.0;
    let rate = rate.filter(|r| r.is_finite() && *r > 0.0);
    let (gross_usd, gross_rub) = match salary.currency {
        Currency::Usd => (Some(salary.amount), rate.map(|r| salary.amount * r)),
        Currency::Rub => (rate.map(|r| salary.amount / r), Some(salary.amount)),
    };
    Money { commission, gross_usd, gross_rub, net_usd: gross_usd.map(|v| v * keep), net_rub: gross_rub.map(|v| v * keep), rate }
}

fn check_commission(c: f64) -> Result<f64> {
    if !c.is_finite() || !(0.0..=100.0).contains(&c) {
        return Err(Error::invalid("The commission has to be between 0 and 100%."));
    }
    Ok(c)
}

fn check_salary(s: &Option<Salary>) -> Result<()> {
    if let Some(s) = s {
        if !s.amount.is_finite() || s.amount <= 0.0 {
            return Err(Error::invalid("Enter how much the task pays."));
        }
        if s.amount >= MAX_AMOUNT {
            return Err(Error::invalid("That amount is too large."));
        }
        if let Some(c) = s.commission {
            check_commission(c)?;
        }
    }
    Ok(())
}

// ---------- exchange rate ----------

/// Reads the dollar rate and its date from the bank's daily XML file.
pub fn parse_cbr_daily(bytes: &[u8]) -> Result<(f64, NaiveDate)> {
    let utf8 = String::from_utf8_lossy(bytes);
    let text = if utf8.contains("windows-1251") { encoding_rs::WINDOWS_1251.decode(bytes).0.into_owned() } else { utf8.into_owned() };
    let bad = || Error::invalid("The exchange rate file couldn't be read.");
    let date = text
        .split("Date=\"")
        .nth(1)
        .and_then(|s| s.split('"').next())
        .and_then(|d| NaiveDate::parse_from_str(d, "%d.%m.%Y").ok())
        .ok_or_else(bad)?;
    let tag = |chunk: &str, name: &str| -> Option<String> {
        let open = format!("<{name}>");
        let close = format!("</{name}>");
        let start = chunk.find(&open)? + open.len();
        let end = chunk[start..].find(&close)? + start;
        Some(chunk[start..end].trim().to_string())
    };
    let usd = text.split("<Valute").find(|c| tag(c, "CharCode").as_deref() == Some("USD")).ok_or_else(bad)?;
    let nominal: f64 = tag(usd, "Nominal").and_then(|n| n.parse().ok()).unwrap_or(1.0);
    let value: f64 = tag(usd, "Value").and_then(|v| v.replace(',', ".").parse().ok()).ok_or_else(bad)?;
    if !(value > 0.0 && nominal > 0.0) {
        return Err(bad());
    }
    Ok((value / nominal, date))
}

impl Store {
    /// The rate cards use: one typed in by hand wins over the bank's.
    pub fn usd_rub_rate(&self) -> Result<Option<RateInfo>> {
        if let Some(v) = self.setting(MANUAL_RATE_KEY)?.and_then(|s| s.parse::<f64>().ok()).filter(|v| *v > 0.0) {
            return Ok(Some(RateInfo { value: v, date: None, manual: true }));
        }
        Ok(self.setting(RATE_KEY)?.and_then(|s| serde_json::from_str::<RateInfo>(&s).ok()))
    }

    /// The bank's rate as last downloaded, whether or not a manual one is set.
    pub fn bank_rate(&self) -> Result<Option<RateInfo>> {
        Ok(self.setting(RATE_KEY)?.and_then(|s| serde_json::from_str::<RateInfo>(&s).ok()))
    }

    pub fn set_bank_rate(&self, value: f64, date: NaiveDate) -> Result<()> {
        let info = RateInfo { value, date: Some(date), manual: false };
        self.set_setting(RATE_KEY, &serde_json::to_string(&info).expect("serialisable"))
    }

    /// Sets a rate by hand, or goes back to the bank's with `None`.
    pub fn set_manual_rate(&self, value: Option<f64>) -> Result<()> {
        match value {
            Some(v) if v.is_finite() && v > 0.0 && v < 100_000.0 => self.set_setting(MANUAL_RATE_KEY, &v.to_string()),
            Some(_) => Err(Error::invalid("Enter a rate above zero.")),
            None => {
                self.conn.execute("DELETE FROM settings WHERE key = ?1", [MANUAL_RATE_KEY])?;
                Ok(())
            }
        }
    }

    /// Whether the bank's rate should be downloaded again: it changes once a day.
    pub fn rate_is_stale(&self, today: NaiveDate) -> Result<bool> {
        Ok(self.bank_rate()?.and_then(|r| r.date).is_none_or(|d| d < today))
    }
}

/// Downloads today's rate and keeps it. Tries the bank first, then its mirror.
pub fn refresh_rate(store: &std::sync::Mutex<Store>, fetcher: &dyn Fetch) -> Result<RateInfo> {
    let mut last = Error::invalid("Couldn't get the exchange rate.");
    for url in RATE_SOURCES {
        match fetcher.get(url).and_then(|b| parse_cbr_daily(&b)) {
            Ok((value, date)) => {
                let s = store.lock().map_err(|_| Error::invalid("The data store is unavailable."))?;
                s.set_bank_rate(value, date)?;
                return Ok(RateInfo { value, date: Some(date), manual: false });
            }
            Err(e) => last = e,
        }
    }
    Err(last)
}

// ---------- storage ----------

fn stamp(s: String) -> rusqlite::Result<NaiveDateTime> {
    NaiveDateTime::parse_from_str(&s, STAMP)
        .map_err(|e| rusqlite::Error::FromSqlConversionFailure(0, rusqlite::types::Type::Text, Box::new(e)))
}
fn ts(t: NaiveDateTime) -> String {
    t.format(STAMP).to_string()
}
fn json<T: serde::de::DeserializeOwned + Default>(s: String) -> T {
    serde_json::from_str(&s).unwrap_or_default()
}

fn clean_text(s: &str, what: &str, max: usize) -> Result<String> {
    let s = s.split_whitespace().collect::<Vec<_>>().join(" ");
    if s.is_empty() {
        return Err(Error::invalid(format!("Give the {what} a name.")));
    }
    if s.chars().count() > max {
        return Err(Error::invalid("That name is too long."));
    }
    Ok(s)
}

/// The labels a new board starts with.
pub fn default_labels() -> Vec<Label> {
    [
        ("design", "Design", "#6f8fc4"),
        ("web", "Web", "#a07fc2"),
        ("study", "Study", "#c9a24a"),
        ("code", "Code", "#5b9f74"),
        ("urgent", "Urgent", "#e0735d"),
        ("personal", "Personal", "#7aa9b3"),
    ]
    .into_iter()
    .map(|(id, name, color)| Label { id: id.into(), name: name.into(), color: color.into() })
    .collect()
}

const CARD_COLS: &str = "c.id, c.list_id, c.position, c.title, c.description, c.labels, c.due, c.checklist, c.salary_amount, c.salary_currency, c.commission, c.paid_at, c.locked_rate, c.done_at, c.created_at, b.commission";

fn card_row(r: &Row, rate: Option<f64>) -> rusqlite::Result<Card> {
    let amount: Option<f64> = r.get(8)?;
    let currency: Option<String> = r.get(9)?;
    let salary = match (amount, currency.as_deref().and_then(Currency::parse)) {
        (Some(amount), Some(currency)) => Some(Salary { amount, currency, commission: r.get(10)? }),
        _ => None,
    };
    let paid_at: Option<NaiveDateTime> = r.get::<_, Option<String>>(11)?.map(stamp).transpose()?;
    let locked_rate: Option<f64> = r.get(12)?;
    let board_commission: f64 = r.get(15)?;
    let used_rate = if paid_at.is_some() { locked_rate.or(rate) } else { rate };
    Ok(Card {
        id: r.get(0)?,
        list_id: r.get(1)?,
        position: r.get(2)?,
        title: r.get(3)?,
        description: r.get(4)?,
        labels: json(r.get(5)?),
        due: r.get::<_, Option<String>>(6)?.and_then(|d| NaiveDate::parse_from_str(&d, DATE).ok()),
        checklist: json(r.get(7)?),
        money: salary.as_ref().map(|s| money(s, board_commission, used_rate)),
        salary,
        paid_at,
        locked_rate,
        done_at: r.get::<_, Option<String>>(13)?.map(stamp).transpose()?,
        created_at: stamp(r.get(14)?)?,
    })
}

impl Store {
    // ---- boards ----

    pub fn task_boards(&self) -> Result<Vec<TaskBoard>> {
        let mut stmt = self.conn.prepare(
            "SELECT b.id, b.name, b.color, b.commission, b.labels,
                    (SELECT COUNT(*) FROM task_cards c JOIN task_lists l ON l.id = c.list_id WHERE l.board_id = b.id)
             FROM task_boards b ORDER BY b.position, b.id",
        )?;
        let rows = stmt
            .query_map([], |r| {
                Ok(TaskBoard {
                    id: r.get(0)?,
                    name: r.get(1)?,
                    color: r.get(2)?,
                    commission: r.get(3)?,
                    labels: json(r.get(4)?),
                    cards: r.get::<_, i64>(5)? as usize,
                })
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    }

    fn check_board(input: &BoardInput) -> Result<(String, String)> {
        let name = clean_text(&input.name, "board", 80)?;
        check_commission(input.commission)?;
        if input.labels.len() > 30 {
            return Err(Error::invalid("A board can have up to 30 labels."));
        }
        let labels = serde_json::to_string(&input.labels).expect("serialisable");
        Ok((name, labels))
    }

    /// Makes a board with the usual lists: To do, In progress, Review and Done.
    pub fn create_task_board(&self, input: &BoardInput, now: NaiveDateTime) -> Result<i64> {
        let (name, labels) = Self::check_board(input)?;
        let tx = self.conn.unchecked_transaction()?;
        let pos: i64 = tx.query_row("SELECT COALESCE(MAX(position), 0) + 1 FROM task_boards", [], |r| r.get(0))?;
        tx.execute(
            "INSERT INTO task_boards(name, color, commission, labels, position, created_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![name, input.color, input.commission, labels, pos, ts(now)],
        )?;
        let id = tx.last_insert_rowid();
        for (i, (list, done)) in [("To do", false), ("In progress", false), ("Review", false), ("Done", true)].into_iter().enumerate() {
            tx.execute(
                "INSERT INTO task_lists(board_id, name, is_done, position) VALUES (?1, ?2, ?3, ?4)",
                params![id, list, done, i as i64],
            )?;
        }
        tx.commit()?;
        Ok(id)
    }

    pub fn update_task_board(&self, id: i64, input: &BoardInput) -> Result<()> {
        let (name, labels) = Self::check_board(input)?;
        let n = self.conn.execute(
            "UPDATE task_boards SET name = ?2, color = ?3, commission = ?4, labels = ?5 WHERE id = ?1",
            params![id, name, input.color, input.commission, labels],
        )?;
        if n == 0 {
            return Err(Error::NotFound("board".into()));
        }
        Ok(())
    }

    pub fn delete_task_board(&self, id: i64) -> Result<()> {
        self.conn.execute("DELETE FROM task_boards WHERE id = ?1", [id])?;
        Ok(())
    }

    // ---- lists ----

    pub fn task_lists(&self, board_id: i64) -> Result<Vec<TaskList>> {
        let mut stmt =
            self.conn.prepare("SELECT id, board_id, name, is_done, position FROM task_lists WHERE board_id = ?1 ORDER BY position, id")?;
        let rows = stmt
            .query_map([board_id], |r| {
                Ok(TaskList { id: r.get(0)?, board_id: r.get(1)?, name: r.get(2)?, is_done: r.get(3)?, position: r.get(4)? })
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    }

    fn task_list(&self, id: i64) -> Result<TaskList> {
        self.conn
            .query_row("SELECT id, board_id, name, is_done, position FROM task_lists WHERE id = ?1", [id], |r| {
                Ok(TaskList { id: r.get(0)?, board_id: r.get(1)?, name: r.get(2)?, is_done: r.get(3)?, position: r.get(4)? })
            })
            .optional()?
            .ok_or_else(|| Error::NotFound("list".into()))
    }

    pub fn add_task_list(&self, board_id: i64, name: &str, is_done: bool) -> Result<i64> {
        let name = clean_text(name, "list", 60)?;
        let pos: i64 =
            self.conn.query_row("SELECT COALESCE(MAX(position), -1) + 1 FROM task_lists WHERE board_id = ?1", [board_id], |r| r.get(0))?;
        self.conn.execute(
            "INSERT INTO task_lists(board_id, name, is_done, position) VALUES (?1, ?2, ?3, ?4)",
            params![board_id, name, is_done, pos],
        )?;
        Ok(self.conn.last_insert_rowid())
    }

    /// Renames a list or changes whether it counts as done. Cards already in it are stamped
    /// done (or no longer done) to match.
    pub fn update_task_list(&self, id: i64, name: &str, is_done: bool, now: NaiveDateTime) -> Result<()> {
        let name = clean_text(name, "list", 60)?;
        let tx = self.conn.unchecked_transaction()?;
        if tx.execute("UPDATE task_lists SET name = ?2, is_done = ?3 WHERE id = ?1", params![id, name, is_done])? == 0 {
            return Err(Error::NotFound("list".into()));
        }
        if is_done {
            tx.execute("UPDATE task_cards SET done_at = ?2 WHERE list_id = ?1 AND done_at IS NULL", params![id, ts(now)])?;
        } else {
            tx.execute("UPDATE task_cards SET done_at = NULL WHERE list_id = ?1", [id])?;
        }
        tx.commit()?;
        Ok(())
    }

    pub fn delete_task_list(&self, id: i64) -> Result<()> {
        self.conn.execute("DELETE FROM task_lists WHERE id = ?1", [id])?;
        Ok(())
    }

    /// Puts a list at `index` among its board's lists.
    pub fn move_task_list(&self, id: i64, index: usize) -> Result<()> {
        let list = self.task_list(id)?;
        let mut ids: Vec<i64> = self.task_lists(list.board_id)?.into_iter().map(|l| l.id).filter(|x| *x != id).collect();
        ids.insert(index.min(ids.len()), id);
        let tx = self.conn.unchecked_transaction()?;
        for (i, lid) in ids.iter().enumerate() {
            tx.execute("UPDATE task_lists SET position = ?2 WHERE id = ?1", params![lid, i as i64])?;
        }
        tx.commit()?;
        Ok(())
    }

    // ---- cards ----

    pub fn task_cards(&self, list_id: i64, rate: Option<f64>) -> Result<Vec<Card>> {
        let sql = format!(
            "SELECT {CARD_COLS} FROM task_cards c JOIN task_lists l ON l.id = c.list_id JOIN task_boards b ON b.id = l.board_id
             WHERE c.list_id = ?1 ORDER BY c.position, c.id"
        );
        let mut stmt = self.conn.prepare(&sql)?;
        let rows = stmt.query_map([list_id], |r| card_row(r, rate))?.collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    }

    pub fn task_card(&self, id: i64) -> Result<Card> {
        let rate = self.usd_rub_rate()?.map(|r| r.value);
        let sql = format!(
            "SELECT {CARD_COLS} FROM task_cards c JOIN task_lists l ON l.id = c.list_id JOIN task_boards b ON b.id = l.board_id WHERE c.id = ?1"
        );
        self.conn.query_row(&sql, [id], |r| card_row(r, rate)).optional()?.ok_or_else(|| Error::NotFound("card".into()))
    }

    fn check_card(input: &CardInput) -> Result<String> {
        let title = input.title.trim().to_string();
        if title.is_empty() {
            return Err(Error::invalid("Give the card a title."));
        }
        if title.chars().count() > 500 || input.description.chars().count() > 20_000 || input.checklist.len() > 200 {
            return Err(Error::invalid("That card is too long."));
        }
        check_salary(&input.salary)?;
        Ok(title)
    }

    /// Adds a card at the bottom of a list.
    pub fn add_task_card(&self, list_id: i64, input: &CardInput, now: NaiveDateTime) -> Result<i64> {
        let title = Self::check_card(input)?;
        let list = self.task_list(list_id)?;
        let pos: i64 =
            self.conn.query_row("SELECT COALESCE(MAX(position), -1) + 1 FROM task_cards WHERE list_id = ?1", [list_id], |r| r.get(0))?;
        let s = input.salary.as_ref();
        self.conn.execute(
            "INSERT INTO task_cards(list_id, position, title, description, labels, due, checklist, salary_amount, salary_currency, commission, done_at, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?12)",
            params![
                list_id,
                pos,
                title,
                input.description,
                serde_json::to_string(&input.labels).expect("serialisable"),
                input.due.map(|d| d.format(DATE).to_string()),
                serde_json::to_string(&input.checklist).expect("serialisable"),
                s.map(|s| s.amount),
                s.map(|s| s.currency.as_str()),
                s.and_then(|s| s.commission),
                list.is_done.then(|| ts(now)),
                ts(now)
            ],
        )?;
        Ok(self.conn.last_insert_rowid())
    }

    pub fn update_task_card(&self, id: i64, input: &CardInput, now: NaiveDateTime) -> Result<()> {
        let title = Self::check_card(input)?;
        let s = input.salary.as_ref();
        let n = self.conn.execute(
            "UPDATE task_cards SET title = ?2, description = ?3, labels = ?4, due = ?5, checklist = ?6,
                salary_amount = ?7, salary_currency = ?8, commission = ?9, updated_at = ?10,
                paid_at = CASE WHEN ?7 IS NULL THEN NULL ELSE paid_at END,
                locked_rate = CASE WHEN ?7 IS NULL THEN NULL ELSE locked_rate END
             WHERE id = ?1",
            params![
                id,
                title,
                input.description,
                serde_json::to_string(&input.labels).expect("serialisable"),
                input.due.map(|d| d.format(DATE).to_string()),
                serde_json::to_string(&input.checklist).expect("serialisable"),
                s.map(|s| s.amount),
                s.map(|s| s.currency.as_str()),
                s.and_then(|s| s.commission),
                ts(now)
            ],
        )?;
        if n == 0 {
            return Err(Error::NotFound("card".into()));
        }
        Ok(())
    }

    pub fn delete_task_card(&self, id: i64) -> Result<()> {
        self.conn.execute("DELETE FROM task_cards WHERE id = ?1", [id])?;
        Ok(())
    }

    /// Puts a card at `index` in a list (possibly another one). Moving into a done list stamps
    /// the card as finished now; moving out of one clears that.
    pub fn move_task_card(&self, id: i64, list_id: i64, index: usize, now: NaiveDateTime) -> Result<()> {
        let target = self.task_list(list_id)?;
        let (from_list, done_at): (i64, Option<String>) = self
            .conn
            .query_row("SELECT list_id, done_at FROM task_cards WHERE id = ?1", [id], |r| Ok((r.get(0)?, r.get(1)?)))
            .optional()?
            .ok_or_else(|| Error::NotFound("card".into()))?;
        let from = self.task_list(from_list)?;
        if from.board_id != target.board_id {
            return Err(Error::invalid("Cards can only move within their board."));
        }
        let mut stmt = self.conn.prepare("SELECT id FROM task_cards WHERE list_id = ?1 AND id != ?2 ORDER BY position, id")?;
        let mut ids: Vec<i64> = stmt.query_map(params![list_id, id], |r| r.get(0))?.collect::<rusqlite::Result<_>>()?;
        ids.insert(index.min(ids.len()), id);
        let new_done = match (target.is_done, done_at) {
            (true, Some(d)) => Some(d),
            (true, None) => Some(ts(now)),
            (false, _) => None,
        };
        let tx = self.conn.unchecked_transaction()?;
        tx.execute("UPDATE task_cards SET list_id = ?2, done_at = ?3 WHERE id = ?1", params![id, list_id, new_done])?;
        for (i, cid) in ids.iter().enumerate() {
            tx.execute("UPDATE task_cards SET position = ?2 WHERE id = ?1", params![cid, i as i64])?;
        }
        tx.commit()?;
        Ok(())
    }

    /// Marks a card paid, keeping today's rate with it, or not paid.
    pub fn set_task_card_paid(&self, id: i64, paid: bool, now: NaiveDateTime) -> Result<()> {
        let n = if paid {
            let rate = self.usd_rub_rate()?.map(|r| r.value);
            self.conn.execute(
                "UPDATE task_cards SET paid_at = COALESCE(paid_at, ?2), locked_rate = COALESCE(locked_rate, ?3) WHERE id = ?1 AND salary_amount IS NOT NULL",
                params![id, ts(now), rate],
            )?
        } else {
            self.conn.execute("UPDATE task_cards SET paid_at = NULL, locked_rate = NULL WHERE id = ?1", [id])?
        };
        if n == 0 {
            return Err(Error::invalid("Only a card with a salary can be marked paid."));
        }
        Ok(())
    }

    // ---- earnings ----

    /// What was earned in each of the last few months, across every board: the salaries of
    /// cards finished (moved to a done list) in that month, after commission.
    pub fn monthly_earnings(&self, today: NaiveDate, rate: Option<f64>) -> Result<Vec<MonthEarnings>> {
        let months: Vec<NaiveDate> = (0..MONTHS_SHOWN as u32)
            .map(|back| {
                let total = today.year() * 12 + today.month0() as i32 - back as i32;
                NaiveDate::from_ymd_opt(total.div_euclid(12), total.rem_euclid(12) as u32 + 1, 1).expect("valid month")
            })
            .collect();
        let first = *months.last().expect("at least one month");
        let mut out: Vec<MonthEarnings> = months
            .iter()
            .map(|m| MonthEarnings { month: m.format("%Y-%m").to_string(), net_usd: 0.0, net_rub: 0.0, cards: 0, unpaid_usd: 0.0 })
            .collect();
        let sql = format!(
            "SELECT {CARD_COLS} FROM task_cards c JOIN task_lists l ON l.id = c.list_id JOIN task_boards b ON b.id = l.board_id
             WHERE c.done_at >= ?1 AND c.salary_amount IS NOT NULL"
        );
        let mut stmt = self.conn.prepare(&sql)?;
        let cards = stmt
            .query_map([ts(first.and_hms_opt(0, 0, 0).expect("midnight"))], |r| card_row(r, rate))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        for c in cards {
            let (Some(done), Some(m)) = (c.done_at, c.money.as_ref()) else { continue };
            let key = done.format("%Y-%m").to_string();
            let Some(slot) = out.iter_mut().find(|e| e.month == key) else { continue };
            slot.cards += 1;
            slot.net_usd += m.net_usd.unwrap_or(0.0);
            slot.net_rub += m.net_rub.unwrap_or(0.0);
            if c.paid_at.is_none() {
                slot.unpaid_usd += m.net_usd.unwrap_or(0.0);
            }
        }
        Ok(out)
    }
}

/// Everything the Tasks screen shows. Without a board id, the first board opens; with no
/// boards at all, one is made so there is somewhere to start.
pub fn tasks_view(store: &Store, board_id: Option<i64>, now: NaiveDateTime) -> Result<TasksView> {
    let mut boards = store.task_boards()?;
    if boards.is_empty() {
        let input =
            BoardInput { name: "My tasks".into(), color: "#8AA7D9".into(), commission: DEFAULT_COMMISSION, labels: default_labels() };
        store.create_task_board(&input, now)?;
        boards = store.task_boards()?;
    }
    let board = board_id.and_then(|id| boards.iter().find(|b| b.id == id)).or(boards.first()).cloned();
    let rate = store.usd_rub_rate()?;
    let r = rate.as_ref().map(|r| r.value);
    let mut lists = vec![];
    if let Some(b) = &board {
        for list in store.task_lists(b.id)? {
            let cards = store.task_cards(list.id, r)?;
            lists.push(ListView { list, cards });
        }
    }
    let months = store.monthly_earnings(now.date(), r)?;
    Ok(TasksView { boards, board, lists, rate, months })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(d: u32, m: u32) -> NaiveDateTime {
        NaiveDate::from_ymd_opt(2026, m, d).unwrap().and_hms_opt(12, 0, 0).unwrap()
    }
    fn close(a: Option<f64>, b: f64) -> bool {
        a.is_some_and(|a| (a - b).abs() < 1e-9)
    }
    fn card(title: &str, salary: Option<Salary>) -> CardInput {
        CardInput { title: title.into(), description: String::new(), labels: vec![], due: None, checklist: vec![], salary }
    }
    fn usd(amount: f64) -> Option<Salary> {
        Some(Salary { amount, currency: Currency::Usd, commission: None })
    }

    #[test]
    fn salary_is_converted_and_commission_taken_off() {
        let m = money(&Salary { amount: 320.0, currency: Currency::Usd, commission: None }, 10.0, Some(80.0));
        assert!(close(m.gross_usd, 320.0) && close(m.gross_rub, 25_600.0));
        assert!(close(m.net_usd, 288.0) && close(m.net_rub, 23_040.0));

        let m = money(&Salary { amount: 8000.0, currency: Currency::Rub, commission: None }, 10.0, Some(80.0));
        assert!(close(m.net_rub, 7200.0) && close(m.net_usd, 90.0));

        // A card's own commission wins over the board's.
        let m = money(&Salary { amount: 100.0, currency: Currency::Usd, commission: Some(0.0) }, 10.0, None);
        assert!(close(m.net_usd, 100.0));
        // Without a rate only the original currency is known.
        assert_eq!(m.net_rub, None);
    }

    #[test]
    fn bank_rate_file_is_read() {
        let xml = "<?xml version=\"1.0\" encoding=\"windows-1251\"?><ValCurs Date=\"14.11.2026\" name=\"Foreign Currency Market\">\
            <Valute ID=\"R01035\"><NumCode>826</NumCode><CharCode>GBP</CharCode><Nominal>1</Nominal><Name>x</Name><Value>104,5</Value></Valute>\
            <Valute ID=\"R01235\"><NumCode>840</NumCode><CharCode>USD</CharCode><Nominal>1</Nominal><Name>Доллар США</Name><Value>81,2345</Value></Valute>\
            <Valute ID=\"R01375\"><CharCode>CNY</CharCode><Nominal>10</Nominal><Value>112,0</Value></Valute></ValCurs>";
        let bytes = encoding_rs::WINDOWS_1251.encode(xml).0;
        let (rate, date) = parse_cbr_daily(&bytes).unwrap();
        assert!((rate - 81.2345).abs() < 1e-9);
        assert_eq!(date, NaiveDate::from_ymd_opt(2026, 11, 14).unwrap());
        assert!(parse_cbr_daily(b"<html>maintenance</html>").is_err());
    }

    #[test]
    fn manual_rate_wins_and_can_be_cleared() {
        let s = Store::open_in_memory().unwrap();
        assert_eq!(s.usd_rub_rate().unwrap(), None);
        assert!(s.rate_is_stale(at(16, 11).date()).unwrap());
        s.set_bank_rate(81.0, at(16, 11).date()).unwrap();
        assert!(!s.rate_is_stale(at(16, 11).date()).unwrap());
        s.set_manual_rate(Some(90.0)).unwrap();
        assert_eq!(s.usd_rub_rate().unwrap().unwrap().value, 90.0);
        assert!(s.set_manual_rate(Some(-1.0)).is_err());
        s.set_manual_rate(None).unwrap();
        assert_eq!(s.usd_rub_rate().unwrap().unwrap().value, 81.0);
    }

    #[test]
    fn first_view_makes_a_board_with_the_usual_lists() {
        let s = Store::open_in_memory().unwrap();
        let v = tasks_view(&s, None, at(16, 11)).unwrap();
        assert_eq!(v.boards.len(), 1);
        let names: Vec<_> = v.lists.iter().map(|l| (l.list.name.as_str(), l.list.is_done)).collect();
        assert_eq!(names, [("To do", false), ("In progress", false), ("Review", false), ("Done", true)]);
        assert_eq!(v.board.unwrap().commission, DEFAULT_COMMISSION);
        assert_eq!(v.months.len(), MONTHS_SHOWN);
        assert_eq!(v.months[0].month, "2026-11");
        assert_eq!(v.months[5].month, "2026-06");
    }

    #[test]
    fn moving_cards_reorders_and_stamps_done() {
        let s = Store::open_in_memory().unwrap();
        let v = tasks_view(&s, None, at(16, 11)).unwrap();
        let (todo, done) = (v.lists[0].list.id, v.lists[3].list.id);
        let a = s.add_task_card(todo, &card("A", usd(100.0)), at(1, 11)).unwrap();
        let b = s.add_task_card(todo, &card("B", None), at(1, 11)).unwrap();
        let c = s.add_task_card(todo, &card("C", None), at(1, 11)).unwrap();
        s.move_task_card(c, todo, 0, at(2, 11)).unwrap();
        let order: Vec<_> = s.task_cards(todo, None).unwrap().into_iter().map(|c| c.id).collect();
        assert_eq!(order, [c, a, b]);

        s.move_task_card(a, done, 0, at(10, 11)).unwrap();
        let moved = s.task_card(a).unwrap();
        assert_eq!(moved.list_id, done);
        assert_eq!(moved.done_at, Some(at(10, 11)));
        // Shuffling inside the done list keeps the original finishing time.
        s.move_task_card(a, done, 5, at(12, 11)).unwrap();
        assert_eq!(s.task_card(a).unwrap().done_at, Some(at(10, 11)));
        // Back to work clears it.
        s.move_task_card(a, todo, 1, at(13, 11)).unwrap();
        assert_eq!(s.task_card(a).unwrap().done_at, None);

        // Other boards' lists are off limits.
        let other = s
            .create_task_board(&BoardInput { name: "Other".into(), color: "#fff".into(), commission: 5.0, labels: vec![] }, at(1, 11))
            .unwrap();
        let other_list = s.task_lists(other).unwrap()[0].id;
        assert!(s.move_task_card(a, other_list, 0, at(13, 11)).is_err());
    }

    #[test]
    fn marking_a_list_done_stamps_its_cards() {
        let s = Store::open_in_memory().unwrap();
        let v = tasks_view(&s, None, at(16, 11)).unwrap();
        let review = v.lists[2].list.id;
        let a = s.add_task_card(review, &card("A", usd(50.0)), at(1, 11)).unwrap();
        s.update_task_list(review, "Shipped", true, at(5, 11)).unwrap();
        assert_eq!(s.task_card(a).unwrap().done_at, Some(at(5, 11)));
        s.update_task_list(review, "Review", false, at(6, 11)).unwrap();
        assert_eq!(s.task_card(a).unwrap().done_at, None);
        // Cards added straight to a done list count as finished when added.
        let done = v.lists[3].list.id;
        let b = s.add_task_card(done, &card("B", None), at(7, 11)).unwrap();
        assert_eq!(s.task_card(b).unwrap().done_at, Some(at(7, 11)));
    }

    #[test]
    fn paid_cards_keep_their_rate() {
        let s = Store::open_in_memory().unwrap();
        let v = tasks_view(&s, None, at(16, 11)).unwrap();
        let todo = v.lists[0].list.id;
        s.set_bank_rate(80.0, at(10, 11).date()).unwrap();
        let a = s.add_task_card(todo, &card("A", usd(100.0)), at(1, 11)).unwrap();
        let plain = s.add_task_card(todo, &card("No pay", None), at(1, 11)).unwrap();
        assert!(s.set_task_card_paid(plain, true, at(10, 11)).is_err());
        s.set_task_card_paid(a, true, at(10, 11)).unwrap();
        s.set_bank_rate(95.0, at(11, 11).date()).unwrap();
        let c = s.task_card(a).unwrap();
        assert_eq!(c.locked_rate, Some(80.0));
        assert!(close(c.money.unwrap().net_rub, 7200.0));
        // Unpaid again: today's rate applies.
        s.set_task_card_paid(a, false, at(12, 11)).unwrap();
        assert!(close(s.task_card(a).unwrap().money.unwrap().net_rub, 100.0 * 0.9 * 95.0));
        // Removing the salary also clears the payment.
        s.set_task_card_paid(a, true, at(12, 11)).unwrap();
        s.update_task_card(a, &card("A", None), at(13, 11)).unwrap();
        let c = s.task_card(a).unwrap();
        assert_eq!((c.paid_at, c.locked_rate, c.money), (None, None, None));
    }

    #[test]
    fn earnings_add_up_by_month_of_finishing() {
        let s = Store::open_in_memory().unwrap();
        let v = tasks_view(&s, None, at(16, 11)).unwrap();
        let (todo, done) = (v.lists[0].list.id, v.lists[3].list.id);
        let rate = Some(80.0);
        let oct = s.add_task_card(todo, &card("Oct", usd(200.0)), at(1, 10)).unwrap();
        s.move_task_card(oct, done, 0, at(20, 10)).unwrap();
        s.set_bank_rate(80.0, at(20, 10).date()).unwrap();
        s.set_task_card_paid(oct, true, at(21, 10)).unwrap();
        let nov = s
            .add_task_card(todo, &card("Nov", Some(Salary { amount: 8000.0, currency: Currency::Rub, commission: None })), at(1, 11))
            .unwrap();
        s.move_task_card(nov, done, 0, at(3, 11)).unwrap();
        s.add_task_card(todo, &card("Not finished", usd(500.0)), at(1, 11)).unwrap();
        let old = s.add_task_card(todo, &card("Too old", usd(999.0)), at(1, 3)).unwrap();
        s.move_task_card(old, done, 0, at(2, 3)).unwrap();

        let m = s.monthly_earnings(at(16, 11).date(), rate).unwrap();
        assert_eq!(m[0].month, "2026-11");
        assert_eq!(m[0].cards, 1);
        assert!((m[0].net_usd - 90.0).abs() < 1e-9 && (m[0].net_rub - 7200.0).abs() < 1e-9);
        assert!((m[0].unpaid_usd - 90.0).abs() < 1e-9);
        assert_eq!(m[1].month, "2026-10");
        assert!((m[1].net_usd - 180.0).abs() < 1e-9 && m[1].unpaid_usd == 0.0);
        assert_eq!(m.iter().map(|e| e.cards).sum::<usize>(), 2);
    }

    #[test]
    fn months_roll_over_the_year() {
        let s = Store::open_in_memory().unwrap();
        let m = s.monthly_earnings(NaiveDate::from_ymd_opt(2027, 2, 3).unwrap(), None).unwrap();
        let names: Vec<_> = m.iter().map(|e| e.month.as_str()).collect();
        assert_eq!(names, ["2027-02", "2027-01", "2026-12", "2026-11", "2026-10", "2026-09"]);
    }

    #[test]
    fn bad_input_is_refused() {
        let s = Store::open_in_memory().unwrap();
        let v = tasks_view(&s, None, at(16, 11)).unwrap();
        let todo = v.lists[0].list.id;
        assert!(s.add_task_card(todo, &card("  ", None), at(1, 11)).is_err());
        assert!(s.add_task_card(todo, &card("x", usd(0.0)), at(1, 11)).is_err());
        assert!(s
            .add_task_card(todo, &card("x", Some(Salary { amount: 5.0, currency: Currency::Usd, commission: Some(120.0) })), at(1, 11))
            .is_err());
        let bad = BoardInput { name: "B".into(), color: "#fff".into(), commission: -1.0, labels: vec![] };
        assert!(s.create_task_board(&bad, at(1, 11)).is_err());
    }

    #[test]
    fn deleting_a_board_removes_its_lists_and_cards() {
        let s = Store::open_in_memory().unwrap();
        let v = tasks_view(&s, None, at(16, 11)).unwrap();
        let id = v.board.unwrap().id;
        s.add_task_card(v.lists[0].list.id, &card("A", None), at(1, 11)).unwrap();
        s.delete_task_board(id).unwrap();
        let n: i64 = s.conn.query_row("SELECT COUNT(*) FROM task_cards", [], |r| r.get(0)).unwrap();
        assert_eq!(n, 0);
    }
}
