"""Offline, idempotent complete skill scrolls for the two mage test fixtures."""
import argparse
import csv
from datetime import datetime
from pathlib import Path
import re
import sqlite3
import subprocess

root = Path(__file__).resolve().parents[1]
parser = argparse.ArgumentParser()
parser.add_argument("--apply", action="store_true")
args = parser.parse_args()
supplies = {370001000, 370002000, 370003000, 379065000, 379069000, 379070000}
dump = subprocess.check_output([
    str(root / "target/debug/reko.exe"), "dump-tbl",
    str(root.parent / "KnightOnline1453/Data/item_org_us.tbl"),
    "--limit", "20000", "--full"], encoding="utf-8")
items = {}
for line in dump.splitlines():
    match = re.match(r"\s*\[\s*\d+\]\s+(.*)", line)
    if match:
        row = next(csv.reader([match[1]], escapechar="\\", skipinitialspace=True))
        if int(row[0]) in supplies:
            assert int(row[19]) == 1 and int(row[23]) == 0
            items[int(row[0])] = row[2]
assert set(items) == supplies
fixtures = [("testkarus", "KarusMage80", 110), ("testelmorad", "ElmoMage80", 210)]
db = sqlite3.connect(root / "reko.db", timeout=10)
db.execute("PRAGMA foreign_keys=ON")
for account, name, klass in fixtures:
    assert db.execute("SELECT class FROM lobby_characters WHERE account=? AND name=?", (account, name)).fetchone() == (klass,)
print("Ensure one of each original mage skill scroll (nonstackable):", items)
if not args.apply:
    print("Dry run; stop the server and pass --apply to add missing scrolls.")
    raise SystemExit(0)
backup = root / "logs" / ("before-mage-supplies-" + datetime.now().strftime("%Y%m%d-%H%M%S") + ".db")
with sqlite3.connect(backup) as target:
    db.backup(target)
with db:
    db.execute("CREATE TABLE IF NOT EXISTS mage_test_supply_grants(account TEXT NOT NULL, slot INTEGER NOT NULL, PRIMARY KEY(account,slot))")
    for account, name, _ in fixtures:
        slot, = db.execute("SELECT slot FROM lobby_characters WHERE account=? AND name=?", (account, name)).fetchone()
        occupied = {r[0] for r in db.execute("SELECT inventory_slot FROM character_inventory WHERE account=? AND slot=?", (account, slot))}
        owned = {r[0] for r in db.execute("SELECT item_id FROM character_inventory WHERE account=? AND slot=? AND count>0", (account, slot))}
        missing = sorted(supplies - owned)
        empty = [pos for pos in range(28) if pos not in occupied]
        assert len(empty) >= len(missing), "Need empty bag slots for " + name
        for pos, item in zip(empty, missing):
            db.execute("INSERT INTO character_inventory VALUES (?,?,?,?,1,1)", (account, slot, pos, item))
        db.execute("INSERT OR IGNORE INTO mage_test_supply_grants VALUES (?,?)", (account, slot))
        print("Added to", name, ":", [items[item] for item in missing])
assert not db.execute("PRAGMA foreign_key_check").fetchall()
print("Backup:", backup)
