//! Your first message each day must be festive.

use anyhow::Result;
use async_trait::async_trait;
use chrono::NaiveDate;

use super::{Msg, Need, Rule, RuleCx};
use crate::punish::Punishment;
use crate::stats::FESTIVE;

/// State key: the last day (bot timezone) this user's first message was judged.
const LAST_DAY: &str = "last_day";

const QUESTION: &str = "The state is someone's first Discord chat message of the day, possibly \
    with attached images. Is it festive, in the Christmas spirit?";
const YES: &str = "Festive: a Christmas greeting, holiday cheer, or festive emoji, images or GIFs, \
    in any wording or language";
const NO: &str = "Not festive: an ordinary message with no Christmas spirit, or sarcastic or \
    anti-Christmas";

pub struct FirstMessage;

#[async_trait]
impl Rule for FirstMessage {
    fn id(&self) -> &'static str {
        "first_message"
    }
    fn name(&self) -> &'static str {
        "Festive First Words"
    }
    fn description(&self) -> &'static str {
        "Your first message each day must be festive"
    }
    fn announcement(&self) -> &'static str {
        "🎄 **New rule: Festive First Words!** From now on, your first message every day must be \
         festive. Start the day with a \"Merry Christmas!\" or face the consequences. 🎅"
    }
    fn default_punishment(&self) -> Punishment {
        Punishment::Timeout { secs: 300 }
    }
    fn needs(&self) -> &'static [Need] {
        &[Need::Jev]
    }

    async fn on_message(&self, cx: &RuleCx, msg: &Msg<'_>) -> Result<()> {
        let user = msg.msg.author.id;
        let today = cx.today().await?;
        if cx.state::<NaiveDate>(user, LAST_DAY).await? == Some(today) {
            return Ok(());
        }
        // Mark before asking so only the first message of the day counts.
        cx.set_state(user, LAST_DAY, &today).await?;

        let Some(score) = cx.ask_msg(msg, QUESTION, Some((YES, NO))).await else {
            // Jev failed: don't punish, judge their next message instead.
            cx.clear_state(user).await?;
            return Ok(());
        };
        let said = if msg.msg.content.trim().is_empty() {
            "(image/GIF)"
        } else {
            &msg.msg.content
        };
        cx.record_judgement(user, score, said).await;
        if score < FESTIVE {
            msg.msg
                .reply(
                    &cx.ctx,
                    format!("Your first message today has to be festive! (jev {score:.2})"),
                )
                .await
                .ok();
            cx.punish(
                user,
                &format!("first message of the day wasn't festive (jev {score:.2})"),
            )
            .await?;
        }
        Ok(())
    }
}
