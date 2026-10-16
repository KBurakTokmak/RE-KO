"""Import source-backed loot/service rules into an offline SQLite save."""
import argparse
import csv
from datetime import datetime
import json
from pathlib import Path
import sqlite3
import subprocess

root=Path(__file__).resolve().parents[1]
p=argparse.ArgumentParser(description=__doc__)
p.add_argument('--apply',action='store_true')
p.add_argument('--db',type=Path,default=root/'reko.db')
a=p.parse_args()
reference=root.parent/'PV_1453/inspection'
def rows(name):
    with (reference/(name+'.csv')).open(encoding='utf-8-sig',newline='') as f:
        return list(csv.DictReader(f))
monsters=rows('K_MONSTER');drops=rows('K_MONSTER_ITEM');groups=rows('MAKE_ITEM_GROUP')
print(f'{len(monsters)} monster loot prototypes, {len(drops)} drop rows, {len(groups)} item groups')
if not a.apply: raise SystemExit('Dry run; use --apply with the daemon stopped.')
if not a.db.exists(): raise SystemExit('Refusing to create an empty save database.')
if a.db.resolve() == (root/'reko.db').resolve():
    active=subprocess.run(['powershell.exe','-NoProfile','-Command',
        '@(Get-Process reko-serverd -ErrorAction SilentlyContinue).Count'],capture_output=True,check=True,text=True)
    if int(active.stdout.strip()): raise SystemExit('Stop RE-KO before importing world rules.')
db=sqlite3.connect(a.db)
backup=root/('logs/before-world-services-'+datetime.now().strftime('%Y%m%d-%H%M%S')+'.db')
with sqlite3.connect(backup) as b: db.backup(b)
db.executescript((root/'crates/reko-server/src/world_services.sql').read_text())
with db:
    for r in monsters:
        db.execute('INSERT OR REPLACE INTO world_monster_loot VALUES (?,?,?)',(int(r['sSid']),int(r['iMoney']),int(r['sItem'])))
    seen=set()
    for r in drops:
        if r['sIndex'] in seen:
            print('Duplicate drop index skipped, matching source PutData first-entry behavior:',r['sIndex'])
            continue
        seen.add(r['sIndex'])
        for i in range(1,6):
            db.execute('INSERT OR REPLACE INTO world_drop_rules VALUES (?,?,?,?)',(int(r['sIndex']),i-1,int(r[f'iItem0{i}'] or 0),int(r[f'sPersent0{i}'] or 0)))
    for r in groups:
        for i in range(1,31):
            db.execute('INSERT OR REPLACE INTO world_drop_groups VALUES (?,?,?)',(int(r['iItemGroupNum']),i-1,int(r[f'iItem_{i}'])))
    for table,key in [('ITEM_UPGRADE','nIndex'),('ITEM_EXCHANGE','nIndex'),('QUEST_HELPER','nIndex'),('EVENT_TRIGGER','nIndex'),('MAKE_ITEM_GRADECODE','byItemIndex'),('MAKE_WEAPON','byLevel'),('MAKE_DEFENSIVE','byLevel'),('MAKE_ITEM_LARECODE','byLevelGrade'),('K_OBJECTPOS','sIndex')]:
        for r in rows(table):
            rule_id=int(r[key])+(int(r['ZoneID'])*100000 if table=='K_OBJECTPOS' else 0)
            db.execute('INSERT OR REPLACE INTO world_reference_rules VALUES (?,?,?)',(table,rule_id,json.dumps(r)))
    for r in rows('ITEM'):
        db.execute('INSERT OR REPLACE INTO world_upgrade_item_types VALUES (?,?,?)',(int(r['Num']),int(r['ItemClass']),int(r['ItemExt'])))
print('Imported rules; backup:',backup)
