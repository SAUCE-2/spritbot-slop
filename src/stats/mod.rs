//! Monthly stats and leaderboards.
//!
//! Everything interesting is recorded as a row in `stat_events` (who, which rule, what kind of
//! event, an optional festiveness score / amount / text). Leaderboards ([`Board`]) are just
//! aggregations over those rows, so:
//!
//! - every rule automatically gets "most punished", "most/least festive", "best average" and
//!   "first blood" boards, as long as it calls [`crate::rules::RuleCx::record_judgement`] when Jev
//!   judges something (punishments are recorded for it by `punish`);
//! - a rule can record its own kinds of event with [`crate::rules::RuleCx::record`] and add boards
//!   for them in [`crate::rules::Rule::boards`].
//!
//! Chat, reactions and voice activity are recorded by [`activity`] for everyone the rules apply to.

pub mod activity;
pub mod boards;

use std::collections::HashMap;
use std::fmt::Write as _;

use anyhow::Result;
use chrono::{Datelike, Months, NaiveDate};
use poise::serenity_prelude::{GuildId, UserId};

pub use boards::{Agg, Board, Format, Judged, Metric, Order, Section};
pub use boards::{all_boards, global_boards, rule_boards};

use crate::app::Data;
use crate::db::{self, Db};
use crate::punish::format_duration;

