//! Judging chat messages. Each enabled rule says what it wants to know about a message
//! ([`Rule::message_check`](super::Rule::message_check)); the judge answers them.

use poise::serenity_prelude::Message;

use super::RuleScope;
use crate::ai::jev::{Image, noul};
use crate::ai::{self, Judge};

/// Whether the message was just posted or its author just changed it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MsgKind {
    Posted,
    Edited,
}

/// A chat message from an enforced member, plus its images, downloaded lazily and at most once.
pub struct Msg<'a> {
    pub msg: &'a Message,
    pub kind: MsgKind,
    images: tokio::sync::OnceCell<Vec<Image>>,
}

impl<'a> Msg<'a> {
    pub fn new(msg: &'a Message, kind: MsgKind) -> Self {
        Self {
            msg,
            kind,
            images: Default::default(),
        }
    }

    pub async fn images(&self) -> Vec<Image> {
        self.images
            .get_or_init(|| ai::message_images(self.msg))
            .await
            .clone()
    }

    /// Nothing to judge: no text and no images (e.g. a sticker or an embed-only message).
    pub async fn is_blank(&self) -> bool {
        self.msg.content.trim().is_empty() && self.images().await.is_empty()
    }
}

/// A yes/no question a rule asks about a message, and what to tell the author when it's "no".
#[derive(Clone, Copy, Debug)]
pub struct MessageCheck {
    /// Asked of the judge, with the message (text and images) as the state.
    pub question: &'static str,
    /// What counts as "yes": the message keeps the rule.
    pub yes: &'static str,
    /// What counts as "no": the message breaks the rule.
    pub no: &'static str,
    /// Replied to a message that breaks the rule.
    pub reply: &'static str,
    /// Why they were punished, for the log.
    pub reason: &'static str,
}

/// A message that broke a rule: the judge answered "no" to that rule's check.
pub struct Breach<'a> {
    pub scope: &'a RuleScope,
    pub check: MessageCheck,
    pub score: f32,
}

impl Breach<'_> {
    pub fn reply(&self) -> String {
        format!("{} (jev {:.2})", self.check.reply, self.score)
    }

    pub fn reason(&self) -> String {
        format!("{} (jev {:.2})", self.check.reason, self.score)
    }
}

/// Ask every rule in `scopes` what it wants to know about `m`, put all of it to `judge` in one
/// call, and return the rules `m` broke. No questions, no call. With no judge, or when the
/// judge fails, nothing is broken (fail open).
pub async fn judge_message<'a>(
    judge: Option<&dyn Judge>,
    scopes: &'a [RuleScope],
    m: &Msg<'_>,
) -> Vec<Breach<'a>> {
    let mut asked = Vec::new();
    for scope in scopes {
        match scope.rule.message_check(scope, m).await {
            Ok(Some(check)) => asked.push((scope, check)),
            Ok(None) => {}
            Err(e) => log_err(scope, e),
        }
    }
    if asked.is_empty() {
        return Vec::new();
    }

    let answers = match judge {
        Some(judge) => {
            let images = m.images().await;
            let state = ai::message_state(&m.msg.content, &images);
            let questions = asked
                .iter()
                .map(|(s, c)| (s.rule.id(), noul(c.question, Some((c.yes, c.no)))))
                .collect();
            judge
                .ask(state, questions, images)
                .await
                .inspect_err(|e| tracing::warn!("judge call failed: {e:#}"))
                .ok()
        }
        None => None,
    };

    let mut breaches = Vec::new();
    for (scope, check) in asked {
        match answers.as_ref().map(|a| a.noul(scope.rule.id())) {
            Some(Ok(score)) if score < ai::YES => breaches.push(Breach {
                scope,
                check,
                score,
            }),
            Some(Ok(_)) => {}
            unanswered => {
                if let Some(Err(e)) = unanswered {
                    log_err(scope, e);
                }
                if let Err(e) = scope.rule.message_unjudged(scope, m).await {
                    log_err(scope, e);
                }
            }
        }
    }
    breaches
}

fn log_err(scope: &RuleScope, e: anyhow::Error) {
    tracing::warn!(guild_id = %scope.guild_id, rule = scope.rule.id(), "rule error: {e:#}");
}

#[cfg(test)]
pub(crate) mod tests {
    use std::collections::HashMap;
    use std::sync::{Arc, Mutex};

    use anyhow::{Result, bail};
    use async_trait::async_trait;
    use poise::serenity_prelude::{GuildId, MessageId, UserId};
    use serde_json::Value;

    use super::*;
    use crate::ai::jev::{Answer, Answers, Question};
    use crate::app::{Data, Inner};
    use crate::config::Config;
    use crate::db::Db;

