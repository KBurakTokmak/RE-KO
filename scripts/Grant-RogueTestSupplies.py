"""One-time offline test supplies; preserves equipment and skill allocations."""
from datetime import datetime
from pathlib import Path
import sqlite3

root = Path(__file__).resolve().parents[1]
db = sqlite3.connect(root / "reko.db", timeout=10)
db.execute("PRAGMA foreign_keys=ON")
character = db.execute(
    "SELECT slot,class FROM lobby_characters WHERE account='admin' AND name='AdminRogue80'"
).fetchone()
assert character and character[1] in (108, 208), "Master rogue fixture missing"
backup = root / "logs" / ("before-rogue-supplies-" + datetime.now().strftime("%Y%m%d-%H%M%S") + ".db")
with sqlite3.connect(backup) as target:
    db.backup(target)
with db:
    if "count" not in {row[1] for row in db.execute("PRAGMA table_info(character_inventory)")}:
        db.execute("ALTER TABLE character_inventory ADD COLUMN count INTEGER NOT NULL DEFAULT 1 CHECK(count BETWEEN 1 AND 9999)")
    db.execute("CREATE TABLE IF NOT EXISTS rogue_test_supply_grants(account TEXT NOT NULL, slot INTEGER NOT NULL, version INTEGER NOT NULL, PRIMARY KEY(account,slot))")
    if db.execute("SELECT 1 FROM rogue_test_supply_grants WHERE account='admin' AND slot=?", (character[0],)).fetchone():
        print("Test supplies already granted; no refill or character reset")
    else:
        occupied = {row[0] for row in db.execute("SELECT inventory_slot FROM character_inventory WHERE account='admin' AND slot=?", (character[0],))}
        empty = [slot for slot in range(28) if slot not in occupied]
        supplies = [(160100001, 5000, 1), (391010000, 1, 9999), (370004000, 1, 20), (379064000, 1, 20), (379060000, 1, 20)]
        assert len(empty) >= len(supplies), "Need five empty inventory slots; no changes performed"
        for slot, (item, durability, count) in zip(empty, supplies):
            db.execute("INSERT INTO character_inventory(account,slot,inventory_slot,item_id,durability,count) VALUES ('admin',?,?,?,?,?)", (character[0], slot, item, durability, count))
            print("Granted", item, "count", count, "bag slot", slot)
        db.execute("INSERT INTO rogue_test_supply_grants VALUES ('admin',?,1)", (character[0],))
assert not db.execute("PRAGMA foreign_key_check").fetchall()
print("Backup:", backup)
