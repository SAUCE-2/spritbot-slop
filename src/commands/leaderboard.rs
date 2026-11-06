use std::fmt::Write as _;

use anyhow::{Context as _, Result, bail};
use poise::CreateReply;
use poise::serenity_prelude::{self as serenity, CreateAttachment, CreateEmbed};

use crate::app::{Ctx, Data};
use crate::stats::{self, Board, Period};

const COLOUR: u32 = 0xf1c40f;
/// Discord allows 6000 characters per embed; stay comfortably under.
const EMBED_BUDGET: usize = 5000;
const FIELD_MAX: usize = 1024;

fn guild(ctx: Ctx<'_>) -> Result<serenity::GuildId> {
    ctx.guild_id().context("use this in a server")
}

fn period(data: &Data, s: Option<&str>) -> Result<Period> {
    let s = s.unwrap_or_default();
    Period::parse(s, data.today()).with_context(|| {
        format!("couldn't understand the month `{s}` (try `2026-12`, `december`, `last` or `all`)")
    })
}

async fn autocomplete_board(ctx: Ctx<'_>, partial: &str) -> serenity::CreateAutocompleteResponse {
    let partial = partial.to_lowercase();
    let choices = stats::all_boards(&ctx.data().rules)
        .into_iter()
        .filter(|b| b.id.contains(&partial) || b.title.to_lowercase().contains(&partial))
        .take(25)
        .map(|b| {
            let mut name = b.title.clone();
            if let Some(r) = b.rule_id {
                name = format!("{name} · {r}");
            }
            name.truncate(100);
            serenity::AutocompleteChoice::new(name, b.id)
        })
        .collect();
    serenity::CreateAutocompleteResponse::new().set_choices(choices)
}

/// Monthly leaderboards: who's been naughty, who's been nice.
#[poise::command(
    slash_command,
    guild_only,
    subcommands("show", "post", "me", "export", "wipe"),
    subcommand_required
)]
pub async fn leaderboard(_ctx: Ctx<'_>) -> Result<()> {
    Ok(())
}

/// Peek at the leaderboards (only you see it).
#[poise::command(slash_command, required_permissions = "ADMINISTRATOR")]
async fn show(
    ctx: Ctx<'_>,
    #[description = "Month: 2026-12, december, last, all (default: this month)"] month: Option<
        String,
    >,
    #[description = "Show the top 10 of one board instead of the overview"]
    #[autocomplete = "autocomplete_board"]
    board: Option<String>,
) -> Result<()> {
    send(ctx, month, board, true).await
}

/// Post the leaderboards in this channel for everyone to see.
#[poise::command(slash_command, required_permissions = "ADMINISTRATOR")]
async fn post(
    ctx: Ctx<'_>,
    #[description = "Month: 2026-12, december, last, all (default: this month)"] month: Option<
        String,
    >,
    #[description = "Post the top 10 of one board instead of the overview"]
    #[autocomplete = "autocomplete_board"]
    board: Option<String>,
) -> Result<()> {
    send(ctx, month, board, false).await
}

async fn send(
    ctx: Ctx<'_>,
    month: Option<String>,
    board: Option<String>,
    ephemeral: bool,
) -> Result<()> {
    let guild_id = guild(ctx)?;
    let data = ctx.data();
    let period = period(data, month.as_deref())?;
    if ephemeral {
        ctx.defer_ephemeral().await.ok();
    } else {
        ctx.defer().await.ok();
    }
    let embeds = match board {
        Some(id) => {
            let boards = stats::all_boards(&data.rules);
            let Some(b) = boards.iter().find(|b| b.id == id) else {
                bail!("no board called `{id}`")
            };
            vec![single(data, guild_id, period, b).await?]
        }
        None => overview(data, guild_id, period).await?,
    };
    for embed in embeds {
        ctx.send(
            CreateReply::default()
                .embed(embed)
                .ephemeral(ephemeral)
                .allowed_mentions(serenity::CreateAllowedMentions::new()),
        )
        .await?;
    }
    Ok(())
}

async fn single(
    data: &Data,
    guild_id: serenity::GuildId,
    period: Period,
    b: &Board,
) -> Result<CreateEmbed> {
    let entries = data
        .db
        .board(guild_id.get(), period, b, None, Some(10))
        .await?;
    let mut title = b.title.clone();
    if let Some(r) = b.rule_id.and_then(|r| data.rule(r)) {
        title = format!("{title} · {}", r.name());
    }
    Ok(CreateEmbed::new()
        .title(title)
        .description(truncate(stats::format_entries(b, &entries), 4000))
        .footer(serenity::CreateEmbedFooter::new(period.label()))
        .colour(COLOUR))
}

