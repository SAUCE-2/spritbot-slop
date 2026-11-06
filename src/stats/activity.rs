//! Turning raw Discord activity (messages, reactions, voice) into stat events.

use std::time::{Duration, Instant};

use dashmap::DashMap;
use poise::serenity_prelude::{GuildId, Message, ReactionType, UserId, VoiceState};

use super::Event;
use super::kind::*;

const FESTIVE_EMOJI_LIST: [&str; 18] = [
    "🎄", "🎅", "🤶", "⛄", "☃", "❄", "🎁", "🦌", "🔔", "🌟", "⭐", "✨", "🍪", "🧦", "🛷", "🎀",
    "🕯", "🥛",
];
const GREETINGS: [&str; 8] = [
    "merry christmas",
    "merry xmas",
    "merry chrimbo",
    "merry crimbo",
    "happy holidays",
    "feliz navidad",
    "season's greetings",
    "seasons greetings",
];
const HUMBUGS: [&str; 4] = ["humbug", "grinch", "hate christmas", "christmas sucks"];

/// What we can tell from a message's text alone.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TextStats {
    pub words: usize,
    pub caps: bool,
    pub question: bool,
    pub emoji: usize,
    pub festive_emoji: usize,
    pub greeting: bool,
    pub ho_ho: bool,
    pub humbug: bool,
    pub christmas: usize,
    pub links: bool,
}

fn is_emoji(c: char) -> bool {
    matches!(c as u32, 0x1F300..=0x1FAFF | 0x2600..=0x27BF | 0x1F000..=0x1F2FF)
}

pub fn analyze_text(text: &str) -> TextStats {
    let lower = text.to_lowercase();
    // Links, mentions and custom emoji don't count towards SHOUTING.
    let letters: Vec<char> = text
        .split_whitespace()
        .filter(|w| !w.contains("://") && !w.starts_with('<'))
        .flat_map(str::chars)
        .filter(|c| c.is_alphabetic())
        .collect();
    let custom_emoji = text.matches("<:").count() + text.matches("<a:").count();
    TextStats {
        words: text.split_whitespace().count(),
        caps: letters.len() >= 8 && letters.iter().all(|c| !c.is_lowercase()),
        question: text.trim_end().ends_with('?'),
        emoji: text.chars().filter(|c| is_emoji(*c)).count() + custom_emoji,
        festive_emoji: FESTIVE_EMOJI_LIST
            .iter()
            .map(|e| text.matches(e).count())
            .sum(),
        greeting: GREETINGS.iter().any(|g| lower.contains(g)),
        ho_ho: lower.contains("ho ho"),
        humbug: HUMBUGS.iter().any(|h| lower.contains(h)),
        christmas: lower.matches("christmas").count() + lower.matches("xmas").count(),
        links: lower.contains("http://") || lower.contains("https://"),
    }
}

/// Events for a message: mostly for its author, plus pings and replies for other people.
/// `hour` is the local hour (bot timezone) it was sent.
pub fn message_events(msg: &Message, hour: u32) -> Vec<(UserId, Event)> {
    let me = msg.author.id;
    let t = analyze_text(&msg.content);
    let mut out = vec![(me, Event::new(MESSAGE).amount(t.words as f64))];
    let mut add = |user: UserId, ev: Event| out.push((user, ev));
    if t.caps {
        add(me, Event::new(CAPS).text(msg.content.clone()));
    }
    if t.question {
        add(me, Event::new(QUESTION));
    }
    if hour < 5 {
        add(me, Event::new(NIGHT_OWL));
    } else if hour < 8 {
        add(me, Event::new(EARLY_BIRD));
    }
    if t.emoji > 0 {
        add(me, Event::new(EMOJI).amount(t.emoji as f64));
    }
    if t.festive_emoji > 0 {
        add(me, Event::new(FESTIVE_EMOJI).amount(t.festive_emoji as f64));
    }
    if t.greeting {
        add(me, Event::new(MERRY));
    }
    if t.ho_ho {
        add(me, Event::new(HO_HO));
    }
    if t.humbug {
        add(me, Event::new(HUMBUG).text(msg.content.clone()));
    }
    if t.christmas > 0 {
        add(me, Event::new(CHRISTMAS).amount(t.christmas as f64));
    }
    if t.links {
        add(me, Event::new(LINK));
    }
    let media = msg
        .attachments
        .iter()
        .filter(|a| {
            a.content_type
                .as_deref()
                .is_some_and(|t| t.starts_with("image/") || t.starts_with("video/"))
        })
        .count()
        + msg
            .embeds
            .iter()
            .filter(|e| e.image.is_some() || e.thumbnail.is_some() || e.video.is_some())
            .count()
        + msg.sticker_items.len();
    if media > 0 {
        add(me, Event::new(MEDIA).amount(media as f64));
    }
    let replied_to = msg.referenced_message.as_ref().map(|m| &m.author);
    if let Some(author) = replied_to {
        add(me, Event::new(REPLY));
        if author.id != me && !author.bot {
            add(author.id, Event::new(REPLIED_TO));
        }
    }
    let pinged: Vec<UserId> = msg
        .mentions
        .iter()
        .filter(|u| u.id != me && !u.bot)
        // A reply pings its author, but that's not really a ping.
        .filter(|u| replied_to.is_none_or(|a| a.id != u.id))
        .map(|u| u.id)
        .collect();
    if !pinged.is_empty() {
        add(me, Event::new(PING_SENT).amount(pinged.len() as f64));
    }
    for u in pinged {
        add(u, Event::new(PINGED));
    }
    out
}

