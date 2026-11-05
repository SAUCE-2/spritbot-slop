//! Voice listening. A bot can only be in one VC per guild, so each guild runs one "driver" task
//! that hops between channels with pending requests. Inside a channel, Discord sends each
//! speaker's audio separately (by SSRC), so several people can be listened to at once.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use async_trait::async_trait;
use poise::serenity_prelude::{self as serenity, ChannelId, GuildId, UserId};
use songbird::events::{CoreEvent, Event, EventContext, EventHandler};
use tokio::sync::oneshot;

use crate::app::Data;

#[derive(Clone, Debug)]
pub enum Outcome {
    /// What Whisper heard during the window ("" if they said nothing).
    Heard(String),
    /// They left the channel before the window finished.
    Left,
    Failed(String),
}

struct Pending {
    channel: ChannelId,
    user: UserId,
    window: Duration,
    tx: oneshot::Sender<Outcome>,
}

struct Window {
    until: Instant,
    /// 16 kHz mono PCM, as configured on the songbird driver.
    samples: Vec<i16>,
    waiters: Vec<oneshot::Sender<Outcome>>,
}

#[derive(Default)]
struct GuildState {
    running: bool,
    current: Option<ChannelId>,
    pending: Vec<Pending>,
    windows: HashMap<UserId, Window>,
    ssrc_to_user: HashMap<u32, UserId>,
}

impl GuildState {
    fn start_window(&mut self, user: UserId, window: Duration, tx: oneshot::Sender<Outcome>) {
        let until = Instant::now() + window;
        let w = self.windows.entry(user).or_insert_with(|| Window {
            until,
            samples: Vec::new(),
            waiters: Vec::new(),
        });
        w.until = w.until.max(until);
        w.waiters.push(tx);
    }

    /// Start windows for everyone waiting on the channel we're in.
    fn adopt_pending(&mut self) -> Vec<UserId> {
        let Some(cur) = self.current else {
            return vec![];
        };
        let (here, rest): (Vec<_>, Vec<_>) = std::mem::take(&mut self.pending)
            .into_iter()
            .partition(|p| p.channel == cur);
        self.pending = rest;
        here.into_iter()
            .map(|p| {
                self.start_window(p.user, p.window, p.tx);
                p.user
            })
            .collect()
    }
}

type Shared = Arc<Mutex<GuildState>>;

#[derive(Default)]
pub struct VoiceListener {
    guilds: Mutex<HashMap<GuildId, Shared>>,
}

enum Step {
    Wait,
    Join(ChannelId),
    Leave,
}

impl VoiceListener {
    fn guild(&self, guild_id: GuildId) -> Shared {
        self.guilds
            .lock()
            .unwrap()
            .entry(guild_id)
            .or_default()
            .clone()
    }

    /// Listen to `user` for `window` once the bot is in `channel`. Resolves when done.
    pub async fn listen(
        &self,
        ctx: &serenity::Context,
        data: &Data,
        guild_id: GuildId,
        channel: ChannelId,
        user: UserId,
        window: Duration,
    ) -> Outcome {
        let (tx, rx) = oneshot::channel();
        let st = self.guild(guild_id);
        let spawn = {
            let mut s = st.lock().unwrap();
            if s.running && s.current == Some(channel) {
                s.start_window(user, window, tx);
            } else {
                s.pending.push(Pending {
                    channel,
                    user,
                    window,
                    tx,
                });
            }
            !std::mem::replace(&mut s.running, true)
        };
        if spawn {
            tokio::spawn(drive(ctx.clone(), data.clone(), guild_id, st));
        }
        rx.await
            .unwrap_or(Outcome::Failed("listener stopped".into()))
    }

