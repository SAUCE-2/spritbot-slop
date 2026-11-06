//! SQLite persistence. Snowflakes are stored as i64; times as RFC3339 text.

use std::str::FromStr;

use anyhow::Result;
use chrono::{DateTime, NaiveDate, Utc};
use serde::de::DeserializeOwned;
use sqlx::Row;
use sqlx::sqlite::{SqliteConnectOptions, SqlitePool, SqlitePoolOptions, SqliteRow};

use crate::punish::Punishment;

#[derive(Clone)]
pub struct Db(pub SqlitePool);

#[derive(Clone, Debug, Default, sqlx::FromRow)]
pub struct GuildConfig {
    pub enforced_role_id: Option<i64>,
    pub announce_channel_id: Option<i64>,
    pub log_channel_id: Option<i64>,
    pub last_setup_hash: Option<String>,
}

#[derive(Clone, Copy, Debug)]
pub enum GuildField {
    EnforcedRole,
    AnnounceChannel,
    LogChannel,
}

/// One rule's settings in one guild, parsed from its stored text once at load time.
#[derive(Clone, Debug)]
pub struct RuleRow {
    pub guild_id: i64,
    pub rule_id: String,
    pub enabled: bool,
    /// `None` = the rule's default punishment.
    pub punishment: Option<Punishment>,
    /// Pending scheduled activation, in the guild's timezone.
    pub start_date: Option<NaiveDate>,
    pub activated_at: Option<DateTime<Utc>>,
}

impl<'r> sqlx::FromRow<'r, SqliteRow> for RuleRow {
    fn from_row(row: &'r SqliteRow) -> sqlx::Result<Self> {
        Ok(Self {
            guild_id: row.try_get("guild_id")?,
            rule_id: row.try_get("rule_id")?,
            enabled: row.try_get("enabled")?,
            punishment: parse_column(row, "punishment", |s| serde_json::from_str(s).ok())?,
            start_date: parse_column(row, "start_date", |s| s.parse().ok())?,
            activated_at: parse_column(row, "activated_at", parse_time)?,
        })
    }
}

/// Read an optional text column and parse it. A value that doesn't parse is logged and
/// treated as unset, so one corrupt cell can't take every rule in the bot down with it.
fn parse_column<T>(
    row: &SqliteRow,
    column: &str,
    parse: impl FnOnce(&str) -> Option<T>,
) -> sqlx::Result<Option<T>> {
    let raw: Option<String> = row.try_get(column)?;
    Ok(raw.and_then(|text| {
        let parsed = parse(&text);
        if parsed.is_none() {
            tracing::warn!(column, value = %text, "ignoring unparseable stored value");
        }
        parsed
    }))
}

impl RuleRow {
    fn empty(guild_id: i64, rule_id: &str) -> Self {
        Self {
            guild_id,
            rule_id: rule_id.to_owned(),
            enabled: false,
            punishment: None,
            start_date: None,
            activated_at: None,
        }
    }
}

/// Punishment and timeout counts for `/status`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, sqlx::FromRow)]
pub struct PunishmentStats {
    pub total: i64,
    pub last_day: i64,
    pub active_timeouts: i64,
}

#[derive(Clone, Debug, sqlx::FromRow)]
pub struct PunishmentLog {
    pub rule_id: String,
    pub user_id: i64,
    pub action: String,
    pub created_at: String,
}

/// Fixed-width UTC timestamp, so stored times compare correctly as strings.
pub fn fmt_time(t: DateTime<Utc>) -> String {
    t.to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
}

pub fn now() -> String {
    fmt_time(Utc::now())
}

pub fn parse_time(s: &str) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(s)
        .ok()
        .map(|t| t.with_timezone(&Utc))
}

impl Db {
    pub async fn connect(url: &str) -> Result<Self> {
        if let Some(path) = url.strip_prefix("sqlite://")
            && let Some(dir) = std::path::Path::new(path).parent()
            && !dir.as_os_str().is_empty()
        {
            std::fs::create_dir_all(dir)?;
        }
        let opts = SqliteConnectOptions::from_str(url)?.create_if_missing(true);
        // Each in-memory connection is its own database, so keep just one.
        let max = if url.contains(":memory:") { 1 } else { 5 };
        let pool = SqlitePoolOptions::new()
            .max_connections(max)
            .connect_with(opts)
            .await?;
        sqlx::migrate!("./migrations").run(&pool).await?;
        Ok(Self(pool))
    }

