-- IANA timezone name (e.g. "America/New_York") that defines "today" for this guild.
-- NULL = the bot-wide default from BOT_TZ.
ALTER TABLE guild_config ADD COLUMN timezone TEXT;
