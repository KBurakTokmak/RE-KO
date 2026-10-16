"""Offline admin roles and one-time class stones/master scrolls for fixtures."""
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
fixtures = [("admin", "AdminRogue80", 2),
            ("testkarus", "KarusWar80", 1), ("testkarus", "KarusMage80", 3),
            ("testkarus", "KarusPriest80", 4), ("testelmorad", "ElmoWar80", 1),
            ("testelmorad", "ElmoMage80", 3), ("testelmorad", "ElmoPriest80", 4)]
dump = subprocess.check_output([str(root / "target/debug/reko.exe"), "dump-tbl",
    str(root.parent / "KnightOnline1453/Data/item_org_us.tbl"), "--limit", "20000", "--full"], encoding="utf-8")
items = {}
for line in dump.splitlines():
    match = re.match(r"\s*\[\s*\d+\]\s+(.*)", line)
    if match:
        row = next(csv.reader([match[1]], escapechar="\\", skipinitialspace=True))
        items[int(row[0])] = row
for _, _, klass in fixtures:
    assert int(items[379058000+klass*1000][23]) == 1
    assert int(items[379062000+klass*1000][23]) == 0
print("Admin characters:", [name for _, name, _ in fixtures])
print("One-time supplies: 100 class stones plus missing Master 2 scroll per character")
if not args.apply:
    print("Dry run; stop the server and pass --apply")
    raise SystemExit(0)
db = sqlite3.connect(root / "reko.db", timeout=10)
db.execute("PRAGMA foreign_keys=ON")
backup = root / "logs" / ("before-admin-class-tests-"+datetime.now().strftime("%Y%m%d-%H%M%S")+".db")
with sqlite3.connect(backup) as target:
    db.backup(target)
with db:
    db.execute("CREATE TABLE IF NOT EXISTS character_privileges(account TEXT NOT NULL COLLATE NOCASE,slot INTEGER NOT NULL,is_admin INTEGER NOT NULL DEFAULT 0 CHECK(is_admin IN (0,1)),PRIMARY KEY(account,slot),FOREIGN KEY(account,slot) REFERENCES lobby_characters(account,slot) ON DELETE CASCADE)")
    if db.execute("SELECT 1 FROM sqlite_master WHERE type='table' AND name='character_test_permissions'").fetchone():
        db.execute("INSERT INTO character_privileges SELECT account,slot,1 FROM character_test_permissions WHERE self_damage=1 ON CONFLICT(account,slot) DO NOTHING")
        db.execute("DROP TABLE character_test_permissions")
    db.execute("CREATE TABLE IF NOT EXISTS class_stone_supply_grants(account TEXT NOT NULL,slot INTEGER NOT NULL,PRIMARY KEY(account,slot))")
    for account, name, klass in fixtures:
        char = db.execute("SELECT slot,class FROM lobby_characters WHERE account=? AND name=?", (account,name)).fetchone()
        assert char and char[1] % 100 == {1:6,2:8,3:10,4:12}[klass], name
        slot = char[0]
        db.execute("INSERT INTO character_privileges VALUES (?,?,1) ON CONFLICT(account,slot) DO UPDATE SET is_admin=1", (account,slot))
        if db.execute("SELECT 1 FROM class_stone_supply_grants WHERE account=? AND slot=?", (account,slot)).fetchone():
            continue
        occupied = {r[0] for r in db.execute("SELECT inventory_slot FROM character_inventory WHERE account=? AND slot=?", (account,slot))}
        empty = [p for p in range(28) if p not in occupied]
        scroll = 379062000+klass*1000
        supplies = [(379058000+klass*1000,100)]
        if not db.execute("SELECT 1 FROM character_inventory WHERE account=? AND slot=? AND item_id=? AND count>0", (account,slot,scroll)).fetchone():
            supplies.append((scroll,1))
        assert len(empty)>=len(supplies), "Need inventory room: "+name
        for pos,(item,count) in zip(empty,supplies):
            db.execute("INSERT INTO character_inventory VALUES (?,?,?,?,1,?)", (account,slot,pos,item,count))
        db.execute("INSERT INTO class_stone_supply_grants VALUES (?,?)", (account,slot))
        print(name, "admin; supplied", [(items[item][2],count) for item,count in supplies])
assert not db.execute("PRAGMA foreign_key_check").fetchall()
print("Backup:",backup)
