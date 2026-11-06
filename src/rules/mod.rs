//! Rules: each daily rule is one file implementing [`Rule`], listed in [`registry`].

mod chat;
mod first_message;
mod sign_off;
mod threshold;
mod vc_greeting;

use std::ops::Deref;
use std::sync::Arc;

use anyhow::Result;
use async_trait::async_trait;
use chrono::NaiveDate;
use poise::serenity_prelude::{self as serenity, ChannelId, GuildId, Member, OnlineStatus, UserId};

use crate::app::Data;
use crate::db::RuleRow;
use crate::punish::{self, Punishment};
use crate::stats::{self, Log};

pub use chat::{MessageCheck, Msg, MsgKind, judge_message};
pub use threshold::{Threshold, ThresholdSetting};

/// Every rule, in display order. Add each new daily rule here (one file per rule in this folder).
pub fn registry() -> Vec<Arc<dyn Rule>> {
    vec![
        Arc::new(first_message::FirstMessage),
        Arc::new(vc_greeting::VcGreeting),
        Arc::new(sign_off::SignOff),
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

    /// What this rule wants the judge to answer about a chat message (new or edited, see
    /// [`MsgKind`]), or `None` to let it through. Every rule's check on one message is answered
    /// in a single judge call; a "no" gets the author a reply and the rule's punishment.
    async fn message_check(&self, _s: &RuleScope, _m: &Msg<'_>) -> Result<Option<MessageCheck>> {
        Ok(None)
    }
    /// The judge couldn't answer this rule's check on `m` (it failed or isn't configured), so
    /// nobody is punished. Undo anything `message_check` reserved for that message.
    async fn message_unjudged(&self, _s: &RuleScope, _m: &Msg<'_>) -> Result<()> {
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

/// One rule in one guild, without a Discord connection: its config and its per-user memory.
#[derive(Clone)]
pub struct RuleScope {
    pub data: Data,
    pub guild_id: GuildId,
    pub row: RuleRow,
    pub rule: Arc<dyn Rule>,
}

impl RuleScope {
    pub async fn load(data: &Data, guild_id: GuildId, rule: Arc<dyn Rule>) -> Result<Self> {
        let row = data.db.rule(guild_id.get(), rule.id()).await?;
        Ok(Self {
            data: data.clone(),
            guild_id,
            row,
            rule,
        })
    }

    /// Every rule enabled in the guild.
    pub async fn enabled(data: &Data, guild_id: GuildId) -> Result<Vec<Self>> {
        Ok(data
            .db
            .enabled_rules(guild_id.get())
            .await?
            .into_iter()
            .filter_map(|row| {
                let rule = data.rule(&row.rule_id)?;
                Some(Self {
                    data: data.clone(),
                    guild_id,
                    row,
                    rule,
                })
            })
            .collect())
    }

    pub fn punishment(&self) -> Punishment {
        self.row
            .punishment
            .unwrap_or_else(|| self.rule.default_punishment())
    }

    /// "Today" in this guild's timezone.
    pub async fn today(&self) -> Result<NaiveDate> {
        self.data.today(self.guild_id).await
    }

    /// Log something that happened to `user` in the activity log, tagged with this rule, for
    /// end-of-month leaderboards. `data` is anything serializable, usually `json!({...})`.
    /// Log generously: it's cheap and you can't go back for it later. Never fails.
    pub async fn log(&self, user: UserId, kind: &'static str, data: impl serde::Serialize) {
        let log = Log::new(kind, data)
            .user(user)
            .rule(self.rule.id())
            .enforced(Some(true));
        stats::record(&self.data, self.guild_id, log).await
    }

    /// This rule's threshold in this guild.
    pub fn threshold(&self) -> Threshold {
        self.row.threshold.unwrap_or_default()
    }

    /// Decide a Jev `score` on what `user` said: whether it keeps the rule (meets the rule's
    /// threshold). Logs the verdict. Every judged thing goes through here, festive or not (for
    /// chat messages, [`judge_message`] does it).
    pub async fn judged(&self, user: UserId, score: f32, said: &str) -> bool {
        let threshold = self.threshold();
        let kept = threshold.kept_by(score);
        let data = serde_json::json!({
            "score": score,
            "said": said,
            "threshold": threshold.get(),
            "festive": kept,
        });
        self.log(user, stats::kind::JUDGED, data).await;
        kept
    }

    /// What to do now that `user` broke the rule: punish them, or in shadow mode only report
    /// it. `score` is Jev's, if it was asked; `reason` says what they did.
    pub fn enforcement(&self, user: UserId, score: Option<f32>, reason: &str) -> Enforcement {
        if self.row.shadow {
            let scored = score
                .map(|s| format!(" (score {s:.2}, threshold {})", self.threshold()))
                .unwrap_or_default();
            let rule = self.rule.id();
            return Enforcement::Report(format!(
                "👻 [shadow] <@{user}> would have broken **{rule}**{scored}: {reason}"
            ));
        }
        let jev = score.map(|s| format!(" (jev {s:.2})")).unwrap_or_default();
        Enforcement::Punish(format!("{reason}{jev}"))
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

    /// Store `new` only if the state still holds `current`; see [`crate::db::Db::swap_state`].
    pub async fn swap_state<T: serde::Serialize>(
        &self,
        user: UserId,
        key: &str,
        current: Option<&T>,
        new: &T,
    ) -> Result<bool> {
        self.data
            .db
            .swap_state(
                self.guild_id.get(),
                self.rule.id(),
                user.get(),
                key,
                current,
                new,
            )
            .await
    }

    pub async fn clear_state(&self, user: UserId) -> Result<()> {
        self.data
            .db
            .clear_state(self.guild_id.get(), self.rule.id(), Some(user.get()))
            .await
    }
}

/// What happens to someone who broke a rule.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Enforcement {
    /// Punish them, for this reason.
    Punish(String),
    /// The rule is in shadow mode: post this to the log channel and nothing else.
    Report(String),
}

/// A [`RuleScope`] plus the Discord connection, for hooks that act on Discord themselves.
/// Owned so rules can move it into spawned tasks.
#[derive(Clone)]
pub struct RuleCx {
    pub ctx: serenity::Context,
    pub scope: RuleScope,
}

impl Deref for RuleCx {
    type Target = RuleScope;
    fn deref(&self) -> &RuleScope {
        &self.scope
    }
}

impl RuleCx {
    pub async fn load(
        ctx: &serenity::Context,
        data: &Data,
        guild_id: GuildId,
        rule: Arc<dyn Rule>,
    ) -> Result<Self> {
        Ok(Self {
            ctx: ctx.clone(),
            scope: RuleScope::load(data, guild_id, rule).await?,
        })
    }

    /// `user` broke the rule: punish them, or in shadow mode only report it to the log channel.
    pub async fn enforce(&self, user: UserId, score: Option<f32>, reason: &str) -> Result<()> {
        self.carry_out(user, self.enforcement(user, score, reason))
            .await
    }

    /// Do what [`RuleScope::enforcement`] decided.
    pub async fn carry_out(&self, user: UserId, enforcement: Enforcement) -> Result<()> {
        match enforcement {
            Enforcement::Punish(reason) => self.punish(user, &reason).await.map(drop),
            Enforcement::Report(line) => {
                punish::post_log(&self.ctx, &self.data, self.guild_id, line).await;
                Ok(())
            }
        }
    }

    async fn punish(&self, user: UserId, reason: &str) -> Result<String> {
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
            Need::Jev if data.judge.is_none() => out.push(format!(
                "`{}` needs `JEV_API_KEY` set on the bot",
                rule.id()
            )),
            _ => {}
        }
    }
    out
}
