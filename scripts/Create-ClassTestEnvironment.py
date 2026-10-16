"""Offline, idempotent class fixtures and reference-backed test merchants."""
import argparse
import csv
from datetime import datetime
from pathlib import Path
import re
import sqlite3
import subprocess

root = Path(__file__).resolve().parents[1]
game = root.parent / "KnightOnline1453"
parser = argparse.ArgumentParser()
parser.add_argument("--apply", action="store_true")
args = parser.parse_args()

def table(name):
    result = subprocess.run([str(root / "target/debug/reko.exe"), "dump-tbl",
                             str(game / "Data" / name), "--limit", "20000", "--full"],
                            capture_output=True, check=True, encoding="utf-8")
    rows = {}
    for line in result.stdout.splitlines():
        match = re.match(r"\s*\[\s*\d+\]\s+(.*)", line)
        if match:
            row = next(csv.reader([match[1]], escapechar="\\", skipinitialspace=True))
            rows[int(row[0])] = row
    assert rows, "No table rows: " + name
    return rows

creation = table("NewChrValue.tbl")
items = table("item_org_us.tbl")
looks = table("NPC_Looks.tbl")
skills = table("Skill_Magic_Main_us.tbl")
with (root / "logs/reference-rules/K_NPC.csv").open(encoding="utf-8-sig", newline="") as stream:
    prototypes = {int(row["sSid"]): row for row in csv.DictReader(stream)}
with (root / "logs/reference-rules/COEFFICIENT.csv").open(encoding="utf-8-sig", newline="") as stream:
    coefficients = list(csv.DictReader(stream))
assert coefficients, "Reference coefficients not exported"
with (root / "logs/reference-rules/BEGINNER_ITEM.csv").open(encoding="utf-8-sig", newline="") as stream:
    assert all(int(row["Gold"]) == 1000000 for row in csv.DictReader(stream))

fixtures = [("testkarus", 1, [("KarusWar80", 1, 106, 120150001),
                              ("KarusMage80", 3, 110, 180110001),
                              ("KarusPriest80", 4, 112, 190250001)]),
            ("testelmorad", 2, [("ElmoWar80", 11, 206, 120150001),
                                ("ElmoMage80", 12, 210, 180110001),
                                ("ElmoPriest80", 13, 212, 190250001)])]