/// Event kinds recorded by the bot itself. Rules may add their own (keep them unique per rule).
pub mod kind {
    // ---- chat ----
    /// A message (`amount` = words).
    pub const MESSAGE: &str = "message";
    /// An ALL CAPS message (`text`).
    pub const CAPS: &str = "caps";
    pub const QUESTION: &str = "question";
    /// Sent between midnight and 5am / 5am and 8am (bot timezone).
    pub const NIGHT_OWL: &str = "night_owl";
    pub const EARLY_BIRD: &str = "early_bird";
    /// Emoji in a message (`amount`).
    pub const EMOJI: &str = "emoji";
    pub const FESTIVE_EMOJI: &str = "festive_emoji";
    /// Said "merry christmas", "happy holidays", ...
    pub const MERRY: &str = "merry";
    pub const HO_HO: &str = "ho_ho";
    /// Said "humbug", "grinch", "hate christmas" (`text`).
    pub const HUMBUG: &str = "humbug";
    /// Mentions of "christmas"/"xmas" (`amount`).
    pub const CHRISTMAS: &str = "christmas";
    /// Images, GIFs, videos and stickers (`amount`).
    pub const MEDIA: &str = "media";
    pub const LINK: &str = "link";
    pub const EDIT: &str = "edit";
    // ---- social ----
    /// Pinged people (`amount` = how many).
    pub const PING_SENT: &str = "ping_sent";
    /// Got pinged.
    pub const PINGED: &str = "pinged";
    pub const REPLY: &str = "reply";
    /// Someone replied to their message.
    pub const REPLIED_TO: &str = "replied_to";
    pub const REACTION_GIVEN: &str = "reaction_given";
    pub const REACTION_RECEIVED: &str = "reaction_received";
    pub const FESTIVE_REACTION: &str = "festive_reaction";
    // ---- voice ----
    pub const VC_JOIN: &str = "vc_join";
    /// Time spent in VC, recorded on leaving (`amount` = seconds).
    pub const VC_TIME: &str = "vc_time";
    /// Left VC within a minute of joining.
    pub const VC_DRIVE_BY: &str = "vc_drive_by";
    /// Moved between voice channels.
    pub const VC_HOP: &str = "vc_hop";
    pub const VC_DEAFEN: &str = "vc_deafen";
    pub const VC_MUTE: &str = "vc_mute";
    pub const VC_STREAM: &str = "vc_stream";
    pub const VC_CAMERA: &str = "vc_camera";
    // ---- rules ----
    /// Jev judged something (`score`, and what they said in `text`).
    pub const JUDGED: &str = "judged";
    /// A rule punished someone (`amount` = timeout seconds, `text` = reason).
    pub const PUNISHED: &str = "punished";
    /// A timed-out user's message was deleted (`text` = what they tried to say).
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

/// Record several events, for several people.
pub async fn record_all(data: &Data, guild_id: GuildId, events: Vec<(UserId, Event)>) {
    for (user, ev) in events {
        record(data, guild_id, user, ev).await;
    }
}

// ---- periods ----

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Period {
    All,
    Month(NaiveDate), // first of the month
}

impl Period {
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

// ---- queries ----

/// One line of a board.
#[derive(Clone, Debug, PartialEq, sqlx::FromRow)]
pub struct Entry {
    pub user_id: i64,
    pub value: f64,
    /// How many events it's based on.
    pub n: i64,
    /// For quotes: what was said. For "most improved": the before → after averages.
    pub text: Option<String>,
    /// For quotes and per-day records: which day.
    pub day: Option<String>,
}

impl Entry {
    pub fn user(&self) -> u64 {
        self.user_id as u64
    }
}

/// A bound SQL value, so queries can be assembled from optional pieces.
enum Val {
    I(i64),
    F(f64),
    S(String),
}

/// Which events a query looks at.
struct Scope {
    guild: i64,
    pattern: String,
    rule: Option<&'static str>,
    user: Option<i64>,
}

impl Scope {
    fn sql(&self, kinds: &[&str], b: &mut Vec<Val>) -> String {
        let mut w = String::from("guild_id = ? AND day LIKE ?");
        b.push(Val::I(self.guild));
        b.push(Val::S(self.pattern.clone()));
        if let Some(r) = self.rule {
            w.push_str(" AND rule_id = ?");
            b.push(Val::S(r.into()));
        }
        if let Some(u) = self.user {
            w.push_str(" AND user_id = ?");
            b.push(Val::I(u));
        }
        let _ = write!(w, " AND kind IN ({})", vec!["?"; kinds.len()].join(", "));
        b.extend(kinds.iter().map(|k| Val::S((*k).into())));
        w
    }
}

fn judged_sql(j: Judged, b: &mut Vec<Val>) -> &'static str {
    let (sql, v) = match j {
        Judged::Any => return "",
        Judged::Festive => (" AND score >= ?", FESTIVE),
        Judged::NotFestive => (" AND score < ?", FESTIVE),
        Judged::AtLeast(x) => (" AND score >= ?", x),
        Judged::AtMost(x) => (" AND score <= ?", x),
    };
    b.push(Val::F(f64::from(v)));
    sql
}

async fn fetch<T>(db: &Db, sql: String, binds: Vec<Val>) -> Result<Vec<T>>
where
    T: for<'r> sqlx::FromRow<'r, sqlx::sqlite::SqliteRow> + Send + Unpin,
{
    // Only fixed fragments are formatted into `sql`; every value is bound.
    let mut q = sqlx::query_as::<_, T>(sqlx::AssertSqlSafe(sql));
    for v in binds {
        q = match v {
            Val::I(i) => q.bind(i),
            Val::F(f) => q.bind(f),
            Val::S(s) => q.bind(s),
        };
    }
    Ok(q.fetch_all(&db.0).await?)
}

/// Server-wide numbers for the top of the overview.
#[derive(Clone, Debug, Default, sqlx::FromRow)]
pub struct Totals {
    pub people: i64,
    pub messages: i64,
    pub words: f64,
    pub judged: i64,
    pub avg_score: Option<f64>,
    pub punishments: i64,
    pub timeout_secs: f64,
    pub vc_secs: f64,
    pub reactions: i64,
    pub greetings: i64,
    pub dodges: i64,
    pub pardons: i64,
    #[sqlx(skip)]
    pub festive_day: Option<(String, f64)>,
    #[sqlx(skip)]
    pub grinchiest_day: Option<(String, f64)>,
    #[sqlx(skip)]
    pub naughtiest_day: Option<(String, i64)>,
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
        let scope = Scope {
            guild: guild_id as i64,
            pattern: period.pattern(),
            rule: board.rule_id,
            user: user.map(|u| u as i64),
        };
        let lim = limit.unwrap_or(-1); // -1 = no limit in SQLite
        let mut b = Vec::new();
        let mut entries: Vec<Entry> = match &board.metric {
            Metric::Rank(r) => {
                let w = scope.sql(&r.kinds, &mut b);
                let mut w = w + judged_sql(r.filter, &mut b);
                if let Some(f) = r.agg.field() {
                    let _ = write!(w, " AND {} IS NOT NULL", f.column());
                }
                if r.clean {
                    w.push_str(
                        " AND user_id NOT IN (SELECT user_id FROM stat_events
                          WHERE guild_id = ? AND day LIKE ? AND kind = ?)",
                    );
                    b.push(Val::I(scope.guild));
                    b.push(Val::S(scope.pattern.clone()));
                    b.push(Val::S(kind::PUNISHED.into()));
                }
                // A total of zero (e.g. only disconnects on Hard Time) isn't worth a place.
                let nonzero = match r.agg {
                    Agg::Sum(_) | Agg::Range(_) => " AND value != 0",
                    _ => "",
                };
                let dir = if r.lowest { "ASC" } else { "DESC" };
                let sql = if r.per_day {
                    // SQLite returns `day` from the row holding the MAX/MIN.
                    let pick = if r.lowest { "MIN" } else { "MAX" };
                    format!(
                        "SELECT user_id, CAST({pick}(v) AS REAL) AS value, SUM(c) AS n,
                                NULL AS text, day
                         FROM (SELECT user_id, day, {agg} AS v, COUNT(*) AS c
                               FROM stat_events WHERE {w} GROUP BY user_id, day)
                         GROUP BY user_id
                         HAVING SUM(c) >= ? {nonzero}
                         ORDER BY value {dir}, n DESC, user_id
                         LIMIT ?",
                        agg = r.agg.sql(),
                    )
                } else {
                    format!(
                        "SELECT user_id, CAST({agg} AS REAL) AS value, COUNT(*) AS n,
                                NULL AS text, NULL AS day
                         FROM stat_events WHERE {w}
                         GROUP BY user_id
                         HAVING COUNT(*) >= ? {nonzero}
                         ORDER BY value {dir}, n DESC, user_id
                         LIMIT ?",
                        agg = r.agg.sql(),
                    )
                };
                b.push(Val::I(r.min_events));
                b.push(Val::I(lim));
                fetch(self, sql, b).await?
            }
            Metric::Quote {
                kind,
                order,
                filter,
            } => {
                let mut w = scope.sql(&[kind], &mut b);
                w.push_str(" AND text IS NOT NULL AND trim(text) != ''");
                if order.by_score() || *filter != Judged::Any {
                    w.push_str(" AND score IS NOT NULL");
                }
                w.push_str(judged_sql(*filter, &mut b));
                let (value, order) = match order {
                    Order::HighestScore => ("score", "score DESC, id"),
                    Order::LowestScore => ("score", "score ASC, id"),
                    Order::Longest => ("length(text)", "length(text) DESC, id"),
                    Order::First => ("COALESCE(score, 0)", "id ASC"),
                    Order::Latest => ("COALESCE(score, 0)", "id DESC"),
                };
                let sql = format!(
                    "SELECT user_id, CAST({value} AS REAL) AS value, 1 AS n, text, day
                     FROM stat_events WHERE {w} ORDER BY {order} LIMIT ?"
                );
                b.push(Val::I(lim));
                fetch(self, sql, b).await?
            }
            Metric::Streak { naughty } => {
                let w = scope.sql(&[kind::JUDGED, kind::PUNISHED], &mut b);
                let sql = format!(
                    "SELECT user_id, day,
                            MAX(kind = '{judged}' AND score >= {FESTIVE}) AS nice,
                            MAX(kind = '{punished}') AS naughty
                     FROM stat_events WHERE {w} GROUP BY user_id, day",
                    judged = kind::JUDGED,
                    punished = kind::PUNISHED,
                );
                let days: Vec<(i64, String, bool, bool)> = fetch(self, sql, b).await?;
                longest_streaks(
                    days.into_iter()
                        .filter(|(_, _, nice, bad)| if *naughty { *bad } else { *nice && !bad })
                        .filter_map(|(u, d, _, _)| Some((u, d.parse().ok()?))),
                )
            }
            Metric::Improved { min_events } => {
                let w = scope.sql(&[kind::JUDGED], &mut b);
                let sql = format!(
                    "SELECT user_id, score FROM stat_events
                     WHERE {w} AND score IS NOT NULL ORDER BY id"
                );
                let rows: Vec<(i64, f64)> = fetch(self, sql, b).await?;
                most_improved(rows, *min_events)
            }
        };
        if matches!(
            board.metric,
            Metric::Streak { .. } | Metric::Improved { .. }
        ) && lim >= 0
        {
            entries.truncate(lim as usize);
        }
        Ok(entries)
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

