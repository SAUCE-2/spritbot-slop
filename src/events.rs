//! Enforces timeouts, then routes Discord events to enabled rules, only for members holding
//! the guild's enforced role.

use anyhow::Result;
use chrono::{DateTime, Utc};
use poise::serenity_prelude::{self as serenity, FullEvent, GuildId, Message, RoleId, UserId};

use crate::app::Data;
use crate::punish;
use crate::rules::{Msg, MsgKind, RuleCx, RuleScope, judge_message};
use crate::setup;

pub async fn handle(ctx: &serenity::Context, event: &FullEvent, data: &Data) -> Result<()> {
    match event {
        FullEvent::GuildCreate { guild, .. } => spawn_report(ctx, data, guild.id),
        FullEvent::GuildRoleDelete { guild_id, .. } => spawn_report(ctx, data, *guild_id),
        FullEvent::ChannelDelete { channel, .. } => spawn_report(ctx, data, channel.guild_id),
        FullEvent::Message { new_message } => on_message(ctx, data, new_message).await?,
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

async fn on_message(ctx: &serenity::Context, data: &Data, msg: &Message) -> Result<()> {
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

    judge_chat(ctx, data, guild_id, &Msg::new(msg, MsgKind::Posted)).await
}

/// One judge call answers every enabled rule's check on the message; then each broken rule
/// replies and punishes.
async fn judge_chat(
    ctx: &serenity::Context,
    data: &Data,
    guild_id: GuildId,
    m: &Msg<'_>,
) -> Result<()> {
    let scopes = RuleScope::enabled(data, guild_id).await?;
    for breach in judge_message(data.judge.as_deref(), &scopes, m).await {
        m.msg.reply(ctx, breach.reply()).await.ok();
        let cx = RuleCx {
            ctx: ctx.clone(),
            scope: breach.scope.clone(),
        };
        let punished = cx.punish(m.msg.author.id, &breach.reason()).await;
        log_err(&cx, punished.map(drop));
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
