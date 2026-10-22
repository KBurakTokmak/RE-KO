"""Offline, bounded import of Mage transport and compatible Delos/CZ/Ardream routes."""
import argparse
from datetime import datetime
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import sqlite3
import subprocess
import struct

ROOT = Path(__file__).resolve().parents[1]
spec = importlib.util.spec_from_file_location('historical_travel', ROOT / 'scripts/Import-1453TravelQuests.py')
travel = importlib.util.module_from_spec(spec)
spec.loader.exec_module(travel)
table_spec = importlib.util.spec_from_file_location('original_tables', ROOT / 'scripts/Repair-1453SuppressionLoot.py')
original_tables = importlib.util.module_from_spec(table_spec)
table_spec.loader.exec_module(original_tables)


def snapshot(db):
    result = {}
    for (name,) in db.execute("SELECT name FROM sqlite_master WHERE type='table' AND name NOT LIKE 'sqlite_%' ORDER BY name"):
        safe = name.replace('"', '""')
        rows = sorted(repr(tuple(row)) for row in db.execute(f'SELECT * FROM "{safe}"'))
        result[name] = hashlib.sha256('\n'.join(rows).encode()).hexdigest()
    return result


def plan(db, reference, historical, supplementary, game):
    bounds = {}
    for key, row in original_tables.table(game, 'Zones.tbl').items():
        if key == 0 or key % 10 or key // 10 > 255:
            continue
        relative = Path(row[1].replace('\\', '/').strip())
        if relative.is_absolute() or '..' in relative.parts:
            raise ValueError('Unsafe terrain asset path.')
        if relative.suffix.lower() != '.gtd':
            continue
        if not (game / relative).is_file():
            print(f'Map bounds unavailable for zone{key // 10}: terrain asset is missing.')
            continue
        with (game / relative).open('rb') as stream:
            header = stream.read(128)
        length = struct.unpack_from('<I', header)[0]
        if length > 30:
            raise ValueError('Unsupported original terrain header.')
        version, size = struct.unpack_from('<II', header, 4 + length)
        if version != 0 or not 1 < size <= 1025 or (size - 1) % 4:
            print(f'Map bounds unavailable for zone{key // 10}: unsupported terrain header.')
            continue
        bounds[key // 10] = dict(extent=(size - 1) * 40, source=str(relative).replace('\\', '/'))
    if not {21,30,71,72}.issubset(bounds):
        raise ValueError('Required original1453 map bounds are unavailable.')
    source = json.loads((reference / 'MAGIC_TYPE8.json').read_text(encoding='utf-8-sig'))
    magic = {int(r['iNum']): r for r in source if int(r['iNum']) // 1000 in (109, 110, 209, 210)
             and int(r['WarpType']) in (1, 12, 20)}
    expected = {prefix * 1000 + suffix for prefix in (109,110,209,210) for suffix in (4,15,35)} | {110774,210774}
    if magic.keys() != expected:
        raise ValueError('Expected14 supplied1453 Mage transport records.')
    warps = {key: json.loads(data) for key, data in db.execute(
        "SELECT rule_id,data FROM world_reference_rules WHERE table_name='HISTORICAL_WARP'")}
    changed = {}

    def offer(zone, row):
        key = zone * 100000 + row['id']
        existing = warps.get(key)
        if existing and existing != row:
            raise ValueError(f'Refusing to overwrite an occupied warp: {key}.')
        warps[key] = row
        changed[key] = row

    # 1298's201 identifies the same Colony Zone layout now named71 by1453.
    for key, original in list(warps.items()):
        if original['zone'] == 201:
            row = dict(original, zone=71, compatibility='Original1453 Zones.tbl71; legacy SMD201')
            warps[key] = row
            changed[key] = row
    for row in travel.smd_warps(historical / 'siege_0722.smd'):
        offer(30, row)

    # Return routes reuse historical home/Moradon geometry, not modern Moradon.
    for row in travel.smd_warps(historical / 'freezone_1217.smd'):
        row = dict(row)
        row['id'] = 7100 + (row['id'] % 100)
        offer(71, row)

    # Supplementary2004 map routes match the original1453 FreeZone_a assets
    # and nation gate anchors. No primary1453 Ardream START_POSITION exists.
    ardream = {}
    for zone, filename in ((1, 'karus2004.smd'), (2, 'elmo2004.smd'), (21, 'moradon_0826.smd')):
        rows = travel.smd_warps(supplementary / filename)
        for row in rows:
            if row['zone'] != 72:
                continue
            group = row['id'] // 10
            if not any(key // 100000 == zone and existing['id'] // 10 == group for key, existing in warps.items()):
                continue
            prior = next((existing for key, existing in warps.items() if key // 100000 == zone
                          and existing['id'] // 10 == group and existing['zone'] == 72), None)
            if prior:
                continue
            row = dict(row)
            while zone * 100000 + row['id'] in warps:
                slot = next((i for i in range(1, 10) if zone * 100000 + group * 10 + i not in warps), None)
                if slot is None:
                    raise ValueError(f'No free destination slot for gate group{group}.')
                row['id'] = group * 10 + slot
            row['text'] = f"Teleport to Ardream. Levels35-59; National Points required. Fee: {row['coins']} Noah."
            row['compatibility'] = 'Supplementary route; original1453 FreeZone_a nation gate geometry verified'
            offer(zone, row)
            if zone in (1, 2):
                ardream.setdefault(zone, row)
    # Idempotent runs may already have the incoming routes.
    for nation in (1, 2):
        if nation not in ardream:
            ardream[nation] = next(r for key, r in warps.items() if key // 100000 == nation and r['zone'] == 72)
        anchor = db.execute("SELECT data FROM world_reference_rules WHERE table_name='K_OBJECTPOS' AND rule_id/100000=72 AND CAST(json_extract(data,'$.Type') AS INTEGER)=5 AND CAST(json_extract(data,'$.Belong') AS INTEGER)=?", (nation,)).fetchone()
        if not anchor:
            raise ValueError('Original1453 Ardream nation gate is unavailable.')
        anchor = json.loads(anchor[0])
        row = ardream[nation]
        if abs(float(anchor['PosX']) - row['x']) > 50 or abs(float(anchor['PosZ']) - row['z']) > 50:
            raise ValueError('Ardream route does not match original1453 gate geometry.')
    starts = [(72, int(ardream[1]['x']), int(ardream[1]['z']), int(ardream[2]['x']), int(ardream[2]['z']), 0, 0,
               'supplementary/2004-SMD; original1453 FreeZone_a gate geometry verified')]
    for row in travel.smd_warps(supplementary / 'freezone_a.smd'):
        row = dict(row)
        nation = row['nation']
        if row['zone'] == 21:
            # Source Delos returns already provide compatible old Moradon points.
            compatible = warps[3000000 + (3014 if nation == 1 else 3024)]
            row.update(x=compatible['x'], y=compatible['y'], z=compatible['z'])
        row['text'] = f"Teleport to {row['name'].strip()}."
        offer(72, row)
    for row in changed.values():
        if row['zone'] not in bounds:
            # Indoor dungeon destinations use their existing validated start rows.
            continue
        extent = bounds[row['zone']]['extent'] / 10
        if not (0 <= row['x'] - row['radius'] <= row['x'] + row['radius'] < extent
                and 0 <= row['z'] - row['radius'] <= row['z'] + row['radius'] < extent):
            raise ValueError('Warp destination is outside the original1453 map bounds.')
    return magic, changed, starts, bounds


def apply(db, magic, warps, starts, bounds):
    for table, values in (('MAGIC_TYPE8', magic), ('HISTORICAL_WARP', warps), ('MAP_BOUNDS',bounds)):
        for key, row in values.items():
            text = json.dumps(row, sort_keys=True)
            existing = db.execute('SELECT data FROM world_reference_rules WHERE table_name=? AND rule_id=?', (table, key)).fetchone()
            if table in ('MAGIC_TYPE8','MAP_BOUNDS') and existing and json.loads(existing[0]) != row:
                raise ValueError('Existing transport rule differs; review before replacing it.')
            db.execute('INSERT INTO world_reference_rules VALUES(?,?,?) ON CONFLICT(table_name,rule_id) DO UPDATE SET data=excluded.data', (table, key, text))
    for row in starts:
        existing = db.execute('SELECT * FROM world_town_starts WHERE zone=?', (row[0],)).fetchone()
        if existing and tuple(existing) != tuple(row):
            raise ValueError('Existing Ardream town differs; review before replacing it.')
        db.execute('INSERT OR IGNORE INTO world_town_starts VALUES(?,?,?,?,?,?,?,?)', row)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--apply', action='store_true')
    parser.add_argument('--db', type=Path, default=ROOT / 'reko.db')
    parser.add_argument('--reference', type=Path, default=ROOT / 'logs/reference1453')
    parser.add_argument('--historical', type=Path, default=ROOT.parent / 'KnightOnline/assets/Server/MAP')
    parser.add_argument('--supplementary', type=Path, default=ROOT.parent / 'kn_2086_patch_private_server/lCAFILES/Map')
    parser.add_argument('--game-dir', type=Path, default=Path(os.environ.get('REKO_GAME_DIR',ROOT.parent / 'KnightOnline1453')))
    args = parser.parse_args()
    if args.apply:
        running = subprocess.check_output(['powershell', '-NoProfile', '-Command',
            "Get-Process | Where-Object ProcessName -eq 'reko-serverd' | Select-Object -ExpandProperty Id"], text=True)
        if running.strip():
            raise SystemExit('Stop RE-KO before importing rules.')
    with sqlite3.connect(f'file:{args.db.resolve().as_posix()}?mode={"rw" if args.apply else "ro"}', uri=True) as db:
        changes = plan(db, args.reference.resolve(), args.historical.resolve(), args.supplementary.resolve(),args.game_dir.resolve())
        check = sqlite3.connect(':memory:')
        db.backup(check)
        with check:
            apply(check, *changes)
        check.close()
        print('Validated:', len(changes[0]), 'Mage transport rules,', len(changes[1]), 'warp records and Ardream town.')
        if not args.apply:
            return
        stamp = datetime.now().strftime('%Y%m%d-%H%M%S-%f')
        backup = ROOT / 'logs' / f'before-mage-travel-{stamp}.db'
        with sqlite3.connect(backup) as target:
            db.backup(target)
        before = snapshot(db)
        with db:
            apply(db, *changes)
            after = snapshot(db)
            changed = [name for name in before if before[name] != after[name]]
            if set(changed) - {'world_reference_rules', 'world_town_starts'}:
                raise ValueError('Unrelated tables changed; rolling back.')
            if db.execute('PRAGMA integrity_check').fetchone()[0] != 'ok':
                raise ValueError('Integrity check failed; rolling back.')
        receipt = dict(backup=str(backup), changed_tables=changed, before=before, after=after,
                       transport_rules=len(changes[0]), warp_records=len(changes[1]), starts=changes[2])
        (ROOT / 'logs' / f'mage-travel-import-{stamp}.json').write_text(json.dumps(receipt, indent=2))
        print('Applied; unrelated tables preserved. Backup:', backup)


if __name__ == '__main__':
    main()
