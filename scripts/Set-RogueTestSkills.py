"""Explicit test-only allocation for AdminRogue80; run while logged out."""
from datetime import datetime
from pathlib import Path
import sqlite3

root = Path(__file__).resolve().parents[1]
db = sqlite3.connect(root / "reko.db", timeout=10)
db.execute("PRAGMA foreign_keys=ON")
character = db.execute(
    "SELECT slot,class FROM lobby_characters WHERE account='admin' AND name='AdminRogue80'"
).fetchone()
assert character and character[1] in (108, 208), "Master rogue test fixture missing"
backup = root / "logs" / ("before-all-rogue-skills-" + datetime.now().strftime("%Y%m%d-%H%M%S") + ".db")
with sqlite3.connect(backup) as target:
    db.backup(target)
points = bytes([0, 0, 0, 0, 0, 80, 80, 80, 20])
with db:
    changed = db.execute(
        "UPDATE character_progress SET points=? WHERE account='admin' AND slot=? AND level=80",
        (points, character[0]),
    ).rowcount
    assert changed == 1, "Level-80 progress row missing; no changes performed"
assert db.execute("SELECT points FROM character_progress WHERE account='admin' AND slot=?", (character[0],)).fetchone()[0] == points
assert not db.execute("PRAGMA foreign_key_check").fetchall()
print("AdminRogue80: Archery80, Assassin80, Explore80, Master20; free points0")
print("Level, stats, equipment, inventory and money preserved")
print("Backup:", backup)
