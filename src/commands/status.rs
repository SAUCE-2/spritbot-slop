use std::fmt::Write as _;

use anyhow::Result;
use chrono::Utc;
use poise::CreateReply;
use poise::serenity_prelude::CreateEmbed;

use crate::app::Ctx;

/// Bot uptime, latency and stats.
#[poise::command(slash_command)]
pub async fn status(ctx: Ctx<'_>) -> Result<()> {
    let data = ctx.data();
    let started = data.started_at;
    let uptime = fmt_duration((Utc::now() - started).num_seconds());

    let ping = ctx.ping().await;
    let ping = if ping.is_zero() {
        "measuring…".to_owned()
    } else {
        format!("{} ms", ping.as_millis())
    };

    let mut bot = String::new();
    let _ = writeln!(
        bot,
        "**Uptime:** {uptime} (since <t:{}:f>)",
        started.timestamp()
    );
    let _ = writeln!(bot, "**Gateway latency:** {ping}");
    let _ = writeln!(bot, "**Servers:** {}", ctx.cache().guild_count());
    if let Some(mb) = memory_mb() {
        let _ = writeln!(bot, "**Memory:** {mb:.1} MB");
    }
    let _ = writeln!(
        bot,
        "**Version:** {}",
        option_env!("SPIRITBOT_VERSION").unwrap_or("dev")
    );
    let _ = writeln!(bot, "**Commit:** {}", fmt_commit(option_env!("GIT_SHA")));
    let _ = write!(
        bot,
        "**Voice (Whisper):** {} · **AI (Jev):** {}",
        on_off(data.stt.is_some()),
        on_off(data.judge.is_some()),
    );

    let mut embed = CreateEmbed::new()
        .title("🤖 Bot status")
        .description(bot)
        .colour(0x27ae60);

    if let Some(guild_id) = ctx.guild_id() {
        let here = data.db.punishment_stats(guild_id.get()).await?;
        let enabled = data.db.enabled_rules(guild_id.get()).await?.len();
        embed = embed.field(
            "This server",
            format!(
                "Rules live: {enabled}/{}\nPunishments: {} ({} in 24h)\nTimed out now: {}",
                data.rules.len(),
                here.total,
                here.last_day,
                here.active_timeouts,
            ),
            true,
        );
    }

    ctx.send(CreateReply::default().embed(embed).ephemeral(true))
        .await?;
    Ok(())
}

fn on_off(b: bool) -> &'static str {
    if b { "✅" } else { "❌" }
}

/// `93784` → `1d 2h 3m 4s`, dropping leading zero units.
fn fmt_duration(secs: i64) -> String {
    let secs = secs.max(0);
    let (d, h, m, s) = (
        secs / 86_400,
        secs % 86_400 / 3_600,
        secs % 3_600 / 60,
        secs % 60,
    );
    let mut out = String::new();
    for (n, unit) in [(d, "d"), (h, "h"), (m, "m")] {
        if n > 0 || !out.is_empty() {
            let _ = write!(out, "{n}{unit} ");
        }
    }
    let _ = write!(out, "{s}s");
    out
}

/// Short SHA linked to the commit; builds without `GIT_SHA` (local `cargo`) show `unknown`.
fn fmt_commit(sha: Option<&str>) -> String {
    match sha.filter(|s| !s.is_empty()) {
        Some(sha) => format!(
            "[`{}`]({}/commit/{sha})",
            sha.get(..7).unwrap_or(sha),
            env!("CARGO_PKG_REPOSITORY")
        ),
        None => "unknown".to_owned(),
    }
}

/// Resident memory from /proc (Linux only; `None` elsewhere).
fn memory_mb() -> Option<f64> {
    let status = std::fs::read_to_string("/proc/self/status").ok()?;
    let kb: f64 = status
        .lines()
        .find_map(|l| l.strip_prefix("VmRSS:"))?
        .trim()
        .strip_suffix("kB")?
        .trim()
        .parse()
        .ok()?;
    Some(kb / 1024.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn durations() {
        assert_eq!(fmt_duration(0), "0s");
        assert_eq!(fmt_duration(59), "59s");
        assert_eq!(fmt_duration(3_600), "1h 0m 0s");
        assert_eq!(fmt_duration(93_784), "1d 2h 3m 4s");
        assert_eq!(fmt_duration(-5), "0s");
    }

    #[test]
    fn commits() {
        assert_eq!(
            fmt_commit(Some("0123456789abcdef")),
            "[`0123456`](https://github.com/SAUCE-2/spritbot-slop/commit/0123456789abcdef)"
        );
        assert_eq!(fmt_commit(Some("")), "unknown");
        assert_eq!(fmt_commit(None), "unknown");
    }
}
