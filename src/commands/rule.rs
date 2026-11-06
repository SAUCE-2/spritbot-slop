use std::fmt::Write as _;

use anyhow::{Context as _, Result, bail};
use chrono::{Datelike, Duration, NaiveDate};
use poise::CreateReply;
use poise::serenity_prelude::{self as serenity, CreateEmbed};

use crate::app::{Ctx, Data};
use crate::punish::{Punishment, parse_duration};
use crate::scheduler;

async fn autocomplete_rule(ctx: Ctx<'_>, partial: &str) -> serenity::CreateAutocompleteResponse {
    let choices = ctx
        .data()
        .rules
        .iter()
        .filter(|r| {
            r.id().contains(partial) || r.name().to_lowercase().contains(&partial.to_lowercase())
        })
        .map(|r| serenity::AutocompleteChoice::new(format!("{} ({})", r.name(), r.id()), r.id()))
        .collect();
    serenity::CreateAutocompleteResponse::new().set_choices(choices)
}

fn guild(ctx: Ctx<'_>) -> Result<serenity::GuildId> {
    ctx.guild_id().context("use this in a server")
}

fn get_rule(data: &Data, id: &str) -> Result<std::sync::Arc<dyn crate::rules::Rule>> {
    data.rule(id)
        .with_context(|| format!("no rule called `{id}` (see `/rule list`)"))
}

async fn reply(ctx: Ctx<'_>, text: impl Into<String>) -> Result<()> {
    ctx.send(CreateReply::default().content(text).ephemeral(true))
        .await?;
    Ok(())
}

/// Manage the christmas rules.
#[poise::command(
    slash_command,
    guild_only,
    subcommands(
        "list",
        "enable",
        "disable",
        "schedule",
        "unschedule",
        "punishment",
        "reset"
    ),
    subcommand_required
)]
pub async fn rule(_ctx: Ctx<'_>) -> Result<()> {
    Ok(())
}

/// Show every rule, whether it's on, and its settings.
#[poise::command(slash_command)]
async fn list(ctx: Ctx<'_>) -> Result<()> {
    let guild_id = guild(ctx)?;
    let data = ctx.data();
    let mut embed = CreateEmbed::new()
        .title("🎄 Christmas rules")
        .colour(0x27ae60);
    for rule in &data.rules {
        let row = data.db.rule(guild_id.get(), rule.id()).await?;
        let status = match (row.enabled, row.start_date) {
            (true, _) => "🟢 active".to_owned(),
            (false, Some(d)) => format!("📅 starts {d}"),
            _ => "⚫ off".to_owned(),
        };
        let punishment = row.punishment.unwrap_or_else(|| rule.default_punishment());
        let body = format!(
            "{status} · punishment: **{punishment}**
{}",
            rule.description()
        );
        embed = embed.field(format!("{} (`{}`)", rule.name(), rule.id()), body, false);
    }
    ctx.send(CreateReply::default().embed(embed)).await?;
    Ok(())
}

/// Turn a rule on now (and announce it).
#[poise::command(slash_command, required_permissions = "ADMINISTRATOR")]
async fn enable(ctx: Ctx<'_>, #[autocomplete = "autocomplete_rule"] rule: String) -> Result<()> {
    let guild_id = guild(ctx)?;
    let data = ctx.data();
    let r = get_rule(data, &rule)?;
    let text = match scheduler::activate(ctx.serenity_context(), data, guild_id, &r).await? {
        Ok(()) => format!("🟢 `{rule}` is live and announced."),
        Err(missing) => format!("Can't enable `{rule}` yet:\n• {}", missing.join("\n• ")),
    };
    reply(ctx, text).await
}

/// Turn a rule off (also cancels its schedule).
#[poise::command(slash_command, required_permissions = "ADMINISTRATOR")]
async fn disable(ctx: Ctx<'_>, #[autocomplete = "autocomplete_rule"] rule: String) -> Result<()> {
    let guild_id = guild(ctx)?;
    let data = ctx.data();
    get_rule(data, &rule)?;
    let mut row = data.db.rule(guild_id.get(), &rule).await?;
    row.enabled = false;
    row.start_date = None;
    data.db.save_rule(&row).await?;
    let by = ctx.author().id.to_string();
    let mut log = crate::stats::Log::new(
        crate::stats::kind::RULE_DISABLED,
        serde_json::json!({ "by": by }),
    );
    log.rule = data.rule(&rule).map(|r| r.id());
    crate::stats::record(data, guild_id, log).await;
    reply(ctx, format!("⚫ `{rule}` is off.")).await
}

/// Parse "2026-12-05", "today", "tomorrow", "+3" (days from now) or "5" (5th of December).
pub fn parse_date(s: &str, today: NaiveDate) -> Option<NaiveDate> {
    let s = s.trim().to_lowercase();
    match s.as_str() {
        "today" => return Some(today),
        "tomorrow" => return Some(today + Duration::days(1)),
        _ => {}
    }
    if let Some(n) = s.strip_prefix('+') {
        return Some(today + Duration::days(n.parse().ok()?));
    }
    if let Ok(day) = s.trim_start_matches("dec").trim().parse::<u32>() {
        return NaiveDate::from_ymd_opt(today.year(), 12, day);
    }
    s.parse().ok()
}

