"""Create the explicitly requested admin test fixture without replacing characters."""
from datetime import datetime
from pathlib import Path
import sqlite3

root = Path(__file__).resolve().parents[1]
db = sqlite3.connect(root / "reko.db", timeout=10)
db.execute("PRAGMA foreign_keys=ON")
account = db.execute("SELECT name,nation FROM lobby_accounts WHERE name='admin'").fetchone()
assert account is not None, "admin account does not exist"
existing = db.execute("SELECT slot,class FROM lobby_characters WHERE account='admin' AND name='AdminRogue80'").fetchone()
if existing:
    print("AdminRogue80 already exists; equipment and spent points were preserved:", existing)
    raise SystemExit(0)
occupied = {row[0] for row in db.execute("SELECT slot FROM lobby_characters WHERE account='admin'")}
slot = next((slot for slot in range(3) if slot not in occupied), None)
assert slot is not None, "No free character slot; no changes performed"
backup = root / "logs" / ("before-rogue80-" + datetime.now().strftime("%Y%m%d-%H%M%S") + ".db")
with sqlite3.connect(backup) as target:
    db.backup(target)
race, character_class = (2,108) if account[1] == 1 else (12,208)
with db:
    db.execute("INSERT INTO lobby_characters(account,slot,name,race,class,face,hair,stats) VALUES (?,?,?,?,?,0,0,?)",
               (account[0],slot,"AdminRogue80",race,character_class,bytes([60,150,255,50,50])))
    db.execute("INSERT INTO character_progress(account,slot,level,points) VALUES (?,?,80,?)",
               (account[0],slot,bytes([160,0,0,0,0,0,0,0,0])))
    for equipment_slot in (6,8):
        db.execute("INSERT INTO character_equipment(account,slot,equipment_slot,item_id,durability) VALUES (?,?,?,110110001,4000)",
                   (account[0],slot,equipment_slot))
    db.execute("INSERT INTO character_wallets(account,slot,coins) VALUES (?,?,20000)", (account[0],slot))
    db.execute("INSERT INTO starter_grants(account,slot,version) VALUES (?,?,1)", (account[0],slot))
    db.execute("INSERT INTO character_positions(account,slot,zone,x,z,y) VALUES (?,?,21,3200,4000,0)", (account[0],slot))
assert not db.execute("PRAGMA foreign_key_check").fetchall()
print("Created AdminRogue80", "slot", slot, "class", character_class, "level 80; 160 free points; STR/STA/DEX/INT/CHA 60/150/255/50/50")
print("Equipped two +1 daggers; positioned near test targets at Moradon 320,400")
print("Backup:", backup)
