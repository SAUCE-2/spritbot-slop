//! Your first message each day must be festive.

use anyhow::Result;
use async_trait::async_trait;
use chrono::NaiveDate;
use serde::{Deserialize, Serialize};

use poise::serenity_prelude::MessageId;

use super::{MessageCheck, Msg, MsgKind, Need, Rule, RuleScope};
use crate::punish::Punishment;

/// State key for [`Judged`].
const JUDGED: &str = "judged";

/// The user's message that was judged as their first of `day` (bot timezone).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
struct Judged {
    day: NaiveDate,
    message: MessageId,
}

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
        let seen: Option<Judged> = s.state(user, JUDGED).await?;
        let today = s.today();
        if seen.is_some_and(|j| j.day == today) {
            return Ok(None);
        }
        // Claim the day before asking, atomically, so only one message can be the first.
        let first = Judged {
            day: today,
            message: m.msg.id,
        };
        let claimed = s.swap_state(user, JUDGED, seen.as_ref(), &first).await?;
        Ok(claimed.then_some(CHECK))
    }

    /// The judge failed: don't punish, judge their next message instead.
    async fn message_unjudged(&self, s: &RuleScope, m: &Msg<'_>) -> Result<()> {
        s.clear_state(m.msg.author.id).await
    }
}
