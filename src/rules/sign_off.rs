//! Every message must end with a Christmas greeting.

use anyhow::Result;
use async_trait::async_trait;

use super::{Msg, Need, Rule, RuleCx};
use crate::punish::Punishment;

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

    async fn on_message(&self, cx: &RuleCx, msg: &Msg<'_>) -> Result<()> {
        // Nothing to judge (e.g. a sticker or an embed-only message): let it through.
        if msg.msg.content.trim().is_empty() && msg.images().await.is_empty() {
            return Ok(());
        }
        // Jev failed: don't punish.
        let Some(score) = cx.ask_msg(msg, QUESTION, Some((YES, NO))).await else {
            return Ok(());
        };
        if score < 0.5 {
            msg.msg
                .reply(
                    &cx.ctx,
                    format!(
                        "Every message has to end with a Christmas greeting! (jev {score:.2}) \
                         Merry Christmas!"
                    ),
                )
                .await
                .ok();
            cx.punish(
                msg.msg.author.id,
                &format!("message didn't end with a Christmas greeting (jev {score:.2})"),
            )
            .await?;
        }
        Ok(())
    }
}
