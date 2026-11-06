//! Monthly stats and leaderboards.
//!
//! Everything interesting is recorded as a row in `stat_events` (who, which rule, what kind of
//! event, an optional festiveness score / amount / text). Leaderboards ([`Board`]) are just
//! aggregations over those rows, so:
//!
//! - every rule automatically gets "most punished", "most festive moment" and "least festive
//!   moment" boards, as long as it calls [`crate::rules::RuleCx::record_judgement`] when Jev judges
//!   something (punishments are recorded for it by `punish`);
//! - a rule can record its own kinds of event with [`crate::rules::RuleCx::record`] and add boards
//!   for them in [`crate::rules::Rule::boards`].

use std::collections::HashMap;
use std::fmt::Write as _;

use anyhow::Result;
use chrono::{Datelike, Months, NaiveDate};
use poise::serenity_prelude::{GuildId, UserId};

use crate::app::Data;
use crate::db::{self, Db};
use crate::punish::format_duration;
use crate::rules::Rule;

/// Event kinds recorded by the bot itself. Rules may add their own (keep them unique per rule).
pub mod kind {
    /// A message from someone the rules apply to.
    pub const MESSAGE: &str = "message";
    /// Someone the rules apply to joined a voice channel.
    pub const VC_JOIN: &str = "vc_join";
    /// Jev judged something (`score`, and what they said in `text`).
    pub const JUDGED: &str = "judged";
    /// A rule punished someone (`amount` = timeout seconds, `text` = reason).
    pub const PUNISHED: &str = "punished";
    /// A timed-out user's message was deleted.
    pub const TIMEOUT_MESSAGE: &str = "timeout_message";
    /// A timed-out user was kicked from VC on joining.
    pub const TIMEOUT_VC: &str = "timeout_vc";
    /// An admin ended someone's timeout early.
    pub const PARDONED: &str = "pardoned";
}

/// Jev score at or above which something counts as festive (what the rules punish below).
pub const FESTIVE: f32 = 0.5;

/// One thing that happened, to be recorded with [`record`].
#[derive(Clone, Debug, Default)]
pub struct Event {
    pub rule_id: Option<&'static str>,
    pub kind: &'static str,
    pub score: Option<f32>,
    pub amount: Option<f64>,
    pub text: Option<String>,
}

impl Event {
    pub fn new(kind: &'static str) -> Self {
        Self {
            kind,
            ..Default::default()
        }
    }
    pub fn rule(mut self, rule_id: &'static str) -> Self {
        self.rule_id = Some(rule_id);
        self
    }
    pub fn score(mut self, score: f32) -> Self {
        self.score = Some(score);
        self
    }
    pub fn amount(mut self, amount: f64) -> Self {
        self.amount = Some(amount);
        self
    }
    pub fn text(mut self, text: impl Into<String>) -> Self {
        self.text = Some(text.into());
        self
    }
}

/// Record an event. Never fails: stats must not get in the way of the rules (errors are logged).
pub async fn record(data: &Data, guild_id: GuildId, user: UserId, ev: Event) {
    if let Err(e) = data
        .db
        .record_stat(guild_id.get(), user.get(), &ev, data.today())
        .await
    {
        tracing::warn!(%guild_id, %user, kind = ev.kind, "recording stat failed: {e:#}");
    }
}

// ---- periods ----

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Period {
    All,
    Month(NaiveDate), // first of the month
}

impl Period {
    pub fn this_month(today: NaiveDate) -> Self {
        Period::Month(today.with_day(1).unwrap())
    }

    /// Parse "2026-12", "december", "dec", "this", "last" or "all". Empty = this month.
    /// A bare month name means the most recent one (so "december" in January is last year's).
    pub fn parse(s: &str, today: NaiveDate) -> Option<Self> {
        let s = s.trim().to_lowercase();
        let this = today.with_day(1).unwrap();
        match s.as_str() {
            "" | "this" | "this month" | "now" => return Some(Period::Month(this)),
            "last" | "last month" => return Some(Period::Month(this - Months::new(1))),
            "all" | "all time" | "ever" => return Some(Period::All),
            _ => {}
        }
        if let Ok(d) = NaiveDate::parse_from_str(&format!("{s}-01"), "%Y-%m-%d") {
            return Some(Period::Month(d));
        }
        const MONTHS: [&str; 12] = [
            "january",
            "february",
            "march",
            "april",
            "may",
            "june",
            "july",
            "august",
            "september",
            "october",
            "november",
            "december",
        ];
        if s.len() < 3 {
            return None;
        }
        let m = MONTHS.iter().position(|name| name.starts_with(&s))? as u32 + 1;
        let year = if m > today.month() {
            today.year() - 1
        } else {
            today.year()
        };
        NaiveDate::from_ymd_opt(year, m, 1).map(Period::Month)
    }

