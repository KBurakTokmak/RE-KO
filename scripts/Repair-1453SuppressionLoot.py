"""Repair missing suppression certificates and owner-approved estimated defense armor drops."""
import argparse
import csv
from datetime import datetime
import hashlib
import json
import os
from pathlib import Path
import re
import sqlite3
import subprocess

ROOT = Path(__file__).resolve().parents[1]
REGULAR = {82: (7005, 7006, 7007, 7009, 7040), 83: (7010, 7011, 7012, 7014)}
CERTIFICATES = {82: 910138000, 83: 910137000}
BOSSES = {82: (7008, 910135000), 83: (7013, 910136000)}
# Relative part weights are a local approximation, not recovered retail rates.
PARTS = ((3, 1, 15, 3), (5, 2, 19, 3), (1, 3, 17, 5), (2, 4, 18, 5), (4, 5, 16, 5))


def table(game, name):
    result = subprocess.run([str(ROOT / 'target/debug/reko.exe'), 'dump-tbl',
                             str(game / 'Data' / name), '--limit', '20000', '--full'],
                            capture_output=True, check=True, encoding='utf-8')
    out = {}
    for line in result.stdout.splitlines():
        match = re.match(r'\s*\[\s*\d+\]\s+(.*)', line)
        if match:
            row = next(csv.reader([match[1]], escapechar='\\', skipinitialspace=True))
            out[int(row[0])] = row
    if not out:
        raise ValueError('No original table rows: ' + name)
    return out


def plan(db, game, light_rate, heavy_rate):
    items = table(game, 'item_org_us.tbl')
    extensions = {group: table(game, f'Item_Ext_{group}_us.tbl') for group in range(13, 18)}
    for exchange, expected in ((94, (910138000, 910135000)), (98, (910137000, 910136000))):
        row = db.execute("SELECT data FROM world_reference_rules WHERE table_name='ITEM_EXCHANGE' AND rule_id=?", (exchange,)).fetchone()
        if not row:
            raise ValueError(f'Import original ITEM_EXCHANGE {exchange} first.')
        row = json.loads(row[0])
        actual = tuple(int(row[f'nOriginItemNum{i}']) for i in (1, 2))
        counts = tuple(int(row[f'nOriginItemCount{i}']) for i in (1, 2))
        if actual != expected or counts != (3, 1):
            raise ValueError(f'Unexpected suppression quest requirements: {exchange}.')

    rules, groups = [], {}
    for zone in (82, 83):
        # MSS II: tier5; MSS III: tier1. Mage/priest have a modest class bias.
        tier = 5 if zone == 82 else 1
        for index, (classes, rate) in enumerate((((3, 4), light_rate), ((1, 2), heavy_rate))):
            group_id = zone * 1000 + index + 1
            group = []
            for part, copies, defense_column, defense in PARTS:
                for job in classes:
                    base = ({1: 200, 2: 240, 3: 260, 4: 280}[job] + tier) * 1000000 + part * 1000
                    row = items.get(base)
                    if not row or int(row[14]) != job or int(row[12]) != part + 4:
                        raise ValueError(f'Original armor base mismatch: {base}.')
                    extension = extensions[int(row[1])].get(541)
                    if not extension or int(extension[defense_column]) != defense:
                        raise ValueError(f'Original +1 defense extension mismatch: {base + 541}.')
                    group.extend([base + 541] * copies)
            assert len(group) == 30
            groups[group_id] = group
            for prototype in REGULAR[zone]:
                rules.append((prototype, index, group_id, rate))
        for prototype in REGULAR[zone]:
            rules.append((prototype, 2, CERTIFICATES[zone], 10000))
        boss, certificate = BOSSES[zone]
        rules.append((boss, 2, certificate, 10000))
        for prototype in (*REGULAR[zone], boss):
            row = db.execute('SELECT drop_id FROM world_monster_loot WHERE prototype=?', (prototype,)).fetchone()
            present = db.execute('SELECT EXISTS(SELECT 1 FROM world_spawns WHERE prototype=? AND zone=? AND monster=1)', (prototype, zone)).fetchone()[0]
            if row != (prototype,) or not present:
                raise ValueError(f'Unexpected monster/zone/drop linkage: {prototype}/{zone}.')
        for item in (CERTIFICATES[zone], certificate):
            if item not in items:
                raise ValueError(f'Certificate absent from original client: {item}.')
    return rules, groups


