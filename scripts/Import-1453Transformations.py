"""Offline import of the46 original-scroll MAGIC_TYPE6 rules from supplied1453."""
import argparse
from datetime import datetime
import importlib.util
import json
from pathlib import Path
import sqlite3
import subprocess

ROOT = Path(__file__).resolve().parents[1]
spec = importlib.util.spec_from_file_location('repair', ROOT / 'scripts/Repair-1453MageTravel.py')
repair = importlib.util.module_from_spec(spec)
spec.loader.exec_module(repair)


def plan(db, reference, game):
    rows = json.loads(reference.read_text(encoding='utf-8-sig'))
    original = repair.original_tables.table(game, 'Skill_Magic_6.tbl')
    expected = {key for key in original if key // 1000 == 472}
    if len(rows) != 46 or {r['iNum'] for r in rows} != expected:
        raise ValueError('The46 original scroll transformations are required.')
    for row in rows:
        client = original[row['iNum']]
        if int(client[4]) != row['TransformID'] or int(client[5]) != row['Duration'] or int(client[21]) != row['Class']:
            raise ValueError('Primary transformation does not match original model/duration/class.')
        if row['UserSkillUse'] != 1 or row['Nation'] != 0 or row['MonsterFriendly'] != 1:
            raise ValueError('Unsupported transformation behavior.')
        if any(row[k] for k in ('MaxHp','MaxMp','Speed','AttackSpeed','TotalHit','TotalAc','TotalHitRate','TotalEvasionRate','TotalFireR','TotalColdR','TotalLightningR','TotalMagicR','TotalDiseaseR','TotalPoisonR')):
            raise ValueError('Transformation combat bonuses need a separate implementation.')
        old = db.execute("SELECT data FROM world_reference_rules WHERE table_name='MAGIC_TYPE6' AND rule_id=?", (row['iNum'],)).fetchone()
        if old and json.loads(old[0]) != row:
            raise ValueError('Existing transformation customization differs; refusing overwrite.')
    return rows


def apply(db, rows):
    for row in rows:
        db.execute("INSERT OR IGNORE INTO world_reference_rules VALUES('MAGIC_TYPE6',?,?)", (row['iNum'], json.dumps(row,sort_keys=True)))


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--db', type=Path, default=ROOT / 'reko.db')
    parser.add_argument('--reference', type=Path, default=ROOT / 'logs/reference1453/MAGIC_TYPE6.json')
    parser.add_argument('--game-dir', type=Path, default=ROOT.parent / 'KnightOnline1453')
    parser.add_argument('--apply', action='store_true')
    args = parser.parse_args()
    if args.apply and args.db.resolve() == (ROOT / 'reko.db').resolve():
        count = subprocess.check_output(['powershell','-NoProfile','-Command','@(Get-Process reko-serverd -ErrorAction SilentlyContinue).Count'],text=True)
        if int(count.strip()):
            raise SystemExit('Stop RE-KO before importing transformation rules.')
    with sqlite3.connect(f'file:{args.db.resolve().as_posix()}?mode={"rw" if args.apply else "ro"}',uri=True) as db:
        rows = plan(db,args.reference,args.game_dir)
        check = sqlite3.connect(':memory:')
        db.backup(check)
        with check:
            apply(check,rows)
        once = repair.snapshot(check)
        with check:
            apply(check,rows)
        assert once == repair.snapshot(check), 'Idempotence failed.'
        if not args.apply:
            print('Verified46 supplied1453 rules against original models, one-hour duration and class masks; dry run/idempotence passed.')
            return
        stamp = datetime.now().strftime('%Y%m%d-%H%M%S')
        backup = ROOT / 'logs' / f'before-transformation-{stamp}.db'
        with sqlite3.connect(backup) as target:
            db.backup(target)
        before = repair.snapshot(db)
        with db:
            apply(db,rows)
            after = repair.snapshot(db)
            changed = [name for name in before if before[name] != after[name]]
            if set(changed) - {'world_reference_rules'}:
                raise ValueError('Unrelated data changed; rolling back.')
            assert db.execute('PRAGMA integrity_check').fetchone()[0] == 'ok'
        receipt = dict(backup=str(backup),before=before,after=after,changed_tables=changed)
        (ROOT / 'logs' / f'transformation-import-{stamp}.json').write_text(json.dumps(receipt,indent=2))
        print('Imported46 transformation rules; all unrelated tables preserved. Backup:',backup)


if __name__ == '__main__':
    main()
