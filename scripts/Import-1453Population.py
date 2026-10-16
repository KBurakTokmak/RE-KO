"""Offline, repeatable Moradon import. Never edits characters or existing fixtures."""
import argparse
import csv
from datetime import datetime
import hashlib
import json
from pathlib import Path
import re
import sqlite3
import subprocess

root = Path(__file__).resolve().parents[1]
parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument("--apply", action="store_true")
parser.add_argument("--all-zones", action="store_true")
parser.add_argument("--db", type=Path, default=root / "reko.db")
args = parser.parse_args()
reference = root.parent / "PV_1453/inspection"
source_name = "private1453/Moradon/K_NPCPOS"


def load(name):
    with (reference / f"{name}.csv").open(encoding="utf-8-sig", newline="") as stream:
        return list(csv.DictReader(stream))


def original_table(name):
    result = subprocess.run([str(root / "target/debug/reko.exe"), "dump-tbl",
        str(root.parent / "KnightOnline1453/Data" / name), "--limit", "20000", "--full"],
        capture_output=True, check=True, encoding="utf-8")
    rows = {}
    for line in result.stdout.splitlines():
        match = re.match(r"\s*\[\s*\d+\]\s+(.*)", line)
        if match:
            row = next(csv.reader([match[1]], escapechar="\\", skipinitialspace=True))
            rows[int(row[0])] = row
    if not rows:
        raise RuntimeError("Could not read original table: " + name)
    return rows


looks = original_table("NPC_Looks.tbl")
npcs = {int(r["sSid"]): r for r in load("K_NPC")}
monsters = {int(r["sSid"]): r for r in load("K_MONSTER")}
spawns = []
conditions = []
zone_counts = {}
moradon_count = 0
other_count = 0
for source_row, row in enumerate(load("K_NPCPOS"), 1):
    zone = int(row["ZoneID"])
    if zone != 21 and not args.all_zones:
        continue
    monster = int(row["ActType"]) < 100
    proto = (monsters if monster else npcs)[int(row["NpcID"])]
    for ordinal in range(int(row["NumNPC"])):
        entity_id=12000+moradon_count if zone==21 else 14000+other_count
        if zone==21: moradon_count+=1
        else: other_count+=1
        entity = dict(id=entity_id, source=source_name if zone==21 else f'private1453/zone{zone}/K_NPCPOS', source_row=source_row,
            ordinal=ordinal, monster=int(monster), zone=zone,
            prototype=int(proto["sSid"]), appearance=int(proto["sPid"]),
            name=proto["strName"].strip(), kind=int(proto["byType"]),
            selling_group=int(proto["iSellingGroup"]), nation=int(proto["byGroup"]),
            level=int(proto["sLevel"]), size=int(proto["sSize"]),
            weapon1=int(proto["iWeapon1"]), weapon2=int(proto["iWeapon2"]),
            direction=int(row["byDirection"]), x0=int(row["LeftX"])*10,
            z0=int(row["TopZ"])*10, x1=int(row["RightX"])*10,
            z1=int(row["BottomZ"])*10, hp=int(proto["iHpPoint"]), armor=int(proto["sAc"]),
            fire_r=int(proto["sFireR"]), cold_r=int(proto["sColdR"]),
            lightning_r=int(proto["sLightningR"]), magic_r=int(proto["sMagicR"]),
            disease_r=int(proto["sDiseaseR"]), poison_r=int(proto["sPoisonR"]),
            respawn_ms=int(row["RegTime"])*1000, regen_type=int(row["RegenType"]),
            move_type=int(row["ActType"]) % 100, path=row["path"])
        assert 12000 <= entity["id"] <= 32767
        reasons=[]
        if int(proto['sPid']) not in looks: reasons.append('model absent from original1453 client')
        if any(ord(c)>255 for c in entity['name']) or len(entity['name'])>255: reasons.append('unsupported name encoding')
        if entity['hp']<=0 or entity['respawn_ms']<=0 or entity['size']<=0: reasons.append('invalid spawn vitality/timer/size')
        if not all(0<=entity[k]<=65535 for k in ('x0','x1','z0','z1')): reasons.append('coordinate outside native protocol')
        if int(row['RegenType']) not in (0,1): reasons.append('event-controlled spawn')
        if any(entity[k]<0 or entity[k]>65535 for k in ('armor','fire_r','cold_r','lightning_r','magic_r','disease_r','poison_r')): reasons.append('unsupported signed attributes')
        conditions.append((entity_id,int(not reasons),'; '.join(reasons),int(row['DungeonFamily']),int(row['SpecialType']),int(row['TrapNumber'])))
        zone_counts[zone]=zone_counts.get(zone,0)+1
        spawns.append(entity)
