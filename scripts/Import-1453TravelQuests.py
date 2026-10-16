"""Read historical SMD travel and private1453 quest objective data, offline only."""
import argparse
import csv
from datetime import datetime
import json
from pathlib import Path
import sqlite3
import struct
import subprocess

ROOT = Path(__file__).resolve().parents[1]
REFERENCE = ROOT.parent / 'PV_1453/inspection'


def rows(name):
    with (REFERENCE / (name + '.csv')).open(encoding='utf-8-sig', newline='') as f:
        return list(csv.DictReader(f))


def smd_warps(path, include_tiles=False):
    data = path.read_bytes()
    offset = 0

    def read(fmt):
        nonlocal offset
        result = struct.unpack_from('<' + fmt, data, offset)
        offset += struct.calcsize('<' + fmt)
        return result

    def skip(count):
        nonlocal offset
        assert 0 <= count <= len(data) - offset, (path, offset, count)
        offset += count

    size, unit = read('if')
    assert 1 < size <= 1025 and unit == 4, path
    skip(size * size * 4)
    width, length, faces = read('ffi')
    assert width == length == (size - 1) * unit and faces >= 0, path
    skip(faces * 36)
    for _ in range(int(width / 16) * int(length / 16)):
        if read('I')[0]:
            skip(read('I')[0] * 2)
            for _ in range(16):
                skip(read('I')[0] * 12)
    skip(read('I')[0] * 24)
    tiles = [(i // size, i % size, event) for i, (event,) in
             enumerate(struct.iter_unpack('<h', data[offset:offset+size*size*2])) if event >= 2] if include_tiles else []
    skip(size * size * 2)
    skip(read('I')[0] * 20)
    count = read('I')[0]
    assert count < 1000 and offset + count * 320 == len(data), (path, offset, count, len(data))
    result = []
    for _ in range(count):
        warp, name, text, _, pay, zone, _, x, y, z, radius, nation, _ = read('h32s256sHIhHffffhH')
        assert warp > 0 and 0 < zone < 256 and nation in (0, 1, 2), path
        assert 0 <= x <= 6553 and 0 <= z <= 6553 and 0 <= radius < 1000, path
        result.append(dict(id=warp, name=name.split(b'\0')[0].decode('cp1252'),
                           text=text.split(b'\0')[0].decode('cp1252'), coins=pay,
                           zone=zone, x=x, y=y, z=z, radius=radius, nation=nation,
                           source=str(path.relative_to(ROOT.parent))))
    return (result, tiles) if include_tiles else result


def collect():
    maps = ROOT.parent / 'KnightOnline/assets/Server/MAP'
    # Required historical maps; modern2086 Moradon is a different layout.
    candidates = {21: 'moradon_0826.smd', 1: 'karus_0730.smd', 2: 'elmo_0730.smd'}
    result = {}
    for zone, filename in candidates.items():
        path = maps / filename
        if not path.exists():
            print('Map unavailable:', path)
            continue
        result[zone] = smd_warps(path)
        print('Historical travel:', zone, len(result[zone]), filename)
    return result


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--apply', action='store_true')
    parser.add_argument('--db', type=Path, default=ROOT / 'reko.db')
    args = parser.parse_args()
    travel = collect()
    objectives = rows('QUEST_MONSTER')
    print('Quest objective records:', len(objectives))
    if not args.apply:
        return
    processes = subprocess.check_output(['powershell', '-NoProfile', '-Command',
        "Get-Process | Where-Object ProcessName -eq 'reko-serverd' | Select-Object -ExpandProperty Id"], text=True)
    if processes.strip():
        raise SystemExit('Stop RE-KO before importing rules.')
    assert args.db.exists()
    db = sqlite3.connect(args.db)
    backup = ROOT / 'logs' / ('before-travel-quests-' + datetime.now().strftime('%Y%m%d-%H%M%S') + '.db')
    with sqlite3.connect(backup) as target:
        db.backup(target)
    with db:
        for zone, entries in travel.items():
            for entry in entries:
                db.execute('INSERT OR REPLACE INTO world_reference_rules VALUES (?,?,?)',
                           ('HISTORICAL_WARP', zone * 100000 + entry['id'], json.dumps(entry)))
        for row in objectives:
            db.execute('INSERT OR REPLACE INTO world_reference_rules VALUES (?,?,?)',
                       ('QUEST_MONSTER', int(row['sQuestNum']), json.dumps(row)))
    print('Backup:', backup)
    print('Integrity:', db.execute('PRAGMA integrity_check').fetchone()[0])


if __name__ == '__main__':
    main()