    const GUILD: GuildId = GuildId::new(1);

    /// Records the question names of every call and answers each with `score`, or fails.
    pub(crate) struct FakeJudge {
        score: Option<f32>,
        calls: Mutex<Vec<Vec<&'static str>>>,
    }

    impl FakeJudge {
        pub(crate) fn answering(score: f32) -> Arc<Self> {
            Arc::new(Self {
                score: Some(score),
                calls: Default::default(),
            })
        }
        pub(crate) fn failing() -> Arc<Self> {
            Arc::new(Self {
                score: None,
                calls: Default::default(),
            })
        }
        pub(crate) fn calls(&self) -> Vec<Vec<&'static str>> {
            self.calls.lock().unwrap().clone()
        }
    }

    #[async_trait]
    impl Judge for FakeJudge {
        async fn ask(
            &self,
            _state: Value,
            questions: Vec<(&'static str, Question)>,
            _images: Vec<Image>,
        ) -> Result<Answers> {
            let mut names: Vec<_> = questions.iter().map(|(n, _)| *n).collect();
            names.sort();
            self.calls.lock().unwrap().push(names.clone());
            let Some(noul) = self.score else {
                bail!("judge is down")
            };
            Ok(Answers {
                model: "fake".into(),
                answers: names
                    .into_iter()
                    .map(|n| (n.to_owned(), Answer::Noul { noul }))
                    .collect::<HashMap<_, _>>(),
            })
        }
    }

    /// Bot state with `enabled` rules switched on in [`GUILD`].
    pub(crate) async fn data(enabled: &[&str]) -> Data {
        let db = Db::connect("sqlite::memory:").await.unwrap();
        for id in enabled {
            let mut row = db.rule(GUILD.get(), id).await.unwrap();
            row.enabled = true;
            db.save_rule(&row).await.unwrap();
        }
        Data::new(Inner {
            cfg: Config {
                discord_token: String::new(),
                dev_guild_id: None,
                database_url: "sqlite::memory:".into(),
                whisper_model: String::new(),
                jev_api_key: None,
                tz: chrono_tz::UTC,
            },
            db,
            rules: super::super::registry(),
            stt: None,
            judge: None,
            voice: Default::default(),
            started_at: chrono::Utc::now(),
        })
    }

    pub(crate) fn message(id: u64, author: u64, content: &str) -> Message {
        let mut m = Message::default();
        m.id = MessageId::new(id);
        m.author.id = UserId::new(author);
        m.content = content.into();
        m
    }

    /// Judge `msg` the way the event handler does; returns the ids of the rules it broke.
    async fn broken_rules(
        data: &Data,
        judge: &Arc<FakeJudge>,
        msg: &Message,
        kind: MsgKind,
    ) -> Vec<&'static str> {
        let scopes = RuleScope::enabled(data, GUILD).await.unwrap();
        let judge: &dyn Judge = judge.as_ref();
        let breaches = judge_message(Some(judge), &scopes, &Msg::new(msg, kind)).await;
        breaches.iter().map(|b| b.scope.rule.id()).collect()
    }

    #[tokio::test]
    async fn one_judge_call_asks_every_enabled_rule_about_a_message() {
        let data = data(&["first_message", "sign_off"]).await;
        let judge = FakeJudge::answering(0.1);
        let mut broken =
            broken_rules(&data, &judge, &message(1, 7, "hello"), MsgKind::Posted).await;
        broken.sort();
        assert_eq!(judge.calls(), vec![vec!["first_message", "sign_off"]]);
        assert_eq!(broken, vec!["first_message", "sign_off"]);
    }

    #[tokio::test]
    async fn a_yes_breaks_nothing() {
        let data = data(&["first_message", "sign_off"]).await;
        let judge = FakeJudge::answering(0.9);
        let broken = broken_rules(
            &data,
            &judge,
            &message(1, 7, "Merry Christmas!"),
            MsgKind::Posted,
        )
        .await;
        assert!(broken.is_empty());
    }

    #[tokio::test]
    async fn no_message_question_means_no_judge_call() {
        let data = data(&["vc_greeting"]).await;
        let judge = FakeJudge::answering(0.1);
        assert!(
            broken_rules(&data, &judge, &message(1, 7, "hello"), MsgKind::Posted)
                .await
                .is_empty()
        );
        assert!(judge.calls().is_empty());
    }

    #[tokio::test]
    async fn a_failing_judge_punishes_nobody() {
        let data = data(&["first_message", "sign_off"]).await;
        let down = FakeJudge::failing();
        assert!(
            broken_rules(&data, &down, &message(1, 7, "hello"), MsgKind::Posted)
                .await
                .is_empty()
        );
        assert_eq!(down.calls().len(), 1);

        // first_message didn't get an answer, so it judges their next message instead.
        let up = FakeJudge::answering(0.1);
        let broken = broken_rules(&data, &up, &message(2, 7, "hello again"), MsgKind::Posted).await;
        assert!(broken.contains(&"first_message"));
    }

    #[tokio::test]
    async fn no_judge_configured_punishes_nobody() {
        let data = data(&["first_message", "sign_off"]).await;
        let scopes = RuleScope::enabled(&data, GUILD).await.unwrap();
        let msg = message(1, 7, "hello");
        assert!(
            judge_message(None, &scopes, &Msg::new(&msg, MsgKind::Posted))
                .await
                .is_empty()
        );
    }

    #[tokio::test]
    async fn first_message_only_judges_the_first_message_of_the_day() {
        let data = data(&["first_message"]).await;
        let judge = FakeJudge::answering(0.1);
        assert_eq!(
            broken_rules(&data, &judge, &message(1, 7, "hello"), MsgKind::Posted).await,
            vec!["first_message"]
        );
        assert!(
            broken_rules(&data, &judge, &message(2, 7, "hello"), MsgKind::Posted)
                .await
                .is_empty()
        );
        // Someone else's first message still counts.
        assert_eq!(
            broken_rules(&data, &judge, &message(3, 8, "hello"), MsgKind::Posted).await,
            vec!["first_message"]
        );
        assert_eq!(judge.calls().len(), 2);
    }

    #[tokio::test]
    async fn sign_off_lets_blank_messages_through_without_asking() {
        let data = data(&["sign_off"]).await;
        let judge = FakeJudge::answering(0.1);
        assert!(
            broken_rules(&data, &judge, &message(1, 7, "  \n"), MsgKind::Posted)
                .await
                .is_empty()
        );
        assert!(judge.calls().is_empty());
    }

    #[tokio::test]
    async fn two_first_messages_at_once_are_judged_once() {
        let data = data(&["first_message"]).await;
        let judge = FakeJudge::answering(0.1);
        let (a, b) = (message(1, 7, "hello"), message(2, 7, "hi"));
        let (a, b) = tokio::join!(
            broken_rules(&data, &judge, &a, MsgKind::Posted),
            broken_rules(&data, &judge, &b, MsgKind::Posted),
        );
        assert_eq!(a.len() + b.len(), 1, "punished {a:?} and {b:?}");
        assert_eq!(judge.calls().len(), 1);
    }

    #[tokio::test]
    async fn first_message_rejudges_edits_to_the_message_it_judged_only() {
        let data = data(&["first_message"]).await;
        let judge = FakeJudge::answering(0.1);
        let first = message(1, 7, "Merry Christmas!");
        let later = message(2, 7, "hello");
        broken_rules(&data, &judge, &first, MsgKind::Posted).await;
        broken_rules(&data, &judge, &later, MsgKind::Posted).await;
        assert_eq!(judge.calls().len(), 1);

        let first = message(1, 7, "bah humbug");
        let later = message(2, 7, "bah humbug");
        assert!(
            broken_rules(&data, &judge, &later, MsgKind::Edited)
                .await
                .is_empty()
        );
        assert_eq!(
            broken_rules(&data, &judge, &first, MsgKind::Edited).await,
            vec!["first_message"]
        );
        assert_eq!(judge.calls().len(), 2);
    }

    #[tokio::test]
    async fn an_edit_the_judge_cant_answer_leaves_the_day_claimed() {
        let data = data(&["first_message"]).await;
        let up = FakeJudge::answering(0.9);
        broken_rules(
            &data,
            &up,
            &message(1, 7, "Merry Christmas!"),
            MsgKind::Posted,
        )
        .await;
        let down = FakeJudge::failing();
        broken_rules(&data, &down, &message(1, 7, "meh"), MsgKind::Edited).await;
        // Their next message is still not their first of the day.
        assert!(
            broken_rules(&data, &up, &message(2, 7, "hi"), MsgKind::Posted)
                .await
                .is_empty()
        );
        assert_eq!(up.calls().len(), 1);
    }

    #[tokio::test]
    async fn sign_off_rejudges_every_edit() {
        let data = data(&["sign_off"]).await;
        let judge = FakeJudge::answering(0.1);
        for _ in 0..2 {
            assert_eq!(
                broken_rules(&data, &judge, &message(1, 7, "hello"), MsgKind::Edited).await,
                vec!["sign_off"]
            );
        }
        assert_eq!(judge.calls().len(), 2);
    }
}
