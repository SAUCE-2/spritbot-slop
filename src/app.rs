use std::ops::Deref;
use std::sync::Arc;

use chrono::{DateTime, NaiveDate, Utc};

use crate::ai::Judge;
use crate::config::Config;
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
    /// Jev, or None if JEV_API_KEY isn't set; AI-judged rules then fail the setup check.
    pub judge: Option<Arc<dyn Judge>>,
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

    /// "Today" in the bot's configured timezone.
    pub fn today(&self) -> NaiveDate {
        chrono::Utc::now().with_timezone(&self.cfg.tz).date_naive()
    }
}
