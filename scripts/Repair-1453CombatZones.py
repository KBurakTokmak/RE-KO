"""Offline Ardream population, Colony Zone rare-boss pools and local Bifrost access."""
import argparse
from collections import defaultdict
import csv
from datetime import datetime
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import sqlite3
import subprocess

ROOT = Path(__file__).resolve().parents[1]
spec = importlib.util.spec_from_file_location('mage_travel', ROOT / 'scripts/Repair-1453MageTravel.py')
travel = importlib.util.module_from_spec(spec)
spec.loader.exec_module(travel)
SOURCE = 'adapted2086/Ardream/K_NPCPOS-primary1453-templates'
RARE = {'lobo', 'lupus', 'lycaon', 'barrkk', 'barkirra', 'lesath', 'shaula',
        'bone collecter', 'duke', 'bishop', 'bach', 'javana', 'samma', 'orc bandit leader'}
NPC_MAP = {31403: 5300, 31404: 5310, 31405: 5400, 31406: 5410}
MONSTER_MAP = {8851: 5401, 8854: 5301}
# Later event/rental systems are not original1453 population replacements.
EXCLUDED = {8947, 32574, 32575, 32576}


def csv_rows(path):
    with path.open(encoding='utf-8-sig', newline='') as stream:
        return list(csv.DictReader(stream))


def npc_template(row):
    return {**{key: int(row[source]) for key, source in {
        'prototype': 'sSid', 'appearance': 'sPid', 'kind': 'byType',
        'group': 'iSellingGroup', 'nation': 'byGroup', 'level': 'sLevel',
        'size': 'sSize', 'hp': 'iHpPoint', 'armor': 'sAc'}.items()},
        'name': row['strName'].strip(),
        'weapons': [int(row[k]) for k in ('iWeapon1', 'iWeapon2')],
        'resistance': [int(row[k]) for k in ('sFireR', 'sColdR', 'sLightningR', 'sMagicR', 'sDiseaseR', 'sPoisonR')]}


def plan(db, reference, game):
    monsters = {key: json.loads(value) for key, value in db.execute(
        "SELECT rule_id,data FROM world_reference_rules WHERE table_name='MONSTER_TEMPLATE'")}
    npcs = {key: json.loads(value) for key, value in db.execute(
        "SELECT rule_id,data FROM world_reference_rules WHERE table_name='OBJECT_NPC'")}
    secondary = {int(r['sSid']): r for r in csv_rows(reference / 'K_NPC.csv')}
    looks = travel.original_tables.table(game, 'NPC_Looks.tbl')
    bound = json.loads(db.execute("SELECT data FROM world_reference_rules WHERE table_name='MAP_BOUNDS' AND rule_id=72").fetchone()[0])['extent']
    rows = csv_rows(reference / 'K_NPCPOS.csv')
    entities, conditions, skipped = [], [], []
    for index, row in enumerate(rows, 1):
        if int(row['ZoneID']) != 72:
            continue
        original = int(row['NpcID'])
        if original in EXCLUDED:
            skipped.append((index, original, row['strName'], int(row['NumNPC'])))
            continue
        monster = int(row['isNPC']) == 1
        prototype = (MONSTER_MAP if monster else NPC_MAP).get(original, original)
        if monster:
            template = monsters[prototype]
            for table in ('world_monster_ai', 'world_monster_loot'):
                if not db.execute(f'SELECT 1 FROM {table} WHERE prototype=?', (prototype,)).fetchone():
                    raise ValueError(f'Missing primary1453 behavior/rewards for {prototype}.')
        else:
            # Zamed is absent from the supplied primary backup; its original-client
            # model and supplementary static NPC template are retained explicitly.
            template = npc_template(npcs.get(prototype) or secondary[prototype])
        if template['appearance'] not in looks:
            raise ValueError(f'Original1453 model missing: {template["name"]}.')
        if template['name'] != row['strName'].strip():
            raise ValueError('Prototype name mismatch; review mapping.')
        x, z = int(row['LeftX']) * 10, int(row['TopZ']) * 10
        radius = int(row['iRange']) * 10 if monster else 0
        bounds = [x - radius, z - radius, x + radius, z + radius]
        if not all(0 <= coordinate < bound for coordinate in bounds):
            raise ValueError('Ardream position is outside original1453 terrain.')
        for ordinal in range(int(row['NumNPC'])):
            entity = dict(id=21000 + len(entities), source=SOURCE, source_row=index, ordinal=ordinal,
                monster=int(monster), zone=72, prototype=prototype, appearance=template['appearance'],
                name=template['name'], kind=template['kind'], selling_group=template['group'],
                nation=template['nation'], level=template['level'], size=template['size'],
                weapon1=template['weapons'][0], weapon2=template['weapons'][1], direction=int(row['byDirection']),
                x0=bounds[0], z0=bounds[1], x1=bounds[2], z1=bounds[3], hp=template['hp'], armor=template['armor'],
                **dict(zip(('fire_r', 'cold_r', 'lightning_r', 'magic_r', 'disease_r', 'poison_r'), template['resistance'])),
                respawn_ms=int(row['RegTime']) * 1000, regen_type=0,
                move_type=int(row['ActType']), path=row['path'])
            if entity['id'] >= 30000 or entity['hp'] <= 0 or entity['respawn_ms'] <= 0:
                raise ValueError('Invalid population record.')
            entities.append(entity)
            conditions.append((entity['id'], 1, '', int(row['DungeonFamily']), int(row['SpecialType']), int(row['TrapNumber'])))
    if not entities:
        raise ValueError('Ardream population input is empty.')
    pools = defaultdict(list)
    for entity, name in db.execute("SELECT id,name FROM world_spawns WHERE zone=71 AND monster=1 ORDER BY id"):
        if name.strip().lower() in RARE:
            pools[name.strip().lower()].append(entity)
    if pools.keys() != RARE:
        raise ValueError('Expected Colony Zone rare-boss candidates are missing.')
    return entities, conditions, dict(pools), skipped


