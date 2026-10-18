CREATE TABLE IF NOT EXISTS clans (
 id INTEGER PRIMARY KEY CHECK(id BETWEEN 1 AND 30000),
 name TEXT NOT NULL COLLATE NOCASE UNIQUE CHECK(length(name) BETWEEN 1 AND 20),
 nation INTEGER NOT NULL CHECK(nation IN (1,2))
);
CREATE TABLE IF NOT EXISTS clan_members (
 account TEXT NOT NULL COLLATE NOCASE, slot INTEGER NOT NULL,
 clan_id INTEGER NOT NULL REFERENCES clans(id) ON DELETE CASCADE,
 fame INTEGER NOT NULL CHECK(fame BETWEEN 1 AND 6),
 PRIMARY KEY(account,slot),
 FOREIGN KEY(account,slot) REFERENCES lobby_characters(account,slot) ON DELETE CASCADE
);
CREATE UNIQUE INDEX IF NOT EXISTS clan_one_chief ON clan_members(clan_id) WHERE fame=1;
CREATE TRIGGER IF NOT EXISTS clan_member_nation BEFORE INSERT ON clan_members
WHEN (SELECT nation FROM clans WHERE id=NEW.clan_id) !=
 (SELECT CASE WHEN race<10 THEN 1 ELSE 2 END FROM lobby_characters WHERE account=NEW.account AND slot=NEW.slot)
BEGIN SELECT RAISE(ABORT,'clan nation mismatch'); END;