    // ---- guild config ----

    pub async fn guild(&self, guild_id: u64) -> Result<GuildConfig> {
        let row = sqlx::query_as::<_, GuildConfig>("SELECT * FROM guild_config WHERE guild_id = ?")
            .bind(guild_id as i64)
            .fetch_optional(&self.0)
            .await?;
        Ok(row.unwrap_or_default())
    }

    pub async fn set_guild_field(
        &self,
        guild_id: u64,
        field: GuildField,
        value: Option<i64>,
    ) -> Result<()> {
        let sql = match field {
            GuildField::EnforcedRole => {
                "INSERT INTO guild_config (guild_id, enforced_role_id) VALUES (?1, ?2)
                 ON CONFLICT (guild_id) DO UPDATE SET enforced_role_id = excluded.enforced_role_id"
            }
            GuildField::AnnounceChannel => {
                "INSERT INTO guild_config (guild_id, announce_channel_id) VALUES (?1, ?2)
                 ON CONFLICT (guild_id) DO UPDATE SET announce_channel_id = excluded.announce_channel_id"
            }
            GuildField::LogChannel => {
                "INSERT INTO guild_config (guild_id, log_channel_id) VALUES (?1, ?2)
                 ON CONFLICT (guild_id) DO UPDATE SET log_channel_id = excluded.log_channel_id"
            }
        };
        sqlx::query(sql)
            .bind(guild_id as i64)
            .bind(value)
            .execute(&self.0)
            .await?;
        Ok(())
    }

    pub async fn set_setup_hash(&self, guild_id: u64, hash: Option<&str>) -> Result<()> {
        sqlx::query(
            "INSERT INTO guild_config (guild_id, last_setup_hash) VALUES (?1, ?2)
             ON CONFLICT (guild_id) DO UPDATE SET last_setup_hash = excluded.last_setup_hash",
        )
        .bind(guild_id as i64)
        .bind(hash)
        .execute(&self.0)
        .await?;
        Ok(())
    }

    // ---- rule config ----

    pub async fn rule(&self, guild_id: u64, rule_id: &str) -> Result<RuleRow> {
        let row = sqlx::query_as::<_, RuleRow>(
            "SELECT * FROM rule_config WHERE guild_id = ? AND rule_id = ?",
        )
        .bind(guild_id as i64)
        .bind(rule_id)
        .fetch_optional(&self.0)
        .await?;
        Ok(row.unwrap_or_else(|| RuleRow::empty(guild_id as i64, rule_id)))
    }

    pub async fn rules_for_guild(&self, guild_id: u64) -> Result<Vec<RuleRow>> {
        Ok(
            sqlx::query_as("SELECT * FROM rule_config WHERE guild_id = ?")
                .bind(guild_id as i64)
                .fetch_all(&self.0)
                .await?,
        )
    }

    pub async fn enabled_rules(&self, guild_id: u64) -> Result<Vec<RuleRow>> {
        Ok(
            sqlx::query_as("SELECT * FROM rule_config WHERE guild_id = ? AND enabled = 1")
                .bind(guild_id as i64)
                .fetch_all(&self.0)
                .await?,
        )
    }

    /// Rules whose scheduled start date has arrived (any guild).
    pub async fn due_scheduled(&self, today: NaiveDate) -> Result<Vec<RuleRow>> {
        Ok(sqlx::query_as(
            "SELECT * FROM rule_config WHERE start_date IS NOT NULL AND start_date <= ?",
        )
        .bind(today.to_string())
        .fetch_all(&self.0)
        .await?)
    }

    pub async fn save_rule(&self, r: &RuleRow) -> Result<()> {
        sqlx::query(
            "INSERT INTO rule_config (guild_id, rule_id, enabled, punishment, start_date, activated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)
             ON CONFLICT (guild_id, rule_id) DO UPDATE SET
               enabled = excluded.enabled, punishment = excluded.punishment,
               start_date = excluded.start_date, activated_at = excluded.activated_at",
        )
        .bind(r.guild_id)
        .bind(&r.rule_id)
        .bind(r.enabled)
        .bind(r.punishment.map(|p| serde_json::to_string(&p)).transpose()?)
        .bind(r.start_date.map(|d| d.to_string()))
        .bind(r.activated_at.map(fmt_time))
        .execute(&self.0)
        .await?;
        Ok(())
    }

