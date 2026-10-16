CREATE TABLE IF NOT EXISTS character_quest_rewards (
    account TEXT NOT NULL COLLATE NOCASE,
    slot INTEGER NOT NULL,
    experience INTEGER NOT NULL DEFAULT 0,
    loyalty INTEGER NOT NULL DEFAULT 300,
    monthly INTEGER NOT NULL DEFAULT 0,
    PRIMARY KEY(account,slot),
    FOREIGN KEY(account,slot) REFERENCES lobby_characters(account,slot) ON DELETE CASCADE
);

CREATE TRIGGER IF NOT EXISTS character_starting_national_points
AFTER INSERT ON lobby_characters
BEGIN
    INSERT OR IGNORE INTO character_quest_rewards(account,slot,experience,loyalty,monthly)
    VALUES(NEW.account,NEW.slot,0,300,0);
END;
