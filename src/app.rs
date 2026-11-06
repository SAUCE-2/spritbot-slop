use std::ops::Deref;
use std::sync::Arc;

use chrono::{DateTime, NaiveDate, Utc};
use chrono_tz::Tz;
use poise::serenity_prelude::GuildId;

use crate::ai::jev::Jev;
use crate::config::{Config, local_date};
use crate::db::Db;
use crate::rules::Rule;
use crate::voice::listener::VoiceListener;
use crate::voice::stt::Stt;

pub type Error = anyhow::Error;
pub type Ctx<'a> = poise::Context<'a, Data, Error>;

/// Shared bot state. Cheap to clone into background tasks.
#[derive(Clone)]
pub struct Data(Arc<Inner>);

pub struct Inner {
    pub cfg: Config,
    pub db: Db,
    pub rules: Vec<Arc<dyn Rule>>,
    /// None if the Whisper model couldn't be loaded; voice rules then fail the setup check.
    pub stt: Option<Stt>,
    /// None if JEV_API_KEY isn't set; AI-judged rules then fail the setup check.
    pub jev: Option<Jev>,
    pub voice: VoiceListener,
    /// When the process started, for `/status` uptime.
    pub started_at: DateTime<Utc>,
}

impl Deref for Data {
    type Target = Inner;
    fn deref(&self) -> &Inner {
        &self.0
    }
}

impl Data {
    pub fn new(inner: Inner) -> Self {
        Self(Arc::new(inner))
    }

    pub fn rule(&self, id: &str) -> Option<Arc<dyn Rule>> {
        self.rules.iter().find(|r| r.id() == id).cloned()
    }

    /// The timezone that defines "today" in this guild (its own, else `BOT_TZ`).
    pub async fn guild_tz(&self, guild_id: GuildId) -> Result<Tz, Error> {
        Ok(self.db.guild(guild_id.get()).await?.tz(self.cfg.tz))
    }

    /// "Today" in this guild's timezone.
    pub async fn today(&self, guild_id: GuildId) -> Result<NaiveDate, Error> {
        Ok(local_date(Utc::now(), self.guild_tz(guild_id).await?))
    }
}
