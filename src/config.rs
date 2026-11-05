use anyhow::{Context, Result};
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
    /// Timezone that defines "today" for daily rules and schedules.
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