def apply(db, entities, conditions, pools, skipped):
    expected_ids = {e['id'] for e in entities}
    existing_ids = {r[0] for r in db.execute('SELECT id FROM world_spawns WHERE zone=72')}
    if existing_ids - expected_ids:
        raise ValueError('Refusing to replace an existing Ardream roster.')
    for entity in entities:
        columns = list(entity)
        old = db.execute('SELECT ' + ','.join(columns) + ' FROM world_spawns WHERE id=?', (entity['id'],)).fetchone()
        if old and tuple(entity.values()) != tuple(old):
            raise ValueError('Occupied or modified entity; refusing overwrite.')
        if db.execute('SELECT 1 FROM world_merchants WHERE id=?', (entity['id'],)).fetchone():
            raise ValueError('Entity conflicts with an existing merchant.')
        db.execute('INSERT OR IGNORE INTO world_spawns (' + ','.join(columns) + ') VALUES (' + ','.join('?' for _ in columns) + ')', tuple(entity.values()))
    for condition in conditions:
        old = db.execute('SELECT * FROM world_spawn_conditions WHERE entity_id=?', (condition[0],)).fetchone()
        if old and tuple(old) != condition:
            raise ValueError('Existing population condition differs.')
        db.execute('INSERT OR IGNORE INTO world_spawn_conditions VALUES(?,?,?,?,?,?)', condition)
    for name, ids in pools.items():
        key = '71:' + name
        pool = (key, 71, name, 21600000)
        old = db.execute('SELECT * FROM world_boss_pools WHERE pool_id=?', (key,)).fetchone()
        if old and tuple(old) != pool:
            raise ValueError('Existing boss policy differs.')
        db.execute('INSERT OR IGNORE INTO world_boss_pools VALUES(?,?,?,?)', pool)
        for entity in ids:
            old = db.execute('SELECT pool_id FROM world_boss_members WHERE entity_id=?', (entity,)).fetchone()
            if old and old[0] != key:
                raise ValueError('Boss already belongs to another pool.')
            db.execute('INSERT OR IGNORE INTO world_boss_members VALUES(?,?)', (entity, key))
    access = {'mode': 'always_open', 'source': 'owner requirement; event/monument transition deferred'}
    old = db.execute("SELECT data FROM world_reference_rules WHERE table_name='BIFROST_ACCESS' AND rule_id=31").fetchone()
    if old and json.loads(old[0]) != access:
        raise ValueError('Existing Bifrost access policy differs; refusing overwrite.')
    db.execute("INSERT OR IGNORE INTO world_reference_rules VALUES('BIFROST_ACCESS',31,?)", (json.dumps(access, sort_keys=True),))


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--db', type=Path, default=ROOT / 'reko.db')
    parser.add_argument('--reference', type=Path, default=ROOT / 'logs/reference-rules')
    parser.add_argument('--game-dir', type=Path, default=Path(os.environ.get('REKO_GAME_DIR', ROOT.parent / 'KnightOnline1453')))
    parser.add_argument('--apply', action='store_true')
    args = parser.parse_args()
    if args.apply:
        running = subprocess.check_output(['powershell', '-NoProfile', '-Command', '@(Get-Process reko-serverd -ErrorAction SilentlyContinue).Count'], text=True)
        if args.db.resolve() == (ROOT / 'reko.db').resolve() and int(running.strip()):
            raise SystemExit('Stop RE-KO before importing combat-zone data.')
    with sqlite3.connect(f'file:{args.db.resolve().as_posix()}?mode={"rw" if args.apply else "ro"}', uri=True) as db:
        changes = plan(db, args.reference, args.game_dir)
        check = sqlite3.connect(':memory:')
        db.backup(check)
        with check:
            apply(check, *changes)
        once = travel.snapshot(check)
        with check:
            apply(check, *changes)
        if once != travel.snapshot(check):
            raise ValueError('Import is not idempotent.')
        print(json.dumps(dict(ardream_instances=len(changes[0]), monsters=sum(e['monster'] for e in changes[0]),
            rare_boss_pools=len(changes[2]), rare_boss_candidates=sum(map(len, changes[2].values())),
            excluded_later_systems=changes[3], respawn_policy_seconds=21600,
            input_sha256=hashlib.sha256((args.reference / 'K_NPCPOS.csv').read_bytes()).hexdigest()), indent=2))
        if not args.apply:
            print('Dry run and idempotence passed; use --apply with the daemon stopped.')
            return
        stamp = datetime.now().strftime('%Y%m%d-%H%M%S')
        backup = ROOT / 'logs' / f'before-combat-zones-{stamp}.db'
        with sqlite3.connect(backup) as target:
            db.backup(target)
        before = travel.snapshot(db)
        with db:
            apply(db, *changes)
            after = travel.snapshot(db)
            changed = [name for name in before if before[name] != after[name]]
            if set(changed) - {'world_spawns', 'world_spawn_conditions', 'world_boss_pools', 'world_boss_members', 'world_reference_rules'}:
                raise ValueError('Unrelated tables changed; rolling back.')
            if db.execute('PRAGMA integrity_check').fetchone()[0] != 'ok':
                raise ValueError('Database integrity failed.')
        receipt = dict(backup=str(backup), before=before, after=after, changed_tables=changed)
        (ROOT / 'logs' / f'combat-zones-import-{stamp}.json').write_text(json.dumps(receipt, indent=2))
        print('Applied; unrelated data preserved. Backup:', backup)


if __name__ == '__main__':
    main()