towns = []
for row in load("START_POSITION"):
    if int(row["ZoneID"]) == 21 or args.all_zones:
        towns.append((int(row['ZoneID']), int(row["sKarusX"]), int(row["sKarusZ"]),
            int(row["sElmoradX"]), int(row["sElmoradZ"]), int(row["bRangeX"]),
            int(row["bRangeZ"]), "private1453/START_POSITION"))
assert moradon_count == 533
report = dict(source=source_name, npc_instances=sum(not s["monster"] for s in spawns),
    monster_instances=sum(s["monster"] for s in spawns), town=towns,
    original_looks_verified=len({s["appearance"] for s in spawns}),
    source_sha256=hashlib.sha256((reference / "K_NPCPOS.csv").read_bytes()).hexdigest())
report['zone_counts']=zone_counts
report['enabled_instances']=sum(c[1] for c in conditions)
report['inactive_reasons']={reason:sum(c[2]==reason for c in conditions) for reason in {c[2] for c in conditions if c[2]}}
print(json.dumps(report, indent=2))
if not args.apply:
    print("Dry run. Use --apply with the RE-KO daemon stopped.")
    raise SystemExit()

if args.db.resolve() == (root / "reko.db").resolve():
    active = subprocess.run(["powershell.exe", "-NoProfile", "-Command",
        "@(Get-Process reko-serverd -ErrorAction SilentlyContinue).Count"],
        capture_output=True, check=True, text=True)
    if int(active.stdout.strip()) != 0:
        raise SystemExit("Stop RE-KO before importing world definitions.")
if not args.db.exists():
    raise SystemExit("Save database does not exist; refusing to create an empty game save.")
stamp = datetime.now().strftime("%Y%m%d-%H%M%S")
db = sqlite3.connect(args.db)
backup_path = root / f"logs/before-moradon-population-{stamp}.db"
with sqlite3.connect(backup_path) as backup:
    db.backup(backup)
db.executescript((root / "crates/reko-server/src/population.sql").read_text())
columns = list(spawns[0])
with db:
    for entity in spawns:
        collision = db.execute("SELECT source FROM world_spawns WHERE id=?", (entity["id"],)).fetchone()
        if collision and collision[0] != entity['source']:
            raise RuntimeError("Entity ID belongs to a different source")
        fixture = db.execute("SELECT 1 FROM world_merchants WHERE id=?", (entity["id"],)).fetchone()
        if fixture:
            raise RuntimeError("Entity ID conflicts with an existing merchant")
        db.execute(f"INSERT INTO world_spawns ({','.join(columns)}) VALUES ({','.join('?' for _ in columns)}) "
            "ON CONFLICT(id) DO UPDATE SET " + ','.join(f"{c}=excluded.{c}" for c in columns if c != "id"),
            tuple(entity[c] for c in columns))
    db.executemany('INSERT OR REPLACE INTO world_spawn_conditions VALUES (?,?,?,?,?,?)',conditions)
    db.executemany("INSERT INTO world_town_starts VALUES (?,?,?,?,?,?,?,?) ON CONFLICT(zone) DO UPDATE SET "
        "karus_x=excluded.karus_x,karus_z=excluded.karus_z,elmo_x=excluded.elmo_x,elmo_z=excluded.elmo_z,"
        "range_x=excluded.range_x,range_z=excluded.range_z,source=excluded.source", towns)
db.close()
report["backup"] = str(backup_path)
(root / f"logs/moradon-population-{stamp}.json").write_text(json.dumps(report, indent=2))
print(f"Imported{len(spawns)} spawns and {len(towns)} town rows; backup:", backup_path)
