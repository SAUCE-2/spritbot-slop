-- Jev score a rule's judged things must reach (0..=1). NULL = the bot default (stats::FESTIVE).
ALTER TABLE rule_config ADD COLUMN threshold REAL;
-- 1 = shadow mode: judge and log as usual, but only report would-be breaches to the log channel.
ALTER TABLE rule_config ADD COLUMN shadow INTEGER NOT NULL DEFAULT 0;
