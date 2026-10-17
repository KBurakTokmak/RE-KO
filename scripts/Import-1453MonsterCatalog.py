"""Import complete K_MONSTER templates for admin summoning; leave normal spawns intact."""
import argparse
import csv
import json
import sqlite3
import subprocess
from datetime import datetime
from pathlib import Path

root = Path(__file__).resolve().parents[1]
parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument('--reference', type=Path, required=True, help='Supplied1453 K_MONSTER.csv export')
parser.add_argument('--db', type=Path, default=root / 'reko.db')
parser.add_argument('--apply', action='store_true')
args = parser.parse_args()
rows = []
with args.reference.open(encoding='utf-8-sig', newline='') as stream:
    for row in csv.DictReader(stream):
        def number(key):
            return int(row[key] or 0)
        template = {key: number(source) for key, source in {
            'prototype': 'sSid', 'appearance': 'sPid', 'kind': 'byType',
            'group': 'iSellingGroup', 'nation': 'byGroup', 'level': 'sLevel',
            'size': 'sSize', 'hp': 'iHpPoint', 'armor': 'sAc',
        }.items()}
        template['name'] = row['strName'].strip()
        template['weapons'] = [number('iWeapon1'), number('iWeapon2')]
        template['resistance'] = [number(k) for k in
            ('sFireR', 'sColdR', 'sLightningR', 'sMagicR', 'sDiseaseR', 'sPoisonR')]
        rows.append((template['prototype'], json.dumps(template)))
if len({key for key, _ in rows}) != len(rows):
    raise SystemExit('Duplicate monster IDs in source.')
print('Monster templates:', len(rows))
if not args.apply:
    raise SystemExit('Dry run. Use --apply with the daemon stopped.')
if not args.db.exists():
    raise SystemExit('Save database must already exist.')
if args.db.resolve() == (root / 'reko.db').resolve():
    active = subprocess.run(['powershell.exe', '-NoProfile', '-Command',
        '@(Get-Process reko-serverd -ErrorAction SilentlyContinue).Count'],
        capture_output=True, text=True, check=True)
    if int(active.stdout.strip()):
        raise SystemExit('Stop RE-KO before importing monster templates.')
with sqlite3.connect(args.db) as db:
    backup = root / 'logs' / ('before-monster-catalog-' + datetime.now().strftime('%Y%m%d-%H%M%S') + '.db')
    with sqlite3.connect(backup) as out:
        db.backup(out)
    with db:
        db.executemany("INSERT INTO world_reference_rules VALUES ('MONSTER_TEMPLATE',?,?) "
            'ON CONFLICT(table_name,rule_id) DO UPDATE SET data=excluded.data', rows)
print('Imported templates. Backup:', backup)
