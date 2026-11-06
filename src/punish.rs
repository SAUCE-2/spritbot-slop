//! Applying punishments. Timeouts are enforced by the bot itself (see [`apply`]).

use std::fmt;

use anyhow::Result;
use chrono::{Duration, Utc};
use poise::serenity_prelude::{self as serenity, EditMember, GuildId, UserId};
use serde::{Deserialize, Serialize};

use crate::app::Data;
use crate::stats::{self, Log};
use serde_json::json;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum Punishment {
    None,
    Timeout { secs: u64 },
    Disconnect,
}

impl fmt::Display for Punishment {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Punishment::None => write!(f, "none (shame only)"),
            Punishment::Timeout { secs } => write!(f, "timeout {}", format_duration(*secs)),
            Punishment::Disconnect => write!(f, "disconnect from VC"),
        }
    }
}

/// Parse "90", "90s", "5m", "1h30m", "2d" into seconds.
pub fn parse_duration(s: &str) -> Option<u64> {
    let s = s.trim().to_lowercase();
    if s.is_empty() {
        return None;
    }
    if let Ok(n) = s.parse::<u64>() {
        return Some(n);
    }
    let (mut total, mut num) = (0u64, String::new());
    for c in s.chars() {
        if c.is_ascii_digit() {
            num.push(c);
            continue;
        }
        let n: u64 = num.parse().ok()?;
        num.clear();
        total += n * match c {
            's' => 1,
            'm' => 60,
            'h' => 3600,
            'd' => 86400,
            _ => return None,
        };
    }
    num.is_empty().then_some(total)
}

pub fn format_duration(secs: u64) -> String {
    let (d, h, m, s) = (secs / 86400, secs / 3600 % 24, secs / 60 % 60, secs % 60);
    let parts: Vec<String> = [(d, "d"), (h, "h"), (m, "m"), (s, "s")]
        .into_iter()
        .filter(|(n, _)| *n > 0)
        .map(|(n, u)| format!("{n}{u}"))
        .collect();
    if parts.is_empty() {
        "0s".into()
    } else {
        parts.concat()
    }
}

/// Apply `p` to `user_id`, post the shaming message, and record it. Returns a summary of what happened.
pub async fn punish(
    ctx: &serenity::Context,
    data: &Data,
    guild_id: GuildId,
    user_id: UserId,
    rule_id: &str,
    p: Punishment,
    reason: &str,
) -> Result<String> {
    let action = match apply(ctx, data, guild_id, user_id, p, reason).await {
        Ok(a) => a,
        Err(e) => {
            tracing::warn!(%guild_id, %user_id, rule_id, "punishment failed: {e:#}");
            format!("tried to {p} but failed: {e}")
        }
    };
    data.db
        .log_punishment(guild_id.get(), rule_id, user_id.get(), &action, reason)
        .await?;
    let secs = match p {
        Punishment::Timeout { secs } => secs,
        _ => 0,
    };
    let mut log = Log::new(
        stats::kind::PUNISHED,
        json!({ "punishment": p, "action": action, "reason": reason, "timeout_secs": secs }),
    )
    .user(user_id)
    .enforced(Some(true));
    log.rule = data.rule(rule_id).map(|r| r.id());
    stats::record(data, guild_id, log).await;
    post_log(
        ctx,
        data,
        guild_id,
        format!("🎅 **Naughty list!** <@{user_id}> broke **{rule_id}**: {reason}\n→ {action}"),
    )
    .await;
    Ok(action)
}

/// Timeouts are tracked by the bot, not Discord, so they work on admins and the owner too:
/// until it ends, their messages are deleted and they're kicked from VC whenever they join.
async fn apply(
    ctx: &serenity::Context,
    data: &Data,
    guild_id: GuildId,
    user_id: UserId,
    p: Punishment,
    reason: &str,
) -> Result<String> {
    match p {
        Punishment::None => Ok("publicly shamed".into()),
        Punishment::Disconnect => Ok(if disconnect(ctx, guild_id, user_id, reason).await? {
            "disconnected from VC".into()
        } else {
            "not in VC, shamed instead".into()
        }),
        Punishment::Timeout { secs } => {
            let until = Utc::now() + Duration::seconds(secs as i64);
            data.db
                .extend_timeout(guild_id.get(), user_id.get(), until)
                .await?;
            disconnect(ctx, guild_id, user_id, reason).await?;
            Ok(format!("timed out for {}", format_duration(secs)))
        }
    }
}

/// Kick `user_id` out of VC if they're in it. Returns whether they were.
pub async fn disconnect(
    ctx: &serenity::Context,
    guild_id: GuildId,
    user_id: UserId,
    reason: &str,
) -> Result<bool> {
    let in_voice = ctx.cache.guild(guild_id).is_some_and(|g| {
        g.voice_states
            .get(&user_id)
            .is_some_and(|v| v.channel_id.is_some())
    });
    if in_voice {
        guild_id
            .edit_member(
                ctx,
                user_id,
                EditMember::new()
                    .disconnect_member()
                    .audit_log_reason(reason),
            )
            .await?;
    }
    Ok(in_voice)
}

/// Post to the guild's log channel, if configured. Failures are only logged.
pub async fn post_log(ctx: &serenity::Context, data: &Data, guild_id: GuildId, content: String) {
    let Ok(g) = data.db.guild(guild_id.get()).await else {
        return;
    };
    let Some(ch) = g.log_channel_id else { return };
    let ch = serenity::ChannelId::new(ch as u64);
    if let Err(e) = ch.say(ctx, content).await {
        tracing::warn!(%guild_id, "log post failed: {e}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn durations() {
        assert_eq!(parse_duration("90"), Some(90));
        assert_eq!(parse_duration("5m"), Some(300));
        assert_eq!(parse_duration("1h30m"), Some(5400));
        assert_eq!(parse_duration("2d"), Some(172800));
        assert_eq!(parse_duration(" 10S "), Some(10));
        assert_eq!(parse_duration("5x"), None);
        assert_eq!(parse_duration("m"), None);
        assert_eq!(parse_duration("5m3"), None);
        assert_eq!(parse_duration(""), None);
        assert_eq!(format_duration(5400), "1h30m");
        assert_eq!(format_duration(0), "0s");
    }

    #[test]
    fn punishment_json_roundtrip() {
        let p = Punishment::Timeout { secs: 60 };
        let s = serde_json::to_string(&p).unwrap();
        assert_eq!(s, r#"{"kind":"timeout","secs":60}"#);
        assert_eq!(serde_json::from_str::<Punishment>(&s).unwrap(), p);
    }
}
