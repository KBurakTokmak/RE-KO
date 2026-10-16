"""Explicit admin fixture quest completions and revocable admin privilege."""
from datetime import datetime
from pathlib import Path
import sqlite3

root = Path(__file__).resolve().parents[1]
db = sqlite3.connect(root / "reko.db", timeout=10)
db.execute("PRAGMA foreign_keys=ON")
character = db.execute("SELECT slot,class FROM lobby_characters WHERE account='admin' AND name='AdminRogue80'").fetchone()
assert character and character[1] in (108, 208), "Master rogue fixture missing"
backup = root / "logs" / ("before-rogue-quest-unlocks-" + datetime.now().strftime("%Y%m%d-%H%M%S") + ".db")
with sqlite3.connect(backup) as target:
    db.backup(target)
with db:
    db.execute("CREATE TABLE IF NOT EXISTS character_quests (account TEXT NOT NULL COLLATE NOCASE, slot INTEGER NOT NULL, quest INTEGER NOT NULL CHECK(quest BETWEEN 1 AND 32767), state INTEGER NOT NULL CHECK(state BETWEEN 0 AND 2), PRIMARY KEY(account,slot,quest), FOREIGN KEY(account,slot) REFERENCES lobby_characters(account,slot) ON DELETE CASCADE)")
    db.execute("CREATE TABLE IF NOT EXISTS character_privileges (account TEXT NOT NULL COLLATE NOCASE, slot INTEGER NOT NULL, is_admin INTEGER NOT NULL DEFAULT 0 CHECK(is_admin IN (0,1)), PRIMARY KEY(account,slot), FOREIGN KEY(account,slot) REFERENCES lobby_characters(account,slot) ON DELETE CASCADE)")
    for quest in (52, 512, 513, 514):
        db.execute("INSERT INTO character_quests VALUES ('admin',?,?,2) ON CONFLICT(account,slot,quest) DO UPDATE SET state=2", (character[0], quest))
    db.execute("INSERT INTO character_privileges VALUES ('admin',?,1) ON CONFLICT(account,slot) DO UPDATE SET is_admin=1", (character[0],))
assert not db.execute("PRAGMA foreign_key_check").fetchall()
print("AdminRogue80: rogue skill quests52/512/513/514 completed; admin privilege granted")
print("Normal characters, skill allocations, stats, gear and money preserved")
print("Backup:", backup)