for account, nation, characters in fixtures:
    for name, race, klass, weapon in characters:
        initial_class = (klass // 100) * 100 + {6: 1, 10: 3, 12: 4}[klass % 100]
        row = creation[race * 10000 + initial_class]
        base = [int(v) for v in row[2:7]]
        assert sum(base) == 290 and int(row[7]) == 10
        item = items[weapon // 1000 * 1000]
        group = int(item[1])
        assert weapon % 1000 in table(f"Item_Ext_{group}_us.tbl")
        assert race < 10 if nation == 1 else race >= 10
        print(account, name, "class", klass, "level80; base", base,
              "287 free stats; branches80/80/80/master20; coins1000000")
for sid in (505, 506):
    row = prototypes[sid]
    assert int(row["sPid"]) in looks
    assert int(row["iSellingGroup"]) in (253000, 255000)
    print("Test merchant:", row["strName"], "appearance", row["sPid"], "group", row["iSellingGroup"])

db = sqlite3.connect(root / "reko.db", timeout=10)
db.execute("PRAGMA foreign_keys=ON")
for account, nation, characters in fixtures:
    existing = db.execute("SELECT nation FROM lobby_accounts WHERE name=?", (account,)).fetchone()
    assert existing is None or existing[0] == nation, "Account nation conflict"
    for slot, (name, race, klass, weapon) in enumerate(characters):
        existing = db.execute("SELECT name,race,class FROM lobby_characters WHERE account=? AND slot=?", (account, slot)).fetchone()
        assert existing is None or existing == (name, race, klass), "Occupied fixture slot"
        owner = db.execute("SELECT account,slot FROM lobby_characters WHERE name=?", (name,)).fetchone()
        assert owner is None or owner == (account, slot), "Character name conflict"
if not args.apply:
    print("Dry run verified original tables and reference prototypes. Use --apply while server stopped.")
    raise SystemExit(0)
listeners = subprocess.run(["powershell", "-NoProfile", "-Command",
                           "@(Get-NetTCPConnection -LocalPort 15001 -State Listen -ErrorAction SilentlyContinue).Count"],
                          capture_output=True, text=True, check=True)
assert listeners.stdout.strip() == "0", "Stop the server before applying fixtures"
backup = root / "logs" / ("before-class-environment-" + datetime.now().strftime("%Y%m%d-%H%M%S") + ".db")
with sqlite3.connect(backup) as target:
    db.backup(target)
db.executescript("""
CREATE TABLE IF NOT EXISTS character_stat_points (
 account TEXT NOT NULL COLLATE NOCASE,slot INTEGER NOT NULL,remaining INTEGER NOT NULL CHECK(remaining BETWEEN 0 AND 65535),
 PRIMARY KEY(account,slot),FOREIGN KEY(account,slot) REFERENCES lobby_characters(account,slot) ON DELETE CASCADE);
CREATE TABLE IF NOT EXISTS character_privileges (
 account TEXT NOT NULL COLLATE NOCASE,slot INTEGER NOT NULL,is_admin INTEGER NOT NULL DEFAULT 0 CHECK(is_admin IN (0,1)),
 PRIMARY KEY(account,slot),FOREIGN KEY(account,slot) REFERENCES lobby_characters(account,slot) ON DELETE CASCADE);
CREATE TABLE IF NOT EXISTS world_merchants (
 id INTEGER PRIMARY KEY CHECK(id BETWEEN 11000 AND 32767),prototype INTEGER NOT NULL,appearance INTEGER NOT NULL,name TEXT NOT NULL,
 selling_group INTEGER NOT NULL,nation INTEGER NOT NULL,level INTEGER NOT NULL,kind INTEGER NOT NULL,
 zone INTEGER NOT NULL,x INTEGER NOT NULL,z INTEGER NOT NULL,y INTEGER NOT NULL DEFAULT 0);
CREATE TABLE IF NOT EXISTS recovery_test_grants (
 account TEXT NOT NULL COLLATE NOCASE,slot INTEGER NOT NULL,version INTEGER NOT NULL,PRIMARY KEY(account,slot),
 FOREIGN KEY(account,slot) REFERENCES lobby_characters(account,slot) ON DELETE CASCADE);
""")

with db:
    for account, nation, characters in fixtures:
        db.execute("INSERT OR IGNORE INTO lobby_accounts(name,nation) VALUES (?,?)", (account, nation))
        for slot, (name, race, klass, weapon) in enumerate(characters):
            if db.execute("SELECT 1 FROM lobby_characters WHERE account=? AND slot=?", (account, slot)).fetchone():
                print("Preserved existing fixture:", name)
                continue
            initial_class = (klass // 100) * 100 + {6: 1, 10: 3, 12: 4}[klass % 100]
            base = bytes(int(v) for v in creation[race * 10000 + initial_class][2:7])
            db.execute("INSERT INTO lobby_characters VALUES (?,?,?,?,?,0,0,?)", (account, slot, name, race, klass, base))
            db.execute("INSERT INTO character_progress VALUES (?,?,80,?)", (account, slot, bytes([0,0,0,0,0,80,80,80,20])))
            db.execute("INSERT INTO character_stat_points VALUES (?,?,287)", (account, slot))
            db.execute("INSERT INTO character_wallets VALUES (?,?,1000000)", (account, slot))
            db.execute("INSERT INTO character_positions VALUES (?,?,21,3200,4000,0)", (account, slot))
            gear = [(6, weapon)]
            if klass % 100 == 12:
                gear.append((8, 170150001))
            for position, item in gear:
                durability = int(items[item // 1000 * 1000][19])
                db.execute("INSERT INTO character_equipment VALUES (?,?,?,?,?)", (account, slot, position, item, durability))
            db.execute("INSERT INTO starter_grants VALUES (?,?,1)", (account, slot))
            quests = {int(row[29]) for row in skills.values() if len(row) == 31 and int(row[16]) // 10 == klass and int(row[29]) > 0}
            for quest in quests:
                db.execute("INSERT INTO character_quests VALUES (?,?,?,2)", (account, slot, quest))
            print("Created", name, "completed skill quest IDs", sorted(quests))
    for account, _, characters in fixtures:
        for name, _, _, _ in characters:
            row=db.execute("SELECT slot FROM lobby_characters WHERE account=? AND name=?", (account,name)).fetchone()
            assert row, "Fixture missing: " + name
            db.execute("INSERT INTO character_privileges VALUES (?,?,1) ON CONFLICT(account,slot) DO UPDATE SET is_admin=1", (account,row[0]))
    for account, _, characters in fixtures:
        for slot, (_, _, klass, _) in enumerate(characters):
            quests = {int(row[29]) for row in skills.values() if len(row) == 31 and int(row[16]) // 10 == klass and int(row[29]) > 0}
            for quest in quests:
                db.execute("INSERT INTO character_quests VALUES (?,?,?,2) ON CONFLICT(account,slot,quest) DO UPDATE SET state=2", (account, slot, quest))
    for offset, sid in enumerate((506, 505)):
        p = prototypes[sid]
        # Training placement, not the historical Moradon spawn coordinates.
        db.execute("INSERT OR IGNORE INTO world_merchants VALUES (?,?,?,?,?,?,?,?,21,?,?,0)",
                   (11000+offset,sid,int(p["sPid"]),p["strName"],int(p["iSellingGroup"]),
                    int(p["byGroup"]),int(p["sLevel"]),int(p["byType"]),3200+offset*50,3950))
    accounts = [account for account, _, _ in fixtures] + ["admin"]
    for account in accounts:
        for slot, name in db.execute("SELECT slot,name FROM lobby_characters WHERE account=?", (account,)).fetchall():
            if account == "admin" and name != "AdminRogue80":
                continue
            if db.execute("SELECT 1 FROM recovery_test_grants WHERE account=? AND slot=?", (account, slot)).fetchone():
                continue
            free = [pos for pos in range(28) if not db.execute("SELECT 1 FROM character_inventory WHERE account=? AND slot=? AND inventory_slot=?", (account, slot, pos)).fetchone()]
            supplies = [(389014000,10),(389020000,10)]
            if name == "AdminRogue80":
                supplies += [(370005000,5),(370006000,5)]
            assert len(free) >= len(supplies), "Insufficient empty bag slots"
            for pos, (item, count) in zip(free, supplies):
                assert item in items and 0 in table(f"Item_Ext_{int(items[item][1])}_us.tbl")
                db.execute("INSERT INTO character_inventory(account,slot,inventory_slot,item_id,durability,count) VALUES (?,?,?,?,?,?)",
                           (account,slot,pos,item,int(items[item][19]),count))
            db.execute("INSERT INTO recovery_test_grants VALUES (?,?,1)", (account,slot))
assert db.execute("PRAGMA integrity_check").fetchone()[0] == "ok"
assert not db.execute("PRAGMA foreign_key_check").fetchall()
print("Environment created; existing character builds and balances preserved. Backup:", backup)
