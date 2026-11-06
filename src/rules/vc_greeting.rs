//! When you join VC, you must say something festive within 10 seconds.

use std::time::Duration;

use anyhow::Result;
use async_trait::async_trait;
use poise::serenity_prelude::{ChannelId, Member, UserId};
use serde_json::json;

use super::{Need, Rule, RuleCx};
use crate::punish::Punishment;
use crate::stats::{Board, Event, FESTIVE};
use crate::voice::listener::Outcome;

const WINDOW: Duration = Duration::from_secs(10);

/// Stat kinds: joined VC and said nothing / left before the 10 seconds were up.
const SILENT: &str = "vc_silent";
const FLED: &str = "vc_fled";

const QUESTION: &str = "The state is a speech-to-text transcript of what someone said in the first \
    10 seconds after joining a Discord voice call. The transcript may contain mishearings. Did they \
    say something festive, in the Christmas spirit?";
const YES: &str = "Festive: a Christmas greeting or holiday cheer such as \"merry Christmas\", \
    \"happy holidays\" or \"ho ho ho\", even if slightly misheard";
const NO: &str = "Not festive: ordinary talk with no Christmas spirit, or sarcastic or \
    anti-Christmas";

pub struct VcGreeting;

#[async_trait]
impl Rule for VcGreeting {
    fn id(&self) -> &'static str {
        "vc_greeting"
    }
    fn name(&self) -> &'static str {
        "Festive Arrival"
    }
    fn description(&self) -> &'static str {
        "When you join VC, say something festive within 10 seconds"
    }
    fn announcement(&self) -> &'static str {
        "🔔 **New rule: Festive Arrival!** Whenever you join a voice channel, you have 10 seconds \
         to say something festive. Walk in with a \"Merry Christmas!\" or get sent back out. 🎅"
    }
    fn default_punishment(&self) -> Punishment {
        Punishment::Disconnect
    }
    fn needs(&self) -> &'static [Need] {
        &[Need::Whisper, Need::Jev]
    }
    fn boards(&self) -> Vec<Board> {
        vec![
            Board::most(
                "silent",
                "🤐 Silent Night (joined and said nothing)",
                &[SILENT],
                "times",
            ),
            Board::most(
                "fled",
                "🏃 Cold Feet (left before greeting)",
                &[FLED],
                "times",
            ),
        ]
    }

    async fn on_voice_join(&self, cx: &RuleCx, member: &Member, channel: ChannelId) -> Result<()> {
        // Listening takes 10s+, so don't hold up other rules' handling of this event.
        let (cx, user) = (cx.clone(), member.user.id);
        tokio::spawn(async move {
            if let Err(e) = judge(&cx, user, channel).await {
                tracing::warn!(guild_id = %cx.guild_id, rule = cx.rule.id(), "rule error: {e:#}");
            }
        });
        Ok(())
    }
}

async fn judge(cx: &RuleCx, user: UserId, channel: ChannelId) -> Result<()> {
    let heard = match cx
        .data
        .voice
        .listen(&cx.ctx, &cx.data, cx.guild_id, channel, user, WINDOW)
        .await
    {
        Outcome::Heard(text) => text,
        Outcome::Left => {
            cx.record(user, Event::new(FLED)).await;
            return Ok(());
        }
        Outcome::Failed(e) => {
            // Our fault, not theirs: don't punish.
            tracing::warn!(guild_id = %cx.guild_id, %user, "vc_greeting listen failed: {e}");
            return Ok(());
        }
    };

    let reason = if heard.is_empty() {
        cx.record(user, Event::new(SILENT)).await;
        "didn't say anything festive within 10s of joining VC".to_owned()
    } else {
        let Some(score) =
            crate::ai::ask_noul(&cx.data, json!(heard), vec![], QUESTION, Some((YES, NO))).await
        else {
            return Ok(()); // Jev failed: don't punish
        };
        cx.record_judgement(user, score, &heard).await;
        if score >= FESTIVE {
            return Ok(());
        }
        format!("VC greeting wasn't festive: \"{heard}\" (jev {score:.2})")
    };
    cx.punish(user, &reason).await?;
    Ok(())
}
