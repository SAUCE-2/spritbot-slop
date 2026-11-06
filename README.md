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
- **Activity log.** Everything that happens is logged raw (messages, reactions, voice, presence,
  judgements, punishments...), so at the end of the month you can build whatever leaderboards you
  like from it.
- **Easy to extend.** Each rule is a single Rust file.

## Rules

| Rule | What it does |
|---|---|
| `first_message` | Your first message each day must be festive (default: 5m timeout) |
| `vc_greeting` | When you join VC, you have 10 seconds to say something festive (default: disconnect) |
| `sign_off` | Every message must end with a Christmas greeting (default: 1m timeout) |

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
| `/status` | Bot uptime, gateway latency, memory, and rule/punishment stats |
| `/stats summary [month]` | How much has been logged, by kind |
| `/stats export [month]` | Download the activity log as gzipped JSON Lines |
| `/stats wipe confirm:true [user]` | Delete logged activity, e.g. after testing |

`month` takes `2026-12`, `december`, `last` or `all` and defaults to the current month.

## Activity log

Rather than deciding up front what to rank, the bot logs everything into the `activity_log` table
and leaderboards get made from it at the end of the month. Each entry has the server, the user, the
channel, a `kind`, the rule involved, whether the user had the enforced role at the time, the day
(in `BOT_TZ`), a UTC timestamp, and a JSON `data` payload. Bots aren't logged.

| kind | data |
|---|---|
| `message` | The whole Discord message: content, attachments, embeds, stickers, mentions, reply |
| `message_edit` / `message_delete` / `message_delete_bulk` | The edit event / the deleted message's id (join it to `message`) |
| `reaction_add` / `reaction_remove` | Emoji, message id, and who wrote the message |
| `voice` | `join`/`leave`/`move`/`update`, with old and new mute, deafen, stream and camera state |
| `vc_session` | Seconds spent in VC, logged on leaving |
| `presence` | Status, devices and activities (games, Spotify, custom status), logged when they change |
| `typing`, `command`, `thread_create` | Who typed where, which slash command was used, new threads |
| `member_seen` / `member_join` / `member_leave` / `member_update` | Names, nick and roles (snapshot at startup, then changes) |
| `judged` | A Jev verdict: `score`, what was `said`, and whether it was `festive` |
| `punished` | Punishment, what actually happened, the reason, and the timeout length |
| `timeout_message` / `timeout_vc` | Tried to talk or join VC while timed out |
| `pardoned`, `rule_activated`, `rule_disabled` | Admin actions |
| `vc_greeting_heard` / `vc_greeting_silent` / `vc_greeting_left` | What people said on joining VC, or that they said nothing or left |

To get the data out, run `/stats export month:december`. It gives you a `.jsonl.gz` file with one JSON
object per line. If it's over Discord's 10 MB upload limit, copy the SQLite database off the server
instead:

```bash
docker compose cp spiritbot:/data/spiritbot.db .
```

Then query it with SQLite's JSON functions, e.g. who got punished the most:

```sql
SELECT user_id, COUNT(*) FROM activity_log
WHERE kind = 'punished' AND day LIKE '2026-12-%' GROUP BY user_id ORDER BY 2 DESC;
```

The log holds message content, so treat the export and the database as private.

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
bot updated automatically. Remove that service if you'd rather update by hand. Data is stored in
SQLite in the `spiritbot-data` volume, so it survives updates.

#### Rootless Podman

Watchtower can't update containers under Podman: the container it recreates fails to start with
`cannot set memory swappiness with cgroupv2`. Use a systemd timer instead. It pulls the image and
recreates the bot when something changed. Point Compose at Podman's socket, start only the bot, and
add the timer:

```bash
systemctl --user enable --now podman.socket podman-restart.service
sudo loginctl enable-linger $USER   # keep running after logout
export DOCKER_HOST=unix:///run/user/$(id -u)/podman/podman.sock   # also add to ~/.bashrc
docker compose up -d spiritbot   # not plain `up -d`, which also starts Watchtower

mkdir -p ~/.config/systemd/user
cat > ~/.config/systemd/user/spiritbot-update.service <<EOF
[Service]
Type=oneshot
WorkingDirectory=$PWD
Environment=DOCKER_HOST=$DOCKER_HOST
ExecStart=$(command -v docker) compose pull -q spiritbot
ExecStart=$(command -v docker) compose up -d spiritbot
ExecStartPost=$(command -v podman) image prune -f
EOF
cat > ~/.config/systemd/user/spiritbot-update.timer <<EOF
[Timer]
OnBootSec=1min
OnUnitActiveSec=2min

[Install]
WantedBy=timers.target
EOF
systemctl --user daemon-reload && systemctl --user enable --now spiritbot-update.timer
```

If you use the standalone `docker-compose` binary, replace `docker compose` with it. The compose
file uses `restart: always`, so `podman-restart.service` starts the bot again after a reboot.

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
     not. Punishments are logged for you.
   - `cx.log(user, "my_kind", json!({...}))` logs anything else that happened. Log generously, since you
     can't go back for data later.
   - `cx.data.jev` is the raw Jev client (Choice/Score questions, several questions in one call)
4. Test it in your dev server: enable the rule, break it yourself, then `/rule reset` to try again.

Pull requests with new rules are welcome. CI runs `cargo fmt`, `clippy` and tests on every PR.
