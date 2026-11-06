//! Guild config health check: tells admins what's missing and how to fix it.

use anyhow::Result;
use poise::serenity_prelude::{
    self as serenity, ChannelId, ChannelType, CreateEmbed, CreateMessage, GuildId, RoleId,
};

use crate::app::Data;
use crate::rules::missing_requirements;

/// Everything wrong with this guild's setup, each with the fix.
pub async fn problems(
    ctx: &serenity::Context,
    data: &Data,
    guild_id: GuildId,
) -> Result<Vec<String>> {
    let gcfg = data.db.guild(guild_id.get()).await?;
    let bot_id = ctx.cache.current_user().id;
    let bot_member = guild_id.member(ctx, bot_id).await.ok();
    let mut out = Vec::new();

    {
        let Some(g) = ctx.cache.guild(guild_id) else {
            return Ok(vec![
                "Guild isn't cached yet; try again in a moment.".into(),
            ]);
        };
        let role = gcfg.enforced_role_id.map(|r| RoleId::new(r as u64));
        match role {
            None => out.push("No enforced role set, so nobody is subject to the rules → `/spirit setup role:@Role`".into()),
            Some(r) if !g.roles.contains_key(&r) => {
                out.push("The enforced role was deleted → `/spirit setup role:@Role`".into())
            }
            _ => {}
        }
        for (id, what, fix) in [
            (
                gcfg.announce_channel_id,
                "announce channel (new rules are announced there)",
                "announce_channel",
            ),
            (
                gcfg.log_channel_id,
                "log channel (punishments + setup notices)",
                "log_channel",
            ),
        ] {
            match id {
                None => out.push(format!("No {what} → `/spirit setup {fix}:#channel`")),
                Some(c) if !g.channels.contains_key(&ChannelId::new(c as u64)) => out.push(
                    format!("The {what} was deleted → `/spirit setup {fix}:#channel`"),
                ),
                _ => {}
            }
        }

        if let Some(me) = &bot_member
            && !g.member_permissions(me).administrator()
        {
            out.push(
                "I need **Administrator** → give my role it in Server Settings → Roles".into(),
            );
        }
    }

    if data.judge.is_none() {
        out.push("`JEV_API_KEY` isn't set, so rules that need Jev can't go live".into());
    }

    for row in data.db.rules_for_guild(guild_id.get()).await? {
        if !row.enabled && row.start_date.is_none() {
            continue;
        }
        if let Some(rule) = data.rule(&row.rule_id) {
            let missing = missing_requirements(data, rule.as_ref());
            if !missing.is_empty() && !row.enabled {
                out.push(format!(
                    "Scheduled rule `{}` will **not** go live until fixed:",
                    rule.id()
                ));
            }
            out.extend(missing);
        }
    }
    Ok(out)
}

pub fn embed(problems: &[String]) -> CreateEmbed {
    if problems.is_empty() {
        return CreateEmbed::new()
            .title("✅ SpiritBot is fully set up")
            .colour(0x2ecc71);
    }
    let body: String = problems.iter().map(|p| format!("• {p}\n")).collect();
    CreateEmbed::new()
        .title("🎅 SpiritBot needs some setup")
        .description(format!(
            "{body}\nAdmins: run `/spirit setup` any time to see this checklist."
        ))
        .colour(0xe74c3c)
}

/// Run the check and post it if the set of problems changed since the last report.
pub async fn report(ctx: &serenity::Context, data: &Data, guild_id: GuildId) -> Result<()> {
    let problems = problems(ctx, data, guild_id).await?;
    let fingerprint = problems.join("\n");
    let last = data.db.guild(guild_id.get()).await?.last_setup_hash;
    let last_was_ok = last.as_deref().is_none_or(str::is_empty);
    if last.as_deref() == Some(fingerprint.as_str()) || (problems.is_empty() && last_was_ok) {
        return Ok(());
    }
    data.db
        .set_setup_hash(guild_id.get(), Some(&fingerprint))
        .await?;
    send(
        ctx,
        data,
        guild_id,
        CreateMessage::new().embed(embed(&problems)),
    )
    .await;
    Ok(())
}

/// Log channel → system channel → first channel I can write in → DM the owner.
async fn send(ctx: &serenity::Context, data: &Data, guild_id: GuildId, msg: CreateMessage) {
    let log = data
        .db
        .guild(guild_id.get())
        .await
        .ok()
        .and_then(|g| g.log_channel_id);
    let (candidates, owner) = {
        let Some(g) = ctx.cache.guild(guild_id) else {
            return;
        };
        let me = g.members.get(&ctx.cache.current_user().id);
        let mut text: Vec<_> = g
            .channels
            .values()
            .filter(|c| c.kind == ChannelType::Text)
            .collect();
        text.sort_by_key(|c| c.position);
        let writable = text
            .into_iter()
            .filter(|c| me.is_some_and(|m| g.user_permissions_in(c, m).send_messages()))
            .map(|c| c.id);
        let candidates: Vec<ChannelId> = log
            .map(|c| ChannelId::new(c as u64))
            .into_iter()
            .chain(g.system_channel_id)
            .chain(writable)
            .collect();
        (candidates, g.owner_id)
    };
    for ch in candidates {
        if ch.send_message(ctx, msg.clone()).await.is_ok() {
            return;
        }
    }
    if let Err(e) = owner.direct_message(ctx, msg).await {
        tracing::warn!(%guild_id, "couldn't deliver setup notice anywhere: {e}");
    }
}
