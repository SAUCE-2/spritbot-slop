-- Everything that happens, for the monthly leaderboards (see src/stats.rs).
-- One row per event; boards aggregate over it, so new rules only need to record events.
CREATE TABLE stat_events (
    id         INTEGER PRIMARY KEY AUTOINCREMENT,
    guild_id   INTEGER NOT NULL,
    user_id    INTEGER NOT NULL,
    rule_id    TEXT,                -- NULL for events not tied to a rule
    kind       TEXT    NOT NULL,    -- see stats::kind and each rule's own kinds
    score      REAL,                -- Jev festiveness score (0..1), for judgements
    amount     REAL,                -- e.g. timeout seconds
    text       TEXT,                -- what they said, why they were punished, ...
    day        TEXT    NOT NULL,    -- YYYY-MM-DD in the bot's timezone
    created_at TEXT    NOT NULL
);

CREATE INDEX stat_events_board ON stat_events (guild_id, kind, day);
CREATE INDEX stat_events_user ON stat_events (guild_id, user_id, day);