/// Top 3 of every global board, then one field per rule that saw any action.
async fn overview(
    data: &Data,
    guild_id: serenity::GuildId,
    period: Period,
) -> Result<Vec<CreateEmbed>> {
    let g = guild_id.get();
    let mut fields: Vec<(String, String)> = Vec::new();
    for b in stats::global_boards() {
        let entries = data.db.board(g, period, &b, None, Some(3)).await?;
        if !entries.is_empty() {
            fields.push((b.title.clone(), stats::format_entries(&b, &entries)));
        }
    }
    for rule in &data.rules {
        if !data.db.rule_has_stats(g, period, rule.id()).await? {
            continue;
        }
        let mut body = String::new();
        for b in stats::rule_boards(rule.as_ref()) {
            let top = data.db.board(g, period, &b, None, Some(1)).await?;
            if let Some(e) = top.first() {
                let _ = writeln!(
                    body,
                    "**{}**: <@{}> {}",
                    b.title,
                    e.user_id,
                    stats::format_value(&b, e)
                );
            }
        }
        if !body.is_empty() {
            fields.push((format!("📜 {} (`{}`)", rule.name(), rule.id()), body));
        }
    }

    let title = format!("🏆 Spirit Leaderboard · {}", period.label());
    if fields.is_empty() {
        return Ok(vec![
            CreateEmbed::new()
                .title(title)
                .description("Nothing recorded yet. Get festive! 🎄")
                .colour(COLOUR),
        ]);
    }
    // Split across embeds to stay within Discord's limits.
    let mut embeds = Vec::new();
    let (mut cur, mut len, mut n) = (CreateEmbed::new().title(&title).colour(COLOUR), 0, 0);
    for (name, value) in fields {
        let value = truncate(value, FIELD_MAX);
        let size = name.len() + value.len();
        if n == 25 || len + size > EMBED_BUDGET {
            embeds.push(cur);
            cur = CreateEmbed::new()
                .title(format!("{title} (cont.)"))
                .colour(COLOUR);
            (len, n) = (0, 0);
        }
        cur = cur.field(name, value, false);
        len += size;
        n += 1;
    }
    embeds.push(cur);
    Ok(embeds)
}

fn truncate(mut s: String, max: usize) -> String {
    if s.len() > max {
        let mut cut = max - 3;
        while !s.is_char_boundary(cut) {
            cut -= 1;
        }
        s.truncate(cut);
        s.push('…');
    }
    s
}

/// Where you (or someone else) stand on every board.
#[poise::command(slash_command)]
async fn me(
    ctx: Ctx<'_>,
    #[description = "Someone else (default: you)"] user: Option<serenity::User>,
    #[description = "Month: 2026-12, december, last, all (default: this month)"] month: Option<
        String,
    >,
) -> Result<()> {
    let guild_id = guild(ctx)?;
    let data = ctx.data();
    let period = period(data, month.as_deref())?;
    let user = user.unwrap_or_else(|| ctx.author().clone());
    ctx.defer_ephemeral().await.ok();
    let mut text = String::new();
    for b in stats::all_boards(&data.rules) {
        let where_ = match b.rule_id.and_then(|r| data.rule(r)) {
            Some(r) => format!(" · {}", r.name()),
            None => String::new(),
        };
        if b.is_quote() {
            // Quote boards: their own standout moment.
            let mine = data
                .db
                .board(guild_id.get(), period, &b, Some(user.id.get()), Some(1))
                .await?;
            if let Some(e) = mine.first() {
                let _ = writeln!(
                    text,
                    "**{}**{where_}: {}",
                    b.title,
                    stats::format_value(&b, e)
                );
            }
            continue;
        }
        let all = data
            .db
            .board(guild_id.get(), period, &b, None, None)
            .await?;
        if let Some(i) = all.iter().position(|e| e.user_id == user.id.get()) {
            let _ = writeln!(
                text,
                "**{}**{where_}: #{} of {} · {}",
                b.title,
                i + 1,
                all.len(),
                stats::format_value(&b, &all[i])
            );
        }
    }
    if text.is_empty() {
        text = "Nothing recorded yet.".into();
    }
    let embed = CreateEmbed::new()
        .title(format!("🎁 {} · {}", user.name, period.label()))
        .description(truncate(text, 4000))
        .colour(COLOUR);
    ctx.send(CreateReply::default().embed(embed).ephemeral(true))
        .await?;
    Ok(())
}

/// Download every recorded event as a CSV, to build your own leaderboard.
#[poise::command(slash_command, required_permissions = "ADMINISTRATOR")]
async fn export(
    ctx: Ctx<'_>,
    #[description = "Month: 2026-12, december, last, all (default: this month)"] month: Option<
        String,
    >,
) -> Result<()> {
    let guild_id = guild(ctx)?;
    let data = ctx.data();
    let period = period(data, month.as_deref())?;
    ctx.defer_ephemeral().await.ok();
    let rows = data.db.stat_events(guild_id.get(), period).await?;
    let names: std::collections::HashMap<u64, String> = ctx
        .cache()
        .guild(guild_id)
        .map(|g| {
            g.members
                .iter()
                .map(|(id, m)| (id.get(), m.display_name().to_owned()))
                .collect()
        })
        .unwrap_or_default();
    let csv = stats::to_csv(&rows, |id| names.get(&id).cloned().unwrap_or_default());
    let file =
        CreateAttachment::bytes(csv.into_bytes(), format!("spiritbot-{}.csv", period.slug()));
    ctx.send(
        CreateReply::default()
            .content(format!("{} events ({}).", rows.len(), period.label()))
            .attachment(file)
            .ephemeral(true),
    )
    .await?;
    Ok(())
}

/// Delete recorded stats (e.g. after testing). Can't be undone.
#[poise::command(slash_command, required_permissions = "ADMINISTRATOR")]
async fn wipe(
    ctx: Ctx<'_>,
    #[description = "Must be true: this deletes stats for good"] confirm: bool,
    #[description = "Only wipe this user (default: everyone, all months)"] user: Option<
        serenity::User,
    >,
) -> Result<()> {
    let guild_id = guild(ctx)?;
    if !confirm {
        bail!("nothing deleted: set `confirm` to true to wipe stats");
    }
    let n = ctx
        .data()
        .db
        .wipe_stats(guild_id.get(), user.as_ref().map(|u| u.id.get()))
        .await?;
    let who = user.map_or("everyone".to_owned(), |u| u.name);
    ctx.send(
        CreateReply::default()
            .content(format!("🧹 Deleted {n} recorded events for {who}."))
            .ephemeral(true),
    )
    .await?;
    Ok(())
}
