# SpiritBot 🎅

A Discord bot that enforces Christmas spirit.

Each day a new rule goes live. Members with the enforced role who break it get timed out or kicked
from voice chat. Admins aren't exempt: timeouts are enforced by the bot itself, which deletes the
offender's messages and disconnects them from VC until the timeout ends.

## Features

- **Daily rules.** Schedule rules ahead of time and the bot announces each one when it goes live.
- **AI judging.** Messages, images and GIFs are judged by [Jev](https://typesafe.ai), so "festive"
  means something more than a keyword list.
- **Voice checks.** The bot listens in VC and transcribes speech locally with
  [Whisper](https://github.com/ggerganov/whisper.cpp). No audio leaves your server.
- **Configurable punishments.** Choose a timeout (any length), a VC disconnect or nothing for each rule.
- **Monthly leaderboards.** Every judgement, punishment and timeout dodge is recorded, so at the end
  of the month you can crown the Naughty List, the Grinch, the longest festive streak and the best
  (and worst) things people said.
- **Easy to extend.** Each rule is a single Rust file.

## Rules

| Rule | What it does |
|---|---|
| `first_message` | Your first message each day must be festive (default: 5m timeout) |
| `vc_greeting` | When you join VC, you have 10 seconds to say something festive (default: disconnect) |

More are on the way. See [Adding a rule](#adding-a-rule) to write your own.

## Commands

Anyone can use the read-only commands. Everything else needs **Administrator**.

| Command | What it does |
|---|---|
| `/rule list` | All rules, whether each is on, and its punishment |
| `/rule enable <rule>` / `/rule disable <rule>` | Go live now (announced) / turn off |
| `/rule schedule <rule> <date>` / `/rule unschedule <rule>` | Auto-activate and announce on a day (`2026-12-05`, `12`, `tomorrow`, `+3`) |
| `/rule punishment <rule> <none\|timeout\|disconnect> [5m]` | Set what happens to rule breakers |
| `/rule reset <rule> [user]` | Forget what a rule remembers about a user (or everyone), so the rule can trigger again |
| `/spirit setup [role] [announce_channel] [log_channel]` | Setup checklist, with a one-shot fix |
| `/spirit role` / `announcechannel` / `logchannel` | Individual settings |
| `/spirit pardon <user>` | End someone's timeout early |
| `/spirit status` | Recent punishments |
| `/leaderboard me [user] [month]` | Where you (or someone) stand on every board |
| `/leaderboard show [month] [section] [board]` | Peek at the leaderboards privately (overview, one section, or top 10 of one board) |
| `/leaderboard post [month] [section] [board]` | Post them in the channel for everyone: the end-of-month reveal |
| `/leaderboard export [month]` | Every recorded event as a CSV, to build your own |
| `/leaderboard wipe confirm:true [user]` | Delete recorded stats, e.g. after testing |

`month` takes `2026-12`, `december`, `last` or `all` and defaults to the current month.

## Leaderboards

Everything is recorded for members with the enforced role, so at the end of the month you can run
`/leaderboard post month:last` and crown everyone. The overview opens with server totals (people,
messages, judgements, timeouts, VC time, the most festive, grinchiest and naughtiest days), then the
top 3 of every board below, then a rule-by-rule summary.

**😈 Naughty**

| Board | Who tops it |
|---|---|
| 🎅 Naughty List | Most punishments |
| ⏳ Hard Time / ⛓️ Longest Sentence | Most timeout time in total / in one go |
| 📉 Worst Day | Most punishments in a single day |
| 🔁 Repeat Offender / 😈 Naughty Streak | Most days punished / most days in a row |
| 🃏 Rule Collector | Most different rules broken |
| 🚪 Can't Stay Away | Most messages and VC joins attempted while timed out |
| 🧱 Talking to a Wall | Longest message typed while timed out (quoted) |
| 🩸 First Blood / ⚰️ Latest Victim | First and most recent punishment |
| 🙄 Bah Humbug | Said "humbug", "grinch" or "hate christmas" the most |
| 🙏 Teacher's Pet | Most pardons |

**😇 Nice**

| Board | Who tops it |
|---|---|
| 😇 Nice List / 🦹 The Grinch | Highest / lowest average festiveness (at least 3 judgements) |
| ✅ Most Reliable | Highest share of judgements passed (at least 5) |
| 🔥 Festive Streak | Most days in a row judged festive and never punished |
| 📈 Most Improved | Biggest rise in average score from the first half of the month to the second |
| 👼 Angel | Most judgements without ever being punished |
| ✨ Christmas Miracle / 🧊 Ice Cold | Most near-perfect (≥ 0.95) / zero-spirit (≤ 0.05) scores |
| 🎢 Mood Swings | Biggest gap between best and worst score |
| 🔍 Under Scrutiny | Judged the most |
| 🌟 Most Festive / 💀 Least Festive Moment | Best and worst single thing said, quoted |
| 😅 Closest Call / 📜 Festive Essay | Least festive thing that still passed / longest festive message |
| 🎄 Merry Christmas! / 🎅 Ho Ho Ho | Most seasonal greetings / ho ho hos |
| 🔔 Said "Christmas" / 🎁 Festive Emoji | Most mentions of Christmas / 🎄🎅❄️🎁… used |

**💬 Chat**

| Board | Who tops it |
|---|---|
| 💬 Chatterbox / ✍️ Wordsmith | Most messages / words |
| 📖 Novelist / 🗣️ Rambler | Longest message / most words per message |
| 📨 Busiest Day / 📅 Ever-Present | Most messages in one day / most days active |
| 📢 SHOUTER / 🔊 Loudest Message | Most all-caps messages / the longest one, quoted |
| ❓ Curious | Most questions |
| 🦉 Night Owl / 🐦 Early Bird | Most messages between midnight and 5am / 5am and 8am |
| 😂 Emoji Fiend / 🖼️ Meme Dealer / 🔗 Link Dropper | Most emoji / images, GIFs and stickers / links |
| ✏️ Second Thoughts | Most messages edited |

**🤝 Social**

| Board | Who tops it |
|---|---|
| ⭐ Crowd Pleaser / 👍 Generous | Most reactions received / given |
| 🎄 Festive Reactor | Most festive reactions given |
| 📣 Most Wanted / 🔔 Pinger | Pinged the most / pinged others the most |
| ↩️ Reply Guy / 🗨️ Conversation Starter | Most replies sent / received |

**🎧 Voice**

| Board | Who tops it |
|---|---|
| 🎧 VC Regular / 📆 VC Every Day | Most joins / most days in VC |
| 🪵 Yule Log / 🕯️ Marathon / 🌙 Longest VC Day | Most time in VC in total / in one session / in one day |
| 🐇 Channel Hopper / 💨 Drive-By | Most channel switches / visits under a minute |
| 🙉 Deaf to the Carols / 🤫 Silent Treatment | Deafened / muted the most |
| 📺 Streamer / 📸 Camera On | Went live / turned the camera on the most |

**📜 Every rule** also gets its own Most punished, Most and Least festive, Best average and First
Blood boards, plus any it adds. `vc_greeting` adds 🤐 Silent Night (joined VC and said nothing) and
🏃 Cold Feet (left before the 10 seconds were up).

VC time is tracked in memory. People already in VC when the bot starts are picked up from then, but
a session that was running during a restart loses the time before the restart.

## Self-hosting

### 1. Create the Discord bot

1. Create an application at <https://discord.com/developers/applications>. Under **Bot**, enable the
   **Presence**, **Server Members** and **Message Content** intents, then copy the token.
2. Invite the bot with the `bot` and `applications.commands` scopes and the **Administrator**
   permission.

### 2. Run it

You need Docker with Compose. A prebuilt image is published at `ghcr.io/sauce-2/spritbot-slop:latest`.

```bash
mkdir spiritbot && cd spiritbot
curl -O https://raw.githubusercontent.com/sauce-2/spritbot-slop/main/docker-compose.yml
curl -o .env https://raw.githubusercontent.com/sauce-2/spritbot-slop/main/.env.example
# Fill in DISCORD_TOKEN and JEV_API_KEY in .env
docker compose up -d
```

The compose file includes [Watchtower](https://github.com/nicholas-fedor/watchtower), which keeps the
bot updated automatically. Remove that service if you'd rather update by hand. If you use rootless
Podman, set `CONTAINER_SOCKET` in `.env` to your Podman socket (for example
`/run/user/1000/podman/podman.sock`). Data is stored in
SQLite in the `spiritbot-data` volume, so it survives updates.

### 3. Configure

The bot posts a setup checklist when it joins your server. To set it up in one go, run:

```
/spirit setup role:@Elves announce_channel:#rules log_channel:#naughty-list
```

Then turn on a rule with `/rule enable`, or schedule rules for the days ahead with `/rule schedule`.

### Configuration

All settings are environment variables in `.env`:

| Variable | Description |
|---|---|
| `DISCORD_TOKEN` | **Required.** Your bot token |
| `JEV_API_KEY` | Needed for AI-judged rules (all current rules). Without it, those rules can't go live |
| `BOT_TZ` | Timezone that defines "today" for daily rules and schedules (default `Europe/London`) |
| `DEV_GUILD_ID` | Register commands in one server only, so they update instantly. Leave empty in production |
| `RUST_LOG` | Log level |
| `CONTAINER_SOCKET` | Socket Watchtower uses to manage containers (default `/var/run/docker.sock`) |

## Development

Builds run in Docker, so you don't need cmake, clang or opus installed locally.

```bash
cp .env.example .env   # set DISCORD_TOKEN, JEV_API_KEY and DEV_GUILD_ID (your test server)
docker compose -f docker-compose.dev.yml up -d --build
```

To run cargo in a toolchain image with the native dependencies:

```bash
./dev.sh cargo test
./dev.sh cargo clippy
```

### Adding a rule

1. Create `src/rules/<rule>.rs` with a struct that implements `Rule`. Implement only the hooks you
   need: `on_message`, `on_voice_join`, `on_presence` and `on_tick`.
2. Add it to `registry()` in `src/rules/mod.rs`.
3. Use the helpers on `RuleCx`:
   - `cx.ask_msg(m, "yes/no question", criteria)` asks Jev anything about a message (text and images).
     Prefer this to hand-written heuristics.
   - `cx.punish(user, reason)` punishes someone
   - `cx.state` / `cx.set_state` store per-user memory
   - `cx.record_judgement(user, score, what_they_said)` whenever Jev judges something, festive or
     not. This feeds the leaderboards, and the rule gets its own most and least festive boards.
     Punishments are recorded for you.
   - `cx.record(user, Event::new("my_kind"))` records anything else worth ranking. Add a board for it
     in the rule's `boards()`, e.g. `Board::most("dodged", "🏃 Most dodged", &["my_kind"], "times")`.
     Boards can also rank totals, averages, records for a single day (`.per_day()`), distinct days,
     or quote standout moments (`Board::quote`); see `src/stats/boards.rs`.
   - `cx.data.jev` is the raw Jev client (Choice/Score questions, several questions in one call)
4. Test it in your dev server: enable the rule, break it yourself, then `/rule reset` to try again.

Pull requests with new rules are welcome. CI runs `cargo fmt`, `clippy` and tests on every PR.