    // ---- per-user rule state ----

    pub async fn state<T: DeserializeOwned>(
        &self,
        guild_id: u64,
        rule_id: &str,
        user_id: u64,
        key: &str,
    ) -> Result<Option<T>> {
        let v: Option<(String,)> = sqlx::query_as(
            "SELECT value FROM rule_state WHERE guild_id = ? AND rule_id = ? AND user_id = ? AND key = ?",
        )
        .bind(guild_id as i64)
        .bind(rule_id)
        .bind(user_id as i64)
        .bind(key)
        .fetch_optional(&self.0)
        .await?;
        Ok(v.and_then(|(s,)| serde_json::from_str(&s).ok()))
    }

    pub async fn set_state<T: serde::Serialize>(
        &self,
        guild_id: u64,
        rule_id: &str,
        user_id: u64,
        key: &str,
        value: &T,
    ) -> Result<()> {
        sqlx::query(
            "INSERT INTO rule_state (guild_id, rule_id, user_id, key, value, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)
             ON CONFLICT (guild_id, rule_id, user_id, key) DO UPDATE SET
               value = excluded.value, updated_at = excluded.updated_at",
        )
        .bind(guild_id as i64)
        .bind(rule_id)
        .bind(user_id as i64)
        .bind(key)
        .bind(serde_json::to_string(value)?)
        .bind(now())
        .execute(&self.0)
        .await?;
        Ok(())
    }

    /// Forget a rule's state for one user, or for everyone (`None`).
    pub async fn clear_state(
        &self,
        guild_id: u64,
        rule_id: &str,
        user_id: Option<u64>,
    ) -> Result<()> {
        sqlx::query(
            "DELETE FROM rule_state WHERE guild_id = ?1 AND rule_id = ?2 AND (?3 IS NULL OR user_id = ?3)",
        )
        .bind(guild_id as i64)
        .bind(rule_id)
        .bind(user_id.map(|u| u as i64))
        .execute(&self.0)
        .await?;
        Ok(())
    }

    // ---- timeouts ----

    /// Time `user_id` out until `until`, unless they're already out for longer.
    pub async fn extend_timeout(
        &self,
        guild_id: u64,
        user_id: u64,
        until: DateTime<Utc>,
    ) -> Result<()> {
        sqlx::query(
            "INSERT INTO timeouts (guild_id, user_id, until) VALUES (?1, ?2, ?3)
             ON CONFLICT (guild_id, user_id) DO UPDATE SET until = max(until, excluded.until)",
        )
        .bind(guild_id as i64)
        .bind(user_id as i64)
        .bind(fmt_time(until))
        .execute(&self.0)
        .await?;
        Ok(())
    }

    /// When `user_id`'s active timeout ends, or `None` if they aren't timed out.
    pub async fn timed_out(&self, guild_id: u64, user_id: u64) -> Result<Option<DateTime<Utc>>> {
        let row: Option<(String,)> = sqlx::query_as(
            "SELECT until FROM timeouts WHERE guild_id = ? AND user_id = ? AND until > ?",
        )
        .bind(guild_id as i64)
        .bind(user_id as i64)
        .bind(now())
        .fetch_optional(&self.0)
        .await?;
        Ok(row.and_then(|(t,)| parse_time(&t)))
    }

    /// End a timeout early. Returns whether one was active.
    pub async fn clear_timeout(&self, guild_id: u64, user_id: u64) -> Result<bool> {
        let active = self.timed_out(guild_id, user_id).await?.is_some();
        sqlx::query("DELETE FROM timeouts WHERE guild_id = ? AND user_id = ?")
            .bind(guild_id as i64)
            .bind(user_id as i64)
            .execute(&self.0)
            .await?;
        Ok(active)
    }

    // ---- punishment log ----