/// Make a rule go live (and be announced) automatically on a given day.
#[poise::command(slash_command, required_permissions = "ADMINISTRATOR")]
async fn schedule(
    ctx: Ctx<'_>,
    #[autocomplete = "autocomplete_rule"] rule: String,
    #[description = "YYYY-MM-DD, today, tomorrow, +N days, or a day of December like 12"]
    date: String,
) -> Result<()> {
    let guild_id = guild(ctx)?;
    let data = ctx.data();
    let r = get_rule(data, &rule)?;
    let today = data.today(guild_id).await?;
    let Some(day) = parse_date(&date, today) else {
        bail!("couldn't understand the date `{date}`")
    };
    let mut row = data.db.rule(guild_id.get(), &rule).await?;
    if row.enabled {
        bail!("`{rule}` is already live; `/rule disable` it first if you want to re-schedule");
    }
    row.start_date = Some(day);
    data.db.save_rule(&row).await?;
    let missing = crate::rules::missing_requirements(data, r.as_ref());
    let mut text = format!("📅 `{rule}` goes live on **{day}**");
    if day <= today {
        text.push_str(" (within a minute)");
    }
    if !missing.is_empty() {
        let _ = write!(
            text,
            "\n⚠️ It won't activate until fixed:\n• {}",
            missing.join("\n• ")
        );
    }
    reply(ctx, text).await
}

/// Cancel a scheduled activation.
#[poise::command(slash_command, required_permissions = "ADMINISTRATOR")]
async fn unschedule(
    ctx: Ctx<'_>,
    #[autocomplete = "autocomplete_rule"] rule: String,
) -> Result<()> {
    let guild_id = guild(ctx)?;
    let data = ctx.data();
    let mut row = data.db.rule(guild_id.get(), &rule).await?;
    row.start_date = None;
    data.db.save_rule(&row).await?;
    reply(ctx, format!("`{rule}` is no longer scheduled.")).await
}

#[derive(Debug, poise::ChoiceParameter)]
enum Kind {
    #[name = "none (shame only)"]
    None,
    Timeout,
    Disconnect,
}

/// Set what happens when someone breaks a rule.
#[poise::command(slash_command, required_permissions = "ADMINISTRATOR")]
async fn punishment(
    ctx: Ctx<'_>,
    #[autocomplete = "autocomplete_rule"] rule: String,
    kind: Kind,
    #[description = "Timeout length, e.g. 30s, 5m, 1h (timeout only)"] duration: Option<String>,
) -> Result<()> {
    let guild_id = guild(ctx)?;
    let data = ctx.data();
    get_rule(data, &rule)?;
    let p = match kind {
        Kind::None => Punishment::None,
        Kind::Disconnect => Punishment::Disconnect,
        Kind::Timeout => {
            let d = duration.as_deref().unwrap_or("60s");
            let secs =
                parse_duration(d).with_context(|| format!("couldn't understand duration `{d}`"))?;
            Punishment::Timeout { secs }
        }
    };
    let mut row = data.db.rule(guild_id.get(), &rule).await?;
    row.punishment = Some(p);
    data.db.save_rule(&row).await?;
    reply(ctx, format!("`{rule}` punishment: **{p}**")).await
}

/// Forget what a rule remembers (for one user, or everyone), so it can trigger again.
#[poise::command(slash_command, required_permissions = "ADMINISTRATOR")]
async fn reset(
    ctx: Ctx<'_>,
    #[autocomplete = "autocomplete_rule"] rule: String,
    #[description = "Only reset this user (default: everyone)"] user: Option<serenity::User>,
) -> Result<()> {
    let guild_id = guild(ctx)?;
    let data = ctx.data();
    get_rule(data, &rule)?;
    data.db
        .clear_state(guild_id.get(), &rule, user.as_ref().map(|u| u.id.get()))
        .await?;
    let who = user.map_or("everyone".to_owned(), |u| u.name);
    reply(ctx, format!("`{rule}` is reset for {who}.")).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dates() {
        let today = NaiveDate::from_ymd_opt(2026, 11, 5).unwrap();
        let d = |y, m, d| NaiveDate::from_ymd_opt(y, m, d);
        assert_eq!(parse_date("today", today), d(2026, 11, 5));
        assert_eq!(parse_date("tomorrow", today), d(2026, 11, 6));
        assert_eq!(parse_date("+3", today), d(2026, 11, 8));
        assert_eq!(parse_date("12", today), d(2026, 12, 12));
        assert_eq!(parse_date("dec 24", today), d(2026, 12, 24));
        assert_eq!(parse_date("2026-12-01", today), d(2026, 12, 1));
        assert_eq!(parse_date("nope", today), None);
        assert_eq!(parse_date("40", today), None);
    }
}
