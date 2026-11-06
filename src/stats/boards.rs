//! Leaderboard definitions. A [`Board`] is a declarative aggregation over `stat_events`; add one
//! here (or in a rule's `boards()`) and it shows up in `/leaderboard` automatically.

use std::sync::Arc;

use super::kind::*;
use crate::rules::Rule;

/// Groups boards in the overview, one embed each.
#[derive(Clone, Copy, Debug, PartialEq, Eq, poise::ChoiceParameter)]
pub enum Section {
    #[name = "😈 Naughty"]
    Naughty,
    #[name = "😇 Nice"]
    Nice,
    #[name = "💬 Chat"]
    Chat,
    #[name = "🤝 Social"]
    Social,
    #[name = "🎧 Voice"]
    Voice,
}

impl Section {
    pub const ALL: [Section; 5] = [
        Section::Naughty,
        Section::Nice,
        Section::Chat,
        Section::Social,
        Section::Voice,
    ];

    pub fn title(self) -> &'static str {
        match self {
            Section::Naughty => "😈 The Naughty Side",
            Section::Nice => "😇 The Nice Side",
            Section::Chat => "💬 Chat",
            Section::Social => "🤝 Social",
            Section::Voice => "🎧 Voice",
        }
    }

    pub fn colour(self) -> u32 {
        match self {
            Section::Naughty => 0xc0392b,
            Section::Nice => 0x27ae60,
            Section::Chat => 0x2980b9,
            Section::Social => 0xf1c40f,
            Section::Voice => 0x8e44ad,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Field {
    Score,
    Amount,
}

impl Field {
    pub(super) fn column(self) -> &'static str {
        match self {
            Field::Score => "score",
            Field::Amount => "amount",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Agg {
    /// Number of events.
    Count,
    Sum(Field),
    Avg(Field),
    Max(Field),
    Min(Field),
    /// Max minus min (e.g. how far someone's scores swing).
    Range(Field),
    /// Distinct days with an event.
    Days,
    /// Distinct rules involved.
    Rules,
    /// Share of judgements that were festive (0..1).
    PassRate,
}

impl Agg {
    pub(super) fn sql(self) -> String {
        match self {
            Agg::Count => "COUNT(*)".into(),
            Agg::Sum(f) => format!("SUM({})", f.column()),
            Agg::Avg(f) => format!("AVG({})", f.column()),
            Agg::Max(f) => format!("MAX({})", f.column()),
            Agg::Min(f) => format!("MIN({})", f.column()),
            Agg::Range(f) => format!("MAX({0}) - MIN({0})", f.column()),
            Agg::Days => "COUNT(DISTINCT day)".into(),
            Agg::Rules => "COUNT(DISTINCT rule_id)".into(),
            Agg::PassRate => format!("AVG(score >= {})", super::FESTIVE),
        }
    }

    /// Column that must be present for the aggregate to mean anything.
    pub(super) fn field(self) -> Option<Field> {
        match self {
            Agg::Count | Agg::Days => None,
            Agg::Rules => None,
            Agg::PassRate => Some(Field::Score),
            Agg::Sum(f) | Agg::Avg(f) | Agg::Max(f) | Agg::Min(f) | Agg::Range(f) => Some(f),
        }
    }
}

/// How a board's values are shown.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Format {
    /// A number, with the plural noun to show after it ("punishments").
    Count(&'static str),
    /// Seconds.
    Duration,
    /// A Jev score.
    Score,
    Days,
    Percent,
}

/// Which judgements count, by Jev score.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Judged {
    Any,
    Festive,
    NotFestive,
    AtLeast(f32),
    AtMost(f32),
}

/// Which single event a quote board picks.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Order {
    HighestScore,
    LowestScore,
    Longest,
    First,
    Latest,
}

impl Order {
    pub fn by_score(self) -> bool {
        matches!(self, Order::HighestScore | Order::LowestScore)
    }
}

#[derive(Clone, Debug)]
pub struct Rank {
    pub kinds: Vec<&'static str>,
    pub agg: Agg,
    /// Lowest first (e.g. the Grinch: lowest average score).
    pub lowest: bool,
    /// Only rank people with at least this many events (stops one-offs topping averages).
    pub min_events: i64,
    pub format: Format,
    pub filter: Judged,
    /// Aggregate per day and keep each person's record day ("most messages in one day").
    pub per_day: bool,
    /// Only people never punished in the period.
    pub clean: bool,
}

#[derive(Clone, Debug)]
pub enum Metric {
    /// Rank people by an aggregate over their events.
    Rank(Rank),
    /// Standout single events, with what was said.
    Quote {
        kind: &'static str,
        order: Order,
        filter: Judged,
    },
    /// Longest run of consecutive days: festive and unpunished, or (naughty) punished.
    Streak { naughty: bool },
    /// Biggest rise in average score from the first half of someone's judgements to the second.
    Improved { min_events: i64 },
}

#[derive(Clone, Debug)]
pub struct Board {
    /// Unique id, e.g. `naughty`. A rule's boards get prefixed with the rule id (`vc_greeting:silent`).
    pub id: String,
    pub title: String,
    pub section: Section,
    /// Only count events from this rule.
    pub rule_id: Option<&'static str>,
    pub metric: Metric,
}

impl Board {
    fn new(id: &str, title: &str, metric: Metric) -> Self {
        Self {
            id: id.into(),
            title: title.into(),
            section: Section::Naughty,
            rule_id: None,
            metric,
        }
    }

    /// Rank people by `agg` over events of `kinds`, highest first.
    pub fn rank(id: &str, title: &str, kinds: &[&'static str], agg: Agg, format: Format) -> Self {
        Self::new(
            id,
            title,
            Metric::Rank(Rank {
                kinds: kinds.to_vec(),
                agg,
                lowest: false,
                min_events: 1,
                format,
                filter: Judged::Any,
                per_day: false,
                clean: false,
            }),
        )
    }

    /// Most events of `kinds`.
    pub fn most(id: &str, title: &str, kinds: &[&'static str], noun: &'static str) -> Self {
        Self::rank(id, title, kinds, Agg::Count, Format::Count(noun))
    }

    /// Highest total `amount` over events of `kinds`.
    pub fn total(id: &str, title: &str, kinds: &[&'static str], noun: &'static str) -> Self {
        Self::rank(
            id,
            title,
            kinds,
            Agg::Sum(Field::Amount),
            Format::Count(noun),
        )
    }

    /// One standout event of `kind`, quoted.
    pub fn quote(id: &str, title: &str, kind: &'static str, order: Order) -> Self {
        Self::new(
            id,
            title,
            Metric::Quote {
                kind,
                order,
                filter: Judged::Any,
            },
        )
    }

    pub fn streak(id: &str, title: &str, naughty: bool) -> Self {
        Self::new(id, title, Metric::Streak { naughty })
    }

    pub fn improved(id: &str, title: &str, min_events: i64) -> Self {
        Self::new(id, title, Metric::Improved { min_events })
    }

    fn rank_mut(&mut self) -> Option<&mut Rank> {
        match &mut self.metric {
            Metric::Rank(r) => Some(r),
            _ => None,
        }
    }

    pub fn section(mut self, s: Section) -> Self {
        self.section = s;
        self
    }

    pub fn lowest(mut self) -> Self {
        if let Some(r) = self.rank_mut() {
            r.lowest = true;
        }
        self
    }

    pub fn min_events(mut self, n: i64) -> Self {
        if let Some(r) = self.rank_mut() {
            r.min_events = n;
        }
        self
    }

    pub fn per_day(mut self) -> Self {
        if let Some(r) = self.rank_mut() {
            r.per_day = true;
        }
        self
    }

    pub fn clean(mut self) -> Self {
        if let Some(r) = self.rank_mut() {
            r.clean = true;
        }
        self
    }

    pub fn filter(mut self, j: Judged) -> Self {
        match &mut self.metric {
            Metric::Rank(r) => r.filter = j,
            Metric::Quote { filter, .. } => *filter = j,
            _ => {}
        }
        self
    }

    fn for_rule(mut self, rule_id: &'static str) -> Self {
        self.id = format!("{rule_id}:{}", self.id);
        self.rule_id = Some(rule_id);
        self
    }

    pub fn is_quote(&self) -> bool {
        matches!(self.metric, Metric::Quote { .. })
    }
}

/// Boards that span all rules, by section.
pub fn global_boards() -> Vec<Board> {
    use Section::*;
    let dur = |id, title, kinds, agg| Board::rank(id, title, kinds, agg, Format::Duration);
    vec![
        // ---- naughty ----
        Board::most("naughty", "🎅 Naughty List", &[PUNISHED], "punishments"),
        dur(
            "hard_time",
            "⏳ Hard Time (total timeouts)",
            &[PUNISHED],
            Agg::Sum(Field::Amount),
        ),
        dur(
            "longest_sentence",
            "⛓️ Longest Sentence (biggest single timeout)",
            &[PUNISHED],
            Agg::Max(Field::Amount),
        ),
        Board::most(
            "worst_day",
            "📉 Worst Day (punishments in one day)",
            &[PUNISHED],
            "punishments",
        )
        .per_day(),
        Board::rank(
            "repeat_offender",
            "🔁 Repeat Offender (days punished)",
            &[PUNISHED],
            Agg::Days,
            Format::Days,
        ),
        Board::rank(
            "rule_collector",
            "🃏 Rule Collector (different rules broken)",
            &[PUNISHED],
            Agg::Rules,
            Format::Count("rules"),
        ),
        Board::streak(
            "naughty_streak",
            "😈 Naughty Streak (days in a row punished)",
            true,
        ),
        Board::most(
            "cant_stay_away",
            "🚪 Can't Stay Away (tried to talk or join VC while timed out)",
            &[TIMEOUT_MESSAGE, TIMEOUT_VC],
            "attempts",
        ),
        Board::quote(
            "talking_to_a_wall",
            "🧱 Talking to a Wall (longest message typed while timed out)",
            TIMEOUT_MESSAGE,
            Order::Longest,
        ),
        Board::quote(
            "first_blood",
            "🩸 First Blood (first punishment)",
            PUNISHED,
            Order::First,
        ),
        Board::quote("latest_victim", "⚰️ Latest Victim", PUNISHED, Order::Latest),
        Board::most(
            "humbug",
            "🙄 Bah Humbug (said humbug/grinch/hate christmas)",
            &[HUMBUG],
            "times",
        ),
        Board::most(
            "pardoned",
            "🙏 Teacher's Pet (pardons)",
            &[PARDONED],
            "pardons",
        ),
        // ---- nice ----
        Board::rank(
            "nice",
            "😇 Nice List (most festive on average)",
            &[JUDGED],
            Agg::Avg(Field::Score),
            Format::Score,
        )
        .min_events(3)
        .section(Nice),
        Board::rank(
            "grinch",
            "🦹 The Grinch (least festive on average)",
            &[JUDGED],
            Agg::Avg(Field::Score),
            Format::Score,
        )
        .lowest()
        .min_events(3)
        .section(Nice),
        Board::rank(
            "reliable",
            "✅ Most Reliable (share of judgements passed)",
            &[JUDGED],
            Agg::PassRate,
            Format::Percent,
        )
        .min_events(5)
        .section(Nice),
        Board::streak(
            "streak",
            "🔥 Festive Streak (days in a row without slipping)",
            false,
        )
        .section(Nice),
        Board::improved("improved", "📈 Most Improved", 4).section(Nice),
        Board::most(
            "angel",
            "👼 Angel (judged a lot, never punished)",
            &[JUDGED],
            "judgements",
        )
        .clean()
        .min_events(3)
        .section(Nice),
        Board::most(
            "miracle",
            "✨ Christmas Miracle (near-perfect scores)",
            &[JUDGED],
            "near-perfect scores",
        )
        .filter(Judged::AtLeast(0.95))
        .section(Nice),
        Board::most(
            "ice_cold",
            "🧊 Ice Cold (zero-spirit moments)",
            &[JUDGED],
            "zero-spirit moments",
        )
        .filter(Judged::AtMost(0.05))
        .section(Nice),
        Board::rank(
            "mood_swings",
            "🎢 Mood Swings (gap between best and worst score)",
            &[JUDGED],
            Agg::Range(Field::Score),
            Format::Score,
        )
        .min_events(3)
        .section(Nice),
        Board::most(
            "scrutiny",
            "🔍 Under Scrutiny (times judged)",
            &[JUDGED],
            "judgements",
        )
        .section(Nice),
        Board::quote(
            "best",
            "🌟 Most Festive Moment",
            JUDGED,
            Order::HighestScore,
        )
        .section(Nice),
        Board::quote(
            "worst",
            "💀 Least Festive Moment",
            JUDGED,
            Order::LowestScore,
        )
        .section(Nice),
        Board::quote(
            "close_call",
            "😅 Closest Call (barely festive enough)",
            JUDGED,
            Order::LowestScore,
        )
        .filter(Judged::Festive)
        .section(Nice),
        Board::quote(
            "festive_essay",
            "📜 Festive Essay (longest festive message)",
            JUDGED,
            Order::Longest,
        )
        .filter(Judged::Festive)
        .section(Nice),
        Board::most(
            "merry",
            "🎄 Merry Christmas! (seasonal greetings)",
            &[MERRY],
            "greetings",
        )
        .section(Nice),
        Board::most("ho_ho_ho", "🎅 Ho Ho Ho", &[HO_HO], "ho ho hos").section(Nice),
        Board::total(
            "christmas",
            "🔔 Said \"Christmas\" the Most",
            &[CHRISTMAS],
            "mentions",
        )
        .section(Nice),
        Board::total(
            "festive_emoji",
            "🎁 Festive Emoji",
            &[FESTIVE_EMOJI],
            "festive emoji",
        )
        .section(Nice),
        // ---- chat ----
        Board::most("chatterbox", "💬 Chatterbox", &[MESSAGE], "messages").section(Chat),
        Board::total(
            "wordsmith",
            "✍️ Wordsmith (total words)",
            &[MESSAGE],
            "words",
        )
        .section(Chat),
        Board::rank(
            "novelist",
            "📖 Novelist (longest message)",
            &[MESSAGE],
            Agg::Max(Field::Amount),
            Format::Count("words"),
        )
        .section(Chat),
        Board::rank(
            "rambler",
            "🗣️ Rambler (most words per message)",
            &[MESSAGE],
            Agg::Avg(Field::Amount),
            Format::Count("words on average"),
        )
        .min_events(10)
        .section(Chat),
        Board::most(
            "busiest_day",
            "📨 Busiest Day (messages in one day)",
            &[MESSAGE],
            "messages",
        )
        .per_day()
        .section(Chat),
        Board::rank(
            "ever_present",
            "📅 Ever-Present (days active)",
            &[MESSAGE],
            Agg::Days,
            Format::Days,
        )
        .section(Chat),
        Board::most(
            "shouter",
            "📢 SHOUTER (all-caps messages)",
            &[CAPS],
            "shouts",
        )
        .section(Chat),
        Board::quote("loudest", "🔊 Loudest Message", CAPS, Order::Longest).section(Chat),
        Board::most(
            "curious",
            "❓ Curious (questions asked)",
            &[QUESTION],
            "questions",
        )
        .section(Chat),
        Board::most(
            "night_owl",
            "🦉 Night Owl (messages after midnight)",
            &[NIGHT_OWL],
            "messages",
        )
        .section(Chat),
        Board::most(
            "early_bird",
            "🐦 Early Bird (messages before 8am)",
            &[EARLY_BIRD],
            "messages",
        )
        .section(Chat),
        Board::total("emoji", "😂 Emoji Fiend", &[EMOJI], "emoji").section(Chat),
        Board::total(
            "memes",
            "🖼️ Meme Dealer (images & GIFs)",
            &[MEDIA],
            "images & GIFs",
        )
        .section(Chat),
        Board::most("links", "🔗 Link Dropper", &[LINK], "links").section(Chat),
        Board::most(
            "edits",
            "✏️ Second Thoughts (messages edited)",
            &[EDIT],
            "edits",
        )
        .section(Chat),
        // ---- social ----
        Board::most(
            "crowd_pleaser",
            "⭐ Crowd Pleaser (reactions received)",
            &[REACTION_RECEIVED],
            "reactions",
        )
        .section(Social),
        Board::most(
            "generous",
            "👍 Generous (reactions given)",
            &[REACTION_GIVEN],
            "reactions",
        )
        .section(Social),
        Board::most(
            "festive_reactor",
            "🎄 Festive Reactor (festive reactions given)",
            &[FESTIVE_REACTION],
            "festive reactions",
        )
        .section(Social),
        Board::most(
            "most_wanted",
            "📣 Most Wanted (times pinged)",
            &[PINGED],
            "pings",
        )
        .section(Social),
        Board::total("pinger", "🔔 Pinger (pings sent)", &[PING_SENT], "pings").section(Social),
        Board::most("reply_guy", "↩️ Reply Guy (replies)", &[REPLY], "replies").section(Social),
        Board::most(
            "conversation_starter",
            "🗨️ Conversation Starter (got replied to)",
            &[REPLIED_TO],
            "replies",
        )
        .section(Social),
        // ---- voice ----
        Board::most("vc_regular", "🎧 VC Regular (joins)", &[VC_JOIN], "joins").section(Voice),
        dur(
            "yule_log",
            "🪵 Yule Log (total time in VC)",
            &[VC_TIME],
            Agg::Sum(Field::Amount),
        )
        .section(Voice),
        dur(
            "marathon",
            "🕯️ Marathon (longest VC session)",
            &[VC_TIME],
            Agg::Max(Field::Amount),
        )
        .section(Voice),
        dur(
            "vc_day",
            "🌙 Longest VC Day",
            &[VC_TIME],
            Agg::Sum(Field::Amount),
        )
        .per_day()
        .section(Voice),
        Board::rank(
            "vc_days",
            "📆 VC Every Day (days in VC)",
            &[VC_JOIN],
            Agg::Days,
            Format::Days,
        )
        .section(Voice),
        Board::most("hopper", "🐇 Channel Hopper", &[VC_HOP], "hops").section(Voice),
        Board::most(
            "drive_by",
            "💨 Drive-By (VC visits under a minute)",
            &[VC_DRIVE_BY],
            "visits",
        )
        .section(Voice),
        Board::most(
            "deafened",
            "🙉 Deaf to the Carols (deafened)",
            &[VC_DEAFEN],
            "times",
        )
        .section(Voice),
        Board::most("muted", "🤫 Silent Treatment (muted)", &[VC_MUTE], "times").section(Voice),
        Board::most(
            "streamer",
            "📺 Streamer (went live)",
            &[VC_STREAM],
            "streams",
        )
        .section(Voice),
        Board::most("camera", "📸 Camera On", &[VC_CAMERA], "times").section(Voice),
    ]
}

/// Boards every rule gets for free.
fn standard_rule_boards() -> Vec<Board> {
    vec![
        Board::most("naughty", "🎅 Most punished", &[PUNISHED], "punishments"),
        Board::quote("best", "🌟 Most festive", JUDGED, Order::HighestScore),
        Board::quote("worst", "💀 Least festive", JUDGED, Order::LowestScore),
        Board::rank(
            "nice",
            "😇 Best average",
            &[JUDGED],
            Agg::Avg(Field::Score),
            Format::Score,
        )
        .min_events(3),
        Board::quote("first_blood", "🩸 First Blood", PUNISHED, Order::First),
    ]
}

/// Every board for one rule: the standard ones plus the rule's own.
pub fn rule_boards(rule: &dyn Rule) -> Vec<Board> {
    standard_rule_boards()
        .into_iter()
        .chain(rule.boards())
        .map(|b| b.for_rule(rule.id()))
        .collect()
}

/// Every board: global ones, then each rule's.
pub fn all_boards(rules: &[Arc<dyn Rule>]) -> Vec<Board> {
    let mut out = global_boards();
    for r in rules {
        out.extend(rule_boards(r.as_ref()));
    }
    out
}
