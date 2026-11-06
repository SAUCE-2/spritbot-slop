use std::fmt::Write as _;

use anyhow::{Context as _, Result, bail};
use poise::CreateReply;
use poise::serenity_prelude::{self as serenity, CreateAttachment, CreateEmbed};

use crate::app::{Ctx, Data};
use crate::stats::{self, Board, Period, Section};

const COLOUR: u32 = 0xf1c40f;
const RULES_COLOUR: u32 = 0x16a085;
/// Discord allows 6000 characters per message (all embeds together); stay under it.
const MESSAGE_BUDGET: usize = 5500;
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
            if name.chars().count() > 100 {
                name = name.chars().take(99).collect::<String>() + "…";
            }
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
    #[description = "Only this section"] section: Option<Section>,
    #[description = "Show the top 10 of one board instead"]
    #[autocomplete = "autocomplete_board"]
    board: Option<String>,
) -> Result<()> {
    send(ctx, month, section, board, true).await
}

/// Post the leaderboards in this channel for everyone to see.
#[poise::command(slash_command, required_permissions = "ADMINISTRATOR")]
async fn post(
    ctx: Ctx<'_>,
    #[description = "Month: 2026-12, december, last, all (default: this month)"] month: Option<
        String,
    >,
    #[description = "Only this section"] section: Option<Section>,
    #[description = "Post the top 10 of one board instead"]
    #[autocomplete = "autocomplete_board"]
    board: Option<String>,
) -> Result<()> {
    send(ctx, month, section, board, false).await
}

async fn send(
    ctx: Ctx<'_>,
    month: Option<String>,
    section: Option<Section>,
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
        None => overview(data, guild_id, period, section).await?,
    };
    send_embeds(ctx, embeds, ephemeral).await
}

/// Send embeds, as few per message as Discord's size limits allow.
async fn send_embeds(ctx: Ctx<'_>, embeds: Vec<Block>, ephemeral: bool) -> Result<()> {
    let mut batch: Vec<CreateEmbed> = Vec::new();
    let mut len = 0;
    let mut batches = Vec::new();
    for (embed, size) in embeds {
        if !batch.is_empty() && (len + size > MESSAGE_BUDGET || batch.len() == 10) {
            batches.push(std::mem::take(&mut batch));
            len = 0;
        }
        batch.push(embed);
        len += size;
    }
    if !batch.is_empty() {
        batches.push(batch);
    }
    for embeds in batches {
        let mut reply = CreateReply::default()
            .ephemeral(ephemeral)
            .allowed_mentions(serenity::CreateAllowedMentions::new());
        for e in embeds {
            reply = reply.embed(e);
        }
        ctx.send(reply).await?;
    }
    Ok(())
}

/// An embed and roughly how many characters it holds.
type Block = (CreateEmbed, usize);

/// Fields as one or more embeds under `title`, within Discord's per-embed limits.
fn pack(title: &str, colour: u32, fields: Vec<(String, String)>) -> Vec<Block> {
    let mut out = Vec::new();
    let new = |cont: bool| {
        let t = if cont {
            format!("{title} (cont.)")
        } else {
            title.to_owned()
        };
        (CreateEmbed::new().title(&t).colour(colour), t.len(), 0)
    };
    let (mut cur, mut len, mut n) = new(false);
    for (name, value) in fields {
        let value = truncate(value, FIELD_MAX);
        let size = name.len() + value.len();
        if n == 25 || (n > 0 && len + size > MESSAGE_BUDGET) {
            out.push((cur, len));
            (cur, len, n) = new(true);
        }
        cur = cur.field(name, value, false);
        len += size;
        n += 1;
    }
    if n > 0 {
        out.push((cur, len));
    }
    out
}

fn board_title(data: &Data, b: &Board) -> String {
    match b.rule_id.and_then(|r| data.rule(r)) {
        Some(r) => format!("{} · {}", b.title, r.name()),
        None => b.title.clone(),
    }
}

