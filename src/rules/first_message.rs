//! Your first message each day must be festive.

use anyhow::Result;
use async_trait::async_trait;
use chrono::NaiveDate;

use super::{MessageCheck, Msg, MsgKind, Need, Rule, RuleScope};
use crate::punish::Punishment;

/// State key: the last day (bot timezone) this user's first message was judged.
const LAST_DAY: &str = "last_day";

const CHECK: MessageCheck = MessageCheck {
    question: QUESTION,
    yes: YES,
    no: NO,
    reply: "Your first message today has to be festive!",
    reason: "first message of the day wasn't festive",
};

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

    async fn message_check(&self, s: &RuleScope, m: &Msg<'_>) -> Result<Option<MessageCheck>> {
        if m.kind == MsgKind::Edited {
            return Ok(None);
        }
        let user = m.msg.author.id;
        let today = s.today();
        if s.state::<NaiveDate>(user, LAST_DAY).await? == Some(today) {
            return Ok(None);
        }
        // Mark before asking so only the first message of the day counts.
        s.set_state(user, LAST_DAY, &today).await?;
        Ok(Some(CHECK))
    }

    /// The judge failed: don't punish, judge their next message instead.
    async fn message_unjudged(&self, s: &RuleScope, m: &Msg<'_>) -> Result<()> {
        s.clear_state(m.msg.author.id).await
    }
}
