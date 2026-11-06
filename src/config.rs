use anyhow::{Context, Result};
use chrono::{DateTime, NaiveDate, Utc};
use chrono_tz::Tz;

/// Process-wide settings from the environment.
#[derive(Clone, Debug)]
pub struct Config {
    pub discord_token: String,
    /// Register slash commands in this guild only (instant updates while developing).
    pub dev_guild_id: Option<u64>,
    pub database_url: String,
    pub whisper_model: String,
    pub jev_api_key: Option<String>,
    /// Default timezone that defines "today" for daily rules and schedules, for guilds
    /// that haven't set their own with `/spirit timezone`.
    pub tz: Tz,
}

impl Config {
    pub fn from_env() -> Result<Self> {
        let var = |k: &str| std::env::var(k).ok().filter(|v| !v.trim().is_empty());
        Ok(Self {
            discord_token: var("DISCORD_TOKEN").context("DISCORD_TOKEN is not set")?,
            dev_guild_id: var("DEV_GUILD_ID")
                .map(|v| v.parse())
                .transpose()
                .context("DEV_GUILD_ID must be a number")?,
            database_url: var("DATABASE_URL")
                .unwrap_or_else(|| "sqlite://data/spiritbot.db".into()),
            whisper_model: var("WHISPER_MODEL").unwrap_or_else(|| "models/ggml-base.en.bin".into()),
            jev_api_key: var("JEV_API_KEY"),
            tz: var("BOT_TZ")
                .unwrap_or_else(|| "Europe/London".into())
                .parse()
                .map_err(|e| anyhow::anyhow!("BOT_TZ is not a valid timezone: {e}"))?,
        })
    }
}

/// The calendar date it is at `now` in `tz`.
pub fn local_date(now: DateTime<Utc>, tz: Tz) -> NaiveDate {
    now.with_timezone(&tz).date_naive()
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{NaiveDate, TimeZone, Utc};

    #[test]
    fn the_same_instant_is_a_different_day_across_the_date_line() {
        let now = Utc.with_ymd_and_hms(2026, 11, 30, 12, 0, 0).unwrap();
        let day = |m, d| NaiveDate::from_ymd_opt(2026, m, d).unwrap();
        assert_eq!(local_date(now, chrono_tz::Pacific::Kiritimati), day(12, 1));
        assert_eq!(local_date(now, chrono_tz::Pacific::Pago_Pago), day(11, 30));
        assert_eq!(local_date(now, chrono_tz::UTC), day(11, 30));
    }
}
