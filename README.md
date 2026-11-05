# SpiritBot 🎅

A Discord bot that enforces Christmas spirit. A new rule goes live each day, and members with the
enforced role who break a rule get timed out or disconnected from VC. Admins aren't exempt either:
timeouts are enforced by the bot itself, which deletes the person's messages and kicks them from VC
until the timeout ends.

- **Rules** are Rust modules in [`src/rules/`](src/rules). There's one file per rule, and every rule is listed in `registry()`.
- **Jev does the judging** ([typesafe.ai](https://typesafe.ai)). Each rule asks Jev its own question, and image attachments and GIF previews are sent along too. New rules should classify with Jev (`cx.ask_msg`) rather than hand-written heuristics.
- **Voice checks** listen with songbird and transcribe with a local Whisper model (`ggml-base.en`). Everyone in a channel is listened to at the same time.
- **Deploys** go GitHub Actions → GHCR → Watchtower on the Steam Machine.

## Commands

Read-only commands are open to everyone. Everything else needs **Administrator**.

| Command | What it does |
|---|---|
| `/rule list` | All rules, whether each is on, and its punishment |
| `/rule enable <rule>` / `/rule disable <rule>` | Go live now (announced) / turn off |
| `/rule schedule <rule> <date>` / `/rule unschedule <rule>` | Auto-activate and announce on a day (`2026-12-05`, `12`, `tomorrow`, `+3`) |
| `/rule punishment <rule> <none\|timeout\|disconnect> [5m]` | Set what happens to rule breakers |
| `/rule reset <rule> [user]` | Forget what a rule remembers about a user (or everyone), so you can trigger it again |
| `/spirit setup [role] [announce_channel] [log_channel]` | Setup checklist, with a one-shot fix |
| `/spirit role` / `announcechannel` / `logchannel` | Individual settings |
| `/spirit pardon <user>` | End someone's timeout early |
| `/spirit status` | Recent punishments |

## Rules

| Rule | What it does |
|---|---|
| `first_message` | Your first message each day must be festive, judged by Jev (default: 5m timeout) |
| `vc_greeting` | When you join VC, say something festive within 10 seconds, heard by Whisper and judged by Jev (default: disconnect) |

Rules are added one per day. Each rule is a new file in `src/rules/`, registered in
`registry()`. See [Adding a rule](#adding-a-rule).

## Discord setup

1. Create an application at https://discord.com/developers/applications. Under **Bot**, enable the
   **Presence**, **Server Members** and **Message Content** intents, then copy the token.
2. Invite the bot with the `bot` and `applications.commands` scopes and the **Administrator**
   permission.
3. The bot posts a setup checklist when it joins. Run `/spirit setup role:@Elves announce_channel:#rules log_channel:#naughty-list`.

## Running locally

Run builds through Docker so you don't need cmake, clang or opus installed:

```bash
cp .env.example .env   # fill in DISCORD_TOKEN, DEV_GUILD_ID (your test server), JEV_API_KEY
docker compose -f docker-compose.dev.yml up -d --build   # container: spiritbot-dev
```

To run cargo in the toolchain image instead (it has whisper/opus deps), use `./dev.sh cargo test`
or `./dev.sh cargo clippy`.

## Deploying to the Steam Machine

CI builds `ghcr.io/sauce-2/spiritbot:latest` on every push to `main`. After the first push, open the
package on GitHub and set its visibility to **public**, so the machine can pull it without logging in.

On the Steam Machine:

```bash
mkdir ~/spiritbot && cd ~/spiritbot
# copy docker-compose.yml and a filled-in .env here
docker compose up -d
```

Watchtower checks for a new image every 2 minutes and restarts the bot when one appears. The SQLite
database lives in the `spiritbot-data` volume, so it survives updates.

> SteamOS has a read-only root filesystem, so packages installed with `pacman` are wiped on OS
> updates. Install Docker in a way that survives updates, or use Podman (it ships with SteamOS)
> with its Docker-compatible socket.

## Adding a rule

1. Create `src/rules/<rule>.rs` with a struct that implements `Rule`. Implement only the hooks you
   need: `on_message`, `on_voice_join`, `on_presence` and `on_tick`.
2. Add it to `registry()` in `src/rules/mod.rs`.
3. Helpers on `RuleCx`:
   - `cx.ask_msg(m, "yes/no question", criteria)` asks Jev anything about a message (text + images)
   - `cx.punish(user, reason)` punishes someone
   - `cx.state` / `cx.set_state` store per-user memory
   - `cx.data.jev` is the raw Jev client (Choice/Score questions, several questions in one call)
4. Push. CI deploys it. Enable it and break it yourself to test, `/rule reset` it to try again, then
   `/rule schedule` it.
