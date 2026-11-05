-- Discord snowflakes are stored as INTEGER (i64); dates/times as RFC3339 / YYYY-MM-DD text.

CREATE TABLE guild_config (
    guild_id            INTEGER PRIMARY KEY,
    enforced_role_id    INTEGER,
    announce_channel_id INTEGER,
    log_channel_id      INTEGER,
    last_setup_hash     TEXT
);

CREATE TABLE rule_config (
    guild_id     INTEGER NOT NULL,
    rule_id      TEXT    NOT NULL,
    enabled      INTEGER NOT NULL DEFAULT 0,
    punishment   TEXT,                       -- JSON Punishment; NULL = rule default
    start_date   TEXT,                       -- pending scheduled activation (YYYY-MM-DD)
    activated_at TEXT,
    PRIMARY KEY (guild_id, rule_id)
);

CREATE TABLE rule_state (
    guild_id   INTEGER NOT NULL,
    rule_id    TEXT    NOT NULL,
    user_id    INTEGER NOT NULL,
    key        TEXT    NOT NULL,
    value      TEXT    NOT NULL,
    updated_at TEXT    NOT NULL,
    PRIMARY KEY (guild_id, rule_id, user_id, key)
);

-- Bot-enforced timeouts: until then, messages are deleted and VC joins are kicked.
CREATE TABLE timeouts (
    guild_id INTEGER NOT NULL,
    user_id  INTEGER NOT NULL,
    until    TEXT    NOT NULL,
    PRIMARY KEY (guild_id, user_id)
);

CREATE TABLE punishments (
    id         INTEGER PRIMARY KEY AUTOINCREMENT,
    guild_id   INTEGER NOT NULL,
    rule_id    TEXT    NOT NULL,
    user_id    INTEGER NOT NULL,
    action     TEXT    NOT NULL,
    reason     TEXT    NOT NULL,
    created_at TEXT    NOT NULL
);
