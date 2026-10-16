"""Import original1453 monster rewards and LEVEL_UP without modifying characters."""
import argparse
import csv
from datetime import datetime
import json
from pathlib import Path
import sqlite3
import subprocess

ROOT = Path(__file__).resolve().parents[1]


def collect():
    records = []
    for source, target, key in [('LEVEL_UP', 'LEVEL_UP', 'level'), ('K_MONSTER', 'MONSTER_REWARD', 'sSid')]:
        with (ROOT.parent / 'PV_1453/inspection' / (source + '.csv')).open(encoding='utf-8-sig', newline='') as stream:
            rows = list(csv.DictReader(stream))
        for row in rows:
            if source == 'LEVEL_UP':
                assert 1 <= int(row['level']) <= 80
                assert 0 < int(row['Exp']) <= 0xffffffff
            else:
                assert int(row['iExp']) >= 0 and int(row['iLoyalty']) >= 0
            records.append((target, int(row[key]), json.dumps(row)))
        print(target, len(rows))
    assert sum(t == 'LEVEL_UP' for t, _, _ in records) == 80
    assert len({(t, k) for t, k, _ in records}) == len(records)
    return records


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--apply', action='store_true')
    args = parser.parse_args()
    records = collect()
    if not args.apply:
        return
    running = subprocess.check_output(['powershell', '-NoProfile', '-Command', "Get-Process | Where-Object ProcessName -eq 'reko-serverd' | Select-Object -ExpandProperty Id"], text=True)
    if running.strip():
        raise SystemExit('Stop RE-KO before importing rewards.')
    db = sqlite3.connect(ROOT / 'reko.db')
    backup = ROOT / 'logs' / ('before-experience-' + datetime.now().strftime('%Y%m%d-%H%M%S') + '.db')
    with sqlite3.connect(backup) as target:
        db.backup(target)
    tables = [r[0] for r in db.execute("SELECT name FROM sqlite_master WHERE type='table' AND name NOT LIKE 'sqlite_%' AND name!='world_reference_rules'")]
    before = {t: db.execute('SELECT * FROM "' + t + '" ORDER BY rowid').fetchall() for t in tables}
    rules = db.execute("SELECT * FROM world_reference_rules WHERE table_name NOT IN ('LEVEL_UP','MONSTER_REWARD') ORDER BY table_name,rule_id").fetchall()
    with db:
        db.executemany('INSERT OR REPLACE INTO world_reference_rules VALUES(?,?,?)', records)
        assert all(before[t] == db.execute('SELECT * FROM "' + t + '" ORDER BY rowid').fetchall() for t in tables)
        assert rules == db.execute("SELECT * FROM world_reference_rules WHERE table_name NOT IN ('LEVEL_UP','MONSTER_REWARD') ORDER BY table_name,rule_id").fetchall()
    assert db.execute('PRAGMA integrity_check').fetchone()[0] == 'ok'
    print('Backup:', backup, 'All character tables and prior unrelated rules unchanged; integrity OK')


if __name__ == '__main__':
    main()