async fn single(
    data: &Data,
    guild_id: serenity::GuildId,
    period: Period,
    b: &Board,
) -> Result<Block> {
    let entries = data
        .db
        .board(guild_id.get(), period, b, None, Some(10))
        .await?;
    let text = truncate(stats::format_entries(b, &entries), 4000);
    let size = text.len() + 200;
    let embed = CreateEmbed::new()
        .title(board_title(data, b))
        .description(text)
        .footer(serenity::CreateEmbedFooter::new(period.label()))
        .colour(b.section.colour());
    Ok((embed, size))
}

/// Server totals, then the top 3 of every board by section, then one field per rule.
async fn overview(
    data: &Data,
    guild_id: serenity::GuildId,
    period: Period,
    only: Option<Section>,
) -> Result<Vec<Block>> {
    let g = guild_id.get();
    let mut out = Vec::new();
    if only.is_none() {
        let totals = stats::format_totals(&data.db.totals(g, period).await?);
        let size = totals.len() + 100;
        out.push((
            CreateEmbed::new()
                .title(format!("🏆 Spirit Leaderboard · {}", period.label()))
                .description(totals)
                .colour(COLOUR),
            size,
        ));
    }
    let boards = stats::global_boards();
    for section in Section::ALL {
        if only.is_some_and(|s| s != section) {
            continue;
        }
        let mut fields = Vec::new();
        for b in boards.iter().filter(|b| b.section == section) {
            let entries = data.db.board(g, period, b, None, Some(3)).await?;
            if !entries.is_empty() {
                fields.push((b.title.clone(), stats::format_entries(b, &entries)));
            }
        }
        out.extend(pack(section.title(), section.colour(), fields));
    }
    if only.is_none() {
        let mut fields = Vec::new();
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
                fields.push((format!("{} (`{}`)", rule.name(), rule.id()), body));
            }
        }
        out.extend(pack("📜 Rule by Rule", RULES_COLOUR, fields));
    }
    if out.is_empty() {
        out.push((
            CreateEmbed::new()
                .title(format!("🏆 Spirit Leaderboard · {}", period.label()))
                .description("Nothing recorded in that section yet.")
                .colour(COLOUR),
            100,
        ));
    }
    Ok(out)
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

    let mut lines: Vec<(Option<Section>, String)> = Vec::new();
    for b in stats::all_boards(&data.rules) {
        let group = b.rule_id.is_none().then_some(b.section);
        let line = if b.is_quote() {
            // Quote boards: their own standout moment.
            let mine = data
                .db
                .board(guild_id.get(), period, &b, Some(user.id.get()), Some(1))
                .await?;
            mine.first()
                .map(|e| format!("**{}**: {}", b.title, stats::format_value(&b, e)))
        } else {
            let all = data
                .db
                .board(guild_id.get(), period, &b, None, None)
                .await?;
            all.iter().position(|e| e.user() == user.id.get()).map(|i| {
                let medal = ["🥇 ", "🥈 ", "🥉 "].get(i).copied().unwrap_or_default();
                format!(
                    "**{}**: {medal}#{} of {} · {}",
                    board_title(data, &b),
                    i + 1,
                    all.len(),
                    stats::format_value(&b, &all[i])
                )
            })
        };
        if let Some(l) = line {
            lines.push((group, l));
        }
    }

    let title = format!("🎁 {} · {}", user.name, period.label());
    if lines.is_empty() {
        let embed = CreateEmbed::new()
            .title(title)
            .description("Nothing recorded yet.")
            .colour(COLOUR);
        ctx.send(CreateReply::default().embed(embed).ephemeral(true))
            .await?;
        return Ok(());
    }
    let mut fields = Vec::new();
    let groups = Section::ALL.map(Some).into_iter().chain([None]);
    for group in groups {
        let mut value = String::new();
        for (_, l) in lines.iter().filter(|(g, _)| *g == group) {
            // Split long sections into several fields rather than cutting them off.
            if value.len() + l.len() + 1 > FIELD_MAX {
                fields.push((group_name(group).to_owned(), std::mem::take(&mut value)));
            }
            value.push_str(l);
            value.push('\n');
        }
        if !value.is_empty() {
            fields.push((group_name(group).to_owned(), value));
        }
    }
    send_embeds(ctx, pack(&title, COLOUR, fields), true).await
}

fn group_name(g: Option<Section>) -> &'static str {
    g.map_or("📜 Rules", Section::title)
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
