"""Group original rare-boss spawn locations; owner policy: one per zone, six hours after death."""
import argparse
from collections import defaultdict
from datetime import datetime
from pathlib import Path
import sqlite3
import subprocess

root = Path(__file__).resolve().parents[1]
p = argparse.ArgumentParser(description=__doc__)
p.add_argument('--db', type=Path, default=root / 'reko.db')
p.add_argument('--apply', action='store_true')
a = p.parse_args()
if not a.db.exists():
    raise SystemExit('Existing imported save required.')
# Named owner examples plus the other rare bosses with long timers in these
# original zone imports. Quest and event monsters are intentionally separate.
names = ('talos', 'harpy queen', 'deruvish founder', 'snake queen', 'attila',
         'kekurikekukaka', 'hyde', 'antares', 'shaula', 'samma', 'troll king', 'dragon tooth')
db = sqlite3.connect(a.db)
rows = db.execute('SELECT id,zone,name,source,source_row,ordinal FROM world_spawns '
    'WHERE monster=1 AND zone IN (1,2,11,12,21) AND lower(trim(name)) IN (' +
    ','.join('?' for _ in names) + ') ORDER BY zone,name,id', names).fetchall()
pools = defaultdict(list)
for row in rows:
    pools[(row[1], row[2].strip().lower())].append(row)
for (zone, key), members in pools.items():
    print(f'{zone}: {members[0][2]}: {len(members)} source instances -> one active boss')
print('Pools:', len(pools), 'candidate entities:', len(rows), 'respawn: 21600 seconds after death')
if not a.apply:
    raise SystemExit('Dry run; use --apply with the daemon stopped.')
if a.db.resolve() == (root / 'reko.db').resolve():
    active = subprocess.run(['powershell.exe', '-NoProfile', '-Command',
        '@(Get-Process reko-serverd -ErrorAction SilentlyContinue).Count'],
        capture_output=True, text=True, check=True)
    if int(active.stdout.strip()):
        raise SystemExit('Stop RE-KO before importing boss pools.')
backup = root / 'logs' / ('before-boss-pools-' + datetime.now().strftime('%Y%m%d-%H%M%S') + '.db')
with sqlite3.connect(backup) as out:
    db.backup(out)
db.executescript((root / 'crates/reko-server/src/bosses.sql').read_text())
with db:
    for (zone, key), members in pools.items():
        pool = f'{zone}:{key}'
        db.execute('INSERT INTO world_boss_pools VALUES (?,?,?,21600000) '
            'ON CONFLICT(pool_id) DO UPDATE SET zone=excluded.zone,name=excluded.name,respawn_ms=excluded.respawn_ms',
            (pool, zone, members[0][2]))
        db.execute('DELETE FROM world_boss_members WHERE pool_id=?', (pool,))
        db.executemany('INSERT INTO world_boss_members VALUES (?,?)', [(row[0], pool) for row in members])
print('Imported boss grouping only; existing spawns, timers, rewards and characters unchanged. Backup:', backup)