    pub async fn totals(&self, guild_id: u64, period: Period) -> Result<Totals> {
        use kind::*;
        let sum = |k: &str| format!("COALESCE(SUM(kind = '{k}'), 0)");
        let amount = |k: &str| {
            format!("CAST(COALESCE(SUM(CASE WHEN kind = '{k}' THEN amount END), 0) AS REAL)")
        };
        let sql = format!(
            "SELECT COUNT(DISTINCT user_id) AS people,
                    {messages} AS messages, {words} AS words,
                    {judged} AS judged,
                    AVG(CASE WHEN kind = '{JUDGED}' THEN score END) AS avg_score,
                    {punishments} AS punishments, {timeout} AS timeout_secs,
                    {vc} AS vc_secs, {reactions} AS reactions, {greetings} AS greetings,
                    COALESCE(SUM(kind IN ('{TIMEOUT_MESSAGE}', '{TIMEOUT_VC}')), 0) AS dodges,
                    {pardons} AS pardons
             FROM stat_events WHERE guild_id = ? AND day LIKE ?",
            messages = sum(MESSAGE),
            words = amount(MESSAGE),
            judged = sum(JUDGED),
            punishments = sum(PUNISHED),
            timeout = amount(PUNISHED),
            vc = amount(VC_TIME),
            reactions = sum(REACTION_GIVEN),
            greetings = sum(MERRY),
            pardons = sum(PARDONED),
        );
        let binds = || vec![Val::I(guild_id as i64), Val::S(period.pattern())];
        let mut t: Totals = fetch(self, sql, binds()).await?.pop().unwrap_or_default();
        let day_avg = |dir: &str| {
            format!(
                "SELECT day, AVG(score) FROM stat_events
                 WHERE guild_id = ? AND day LIKE ? AND kind = '{JUDGED}' AND score IS NOT NULL
                 GROUP BY day HAVING COUNT(*) >= 3 ORDER BY 2 {dir}, day LIMIT 1"
            )
        };
        t.festive_day = fetch(self, day_avg("DESC"), binds()).await?.pop();
        t.grinchiest_day = fetch(self, day_avg("ASC"), binds()).await?.pop();
        t.naughtiest_day = fetch(
            self,
            format!(
                "SELECT day, COUNT(*) FROM stat_events
                 WHERE guild_id = ? AND day LIKE ? AND kind = '{PUNISHED}'
                 GROUP BY day ORDER BY 2 DESC, day LIMIT 1"
            ),
            binds(),
        )
        .await?
        .pop();
        Ok(t)
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

/// Longest run of consecutive days per user, longest first. `days` = (user, a day that counts).
fn longest_streaks(days: impl IntoIterator<Item = (i64, NaiveDate)>) -> Vec<Entry> {
    let mut by_user: HashMap<i64, Vec<NaiveDate>> = HashMap::new();
    for (u, d) in days {
        by_user.entry(u).or_default().push(d);
    }
    let mut out: Vec<Entry> = by_user
        .into_iter()
        .map(|(user_id, mut ds)| {
            ds.sort();
            ds.dedup();
            let (mut best, mut run, mut end) = (0i64, 0i64, ds[0]);
            for (i, d) in ds.iter().enumerate() {
                run = if i > 0 && ds[i - 1].succ_opt() == Some(*d) {
                    run + 1
                } else {
                    1
                };
                if run > best {
                    (best, end) = (run, *d);
                }
            }
            Entry {
                user_id,
                value: best as f64,
                n: ds.len() as i64,
                text: None,
                day: Some(end.to_string()),
            }
        })
        .collect();
    sort_desc(&mut out);
    out
}

/// Rise in average score from the first half of each person's judgements to the second.
/// `rows` = (user, score) in time order. Only rises are listed.
fn most_improved(rows: Vec<(i64, f64)>, min_events: i64) -> Vec<Entry> {
    let mut by_user: HashMap<i64, Vec<f64>> = HashMap::new();
    for (u, s) in rows {
        by_user.entry(u).or_default().push(s);
    }
    let avg = |s: &[f64]| s.iter().sum::<f64>() / s.len() as f64;
    let mut out: Vec<Entry> = by_user
        .into_iter()
        .filter(|(_, s)| s.len() as i64 >= min_events.max(2))
        .filter_map(|(user_id, s)| {
            let (before, after) = s.split_at(s.len() / 2);
            let (before, after) = (avg(before), avg(after));
            (after > before).then(|| Entry {
                user_id,
                value: after - before,
                n: s.len() as i64,
                text: Some(format!("{before:.2} → {after:.2}")),
                day: None,
            })
        })
        .collect();
    sort_desc(&mut out);
    out
}

fn sort_desc(v: &mut [Entry]) {
    v.sort_by(|a, b| {
        b.value
            .total_cmp(&a.value)
            .then(b.n.cmp(&a.n))
            .then(a.user_id.cmp(&b.user_id))
    });
}

// ---- rendering ----

const MEDALS: [&str; 3] = ["🥇", "🥈", "🥉"];

fn place(i: usize) -> String {
    MEDALS
        .get(i)
        .map_or_else(|| format!("`{}.`", i + 1), |m| (*m).to_owned())
}

/// "2026-12-03" → "Dec 3".
pub fn short_day(day: &str) -> String {
    day.parse::<NaiveDate>()
        .map(|d| d.format("%b %-d").to_string())
        .unwrap_or_else(|_| day.to_owned())
}

fn plural(n: i64, one: &str, many: &str) -> String {
    format!("{n} {}", if n == 1 { one } else { many })
}

/// The value part of an entry, e.g. "5 punishments" or "0.93 avg over 12".
pub fn format_value(board: &Board, e: &Entry) -> String {
    let on_day = || {
        e.day
            .as_deref()
            .map(|d| format!(" ({})", short_day(d)))
            .unwrap_or_default()
    };
    match &board.metric {
        Metric::Rank(r) => {
            let mut s = match r.format {
                Format::Count(noun) => {
                    let v = e.value;
                    if v.fract() == 0.0 {
                        format!("{} {noun}", v as i64)
                    } else {
                        format!("{v:.1} {noun}")
                    }
                }
                Format::Duration => format_duration(e.value.max(0.0).round() as u64),
                Format::Score => match r.agg {
                    Agg::Avg(_) => format!("{:.2} avg over {}", e.value, e.n),
                    Agg::Range(_) => format!("{:.2} swing", e.value),
                    _ => format!("{:.2}", e.value),
                },
                Format::Days => plural(e.value as i64, "day", "days"),
                Format::Percent => format!("{:.0}% of {}", e.value * 100.0, e.n),
            };
            if r.per_day {
                s.push_str(&on_day());
            }
            s
        }
        Metric::Quote { order, .. } => {
            let text = quote_text(e.text.as_deref().unwrap_or_default());
            if order.by_score() {
                format!("“{text}” ({:.2})", e.value)
            } else {
                format!("“{text}”{}", on_day())
            }
        }
        Metric::Streak { .. } => {
            let mut s = plural(e.value as i64, "day", "days");
            if let Some(d) = &e.day {
                let _ = write!(s, " (to {})", short_day(d));
            }
            s
        }
        Metric::Improved { .. } => format!(
            "+{:.2} ({})",
            e.value,
            e.text.as_deref().unwrap_or_default()
        ),
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

pub fn format_totals(t: &Totals) -> String {
    if t.people == 0 {
        return "Nothing recorded yet. Get festive! 🎄".into();
    }
    let mut s = String::new();
    let mut line = |emoji: &str, text: String| {
        let _ = writeln!(s, "{emoji} {text}");
    };
    line(
        "👥",
        plural(t.people, "person took part", "people took part"),
    );
    line(
        "💬",
        format!(
            "{} · {} words",
            plural(t.messages, "message", "messages"),
            t.words as i64
        ),
    );
    if t.judged > 0 {
        line(
            "⚖️",
            format!(
                "{} · average festiveness **{:.2}**",
                plural(t.judged, "judgement", "judgements"),
                t.avg_score.unwrap_or_default()
            ),
        );
    }
    line(
        "🎅",
        format!(
            "{} · {} of timeouts",
            plural(t.punishments, "punishment", "punishments"),
            format_duration(t.timeout_secs as u64)
        ),
    );
    if t.dodges > 0 || t.pardons > 0 {
        line(
            "🚪",
            format!(
                "{} while timed out · {}",
                plural(t.dodges, "attempt to talk", "attempts to talk"),
                plural(t.pardons, "pardon", "pardons")
            ),
        );
    }
    line(
        "🎧",
        format!("{} spent in VC", format_duration(t.vc_secs as u64)),
    );
    line(
        "🎄",
        format!(
            "{} · {}",
            plural(t.greetings, "seasonal greeting", "seasonal greetings"),
            plural(t.reactions, "reaction", "reactions")
        ),
    );
    if let Some((d, v)) = &t.festive_day {
        line(
            "🌟",
            format!("Most festive day: **{}** ({v:.2} avg)", short_day(d)),
        );
    }
    if let Some((d, v)) = &t.grinchiest_day {
        line(
            "🦹",
            format!("Grinchiest day: **{}** ({v:.2} avg)", short_day(d)),
        );
    }
    if let Some((d, n)) = &t.naughtiest_day {
        line(
            "📉",
            format!(
                "Naughtiest day: **{}** ({})",
                short_day(d),
                plural(*n, "punishment", "punishments")
            ),
        );
    }
    s
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
mod tests;
