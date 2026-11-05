use std::fmt::Write as _;

use anyhow::{Context as _, Result};
use poise::CreateReply;
use poise::serenity_prelude::{self as serenity, Channel, CreateEmbed, Role};

use crate::app::Ctx;
use crate::db::GuildField;
use crate::setup;

fn guild(ctx: Ctx<'_>) -> Result<serenity::GuildId> {
    ctx.guild_id().context("use this in a server")
}

async fn reply(ctx: Ctx<'_>, text: impl Into<String>) -> Result<()> {
    ctx.send(CreateReply::default().content(text).ephemeral(true))
        .await?;
    Ok(())
}

/// Bot setup and tools.
#[poise::command(
    slash_command,
    guild_only,
    subcommands("setup", "role", "announcechannel", "logchannel", "pardon", "status"),
    subcommand_required
)]
pub async fn spirit(_ctx: Ctx<'_>) -> Result<()> {
    Ok(())
}

/// Show the setup checklist, optionally fixing things in one go.
#[poise::command(slash_command, required_permissions = "ADMINISTRATOR")]
async fn setup(
    ctx: Ctx<'_>,
    #[description = "Role whose members must follow the rules"] role: Option<Role>,
    #[description = "Where new rules are announced"] announce_channel: Option<Channel>,
    #[description = "Where punishments and setup notices go"] log_channel: Option<Channel>,
) -> Result<()> {
    let guild_id = guild(ctx)?;
    let db = &ctx.data().db;
    if let Some(r) = role {
        db.set_guild_field(
            guild_id.get(),
            GuildField::EnforcedRole,
            Some(r.id.get() as i64),
        )
        .await?;
    }
    if let Some(c) = announce_channel {
        db.set_guild_field(
            guild_id.get(),
            GuildField::AnnounceChannel,
            Some(c.id().get() as i64),
        )
        .await?;
    }
    if let Some(c) = log_channel {
        db.set_guild_field(
            guild_id.get(),
            GuildField::LogChannel,
            Some(c.id().get() as i64),
        )
        .await?;
    }
    let problems = setup::problems(ctx.serenity_context(), ctx.data(), guild_id).await?;
    // Keep the "changed since last report" fingerprint in sync with what the admin just saw.
    db.set_setup_hash(guild_id.get(), Some(&problems.join("\n")))
        .await?;
    ctx.send(
        CreateReply::default()
            .embed(setup::embed(&problems))
            .ephemeral(true),
    )
    .await?;
    Ok(())
}

/// Set the role whose members must follow the rules.
#[poise::command(slash_command, required_permissions = "ADMINISTRATOR")]
async fn role(ctx: Ctx<'_>, role: Role) -> Result<()> {
    let guild_id = guild(ctx)?;
    ctx.data()
        .db
        .set_guild_field(
            guild_id.get(),
            GuildField::EnforcedRole,
            Some(role.id.get() as i64),
        )
        .await?;
    reply(ctx, format!("Rules now apply to <@&{}>.", role.id)).await
}

/// Set where new rules are announced.
#[poise::command(slash_command, required_permissions = "ADMINISTRATOR")]
async fn announcechannel(ctx: Ctx<'_>, channel: Channel) -> Result<()> {
    let guild_id = guild(ctx)?;
    ctx.data()
        .db
        .set_guild_field(
            guild_id.get(),
            GuildField::AnnounceChannel,
            Some(channel.id().get() as i64),
        )
        .await?;
    reply(
        ctx,
        format!("New rules will be announced in <#{}>.", channel.id()),
    )
    .await
}

/// Set where punishments and setup notices are posted.
#[poise::command(slash_command, required_permissions = "ADMINISTRATOR")]
async fn logchannel(ctx: Ctx<'_>, channel: Channel) -> Result<()> {
    let guild_id = guild(ctx)?;
    ctx.data()
        .db
        .set_guild_field(
            guild_id.get(),
            GuildField::LogChannel,
            Some(channel.id().get() as i64),
        )
        .await?;
    reply(ctx, format!("Logging to <#{}>.", channel.id())).await
}

/// End someone's timeout early.
#[poise::command(slash_command, required_permissions = "ADMINISTRATOR")]
async fn pardon(ctx: Ctx<'_>, user: serenity::User) -> Result<()> {
    let guild_id = guild(ctx)?;
    let done = ctx
        .data()
        .db
        .clear_timeout(guild_id.get(), user.id.get())
        .await?;
    reply(
        ctx,
        if done {
            format!("{} is free to go. 🎄", user.name)
        } else {
            format!("{} isn't timed out.", user.name)
        },
    )
    .await
}

/// Recent punishments.
#[poise::command(slash_command)]
async fn status(ctx: Ctx<'_>) -> Result<()> {
    let guild_id = guild(ctx)?;
    let db = &ctx.data().db;
    let mut text = String::from(
        "**Recent punishments**
",
    );
    let recent = db.recent_punishments(guild_id.get(), 10).await?;
    if recent.is_empty() {
        text.push_str("none yet");
    }
    for p in recent {
        let at = crate::db::parse_time(&p.created_at)
            .map(|t| t.timestamp())
            .unwrap_or(0);
        let _ = writeln!(
            text,
            "• <t:{at}:R> <@{}> (`{}`): {}",
            p.user_id, p.rule_id, p.action
        );
    }
    let embed = CreateEmbed::new()
        .title("🎅 Spirit status")
        .description(text)
        .colour(0xc0392b);
    ctx.send(
        CreateReply::default()
            .embed(embed)
            .allowed_mentions(serenity::CreateAllowedMentions::new()),
    )
    .await?;
    Ok(())
}
