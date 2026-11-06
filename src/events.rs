//! Enforces timeouts, then routes Discord events to enabled rules, only for members holding
//! the guild's enforced role.

use anyhow::Result;
use chrono::{DateTime, Utc};
use poise::serenity_prelude::{
    self as serenity, FullEvent, GuildId, Message, MessageUpdateEvent, RoleId, UserId,
};

use crate::app::Data;
use crate::punish;
use crate::rules::{Enforcement, Msg, MsgKind, RuleCx, RuleScope, judge_message};
use crate::setup;
use crate::stats::{self, Log, kind};
use serde_json::json;

pub async fn handle(ctx: &serenity::Context, event: &FullEvent, data: &Data) -> Result<()> {
    // Log everything first, whatever the rules then do with it.
    stats::capture::capture(ctx, event, data).await;
    match event {
        FullEvent::GuildCreate { guild, .. } => spawn_report(ctx, data, guild.id),
        FullEvent::GuildRoleDelete { guild_id, .. } => spawn_report(ctx, data, *guild_id),
        FullEvent::ChannelDelete { channel, .. } => spawn_report(ctx, data, channel.guild_id),
        FullEvent::Message { new_message } => {
            on_chat(ctx, data, new_message, MsgKind::Posted).await?
        }
        FullEvent::MessageUpdate {
            old_if_available,
            new,
            event,
        } => {
            if let Some(msg) = author_edit(old_if_available.as_ref(), new.as_ref(), event) {
                on_chat(ctx, data, &msg, MsgKind::Edited).await?
            }
        }
        FullEvent::VoiceStateUpdate { old, new } => {
            let Some(guild_id) = new.guild_id else {
                return Ok(());
            };
            if new.member.as_ref().is_some_and(|m| m.user.bot) {
                return Ok(());
            }
            let old_ch = old.as_ref().and_then(|o| o.channel_id);
            if old_ch == new.channel_id {
                return Ok(()); // mute/deafen/stream toggles
            }
            if let Some(ch) = old_ch {
                data.voice.user_left(guild_id, new.user_id, ch);
            }
            let (Some(ch), Some(member)) = (new.channel_id, &new.member) else {
                return Ok(());
            };
            if let Some(until) = data.db.timed_out(guild_id.get(), new.user_id.get()).await? {
                punish::disconnect(ctx, guild_id, new.user_id, "SpiritBot: timed out").await?;
                let log = Log::new(kind::TIMEOUT_VC, json!({ "until": until }))
                    .user(new.user_id)
                    .channel(ch);
                stats::record(data, guild_id, log).await;
                let dm = serenity::CreateMessage::new().content(timeout_notice(new.user_id, until));
                new.user_id.direct_message(ctx, dm).await.ok();
                return Ok(());
            }
            if !is_enforced(ctx, data, guild_id, new.user_id, Some(&member.roles)).await? {
                return Ok(());
            }
            for cx in enabled_rules(ctx, data, guild_id).await? {
                log_err(&cx, cx.rule.on_voice_join(&cx, member, ch).await);
            }
        }
        FullEvent::PresenceUpdate { new_data } => {
            let Some(guild_id) = new_data.guild_id else {
                return Ok(());
            };
            if new_data.user.bot == Some(true) {
                return Ok(());
            }
            let user = new_data.user.id;
            if !is_enforced(ctx, data, guild_id, user, None).await? {
                return Ok(());
            }
            for cx in enabled_rules(ctx, data, guild_id).await? {
                log_err(&cx, cx.rule.on_presence(&cx, user, new_data.status).await);
            }
        }
        _ => {}
    }
    Ok(())
}

fn spawn_report(ctx: &serenity::Context, data: &Data, guild_id: GuildId) {
    let (ctx, data) = (ctx.clone(), data.clone());
    tokio::spawn(async move {
        if let Err(e) = setup::report(&ctx, &data, guild_id).await {
            tracing::warn!(%guild_id, "setup report failed: {e:#}");
        }
    });
}

/// The message as it now reads, if this update is its author changing what it says (text or
/// attachments). Embed unfurls and other updates that don't change it give `None`. Without a
/// cached copy to compare against, Discord's edit timestamp says whether the author edited it.
fn author_edit(
    old: Option<&Message>,
    new: Option<&Message>,
    event: &MessageUpdateEvent,
) -> Option<Message> {
    // A partial update without the text (or, uncached, without the author) isn't an edit we can judge.
    if event.content.is_none() || (new.is_none() && event.author.is_none()) {
        return None;
    }
    let msg = new.cloned().unwrap_or_else(|| {
        let mut m = Message::default();
        event.apply_to_message(&mut m);
        m
    });
    let attachments = |m: &Message| m.attachments.iter().map(|a| a.id).collect::<Vec<_>>();
    let changed = match old {
        Some(old) => old.content != msg.content || attachments(old) != attachments(&msg),
        None => event.edited_timestamp.is_some(),
    };
    changed.then_some(msg)
}