pub fn is_festive_reaction(r: &ReactionType) -> bool {
    match r {
        ReactionType::Unicode(s) => FESTIVE_EMOJI_LIST.iter().any(|e| s.contains(e)),
        ReactionType::Custom { name, .. } => name.as_deref().is_some_and(|n| {
            let n = n.to_lowercase();
            ["christmas", "xmas", "santa", "festive", "snow", "gift"]
                .iter()
                .any(|w| n.contains(w))
        }),
        _ => false,
    }
}

/// Voice toggles worth counting, as (deafened, muted, streaming, camera).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct VoiceFlags {
    pub deaf: bool,
    pub mute: bool,
    pub stream: bool,
    pub video: bool,
}

impl From<&VoiceState> for VoiceFlags {
    fn from(v: &VoiceState) -> Self {
        Self {
            deaf: v.self_deaf,
            mute: v.self_mute,
            stream: v.self_stream == Some(true),
            video: v.self_video,
        }
    }
}

/// Events for switching things on (deafen, mute, go live, camera) while staying in a channel.
pub fn voice_toggles(old: VoiceFlags, new: VoiceFlags) -> Vec<Event> {
    let mut out = Vec::new();
    if new.deaf && !old.deaf {
        out.push(Event::new(VC_DEAFEN));
    } else if new.mute && !old.mute && !new.deaf {
        out.push(Event::new(VC_MUTE));
    }
    if new.stream && !old.stream {
        out.push(Event::new(VC_STREAM));
    }
    if new.video && !old.video {
        out.push(Event::new(VC_CAMERA));
    }
    out
}

/// Visits shorter than this count as drive-bys.
pub const DRIVE_BY: Duration = Duration::from_secs(60);

/// When each person currently in VC joined, so we can record time spent there.
/// In memory only: sessions running when the bot restarts are re-seeded from the guild cache.
#[derive(Default)]
pub struct VcSessions(DashMap<(GuildId, UserId), Instant>);

impl VcSessions {
    /// Start a session unless one is already running (moving channels keeps it).
    pub fn start(&self, guild_id: GuildId, user: UserId) {
        self.0.entry((guild_id, user)).or_insert_with(Instant::now);
    }

    /// End a session, returning how long it lasted.
    pub fn end(&self, guild_id: GuildId, user: UserId) -> Option<Duration> {
        self.0.remove(&(guild_id, user)).map(|(_, t)| t.elapsed())
    }
}

/// Events for leaving VC after `spent`.
pub fn vc_leave_events(spent: Duration) -> Vec<Event> {
    let mut out = vec![Event::new(VC_TIME).amount(spent.as_secs() as f64)];
    if spent < DRIVE_BY {
        out.push(Event::new(VC_DRIVE_BY));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn text() {
        let t = analyze_text("MERRY CHRISTMAS EVERYONE 🎄🎄 ho ho ho! <:santa:123> christmas?");
        assert_eq!(t.words, 9);
        assert!(!t.caps, "has lowercase");
        assert!(t.question && t.greeting && t.ho_ho && !t.humbug && !t.links);
        assert_eq!(t.festive_emoji, 2);
        assert_eq!(t.emoji, 3);
        assert_eq!(t.christmas, 2);

        let t = analyze_text("BAH HUMBUG I HATE THIS https://x.y");
        assert!(t.caps && t.humbug && t.links && !t.question);
        assert!(!analyze_text("OK").caps, "too short to be shouting");
    }

    #[test]
    fn toggles() {
        let off = VoiceFlags::default();
        let deaf = VoiceFlags {
            deaf: true,
            mute: true,
            ..off
        };
        let kinds = |v: Vec<Event>| v.iter().map(|e| e.kind).collect::<Vec<_>>();
        assert_eq!(kinds(voice_toggles(off, deaf)), [VC_DEAFEN]);
        assert_eq!(
            kinds(voice_toggles(off, VoiceFlags { mute: true, ..off })),
            [VC_MUTE]
        );
        assert_eq!(
            kinds(voice_toggles(
                off,
                VoiceFlags {
                    stream: true,
                    video: true,
                    ..off
                }
            )),
            [VC_STREAM, VC_CAMERA]
        );
        assert!(
            voice_toggles(deaf, off).is_empty(),
            "switching off doesn't count"
        );
        assert_eq!(
            kinds(vc_leave_events(Duration::from_secs(5))),
            [VC_TIME, VC_DRIVE_BY]
        );
        assert_eq!(kinds(vc_leave_events(Duration::from_secs(600))), [VC_TIME]);
    }

    #[test]
    fn sessions() {
        let s = VcSessions::default();
        let (g, u) = (GuildId::new(1), UserId::new(2));
        assert!(s.end(g, u).is_none());
        s.start(g, u);
        s.start(g, u);
        assert!(s.end(g, u).is_some());
        assert!(s.end(g, u).is_none());
    }
}