    /// Someone left (or moved out of) `channel`: their open requests there end as `Left`.
    pub fn user_left(&self, guild_id: GuildId, user: UserId, channel: ChannelId) {
        let st = self.guild(guild_id);
        let mut s = st.lock().unwrap();
        if s.current == Some(channel)
            && let Some(w) = s.windows.remove(&user)
        {
            w.waiters
                .into_iter()
                .for_each(|tx| drop(tx.send(Outcome::Left)));
        }
        let (gone, keep): (Vec<_>, Vec<_>) = std::mem::take(&mut s.pending)
            .into_iter()
            .partition(|p| p.user == user && p.channel == channel);
        s.pending = keep;
        gone.into_iter()
            .for_each(|p| drop(p.tx.send(Outcome::Left)));
    }
}

/// One per guild while there's listening to do.
async fn drive(ctx: serenity::Context, data: Data, guild_id: GuildId, st: Shared) {
    let Some(manager) = songbird::get(&ctx).await else {
        tracing::error!("songbird not registered");
        return;
    };
    loop {
        let now = Instant::now();
        let (expired, step) = {
            let mut s = st.lock().unwrap();
            s.adopt_pending();
            let done: Vec<UserId> = s
                .windows
                .iter()
                .filter(|(_, w)| w.until <= now)
                .map(|(u, _)| *u)
                .collect();
            let expired: Vec<Window> = done.iter().filter_map(|u| s.windows.remove(u)).collect();
            let step = if !s.windows.is_empty() {
                Step::Wait
            } else if let Some(p) = s.pending.first() {
                Step::Join(p.channel)
            } else {
                s.running = false;
                s.current = None;
                Step::Leave
            };
            (expired, step)
        };
        for w in expired {
            tokio::spawn(finish(data.clone(), w));
        }
        match step {
            Step::Wait => tokio::time::sleep(Duration::from_millis(200)).await,
            Step::Leave => {
                manager.leave(guild_id).await.ok();
                return;
            }
            Step::Join(channel) => {
                match manager.join(guild_id, channel).await {
                    Ok(call) => {
                        let mut call = call.lock().await;
                        call.remove_all_global_events();
                        call.add_global_event(
                            CoreEvent::SpeakingStateUpdate.into(),
                            Receiver(st.clone()),
                        );
                        call.add_global_event(CoreEvent::VoiceTick.into(), Receiver(st.clone()));
                    }
                    Err(e) => {
                        tracing::warn!(%guild_id, %channel, "voice join failed: {e}");
                        let mut s = st.lock().unwrap();
                        let (failed, keep): (Vec<_>, Vec<_>) = std::mem::take(&mut s.pending)
                            .into_iter()
                            .partition(|p| p.channel == channel);
                        s.pending = keep;
                        for p in failed {
                            p.tx.send(Outcome::Failed(format!("couldn't join VC: {e}")))
                                .ok();
                        }
                        continue;
                    }
                }
                let mut s = st.lock().unwrap();
                s.current = Some(channel);
                s.ssrc_to_user.clear(); // SSRCs are per voice session
                s.adopt_pending();
            }
        }
    }
}

async fn finish(data: Data, w: Window) {
    let outcome = match &data.stt {
        Some(stt) => match stt.transcribe(w.samples).await {
            Ok(text) => Outcome::Heard(text),
            Err(e) => Outcome::Failed(format!("transcription failed: {e}")),
        },
        None => Outcome::Failed("Whisper model not loaded".into()),
    };
    for tx in w.waiters {
        tx.send(outcome.clone()).ok();
    }
}

struct Receiver(Shared);

#[async_trait]
impl EventHandler for Receiver {
    async fn act(&self, ev: &EventContext<'_>) -> Option<Event> {
        let mut s = self.0.lock().unwrap();
        match ev {
            EventContext::SpeakingStateUpdate(sp) => {
                if let Some(u) = sp.user_id {
                    s.ssrc_to_user.insert(sp.ssrc, UserId::new(u.0));
                }
            }
            EventContext::VoiceTick(tick) => {
                for (ssrc, d) in &tick.speaking {
                    let Some(user) = s.ssrc_to_user.get(ssrc).copied() else {
                        continue;
                    };
                    if let (Some(w), Some(pcm)) = (s.windows.get_mut(&user), &d.decoded_voice) {
                        w.samples.extend_from_slice(pcm);
                    }
                }
            }
            _ => {}
        }
        None
    }
}
