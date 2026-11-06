use std::fmt::Write as _;

use anyhow::{Context as _, Result, anyhow};
use chrono::{DateTime, Utc};
use chrono_tz::Tz;
use poise::CreateReply;
use poise::serenity_prelude::{self as serenity, Channel, CreateEmbed, Role};

use crate::app::Ctx;
use crate::config::local_date;
use crate::db::GuildField;
use crate::setup;

/// What `/spirit timezone` accepts to go back to the bot-wide `BOT_TZ`.
const DEFAULT_TZ: &str = "default";

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
    subcommands(
        "setup",
        "role",
        "announcechannel",
        "logchannel",
        "timezone",
        "pardon",
        "status"
    ),
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
    let tz = db.guild(guild_id.get()).await?.timezone;
    let embed = setup::embed(&problems).field(
        "Timezone",
        timezone_line(tz, ctx.data().cfg.tz, Utc::now()),
        false,
    );
    ctx.send(CreateReply::default().embed(embed).ephemeral(true))
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

/// Parse an IANA timezone name, or `default` (→ `None`) for the bot-wide `BOT_TZ`.
fn parse_timezone(name: &str) -> Result<Option<Tz>> {
    let name = name.trim();
    if name.eq_ignore_ascii_case(DEFAULT_TZ) {
        return Ok(None);
    }
    chrono_tz::TZ_VARIANTS
        .into_iter()
        .find(|tz| tz.name().eq_ignore_ascii_case(name))
        .map(Some)
        .ok_or_else(|| {
            anyhow!("`{name}` isn't a timezone I know; use an IANA name like `America/New_York`")
        })
}

/// Up to 25 (Discord's limit) timezone names containing `partial`, `default` first.
fn timezone_choices(partial: &str) -> Vec<&'static str> {
    let partial = partial.to_lowercase();
    std::iter::once(DEFAULT_TZ)
        .chain(chrono_tz::TZ_VARIANTS.iter().map(|tz| tz.name()))
        .filter(|name| name.to_lowercase().contains(&partial))
        .take(25)
        .collect()
}

/// The timezone row of the `/spirit setup` checklist.
fn timezone_line(own: Option<Tz>, default: Tz, now: DateTime<Utc>) -> String {
    let tz = own.unwrap_or(default);
    let note = if own.is_some() { "" } else { "bot default; " };
    format!(
        "{} ({note}today is {}) → `/spirit timezone`",
        tz.name(),
        local_date(now, tz)
    )
}

async fn autocomplete_timezone(
    _ctx: Ctx<'_>,
    partial: &str,
) -> serenity::CreateAutocompleteResponse {
    let choices = timezone_choices(partial)
        .into_iter()
        .map(|name| serenity::AutocompleteChoice::new(name, name))
        .collect();
    serenity::CreateAutocompleteResponse::new().set_choices(choices)
}

/// Set the timezone that decides when "today" starts for daily rules and schedules.
#[poise::command(slash_command, required_permissions = "ADMINISTRATOR")]
async fn timezone(
    ctx: Ctx<'_>,
    #[description = "IANA name like America/New_York, or \"default\" for the bot default"]
    #[autocomplete = "autocomplete_timezone"]
    timezone: String,
) -> Result<()> {
    let guild_id = guild(ctx)?;
    let tz = parse_timezone(&timezone)?;
    ctx.data().db.set_timezone(guild_id.get(), tz).await?;
    reply(
        ctx,
        format!(
            "🕰️ Timezone: {}",
            timezone_line(tz, ctx.data().cfg.tz, Utc::now())
        ),
    )
    .await
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

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    #[test]
    fn timezone_names_are_validated_case_insensitively() {
        assert_eq!(
            parse_timezone("america/new_york").unwrap(),
            Some(chrono_tz::America::New_York)
        );
        assert_eq!(parse_timezone(" default ").unwrap(), None);
        let err = parse_timezone("Mars/Olympus").unwrap_err().to_string();
        assert!(err.contains("Mars/Olympus"), "{err}");
    }

    #[test]
    fn timezone_autocomplete_matches_substrings_and_stays_within_discords_limit() {
        let all = timezone_choices("");
        assert_eq!(all.len(), 25);
        assert_eq!(all[0], "default");
        let york = timezone_choices("york");
        assert!(york.contains(&"America/New_York"), "{york:?}");
        assert!(york.iter().all(|n| n.to_lowercase().contains("york")));
    }

    #[test]
    fn setup_shows_the_timezone_and_whether_it_is_the_default() {
        let now = chrono::Utc
            .with_ymd_and_hms(2026, 11, 30, 12, 0, 0)
            .unwrap();
        let london = chrono_tz::Europe::London;
        assert_eq!(
            timezone_line(None, london, now),
            "Europe/London (bot default; today is 2026-11-30) → `/spirit timezone`"
        );
        assert_eq!(
            timezone_line(Some(chrono_tz::Pacific::Kiritimati), london, now),
            "Pacific/Kiritimati (today is 2026-12-01) → `/spirit timezone`"
        );
    }
}