    pub async fn log_punishment(
        &self,
        guild_id: u64,
        rule_id: &str,
        user_id: u64,
        action: &str,
        reason: &str,
    ) -> Result<()> {
        sqlx::query(
            "INSERT INTO punishments (guild_id, rule_id, user_id, action, reason, created_at)
             VALUES (?, ?, ?, ?, ?, ?)",
        )
        .bind(guild_id as i64)
        .bind(rule_id)
        .bind(user_id as i64)
        .bind(action)
        .bind(reason)
        .bind(now())
        .execute(&self.0)
        .await?;
        Ok(())
    }

    pub async fn recent_punishments(
        &self,
        guild_id: u64,
        limit: i64,
    ) -> Result<Vec<PunishmentLog>> {
        Ok(sqlx::query_as(
            "SELECT rule_id, user_id, action, created_at FROM punishments
             WHERE guild_id = ? ORDER BY id DESC LIMIT ?",
        )
        .bind(guild_id as i64)
        .bind(limit)
        .fetch_all(&self.0)
        .await?)
    }

    /// Counts for one guild, or across every guild when `guild_id` is `None`.
    pub async fn punishment_stats(&self, guild_id: Option<u64>) -> Result<PunishmentStats> {
        let now = Utc::now();
        Ok(sqlx::query_as(
            "SELECT
                (SELECT count(*) FROM punishments WHERE ?1 IS NULL OR guild_id = ?1) AS total,
                (SELECT count(*) FROM punishments
                    WHERE (?1 IS NULL OR guild_id = ?1) AND created_at > ?2) AS last_day,
                (SELECT count(*) FROM timeouts
                    WHERE (?1 IS NULL OR guild_id = ?1) AND until > ?3) AS active_timeouts",
        )
        .bind(guild_id.map(|g| g as i64))
        .bind(fmt_time(now - chrono::Duration::days(1)))
        .bind(fmt_time(now))
        .fetch_one(&self.0)
        .await?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Duration;

    async fn db() -> Db {
        Db::connect("sqlite::memory:").await.unwrap()
    }

    #[tokio::test]
    async fn guild_config_defaults_and_updates() {
        let db = db().await;
        let g = db.guild(1).await.unwrap();
        assert!(g.enforced_role_id.is_none());
        db.set_guild_field(1, GuildField::EnforcedRole, Some(42))
            .await
            .unwrap();
        db.set_setup_hash(1, Some("x")).await.unwrap();
        let g = db.guild(1).await.unwrap();
        assert_eq!(g.enforced_role_id, Some(42));
        assert_eq!(g.last_setup_hash.as_deref(), Some("x"));
    }

    #[tokio::test]
    async fn rules_and_schedule() {
        let db = db().await;
        let mut r = db.rule(1, "a").await.unwrap();
        assert!(!r.enabled);
        let today = |s: &str| s.parse::<NaiveDate>().unwrap();
        r.start_date = Some(today("2026-12-01"));
        r.punishment = Some(Punishment::Disconnect);
        db.save_rule(&r).await.unwrap();
        assert!(
            db.due_scheduled(today("2026-11-30"))
                .await
                .unwrap()
                .is_empty()
        );
        assert_eq!(
            db.due_scheduled(today("2026-12-01")).await.unwrap().len(),
            1
        );
        assert_eq!(
            db.rule(1, "a").await.unwrap().punishment,
            Some(Punishment::Disconnect)
        );
        r.enabled = true;
        db.save_rule(&r).await.unwrap();
        assert_eq!(db.enabled_rules(1).await.unwrap().len(), 1);
        assert!(db.enabled_rules(2).await.unwrap().is_empty());
    }

    async fn raw_rule_columns(db: &Db) -> (Option<String>, Option<String>, Option<String>) {
        sqlx::query_as("SELECT punishment, start_date, activated_at FROM rule_config")
            .fetch_one(&db.0)
            .await
            .unwrap()
    }

    #[tokio::test]
    async fn rule_rows_round_trip_typed_values_in_the_existing_text_format() {
        let db = db().await;
        let mut r = db.rule(1, "a").await.unwrap();
        let at = parse_time("2026-12-01T09:30:00.000Z").unwrap();
        r.punishment = Some(Punishment::Timeout { secs: 90 });
        r.start_date = Some(NaiveDate::from_ymd_opt(2026, 12, 1).unwrap());
        r.activated_at = Some(at);
        db.save_rule(&r).await.unwrap();

        let back = db.rule(1, "a").await.unwrap();
        assert_eq!(back.punishment, Some(Punishment::Timeout { secs: 90 }));
        assert_eq!(back.start_date, NaiveDate::from_ymd_opt(2026, 12, 1));
        assert_eq!(back.activated_at, Some(at));
        assert_eq!(
            raw_rule_columns(&db).await,
            (
                Some(r#"{"kind":"timeout","secs":90}"#.into()),
                Some("2026-12-01".into()),
                Some("2026-12-01T09:30:00.000Z".into()),
            )
        );
    }

    #[tokio::test]
    async fn corrupt_rule_columns_load_as_unset_without_hiding_the_row() {
        let db = db().await;
        sqlx::query(
            "INSERT INTO rule_config (guild_id, rule_id, enabled, punishment, start_date, activated_at)
             VALUES (1, 'a', 1, 'not json', 'someday', 'yesterday')",
        )
        .execute(&db.0)
        .await
        .unwrap();
        let rows = db.enabled_rules(1).await.unwrap();
        assert_eq!(rows.len(), 1);
        let r = &rows[0];
        assert_eq!(r.rule_id, "a");
        assert_eq!(
            (r.punishment, r.start_date, r.activated_at),
            (None, None, None)
        );
    }

    #[tokio::test]
    async fn state_and_timeouts() {
        let db = db().await;
        db.set_state(1, "r", 7, "k", &"2026-11-05").await.unwrap();
        db.set_state(1, "r", 8, "k", &"2026-11-05").await.unwrap();
        assert_eq!(
            db.state::<String>(1, "r", 7, "k").await.unwrap().as_deref(),
            Some("2026-11-05")
        );
        db.clear_state(1, "r", Some(7)).await.unwrap();
        assert_eq!(db.state::<String>(1, "r", 7, "k").await.unwrap(), None);
        assert!(db.state::<String>(1, "r", 8, "k").await.unwrap().is_some());
        db.clear_state(1, "r", None).await.unwrap();
        assert_eq!(db.state::<String>(1, "r", 8, "k").await.unwrap(), None);

        assert!(db.timed_out(1, 7).await.unwrap().is_none());
        db.extend_timeout(1, 7, Utc::now() + Duration::hours(1))
            .await
            .unwrap();
        // A shorter timeout doesn't cut the longer one short.
        db.extend_timeout(1, 7, Utc::now() - Duration::seconds(1))
            .await
            .unwrap();
        assert!(db.timed_out(1, 7).await.unwrap().is_some());
        assert!(db.clear_timeout(1, 7).await.unwrap());
        assert!(db.timed_out(1, 7).await.unwrap().is_none());
        db.extend_timeout(1, 8, Utc::now() - Duration::seconds(1))
            .await
            .unwrap();
        assert!(db.timed_out(1, 8).await.unwrap().is_none(), "expired");

        db.log_punishment(1, "r", 7, "timed out", "why")
            .await
            .unwrap();
        assert_eq!(
            db.recent_punishments(1, 10).await.unwrap()[0].action,
            "timed out"
        );
    }

    #[tokio::test]
    async fn punishment_stats() {
        let db = db().await;
        assert_eq!(
            db.punishment_stats(None).await.unwrap(),
            PunishmentStats::default()
        );
        db.log_punishment(1, "r", 7, "timed out", "why")
            .await
            .unwrap();
        db.log_punishment(2, "r", 8, "disconnected", "why")
            .await
            .unwrap();
        sqlx::query("UPDATE punishments SET created_at = ? WHERE guild_id = 2")
            .bind(fmt_time(Utc::now() - Duration::days(2)))
            .execute(&db.0)
            .await
            .unwrap();
        db.extend_timeout(1, 7, Utc::now() + Duration::hours(1))
            .await
            .unwrap();
        db.extend_timeout(2, 8, Utc::now() - Duration::seconds(1))
            .await
            .unwrap();
        let all = db.punishment_stats(None).await.unwrap();
        assert_eq!((all.total, all.last_day, all.active_timeouts), (2, 1, 1));
        let g2 = db.punishment_stats(Some(2)).await.unwrap();
        assert_eq!((g2.total, g2.last_day, g2.active_timeouts), (1, 0, 0));
    }
}
