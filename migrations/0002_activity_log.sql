-- Raw record of everything that happens, kept so leaderboards (or anything else) can be worked
-- out from it after the fact. See src/stats/.
CREATE TABLE activity_log (
    id         INTEGER PRIMARY KEY AUTOINCREMENT,
    guild_id   INTEGER NOT NULL,
    user_id    INTEGER,             -- who it's about; NULL for server-wide events
    channel_id INTEGER,
    kind       TEXT    NOT NULL,    -- message, voice, reaction_add, punished, judged, ...
    rule_id    TEXT,                -- the rule involved, if any
    enforced   INTEGER,             -- did the user have the enforced role (NULL = unknown)
    data       TEXT    NOT NULL,    -- JSON: the Discord payload, or the bot's own details
    day        TEXT    NOT NULL,    -- YYYY-MM-DD in the bot's timezone
    at         TEXT    NOT NULL     -- RFC3339 UTC
);

CREATE INDEX activity_log_kind ON activity_log (guild_id, kind, day);
CREATE INDEX activity_log_user ON activity_log (guild_id, user_id, day);
