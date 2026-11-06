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

    /// Judges a user's first message of the day, and edits to that message (a festive first
    /// message can't be edited into something else). Edits to later messages are ignored.
    async fn message_check(&self, s: &RuleScope, m: &Msg<'_>) -> Result<Option<MessageCheck>> {
        let user = m.msg.author.id;
        let seen: Option<Judged> = s.state(user, JUDGED).await?;
        let this = Judged {
            day: s.today().await?,
            message: m.msg.id,
        };
        Ok(match m.kind {
            MsgKind::Edited => (seen == Some(this)).then_some(CHECK),
            MsgKind::Posted if seen.is_some_and(|j| j.day == this.day) => None,
            // Claim the day before asking, atomically, so only one message can be the first.
            MsgKind::Posted => s
                .swap_state(user, JUDGED, seen.as_ref(), &this)
                .await?
                .then_some(CHECK),
        })
    }

    /// The judge failed: don't punish. For a new message, release the day so their next message
    /// is judged instead; an edit keeps the claim (the original was already judged).
    async fn message_unjudged(&self, s: &RuleScope, m: &Msg<'_>) -> Result<()> {
        match m.kind {
            MsgKind::Posted => s.clear_state(m.msg.author.id).await,
            MsgKind::Edited => Ok(()),
        }
    }
}
