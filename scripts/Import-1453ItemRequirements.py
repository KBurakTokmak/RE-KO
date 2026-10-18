"""Import server ITEM level bounds; race/class/stats continue to use original client tables."""
import argparse
import csv
from datetime import datetime
import json
from pathlib import Path
import sqlite3
import subprocess

root = Path(__file__).resolve().parents[1]
p = argparse.ArgumentParser(description=__doc__)
p.add_argument('--reference', type=Path, required=True, help='ITEM CSV with Num, ReqLevel, ReqLevelMax')
p.add_argument('--db', type=Path, default=root / 'reko.db')
p.add_argument('--apply', action='store_true')
a = p.parse_args()
with a.reference.open(encoding='utf-8-sig', newline='') as stream:
    rows = [(int(r['Num']), json.dumps([int(r['ReqLevel']),int(r['ReqLevelMax'])])) for r in csv.DictReader(stream)]
if any(not (0 <= lo <= hi <= 255) for _, data in rows for lo, hi in [json.loads(data)]):
    raise SystemExit('Invalid server level bounds; no rules imported.')
unique = {}
for item, data in rows:
    if item in unique and unique[item] != data:
        raise SystemExit(f'Conflicting source level rules for item {item}; no rules imported.')
    unique[item] = data
rows = list(unique.items())
print('Unique server item level rules:', len(rows))
if not a.apply:
    raise SystemExit('Dry run; use --apply with daemon stopped.')
if not a.db.exists():
    raise SystemExit('Existing imported save required.')
if a.db.resolve() == (root / 'reko.db').resolve():
    active = subprocess.run(['powershell.exe','-NoProfile','-Command',
        '@(Get-Process reko-serverd -ErrorAction SilentlyContinue).Count'],capture_output=True,text=True,check=True)
    if int(active.stdout.strip()):
        raise SystemExit('Stop RE-KO before importing requirements.')
with sqlite3.connect(a.db) as db:
    backup = root / 'logs' / ('before-item-requirements-' + datetime.now().strftime('%Y%m%d-%H%M%S') + '.db')
    with sqlite3.connect(backup) as out:
        db.backup(out)
    with db:
        db.executemany("INSERT INTO world_reference_rules VALUES ('ITEM_LEVEL',?,?) "
            'ON CONFLICT(table_name,rule_id) DO UPDATE SET data=excluded.data', rows)
print('Imported only equipment level rules. Backup:', backup)
