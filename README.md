# SpiritBot 🎅

A Discord bot that enforces Christmas spirit.

Each day a new rule goes live. Members with the enforced role who break it get timed out or kicked
from voice chat. Admins aren't exempt: timeouts are enforced by the bot itself, which deletes the
offender's messages and disconnects them from VC until the timeout ends.

## Features

- **Daily rules.** Schedule rules ahead of time and the bot announces each one when it goes live.
- **AI judging.** Messages, images and GIFs are judged by [Jev](https://typesafe.ai), so "festive"
  means something more than a keyword list. Edits are judged too, so a festive message can't be
  edited into something else afterwards. Each message costs one Jev call, however many rules are on.
- **Voice checks.** The bot listens in VC and transcribes speech locally with
  [Whisper](https://github.com/ggerganov/whisper.cpp). Audio never leaves your server, but the
  transcript is judged by Jev (see [Privacy](#privacy)).
- **Configurable punishments.** Choose a timeout (any length), a VC disconnect or nothing for each rule.
- **Activity log.** Everything that happens is logged raw (messages, reactions, voice, presence,
  judgements, punishments...), so at the end of the month you can build whatever leaderboards you
  like from it.
- **Easy to extend.** Each rule is a single Rust file.

## Rules

| Rule | What it does |
|---|---|
| `first_message` | Your first message each day must be festive, including if you edit it later (default: 5m timeout) |
| `vc_greeting` | When you join VC, you have 10 seconds to say something festive (default: disconnect) |
| `sign_off` | Every message must end with a Christmas greeting, and every edit is checked again (default: 1m timeout) |

More are on the way. See [Adding a rule](#adding-a-rule) to write your own.

## Privacy

The bot only judges members with the enforced role, and only while a rule that needs it is live.

- **Sent to [Jev](https://typesafe.ai):** the text of their messages, plus attached images and
  embed previews (GIFs, link thumbnails), and transcripts of what was said in VC. Voice audio
  itself is transcribed locally and never sent anywhere.
- **Posted to the log channel:** every punishment (and, for rules in shadow mode, every would-be
  punishment), with the user, the rule and the reason. Some
  reasons quote what the user said: `vc_greeting` includes the full VC transcript. The same reason
  is stored in the bot's database and, when the bot disconnects someone from VC, in the Discord
  audit log.

## Commands

Anyone can use the read-only commands. Everything else needs **Administrator**.

| Command | What it does |
|---|---|
| `/rule list` | All rules, whether each is on (and shadowed), its punishment and its threshold |
| `/rule enable <rule>` / `/rule disable <rule>` | Go live now (announced, unless shadowed) / turn off |
| `/rule schedule <rule> <date>` / `/rule unschedule <rule>` | Auto-activate and announce on a day (`2026-12-05`, `12`, `tomorrow`, `+3`) in the server's timezone |
| `/rule punishment <rule> <none\|timeout\|disconnect> [5m]` | Set what happens to rule breakers |
| `/rule threshold <rule> <0-1\|default>` | Set the Jev score a rule's judged messages and VC greetings must reach (default `0.50`; lower is more lenient) |
| `/rule shadow <rule> <on\|off>` | Shadow mode: judge and log as usual, but only report would-be breaches to the log channel (no replies, no punishments) |
| `/rule reset <rule> [user]` | Forget what a rule remembers about a user (or everyone), so the rule can trigger again |
| `/spirit setup [role] [announce_channel] [log_channel]` | Setup checklist, with a one-shot fix |
| `/spirit role` / `announcechannel` / `logchannel` | Individual settings |
| `/spirit timezone <tz>` | Set the server's timezone (IANA name like `America/New_York`, or `default` for `BOT_TZ`), which decides when "today" starts for daily rules and schedules |
| `/spirit pardon <user>` | End someone's timeout early |
| `/spirit status` | Recent punishments |
| `/status` | Bot uptime, gateway latency, memory and running commit, plus this server's rule and punishment stats |
| `/stats summary [month]` | How much has been logged, by kind |
| `/stats export [month]` | Download the activity log as gzipped JSON Lines |
| `/stats wipe confirm:true [user]` | Delete logged activity, e.g. after testing |

`month` takes `2026-12`, `december`, `last` or `all` and defaults to the current month.

### Tuning a rule

Jev scores everything it judges from 0 to 1, and a rule is broken when the score falls below the
rule's threshold. To find the right threshold for your server before anyone gets punished:

1. `/rule shadow <rule> on`, then `/rule enable <rule>`. A shadowed rule is judged as usual and
   every verdict is logged, but instead of replying and punishing, the bot posts to the log channel:
   `👻 [shadow] @user would have broken **rule** (score 0.31, threshold 0.50): <reason>`. It isn't
   announced publicly while shadowed (the log channel gets a note instead).
2. Watch the log channel. Too many false alarms? Lower the bar with `/rule threshold <rule> 0.35`.
   Letting too much through? Raise it. `/rule threshold <rule> default` goes back to `0.50`.
3. `/rule shadow <rule> off` when you're happy. The rule then goes live for real and is announced.

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

If your server isn't on UK time, set its timezone with `/spirit timezone` (for example
`/spirit timezone America/New_York`). Then turn on a rule with `/rule enable`, or schedule rules
for the days ahead with `/rule schedule`.

### Configuration

All settings are environment variables in `.env`:

| Variable | Description |
|---|---|
| `DISCORD_TOKEN` | **Required.** Your bot token |
| `JEV_API_KEY` | Needed for AI-judged rules (all current rules). Without it, those rules can't go live |
| `BOT_TZ` | Default timezone that defines "today" for daily rules and schedules, for servers that haven't set their own with `/spirit timezone` (default `Europe/London`) |
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
   need:
   - `message_check` for chat messages: return a `MessageCheck` (a yes/no question for Jev, what
     counts as yes and no, and the reply and log reason for a "no"), or `None` to let the message
     through. Don't call Jev yourself: the bot puts every enabled rule's check on a message into one
     Jev call, logs each verdict, and replies and punishes for each "no". It's called for new
     messages and for edits (`m.kind` is `MsgKind::Posted` or `MsgKind::Edited`), so decide what an
     edit means for your rule.
   - `message_unjudged` if Jev couldn't answer your check (nobody is punished): undo anything
     `message_check` reserved, e.g. the day `first_message` claimed.
   - `on_voice_join`, `on_presence` and `on_tick` for everything else.
2. Add it to `registry()` in `src/rules/mod.rs`.
3. Use the helpers on `RuleScope` (passed to the message hooks; `RuleCx` in the other hooks derefs to it):
   - `s.state` reads per-user memory; `s.swap_state` writes it only if it still holds what you read,
     so two messages at once can't both claim the same thing. `s.clear_state` forgets it.
   - `s.today()` is today in the guild's timezone
   - `s.log(user, "my_kind", json!({...}))` logs anything else that happened. Log generously, since
     you can't go back for data later. Judgements and punishments are logged for you.
   - Outside messages, `crate::ai::ask_noul` asks Jev one question, `cx.judged(user, score,
     what_they_said)` logs its verdict and says whether it meets the rule's threshold, and
     `cx.enforce(user, Some(score), reason)` punishes someone (or, in shadow mode, only reports it
     to the log channel). Don't compare scores or punish yourself, so thresholds and shadow mode
     work for your rule too.
4. Test it in your dev server: enable the rule, break it yourself, then `/rule reset` to try again.

Pull requests with new rules are welcome. CI runs `cargo fmt`, `clippy` and tests on every PR.