/// A new or edited chat message: timed-out authors get it deleted, enforced members get it judged.
async fn on_chat(ctx: &serenity::Context, data: &Data, msg: &Message, kind: MsgKind) -> Result<()> {
    let Some(guild_id) = msg.guild_id else {
        return Ok(());
    };
    if msg.author.bot {
        return Ok(());
    }
    if let Some(until) = data
        .db
        .timed_out(guild_id.get(), msg.author.id.get())
        .await?
    {
        msg.delete(ctx).await.ok();
        let log = Log::new(
            kind::TIMEOUT_MESSAGE,
            json!({ "message_id": msg.id, "content": msg.content, "until": until }),
        )
        .user(msg.author.id)
        .channel(msg.channel_id);
        stats::record(data, guild_id, log).await;
        // Self-deleting, so a timed-out spammer doesn't flood the channel with notices.
        if let Ok(notice) = msg
            .channel_id
            .say(ctx, timeout_notice(msg.author.id, until))
            .await
        {
            let ctx = ctx.clone();
            tokio::spawn(async move {
                tokio::time::sleep(std::time::Duration::from_secs(10)).await;
                notice.delete(&ctx).await.ok();
            });
        }
        return Ok(());
    }
    let roles = msg.member.as_ref().map(|m| m.roles.clone());
    if !is_enforced(ctx, data, guild_id, msg.author.id, roles.as_ref()).await? {
        return Ok(());
    }

    judge_chat(ctx, data, guild_id, &Msg::new(msg, kind)).await
}

/// One judge call answers every enabled rule's check on the message; then each broken rule
/// replies and punishes, or in shadow mode only reports to the log channel.
async fn judge_chat(
    ctx: &serenity::Context,
    data: &Data,
    guild_id: GuildId,
    m: &Msg<'_>,
) -> Result<()> {
    let scopes = RuleScope::enabled(data, guild_id).await?;
    let author = m.msg.author.id;
    for breach in judge_message(data.judge.as_deref(), &scopes, m).await {
        let enforcement = breach.enforcement(author);
        if let Enforcement::Punish(_) = enforcement {
            m.msg.reply(ctx, breach.reply()).await.ok();
        }
        let cx = RuleCx {
            ctx: ctx.clone(),
            scope: breach.scope.clone(),
        };
        log_err(&cx, cx.carry_out(author, enforcement).await);
    }
    Ok(())
}

fn timeout_notice(user: UserId, until: DateTime<Utc>) -> String {
    format!(
        "⛔ <@{user}> you're timed out until <t:{0}:t> (<t:{0}:R>). An admin can end it early with `/spirit pardon`.",
        until.timestamp()
    )
}

/// Does this user hold the guild's enforced role? `roles` saves a lookup when the event has them.
async fn is_enforced(
    ctx: &serenity::Context,
    data: &Data,
    guild_id: GuildId,
    user: UserId,
    roles: Option<&Vec<RoleId>>,
) -> Result<bool> {
    let Some(role) = data.db.guild(guild_id.get()).await?.enforced_role_id else {
        return Ok(false);
    };
    let role = RoleId::new(role as u64);
    Ok(match roles {
        Some(r) => r.contains(&role),
        None => guild_id
            .member(ctx, user)
            .await
            .is_ok_and(|m| m.roles.contains(&role)),
    })
}

async fn enabled_rules(
    ctx: &serenity::Context,
    data: &Data,
    guild_id: GuildId,
) -> Result<Vec<RuleCx>> {
    Ok(RuleScope::enabled(data, guild_id)
        .await?
        .into_iter()
        .map(|scope| RuleCx {
            ctx: ctx.clone(),
            scope,
        })
        .collect())
}

fn log_err(cx: &RuleCx, r: Result<()>) {
    if let Err(e) = r {
        tracing::warn!(guild_id = %cx.guild_id, rule = cx.rule.id(), "rule error: {e:#}");
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn update(fields: serde_json::Value) -> MessageUpdateEvent {
        let mut event = json!({ "id": "5", "channel_id": "6", "guild_id": "1" });
        event
            .as_object_mut()
            .unwrap()
            .extend(fields.as_object().unwrap().clone());
        serde_json::from_value(event).unwrap()
    }

    fn author() -> serde_json::Value {
        json!({ "id": "7", "username": "elf", "discriminator": "0", "avatar": null })
    }

    fn cached(content: &str) -> Message {
        let mut m = Message::default();
        m.content = content.into();
        m
    }

    #[test]
    fn a_changed_text_is_an_edit() {
        let event = update(json!({ "content": "bah humbug", "author": author() }));
        let (old, new) = (cached("Merry Christmas!"), cached("bah humbug"));
        let msg = author_edit(Some(&old), Some(&new), &event).unwrap();
        assert_eq!(msg.content, "bah humbug");
    }

    #[test]
    fn an_unfurl_that_keeps_the_text_is_not_an_edit() {
        let event = update(json!({
            "content": "see https://x.y", "author": author(),
            "edited_timestamp": "2026-12-25T10:00:00Z"
        }));
        let (old, new) = (cached("see https://x.y"), cached("see https://x.y"));
        assert!(author_edit(Some(&old), Some(&new), &event).is_none());
    }

    #[test]
    fn a_partial_update_without_text_is_not_an_edit() {
        let ts = "2026-12-25T10:00:00Z";
        let no_text = update(json!({ "embeds": [], "author": author(), "edited_timestamp": ts }));
        assert!(author_edit(None, None, &no_text).is_none());
        let no_author = update(json!({ "content": "hi", "edited_timestamp": ts }));
        assert!(author_edit(None, None, &no_author).is_none());
    }

    #[test]
    fn uncached_messages_trust_the_edit_timestamp() {
        let edited = update(json!({
            "content": "bah humbug", "author": author(),
            "edited_timestamp": "2026-12-25T10:00:00Z"
        }));
        let msg = author_edit(None, None, &edited).unwrap();
        assert_eq!(
            (msg.id.get(), msg.author.id.get(), msg.content.as_str()),
            (5, 7, "bah humbug")
        );
        assert_eq!(msg.guild_id, Some(GuildId::new(1)));

        let unfurled = update(json!({ "content": "see https://x.y", "author": author() }));
        assert!(author_edit(None, None, &unfurled).is_none());
    }
}
