"""Import NPC behavior fields only; never edits loot, shops, items or characters."""
import argparse,csv,sqlite3,subprocess,json
from pathlib import Path
from datetime import datetime
root=Path(__file__).resolve().parents[1]
p=argparse.ArgumentParser(description=__doc__)
p.add_argument('--db',type=Path,default=root/'reko.db');p.add_argument('--apply',action='store_true');a=p.parse_args()
rows=list(csv.DictReader((root.parent/'PV_1453/inspection/K_MONSTER.csv').open(encoding='utf-8-sig',newline='')))
print('AI prototypes:',len(rows))
if not a.apply:raise SystemExit('Dry run; use --apply with the daemon stopped.')
if not a.db.exists():raise SystemExit('Database must exist.')
if a.db.resolve()==(root/'reko.db').resolve():
    active=subprocess.run(['powershell.exe','-NoProfile','-Command','@(Get-Process reko-serverd -ErrorAction SilentlyContinue).Count'],capture_output=True,text=True,check=True)
    if int(active.stdout.strip()):raise SystemExit('Stop daemon before importing AI.')
d=sqlite3.connect(a.db)
backup=root/('logs/before-npc-ai-'+datetime.now().strftime('%Y%m%d-%H%M%S')+'.db')
with sqlite3.connect(backup) as b:d.backup(b)
d.executescript((root/'crates/reko-server/src/npc_ai.sql').read_text())
fields=['sSid','byActType','sDamage','sHitRate','sAttackDelay','bySpeed1','bySpeed2','byAttackRange','bySearchRange','byTracingRange','sBulk','byDirectAttack','iMagic1','iMagic2','iMagic3','byFamily','sStandtime']
with d:
    for r in rows:d.execute('INSERT OR REPLACE INTO world_monster_ai VALUES ('+','.join('?' for _ in fields)+')',tuple(int(r[f] or 0) for f in fields))
npcs=list(csv.DictReader((root.parent/'PV_1453/inspection/K_NPC.csv').open(encoding='utf-8-sig',newline='')))
with d:
    for r in npcs:
        d.execute("INSERT OR REPLACE INTO world_reference_rules VALUES ('OBJECT_NPC',?,?)",(int(r['sSid']),json.dumps(r)))
print('Imported AI and map object prototypes; backup:',backup)