    /// `LIKE` pattern matching this period's `day` values.
    fn pattern(&self) -> String {
        match self {
            Period::All => "%".into(),
            Period::Month(d) => d.format("%Y-%m-%%").to_string(),
        }
    }

    pub fn label(&self) -> String {
        match self {
            Period::All => "all time".into(),
            Period::Month(d) => d.format("%B %Y").to_string(),
        }
    }

    /// For file names.
    pub fn slug(&self) -> String {
        match self {
            Period::All => "all".into(),
            Period::Month(d) => d.format("%Y-%m").to_string(),
        }
    }
}

// ---- boards ----

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Field {
    Score,
    Amount,
}

impl Field {
    fn column(self) -> &'static str {
        match self {
            Field::Score => "score",
            Field::Amount => "amount",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Agg {
    Count,
    Sum(Field),
    Avg(Field),
    Max(Field),
    Min(Field),
}

impl Agg {
    fn sql(self) -> String {
        match self {
            Agg::Count => "COUNT(*)".into(),
            Agg::Sum(f) => format!("SUM({})", f.column()),
            Agg::Avg(f) => format!("AVG({})", f.column()),
            Agg::Max(f) => format!("MAX({})", f.column()),
            Agg::Min(f) => format!("MIN({})", f.column()),
        }
    }
    fn field(self) -> Option<Field> {
        match self {
            Agg::Count => None,
            Agg::Sum(f) | Agg::Avg(f) | Agg::Max(f) | Agg::Min(f) => Some(f),
        }
    }
}

/// How a board's values are shown.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Format {
    /// A count, with the plural noun to show after it ("punishments").
    Count(&'static str),
    /// Seconds.
    Duration,
    /// A Jev score.
    Score,
    Days,
}

/// Which judgements a quote board considers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Judged {
    Any,
    Festive,
    NotFestive,
}

#[derive(Clone, Debug)]
pub enum Metric {
    /// Rank people by an aggregate over their events of these kinds.
    Rank {
        kinds: Vec<&'static str>,
        agg: Agg,
        /// Lowest first (e.g. the Grinch: lowest average score).
        lowest: bool,
        /// Only rank people with at least this many events (stops one-offs topping averages).
        min_events: i64,
        format: Format,
    },
    /// Standout single events, with what was said: highest (or lowest) scores.
    Quote {
        kind: &'static str,
        lowest: bool,
        filter: Judged,
    },
    /// Longest run of consecutive days with a festive judgement and no punishment.
    Streak,
}

#[derive(Clone, Debug)]
pub struct Board {
    /// Unique id, e.g. `naughty`. A rule's boards get prefixed with the rule id (`vc_greeting:silent`).
    pub id: String,
    pub title: String,
    /// Only count events from this rule.
    pub rule_id: Option<&'static str>,
    pub metric: Metric,
}

impl Board {
    /// Most events of `kinds` (counted).
    pub fn most(id: &str, title: &str, kinds: &[&'static str], noun: &'static str) -> Self {
        Self::rank(id, title, kinds, Agg::Count, false, 1, Format::Count(noun))
    }

    pub fn rank(
        id: &str,
        title: &str,
        kinds: &[&'static str],
        agg: Agg,
        lowest: bool,
        min_events: i64,
        format: Format,
    ) -> Self {
        Self {
            id: id.into(),
            title: title.into(),
            rule_id: None,
            metric: Metric::Rank {
                kinds: kinds.to_vec(),
                agg,
                lowest,
                min_events,
                format,
            },
        }
    }

    pub fn quote(id: &str, title: &str, kind: &'static str, lowest: bool, filter: Judged) -> Self {
        Self {
            id: id.into(),
            title: title.into(),
            rule_id: None,
            metric: Metric::Quote {
                kind,
                lowest,
                filter,
            },
        }
    }

    fn for_rule(mut self, rule_id: &'static str) -> Self {
        self.id = format!("{rule_id}:{}", self.id);
        self.rule_id = Some(rule_id);
        self
    }

    pub fn is_quote(&self) -> bool {
        matches!(self.metric, Metric::Quote { .. })
    }
}

/// Boards that span all rules.
pub fn global_boards() -> Vec<Board> {
    use kind::*;
    vec![
        Board::most("naughty", "🎅 Naughty List", &[PUNISHED], "punishments"),
        Board::rank(
            "hard_time",
            "⏳ Hard Time (timeouts handed out)",
            &[PUNISHED],
            Agg::Sum(Field::Amount),
            false,
            1,
            Format::Duration,
        ),
        Board::rank(
            "nice",
            "😇 Nice List (most festive on average)",
            &[JUDGED],
            Agg::Avg(Field::Score),
            false,
            3,
            Format::Score,
        ),
        Board::rank(
            "grinch",
            "🦹 The Grinch (least festive on average)",
            &[JUDGED],
            Agg::Avg(Field::Score),
            true,
            3,
            Format::Score,
        ),
        Board {
            id: "streak".into(),
            title: "🔥 Festive Streak (days in a row without slipping)".into(),
            rule_id: None,
            metric: Metric::Streak,
        },
        Board::quote("best", "🌟 Most Festive Moment", JUDGED, false, Judged::Any),
        Board::quote(
            "worst",
            "💀 Least Festive Moment",
            JUDGED,
            true,
            Judged::Any,
        ),
        Board::quote(
            "close_call",
            "😅 Closest Call (barely festive enough)",
            JUDGED,
            true,
            Judged::Festive,
        ),
        Board::most(
            "cant_stay_away",
            "🚪 Can't Stay Away (tried to talk or join VC while timed out)",
            &[TIMEOUT_MESSAGE, TIMEOUT_VC],
            "attempts",
        ),
        Board::most(
            "pardoned",
            "🙏 Teacher's Pet (pardons)",
            &[PARDONED],
            "pardons",
        ),
        Board::most("chatterbox", "💬 Chatterbox", &[MESSAGE], "messages"),
        Board::most("vc_regular", "🎧 VC Regular", &[VC_JOIN], "joins"),
    ]
}

/// Boards every rule gets for free.
fn standard_rule_boards() -> Vec<Board> {
    use kind::*;
    vec![
        Board::most("naughty", "🎅 Most punished", &[PUNISHED], "punishments"),
        Board::quote("best", "🌟 Most festive", JUDGED, false, Judged::Any),
        Board::quote("worst", "💀 Least festive", JUDGED, true, Judged::Any),
    ]
}

/// Every board for one rule: the standard ones plus the rule's own.
pub fn rule_boards(rule: &dyn Rule) -> Vec<Board> {
    standard_rule_boards()
        .into_iter()
        .chain(rule.boards())
        .map(|b| b.for_rule(rule.id()))
        .collect()
}

/// Every board: global ones, then each rule's.
pub fn all_boards(rules: &[std::sync::Arc<dyn Rule>]) -> Vec<Board> {
    let mut out = global_boards();
    for r in rules {
        out.extend(rule_boards(r.as_ref()));
    }
    out
}

/// One line of a board.
#[derive(Clone, Debug, PartialEq)]
pub struct Entry {
    pub user_id: u64,
    pub value: f64,
    /// How many events it's based on.
    pub n: i64,
    /// For quotes: what was said.
    pub text: Option<String>,
}

#[derive(sqlx::FromRow)]
struct EntryRow {
    user_id: i64,
    value: f64,
    n: i64,
    text: Option<String>,
}

impl Db {
    pub async fn record_stat(
        &self,
        guild_id: u64,
        user_id: u64,
        ev: &Event,
        day: NaiveDate,
    ) -> Result<()> {
        sqlx::query(
            "INSERT INTO stat_events (guild_id, user_id, rule_id, kind, score, amount, text, day, created_at)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(guild_id as i64)
        .bind(user_id as i64)
        .bind(ev.rule_id)
        .bind(ev.kind)
        .bind(ev.score.map(f64::from))
        .bind(ev.amount)
        .bind(ev.text.as_deref())
        .bind(day.to_string())
        .bind(db::now())
        .execute(&self.0)
        .await?;
        Ok(())
    }

    /// A board's standings, best first. `user` limits it to one person's entries.
    pub async fn board(
        &self,
        guild_id: u64,
        period: Period,
        board: &Board,
        user: Option<u64>,
        limit: Option<i64>,
    ) -> Result<Vec<Entry>> {
        let limit = limit.unwrap_or(-1); // -1 = no limit in SQLite
        let user = user.map(|u| u as i64);
        let rows: Vec<EntryRow> = match &board.metric {
            Metric::Rank {
                kinds,
                agg,
                lowest,
                min_events,
                ..
            } => {
                // A total of zero (e.g. only disconnects on Hard Time) isn't worth a place.
                let nonzero = if matches!(agg, Agg::Sum(_)) {
                    "AND value != 0"
                } else {
                    ""
                };
                let not_null = agg
                    .field()
                    .map(|f| format!("AND {} IS NOT NULL", f.column()))
                    .unwrap_or_default();
                let sql = format!(
                    "SELECT user_id, CAST({agg} AS REAL) AS value, COUNT(*) AS n, NULL AS text
                     FROM stat_events
                     WHERE guild_id = ? AND day LIKE ? AND (? IS NULL OR rule_id = ?)
                       AND (? IS NULL OR user_id = ?) AND kind IN ({kinds}) {not_null}
                     GROUP BY user_id
                     HAVING COUNT(*) >= ? {nonzero}
                     ORDER BY value {dir}, n DESC, user_id
                     LIMIT ?",
                    agg = agg.sql(),
                    kinds = vec!["?"; kinds.len()].join(", "),
                    dir = if *lowest { "ASC" } else { "DESC" },
                );
                // Only fixed fragments are formatted in; all values are bound.
                let mut q = sqlx::query_as(sqlx::AssertSqlSafe(sql))
                    .bind(guild_id as i64)
                    .bind(period.pattern())
                    .bind(board.rule_id)
                    .bind(board.rule_id)
                    .bind(user)
                    .bind(user);
                for k in kinds {
                    q = q.bind(*k);
                }
                q.bind(*min_events).bind(limit).fetch_all(&self.0).await?
            }
            Metric::Quote {
                kind,
                lowest,
                filter,
            } => {
                let filter = match filter {
                    Judged::Any => "",
                    Judged::Festive => "AND score >= ?",
                    Judged::NotFestive => "AND score < ?",
                };
                let sql = format!(
                    "SELECT user_id, score AS value, 1 AS n, text
                     FROM stat_events
                     WHERE guild_id = ? AND day LIKE ? AND (? IS NULL OR rule_id = ?)
                       AND (? IS NULL OR user_id = ?) AND kind = ?
                       AND score IS NOT NULL AND text IS NOT NULL AND text != '' {filter}
                     ORDER BY score {dir}, id
                     LIMIT ?",
                    dir = if *lowest { "ASC" } else { "DESC" },
                );
                // Only fixed fragments are formatted in; all values are bound.
                let mut q = sqlx::query_as(sqlx::AssertSqlSafe(sql))
                    .bind(guild_id as i64)
                    .bind(period.pattern())
                    .bind(board.rule_id)
                    .bind(board.rule_id)
                    .bind(user)
                    .bind(user)
                    .bind(*kind);
                if !filter.is_empty() {
                    q = q.bind(f64::from(FESTIVE));
                }
                q.bind(limit).fetch_all(&self.0).await?
            }
            Metric::Streak => {
                let days: Vec<(i64, String, bool, bool)> = sqlx::query_as(
                    "SELECT user_id, day,
                            MAX(kind = ? AND score >= ?) AS nice,
                            MAX(kind = ?) AS naughty
                     FROM stat_events
                     WHERE guild_id = ? AND day LIKE ? AND (? IS NULL OR user_id = ?)
                       AND kind IN (?, ?)
                     GROUP BY user_id, day",
                )
                .bind(kind::JUDGED)
                .bind(f64::from(FESTIVE))
                .bind(kind::PUNISHED)
                .bind(guild_id as i64)
                .bind(period.pattern())
                .bind(user)
                .bind(user)
                .bind(kind::JUDGED)
                .bind(kind::PUNISHED)
                .fetch_all(&self.0)
                .await?;
                let mut entries = longest_streaks(
                    days.into_iter()
                        .filter(|(_, _, nice, naughty)| *nice && !naughty)
                        .filter_map(|(u, d, _, _)| Some((u as u64, d.parse().ok()?))),
                );
                if limit >= 0 {
                    entries.truncate(limit as usize);
                }
                return Ok(entries);
            }
        };
        Ok(rows
            .into_iter()
            .map(|r| Entry {
                user_id: r.user_id as u64,
                value: r.value,
                n: r.n,
                text: r.text,
            })
            .collect())
    }

    /// Does anything from this rule exist in the period?
    pub async fn rule_has_stats(
        &self,
        guild_id: u64,
        period: Period,
        rule_id: &str,
    ) -> Result<bool> {
        let n: (i64,) = sqlx::query_as(
            "SELECT EXISTS (SELECT 1 FROM stat_events WHERE guild_id = ? AND day LIKE ? AND rule_id = ?)",
        )
        .bind(guild_id as i64)
        .bind(period.pattern())
        .bind(rule_id)
        .fetch_one(&self.0)
        .await?;
        Ok(n.0 != 0)
    }

    /// Every raw event in the period, oldest first, for export.
    pub async fn stat_events(&self, guild_id: u64, period: Period) -> Result<Vec<StatRow>> {
        Ok(sqlx::query_as(
            "SELECT created_at, day, user_id, rule_id, kind, score, amount, text FROM stat_events
             WHERE guild_id = ? AND day LIKE ? ORDER BY id",
        )
        .bind(guild_id as i64)
        .bind(period.pattern())
        .fetch_all(&self.0)
        .await?)
    }

    /// Delete stats for the guild (or one user). Returns how many events were deleted.
    pub async fn wipe_stats(&self, guild_id: u64, user: Option<u64>) -> Result<u64> {
        Ok(sqlx::query(
            "DELETE FROM stat_events WHERE guild_id = ?1 AND (?2 IS NULL OR user_id = ?2)",
        )
        .bind(guild_id as i64)
        .bind(user.map(|u| u as i64))
        .execute(&self.0)
        .await?
        .rows_affected())
    }
}

#[derive(Clone, Debug, sqlx::FromRow)]
pub struct StatRow {
    pub created_at: String,
    pub day: String,
    pub user_id: i64,
    pub rule_id: Option<String>,
    pub kind: String,
    pub score: Option<f64>,
    pub amount: Option<f64>,
    pub text: Option<String>,
}

/// Longest run of consecutive days per user, longest first. `days` = (user, a good day).
fn longest_streaks(days: impl IntoIterator<Item = (u64, NaiveDate)>) -> Vec<Entry> {
    let mut by_user: HashMap<u64, Vec<NaiveDate>> = HashMap::new();
    for (u, d) in days {
        by_user.entry(u).or_default().push(d);
    }
    let mut out: Vec<Entry> = by_user
        .into_iter()
        .map(|(user_id, mut ds)| {
            ds.sort();
            ds.dedup();
            let (mut best, mut run) = (0i64, 0i64);
            for (i, d) in ds.iter().enumerate() {
                run = if i > 0 && ds[i - 1].succ_opt() == Some(*d) {
                    run + 1
                } else {
                    1
                };
                best = best.max(run);
            }
            Entry {
                user_id,
                value: best as f64,
                n: ds.len() as i64,
                text: None,
            }
        })
        .collect();
    out.sort_by(|a, b| {
        b.value
            .total_cmp(&a.value)
            .then(b.n.cmp(&a.n))
            .then(a.user_id.cmp(&b.user_id))
    });
    out
}

// ---- rendering ----

const MEDALS: [&str; 3] = ["🥇", "🥈", "🥉"];

fn place(i: usize) -> String {
    MEDALS
        .get(i)
        .map_or_else(|| format!("`{}.`", i + 1), |m| (*m).to_owned())
}

/// The value part of an entry, e.g. "5 punishments" or "0.93 avg over 12".
pub fn format_value(board: &Board, e: &Entry) -> String {
    match &board.metric {
        Metric::Rank { format, agg, .. } => match format {
            Format::Count(noun) => format!("{} {noun}", e.value as i64),
            Format::Duration => format_duration(e.value.max(0.0) as u64),
            Format::Score if matches!(agg, Agg::Avg(_)) => {
                format!("{:.2} avg over {}", e.value, e.n)
            }
            Format::Score => format!("{:.2}", e.value),
            Format::Days => format!("{} days", e.value as i64),
        },
        Metric::Quote { .. } => format!(
            "“{}” ({:.2})",
            quote_text(e.text.as_deref().unwrap_or_default()),
            e.value
        ),
        Metric::Streak => {
            let d = e.value as i64;
            format!("{d} day{}", if d == 1 { "" } else { "s" })
        }
    }
}

/// One line per entry, with medals.
pub fn format_entries(board: &Board, entries: &[Entry]) -> String {
    if entries.is_empty() {
        return "_nobody yet_".into();
    }
    let mut out = String::new();
    for (i, e) in entries.iter().enumerate() {
        let _ = writeln!(
            out,
            "{} <@{}> {}",
            place(i),
            e.user_id,
            format_value(board, e)
        );
    }
    out
}

/// Make user text safe to show inside an embed: one line, no markdown, not too long.
pub fn quote_text(s: &str) -> String {
    let mut out = String::new();
    for c in s.split_whitespace().collect::<Vec<_>>().join(" ").chars() {
        if out.chars().count() >= 120 {
            out.push('…');
            break;
        }
        if matches!(c, '*' | '_' | '~' | '`' | '|' | '>' | '\\' | '[' | ']') {
            out.push('\\');
        }
        out.push(c);
    }
    out
}

/// CSV of raw events (for building your own leaderboard). `name` resolves user ids to names.
pub fn to_csv(rows: &[StatRow], name: impl Fn(u64) -> String) -> String {
    fn cell(s: &str) -> String {
        if s.contains([',', '"', '\n', '\r']) {
            format!("\"{}\"", s.replace('"', "\"\""))
        } else {
            s.to_owned()
        }
    }
    let mut out = String::from("created_at,day,user_id,user,rule,kind,score,amount,text\n");
    for r in rows {
        let fields = [
            r.created_at.clone(),
            r.day.clone(),
            r.user_id.to_string(),
            name(r.user_id as u64),
            r.rule_id.clone().unwrap_or_default(),
            r.kind.clone(),
            r.score.map(|s| format!("{s:.3}")).unwrap_or_default(),
            r.amount.map(|a| a.to_string()).unwrap_or_default(),
            r.text.clone().unwrap_or_default(),
        ];
        let line: Vec<String> = fields.iter().map(|f| cell(f)).collect();
        out.push_str(&line.join(","));
        out.push('\n');
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn d(s: &str) -> NaiveDate {
        s.parse().unwrap()
    }

    async fn db() -> Db {
        Db::connect("sqlite::memory:").await.unwrap()
    }

    async fn rec(db: &Db, user: u64, ev: Event, day: &str) {
        db.record_stat(1, user, &ev, d(day)).await.unwrap();
    }

    fn find<'a>(boards: &'a [Board], id: &str) -> &'a Board {
        boards.iter().find(|b| b.id == id).unwrap()
    }

    #[test]
    fn periods() {
        let today = d("2027-01-10");
        assert_eq!(
            Period::parse("", today),
            Some(Period::Month(d("2027-01-01")))
        );
        assert_eq!(
            Period::parse("last", today),
            Some(Period::Month(d("2026-12-01")))
        );
        assert_eq!(
            Period::parse("dec", today),
            Some(Period::Month(d("2026-12-01")))
        );
        assert_eq!(
            Period::parse("January", today),
            Some(Period::Month(d("2027-01-01")))
        );
        assert_eq!(
            Period::parse("2025-11", today),
            Some(Period::Month(d("2025-11-01")))
        );
        assert_eq!(Period::parse("all", today), Some(Period::All));
        assert_eq!(Period::parse("ju", today), None);
        assert_eq!(Period::parse("nope", today), None);
        assert_eq!(Period::Month(d("2026-12-01")).pattern(), "2026-12-%");
        assert_eq!(Period::Month(d("2026-12-01")).label(), "December 2026");
    }

    #[test]
    fn streaks() {
        let s = longest_streaks([
            (1, d("2026-12-01")),
            (1, d("2026-12-02")),
            (1, d("2026-12-04")),
            (2, d("2026-12-01")),
            (2, d("2026-12-02")),
            (2, d("2026-12-03")),
            (2, d("2026-12-03")),
        ]);
        assert_eq!((s[0].user_id, s[0].value), (2, 3.0));
        assert_eq!((s[1].user_id, s[1].value), (1, 2.0));
    }

    #[test]
    fn text_is_escaped() {
        assert_eq!(quote_text("ho *ho*\n  ho"), "ho \\*ho\\* ho");
        assert!(quote_text(&"a".repeat(500)).ends_with('…'));
        let csv = to_csv(
            &[StatRow {
                created_at: "t".into(),
                day: "2026-12-01".into(),
                user_id: 7,
                rule_id: None,
                kind: "judged".into(),
                score: Some(0.5),
                amount: None,
                text: Some("say \"hi\", ok".into()),
            }],
            |_| "bob".into(),
        );
        assert!(csv.ends_with("7,bob,,judged,0.500,,\"say \"\"hi\"\", ok\"\n"));
    }

    #[tokio::test]
    async fn boards_rank_and_filter() {
        let db = db().await;
        let boards = all_boards(&crate::rules::registry());
        let dec = Period::Month(d("2026-12-01"));
        let judged = |s: f32, t: &str| {
            Event::new(kind::JUDGED)
                .rule("first_message")
                .score(s)
                .text(t)
        };
        let punished = |secs: f64| {
            Event::new(kind::PUNISHED)
                .rule("first_message")
                .amount(secs)
                .text("x")
        };
        rec(&db, 1, judged(0.9, "merry christmas"), "2026-12-01").await;
        rec(&db, 1, judged(0.8, "ho ho"), "2026-12-02").await;
        rec(&db, 1, judged(0.55, "meh xmas"), "2026-12-03").await;
        rec(&db, 2, judged(0.1, "bah humbug"), "2026-12-01").await;
        rec(&db, 2, judged(0.2, "no"), "2026-12-02").await;
        rec(&db, 2, judged(0.3, "nah"), "2026-12-03").await;
        rec(&db, 2, punished(300.0), "2026-12-01").await;
        rec(&db, 2, punished(60.0), "2026-12-02").await;
        rec(&db, 1, punished(0.0), "2026-12-03").await;
        // Another month and another guild don't count.
        rec(&db, 3, punished(60.0), "2026-11-30").await;
        db.record_stat(2, 3, &punished(60.0), d("2026-12-01"))
            .await
            .unwrap();

        let top = |id: &str| {
            let b = find(&boards, id).clone();
            let db = db.clone();
            async move { db.board(1, dec, &b, None, Some(10)).await.unwrap() }
        };
        let users = |es: Vec<Entry>| es.iter().map(|e| e.user_id).collect::<Vec<_>>();

        assert_eq!(users(top("naughty").await), [2, 1]);
        // Zero-second "timeouts" (disconnects) don't put you on Hard Time.
        let hard = top("hard_time").await;
        assert_eq!((users(hard.clone()), hard[0].value), (vec![2], 360.0));
        assert_eq!(users(top("nice").await), [1, 2]);
        assert_eq!(users(top("grinch").await), [2, 1]);
        let best = top("best").await;
        assert_eq!(best[0].text.as_deref(), Some("merry christmas"));
        assert_eq!(top("worst").await[0].text.as_deref(), Some("bah humbug"));
        assert_eq!(top("close_call").await[0].text.as_deref(), Some("meh xmas"));
        // User 1 was punished on the 3rd, so their streak is the 1st-2nd.
        let streak = top("streak").await;
        assert_eq!((users(streak.clone()), streak[0].value), (vec![1], 2.0));
        assert_eq!(users(top("first_message:naughty").await), [2, 1]);
        assert!(top("vc_greeting:naughty").await.is_empty());
        assert!(db.rule_has_stats(1, dec, "first_message").await.unwrap());
        assert!(!db.rule_has_stats(1, dec, "vc_greeting").await.unwrap());

        // Minimum sample size for averages.
        rec(&db, 4, judged(1.0, "perfect"), "2026-12-05").await;
        assert_eq!(users(top("nice").await), [1, 2]);

        // One user's view, and all time.
        let mine = db
            .board(1, dec, find(&boards, "naughty"), Some(1), None)
            .await
            .unwrap();
        assert_eq!(users(mine), [1]);
        let all = db
            .board(1, Period::All, find(&boards, "naughty"), None, None)
            .await
            .unwrap();
        assert_eq!(users(all), [2, 1, 3]);

        assert_eq!(db.stat_events(1, dec).await.unwrap().len(), 10);
        assert_eq!(db.wipe_stats(1, Some(2)).await.unwrap(), 5);
        assert_eq!(users(top("naughty").await), [1]);
    }

    #[test]
    fn board_ids_are_unique() {
        let boards = all_boards(&crate::rules::registry());
        let mut ids: Vec<_> = boards.iter().map(|b| b.id.as_str()).collect();
        let n = ids.len();
        ids.sort();
        ids.dedup();
        assert_eq!(ids.len(), n);
    }
}
