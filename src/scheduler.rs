//! Background loop: scheduled activations + announcements, and rule ticks.

use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use poise::serenity_prelude::{
    self as serenity, ChannelId, CreateAllowedMentions, CreateEmbed, CreateMessage, GuildId, RoleId,
};

use crate::app::Data;
use crate::db::RuleRow;
use crate::rules::{Rule, RuleCx, missing_requirements};
use crate::setup;

/// Turn a rule on and announce it. Returns the unmet requirements instead if it can't go live.
pub async fn activate(
    ctx: &serenity::Context,
    data: &Data,
    guild_id: GuildId,
    rule: &Arc<dyn Rule>,
) -> Result<Result<(), Vec<String>>> {
    let mut row = data.db.rule(guild_id.get(), rule.id()).await?;
    let missing = missing_requirements(data, rule.as_ref());
    if !missing.is_empty() {
        return Ok(Err(missing));
    }
    row.enabled = true;
    row.start_date = None;
    row.activated_at = Some(chrono::Utc::now());
    data.db.save_rule(&row).await?;
    let punishment = row.punishment.unwrap_or_else(|| rule.default_punishment());
    let log = crate::stats::Log::new(
        crate::stats::kind::RULE_ACTIVATED,
        serde_json::json!({ "name": rule.name(), "punishment": punishment }),
    )
    .rule(rule.id());
    crate::stats::record(data, guild_id, log).await;
    announce(ctx, data, guild_id, rule.as_ref(), &row).await;
    Ok(Ok(()))
}

async fn announce(
    ctx: &serenity::Context,
    data: &Data,
    guild_id: GuildId,
    rule: &dyn Rule,
    row: &RuleRow,
) {
    let punishment = row.punishment.unwrap_or_else(|| rule.default_punishment());
    let Ok(g) = data.db.guild(guild_id.get()).await else {
        return;
    };
    let Some(ch) = g.announce_channel_id.or(g.log_channel_id) else {
        return;
    };
    let role = g.enforced_role_id.map(|r| RoleId::new(r as u64));
    let embed = CreateEmbed::new()
        .title(format!("🎄 New rule: {}", rule.name()))
        .description(rule.announcement())
        .field("Punishment", punishment.to_string(), true)
        .field("Rule id", format!("`{}`", rule.id()), true)
        .colour(0xc0392b);
    let mut msg = CreateMessage::new().embed(embed);
    if let Some(r) = role {
        msg = msg
            .content(format!("<@&{r}>"))
            .allowed_mentions(CreateAllowedMentions::new().roles([r]));
    }
    if let Err(e) = ChannelId::new(ch as u64).send_message(ctx, msg).await {
        tracing::warn!(%guild_id, "announcement failed: {e}");
    }
}

pub fn spawn_loops(ctx: serenity::Context, data: Data) {
    tokio::spawn(every(Duration::from_secs(60), move || {
        let (c, d) = (ctx.clone(), data.clone());
        async move {
            activate_scheduled(&c, &d).await?;
            tick_rules(&c, &d).await
        }
    }));
}

async fn every<F, Fut>(period: Duration, mut f: F)
where
    F: FnMut() -> Fut,
    Fut: Future<Output = Result<()>>,
{
    let mut interval = tokio::time::interval(period);
    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    loop {
        interval.tick().await;
        if let Err(e) = f().await {
            tracing::warn!("background loop error: {e:#}");
        }
    }
}

async fn activate_scheduled(ctx: &serenity::Context, data: &Data) -> Result<()> {
    for row in data
        .db
        .due_scheduled(chrono::Utc::now(), data.cfg.tz)
        .await?
    {
        let guild_id = GuildId::new(row.guild_id as u64);
        let Some(rule) = data.rule(&row.rule_id) else {
            continue;
        };
        if let Err(missing) = activate(ctx, data, guild_id, &rule).await? {
            tracing::info!(%guild_id, rule = rule.id(), ?missing, "scheduled rule held back");
            // The setup notice explains why (only re-posted when the problem set changes).
            setup::report(ctx, data, guild_id).await?;
        }
    }
    Ok(())
}

async fn tick_rules(ctx: &serenity::Context, data: &Data) -> Result<()> {
    for guild_id in ctx.cache.guilds() {
        for row in data.db.enabled_rules(guild_id.get()).await? {
            let Some(rule) = data.rule(&row.rule_id) else {
                continue;
            };
            let cx = RuleCx::load(ctx, data, guild_id, rule.clone()).await?;
            if let Err(e) = rule.on_tick(&cx).await {
                tracing::warn!(%guild_id, rule = rule.id(), "on_tick failed: {e:#}");
            }
        }
    }
    Ok(())
}