def apply(db, rules, groups):
    # Refuse to replace a nonempty source or independently customized drop.
    for drop, slot, item, chance in rules:
        existing = db.execute('SELECT item,chance FROM world_drop_rules WHERE drop_id=? AND slot=?', (drop, slot)).fetchone()
        if existing and existing[0] not in (0, item):
            raise ValueError(f'Refusing to overwrite occupied drop slot: {drop}/{slot}.')
        if existing and existing[0] == item and existing[1] != chance:
            raise ValueError(f'Existing rate differs; review before changing {drop}/{slot}.')
    for group, items in groups.items():
        existing = db.execute('SELECT slot,item FROM world_drop_groups WHERE group_id=? ORDER BY slot', (group,)).fetchall()
        if existing and existing != list(enumerate(items)):
            raise ValueError(f'Refusing to overwrite existing loot group: {group}.')
    db.executemany('INSERT INTO world_drop_rules VALUES (?,?,?,?) ON CONFLICT(drop_id,slot) DO UPDATE SET item=excluded.item,chance=excluded.chance WHERE item!=excluded.item OR chance!=excluded.chance', rules)
    for group, items in groups.items():
        db.executemany('INSERT OR IGNORE INTO world_drop_groups VALUES (?,?,?)', ((group, slot, item) for slot, item in enumerate(items)))


def snapshot(db):
    out = {}
    for (name,) in db.execute("SELECT name FROM sqlite_master WHERE type='table' AND name NOT LIKE 'sqlite_%' ORDER BY name"):
        quoted = '"' + name.replace('"', '""') + '"'
        rows = sorted(db.execute('SELECT * FROM ' + quoted).fetchall(), key=repr)
        out[name] = hashlib.sha256(repr(rows).encode()).hexdigest()
    return out


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--db', type=Path, default=ROOT / 'reko.db')
    parser.add_argument('--game-dir', type=Path, default=Path(os.environ.get('REKO_GAME_DIR', ROOT.parent / 'KnightOnline1453')))
    parser.add_argument('--light-rate', type=int, default=150, help='Mage/priest pool chance out of10000; local estimate')
    parser.add_argument('--heavy-rate', type=int, default=100, help='Warrior/rogue pool chance out of10000; local estimate')
    parser.add_argument('--apply', action='store_true')
    args = parser.parse_args()
    if not args.db.is_file() or not all(0 <= n <= 10000 for n in (args.light_rate, args.heavy_rate)):
        raise SystemExit('Existing save and rates in0..10000 required.')
    if args.apply and args.db.resolve() == (ROOT / 'reko.db').resolve():
        running = subprocess.check_output(['powershell.exe', '-NoProfile', '-Command',
            '@(Get-Process reko-serverd -ErrorAction SilentlyContinue).Count'], text=True)
        if int(running.strip()):
            raise SystemExit('Stop RE-KO before repairing the live save.')
    with sqlite3.connect(f'file:{args.db.resolve().as_posix()}?mode={"rw" if args.apply else "ro"}', uri=True) as db:
        rules, groups = plan(db, args.game_dir.resolve(), args.light_rate, args.heavy_rate)
        print(f'Plan: {len(rules)} bounded drop slots, {len(groups)} original-client armor groups; certificates100%, armor pools{args.light_rate}/{args.heavy_rate} per10000.')
        # Validate collisions in a disposable in-memory copy even during dry run.
        check = sqlite3.connect(':memory:')
        db.backup(check)
        with check:
            apply(check, rules, groups)
        check.close()
        if not args.apply:
            return
        stamp = datetime.now().strftime('%Y%m%d-%H%M%S-%f')
        backup = ROOT / 'logs' / f'before-suppression-loot-{stamp}.db'
        with sqlite3.connect(backup) as target:
            db.backup(target)
        before = snapshot(db)
        with db:
            apply(db, rules, groups)
            after = snapshot(db)
            changed = [name for name in before if before[name] != after[name]]
            if set(changed) - {'world_drop_rules', 'world_drop_groups'}:
                raise ValueError('Unrelated save data changed; rolling back.')
            if db.execute('PRAGMA integrity_check').fetchone()[0] != 'ok':
                raise ValueError('Integrity check failed; rolling back.')
        receipt = dict(backup=str(backup), changed_tables=changed, before=before, after=after,
                       rules=rules, groups=groups, armor_rates_are_owner_approved_estimates=True,
                       reference='Supplied1453 quest exchanges, original1453 items; Kalais MSS guide for armor categories only')
        (ROOT / 'logs' / f'suppression-loot-{stamp}.json').write_text(json.dumps(receipt, indent=2), encoding='utf-8')
        print('Applied; unrelated tables preserved. Backup:', backup)


if __name__ == '__main__':
    main()
