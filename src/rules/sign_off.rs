//! Every message must end with a Christmas greeting.

use anyhow::Result;
use async_trait::async_trait;

use super::{MessageCheck, Msg, Need, Rule, RuleScope};
use crate::punish::Punishment;

const CHECK: MessageCheck = MessageCheck {
    question: QUESTION,
    yes: YES,
    no: NO,
    reply: "Every message has to end with a Christmas greeting! Merry Christmas!",
    reason: "message didn't end with a Christmas greeting",
};

const QUESTION: &str = "The state is a Discord chat message, possibly with attached images. Does \
    it end with a Christmas greeting, as its sign-off?";
const YES: &str = "Ends with a Christmas greeting: the last words are a festive sign-off such as \
    \"Merry Christmas!\", \"Happy holidays\", \"Season's greetings\" or \"Ho ho ho\", in any \
    wording or language, or the message closes with a festive greeting image or GIF";
const NO: &str = "Doesn't end with a Christmas greeting: no greeting at all, a greeting only at \
    the start or middle with something else after it, festive emoji or words that aren't a \
    greeting, or a sarcastic or anti-Christmas sign-off";

pub struct SignOff;

#[async_trait]
impl Rule for SignOff {
    fn id(&self) -> &'static str {
        "sign_off"
    }
    fn name(&self) -> &'static str {
        "Festive Sign-Off"
    }
    fn description(&self) -> &'static str {
        "Every message must end with a Christmas greeting"
    }
    fn announcement(&self) -> &'static str {
        "✉️ **New rule: Festive Sign-Off!** From now on, every message you send must end with a \
         Christmas greeting. Finish each one with a \"Merry Christmas!\" or face the consequences. \
         Merry Christmas! 🎅"
    }
    fn default_punishment(&self) -> Punishment {
        Punishment::Timeout { secs: 60 }
    }
    fn needs(&self) -> &'static [Need] {
        &[Need::Jev]
    }

    /// Judges every message, and every edit to one.
    async fn message_check(&self, _s: &RuleScope, m: &Msg<'_>) -> Result<Option<MessageCheck>> {
        Ok((!m.is_blank().await).then_some(CHECK))
    }
}
