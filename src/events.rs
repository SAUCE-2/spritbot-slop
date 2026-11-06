//! Enforces timeouts, then routes Discord events to enabled rules, only for members holding
//! the guild's enforced role.

use anyhow::Result;
use chrono::{DateTime, Timelike, Utc};
use poise::serenity_prelude::{self as serenity, FullEvent, GuildId, Message, RoleId, UserId};

use crate::app::Data;
use crate::punish;
use crate::rules::{Msg, RuleCx};
use crate::setup;
use crate::stats::{self, Event, activity, kind};

pub async fn handle(ctx: &serenity::Context, event: &FullEvent, data: &Data) -> Result<()> {
    match event {
        FullEvent::GuildCreate { guild, .. } => {
            // Pick up people already in VC (e.g. after a restart) so their time counts.
            for (user, vs) in &guild.voice_states {
                if vs.channel_id.is_some() && !vs.member.as_ref().is_some_and(|m| m.user.bot) {
                    data.vc_sessions.start(guild.id, *user);
                }
            }
            spawn_report(ctx, data, guild.id)
        }
        FullEvent::GuildRoleDelete { guild_id, .. } => spawn_report(ctx, data, *guild_id),
        FullEvent::ChannelDelete { channel, .. } => spawn_report(ctx, data, channel.guild_id),
        FullEvent::Message { new_message } => on_message(ctx, data, new_message).await?,
        FullEvent::VoiceStateUpdate { old, new } => on_voice(ctx, data, old.as_ref(), new).await?,
        FullEvent::ReactionAdd { add_reaction } => on_reaction(ctx, data, add_reaction).await?,
        FullEvent::MessageUpdate { event, .. } => {
            // Embeds unfurling also send updates; only count real edits.
            let (Some(guild_id), Some(author)) = (event.guild_id, &event.author) else {
                return Ok(());
            };
            if author.bot || event.edited_timestamp.is_none() || event.content.is_none() {
                return Ok(());
            }
            if is_enforced(ctx, data, guild_id, author.id, None).await? {
                stats::record(data, guild_id, author.id, Event::new(kind::EDIT)).await;
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

async fn on_voice(
    ctx: &serenity::Context,
    data: &Data,
    old: Option<&serenity::VoiceState>,
    new: &serenity::VoiceState,
) -> Result<()> {
    let Some(guild_id) = new.guild_id else {
        return Ok(());
    };
    if new.member.as_ref().is_some_and(|m| m.user.bot) {
        return Ok(());
    }
    let user = new.user_id;
    let roles = new.member.as_ref().map(|m| &m.roles);
    let old_ch = old.and_then(|o| o.channel_id);
    if old_ch == new.channel_id {
        // Mute/deafen/stream/camera toggles.
        if let Some(old) = old
            && new.channel_id.is_some()
        {
            let toggles = activity::voice_toggles(old.into(), new.into());
            if !toggles.is_empty() && is_enforced(ctx, data, guild_id, user, roles).await? {
                for ev in toggles {
                    stats::record(data, guild_id, user, ev).await;
                }
            }
        }
        return Ok(());
    }
    if let Some(ch) = old_ch {
        data.voice.user_left(guild_id, user, ch);
    }
    if old_ch.is_some() && new.channel_id.is_none() {
        if let Some(spent) = data.vc_sessions.end(guild_id, user)
            && is_enforced(ctx, data, guild_id, user, roles).await?
        {
            for ev in activity::vc_leave_events(spent) {
                stats::record(data, guild_id, user, ev).await;
            }
        }
        return Ok(());
    }
    if old_ch.is_some() && is_enforced(ctx, data, guild_id, user, roles).await? {
        stats::record(data, guild_id, user, Event::new(kind::VC_HOP)).await;
    }
    let (Some(ch), Some(member)) = (new.channel_id, &new.member) else {
        return Ok(());
    };
    if let Some(until) = data.db.timed_out(guild_id.get(), user.get()).await? {
        punish::disconnect(ctx, guild_id, user, "SpiritBot: timed out").await?;
        stats::record(data, guild_id, user, Event::new(kind::TIMEOUT_VC)).await;
        let dm = serenity::CreateMessage::new().content(timeout_notice(user, until));
        user.direct_message(ctx, dm).await.ok();
        return Ok(());
    }
    if !is_enforced(ctx, data, guild_id, user, Some(&member.roles)).await? {
        return Ok(());
    }
    data.vc_sessions.start(guild_id, user);
    if old_ch.is_none() {
        stats::record(data, guild_id, user, Event::new(kind::VC_JOIN)).await;
    }
    for cx in enabled_rules(ctx, data, guild_id).await? {
        log_err(&cx, cx.rule.on_voice_join(&cx, member, ch).await);
    }
    Ok(())
}

async fn on_reaction(ctx: &serenity::Context, data: &Data, r: &serenity::Reaction) -> Result<()> {
    let (Some(guild_id), Some(user)) = (r.guild_id, r.user_id) else {
        return Ok(());
    };
    if user == ctx.cache.current_user().id || r.member.as_ref().is_some_and(|m| m.user.bot) {
        return Ok(());
    }
    let roles = r.member.as_ref().map(|m| &m.roles);
    if is_enforced(ctx, data, guild_id, user, roles).await? {
        stats::record(data, guild_id, user, Event::new(kind::REACTION_GIVEN)).await;
        if activity::is_festive_reaction(&r.emoji) {
            stats::record(data, guild_id, user, Event::new(kind::FESTIVE_REACTION)).await;
        }
    }
    if let Some(author) = r.message_author_id
        && author != user
        && author != ctx.cache.current_user().id
        && is_enforced(ctx, data, guild_id, author, None).await?
    {
        stats::record(data, guild_id, author, Event::new(kind::REACTION_RECEIVED)).await;
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
        let ev = Event::new(kind::TIMEOUT_MESSAGE).text(msg.content.clone());
        stats::record(data, guild_id, msg.author.id, ev).await;
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

    let hour = Utc::now().with_timezone(&data.cfg.tz).hour();
    for (user, ev) in activity::message_events(msg, hour) {
        // Pings and replies credit other people: only count those the rules apply to.
        if user == msg.author.id || is_enforced(ctx, data, guild_id, user, None).await? {
            stats::record(data, guild_id, user, ev).await;
        }
    }
    let m = Msg::new(msg);

    for cx in enabled_rules(ctx, data, guild_id).await? {
        log_err(&cx, cx.rule.on_message(&cx, &m).await);
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
    Ok(data
        .db
        .enabled_rules(guild_id.get())
        .await?
        .into_iter()
        .filter_map(|row| {
            let rule = data.rule(&row.rule_id)?;
            Some(RuleCx {
                ctx: ctx.clone(),
                data: data.clone(),
                guild_id,
                row,
                rule,
            })
        })
        .collect())
}

fn log_err(cx: &RuleCx, r: Result<()>) {
    if let Err(e) = r {
        tracing::warn!(guild_id = %cx.guild_id, rule = cx.rule.id(), "rule error: {e:#}");
    }
}
