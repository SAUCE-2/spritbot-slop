//! Rules: each daily rule is one file implementing [`Rule`], listed in [`registry`].

mod first_message;
mod vc_greeting;

use std::sync::Arc;

use anyhow::Result;
use async_trait::async_trait;
use chrono::NaiveDate;
use poise::serenity_prelude::{
    self as serenity, ChannelId, GuildId, Member, Message, OnlineStatus, UserId,
};

use crate::ai::jev::Image;
use crate::app::Data;
use crate::db::RuleRow;
use crate::punish::{self, Punishment};
use crate::stats::{self, Board, Event};

/// Every rule, in display order. Add each new daily rule here (one file per rule in this folder).
pub fn registry() -> Vec<Arc<dyn Rule>> {
    vec![
        Arc::new(first_message::FirstMessage),
        Arc::new(vc_greeting::VcGreeting),
    ]
}

/// External capabilities a rule depends on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Need {
    /// Local Whisper speech-to-text model.
    Whisper,
    /// Jev API key.
    Jev,
}

#[async_trait]
pub trait Rule: Send + Sync {
    fn id(&self) -> &'static str;
    fn name(&self) -> &'static str;
    fn description(&self) -> &'static str;
    /// Festive text posted when the rule goes live.
    fn announcement(&self) -> &'static str;
    fn default_punishment(&self) -> Punishment;
    fn needs(&self) -> &'static [Need] {
        &[]
    }
    /// Extra leaderboards for events this rule records with [`RuleCx::record`] (ids are
    /// prefixed with the rule id). Every rule already gets "most punished", "most festive"
    /// and "least festive" boards, see [`crate::stats`].
    fn boards(&self) -> Vec<Board> {
        vec![]
    }

    async fn on_message(&self, _cx: &RuleCx, _msg: &Msg<'_>) -> Result<()> {
        Ok(())
    }
    async fn on_voice_join(
        &self,
        _cx: &RuleCx,
        _member: &Member,
        _channel: ChannelId,
    ) -> Result<()> {
        Ok(())
    }
    async fn on_presence(&self, _cx: &RuleCx, _user: UserId, _status: OnlineStatus) -> Result<()> {
        Ok(())
    }
    /// Every 60s for each guild where the rule is enabled.
    async fn on_tick(&self, _cx: &RuleCx) -> Result<()> {
        Ok(())
    }
}

/// A chat message plus its images, downloaded lazily and at most once across all rules.
pub struct Msg<'a> {
    pub msg: &'a Message,
    images: tokio::sync::OnceCell<Vec<Image>>,
}

impl<'a> Msg<'a> {
    pub fn new(msg: &'a Message) -> Self {
        Self {
            msg,
            images: Default::default(),
        }
    }

    pub async fn images(&self) -> Vec<Image> {
        self.images
            .get_or_init(|| crate::ai::message_images(self.msg))
            .await
            .clone()
    }
}

/// Everything a rule needs for one guild. Owned so rules can move it into spawned tasks.
#[derive(Clone)]
pub struct RuleCx {
    pub ctx: serenity::Context,
    pub data: Data,
    pub guild_id: GuildId,
    pub row: RuleRow,
    pub rule: Arc<dyn Rule>,
}

impl RuleCx {
    pub async fn load(
        ctx: &serenity::Context,
        data: &Data,
        guild_id: GuildId,
        rule: Arc<dyn Rule>,
    ) -> Result<Self> {
        let row = data.db.rule(guild_id.get(), rule.id()).await?;
        Ok(Self {
            ctx: ctx.clone(),
            data: data.clone(),
            guild_id,
            row,
            rule,
        })
    }

    pub fn punishment(&self) -> Punishment {
        self.row
            .punishment()
            .unwrap_or_else(|| self.rule.default_punishment())
    }

    pub async fn punish(&self, user: UserId, reason: &str) -> Result<String> {
        punish::punish(
            &self.ctx,
            &self.data,
            self.guild_id,
            user,
            self.rule.id(),
            self.punishment(),
            reason,
        )
        .await
    }

    /// Ask Jev one yes/no question about a chat message (text + images). `None` = no answer.
    pub async fn ask_msg(
        &self,
        m: &Msg<'_>,
        question: &str,
        criteria: Option<(&str, &str)>,
    ) -> Option<f32> {
        let images = m.images().await;
        let state = crate::ai::message_state(&m.msg.content, &images);
        crate::ai::ask_noul(&self.data, state, images, question, criteria).await
    }

    /// Record something for the leaderboards (tagged with this rule). Never fails.
    pub async fn record(&self, user: UserId, ev: Event) {
        stats::record(&self.data, self.guild_id, user, ev.rule(self.rule.id())).await
    }

    /// Record a Jev verdict and what was said, for the festive/grinch leaderboards.
    /// Call this whenever the rule judges something, festive or not.
    pub async fn record_judgement(&self, user: UserId, score: f32, said: &str) {
        let ev = Event::new(stats::kind::JUDGED).score(score).text(said);
        self.record(user, ev).await
    }

    pub fn today(&self) -> NaiveDate {
        self.data.today()
    }

    pub async fn state<T: serde::de::DeserializeOwned>(
        &self,
        user: UserId,
        key: &str,
    ) -> Result<Option<T>> {
        self.data
            .db
            .state(self.guild_id.get(), self.rule.id(), user.get(), key)
            .await
    }

    pub async fn set_state<T: serde::Serialize>(
        &self,
        user: UserId,
        key: &str,
        v: &T,
    ) -> Result<()> {
        self.data
            .db
            .set_state(self.guild_id.get(), self.rule.id(), user.get(), key, v)
            .await
    }

    pub async fn clear_state(&self, user: UserId) -> Result<()> {
        self.data
            .db
            .clear_state(self.guild_id.get(), self.rule.id(), Some(user.get()))
            .await
    }
}

/// Problems that stop `rule` going live (missing capabilities).
pub fn missing_requirements(data: &Data, rule: &dyn Rule) -> Vec<String> {
    let mut out = Vec::new();
    for need in rule.needs() {
        match need {
            Need::Whisper if data.stt.is_none() => out.push(format!(
                "`{}` needs the Whisper model, which isn't loaded (check WHISPER_MODEL)",
                rule.id()
            )),
            Need::Jev if data.jev.is_none() => out.push(format!(
                "`{}` needs `JEV_API_KEY` set on the bot",
                rule.id()
            )),
            _ => {}
        }
    }
    out
}
