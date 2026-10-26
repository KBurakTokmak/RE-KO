"""Offline save preparation. World templates are private local inputs, never Git data."""
import argparse
import csv
from contextlib import closing, contextmanager
from datetime import datetime
import hashlib
import importlib.util
import io
import json
import os
from pathlib import Path
import sqlite3
import subprocess
import tempfile

ROOT = Path(__file__).resolve().parents[1]
SCHEMA = 1
REVISION = "001-delos-travel"
TABLES = (
    "world_merchants", "world_spawns", "world_spawn_conditions", "world_town_starts",
    "world_monster_ai", "world_monster_loot", "world_drop_rules", "world_drop_groups",
    "world_reference_rules", "world_upgrade_item_types", "world_boss_pools", "world_boss_members",
)
INPUTS = ("reference-coefficients.csv", "reference-rules/MAGIC_TYPE5.csv")
DELOS_IDS = {3014, 3024, 3034, 3044, 3055}


@contextmanager
def connect(path, readonly=False):
    with closing(sqlite3.connect(path.resolve().as_uri() + ("?mode=ro" if readonly else "?mode=rw"), uri=True)) as db:
        with db:
            yield db


def columns(db, name):
    return [r[1] for r in db.execute(f'PRAGMA table_info("{name}")')]


def digest(value):
    return hashlib.sha256(json.dumps(value, sort_keys=True, separators=(",", ":")).encode()).hexdigest()


def verify(db):
    if db.execute("PRAGMA integrity_check").fetchone()[0] != "ok":
        raise ValueError("Database integrity check failed.")
    if db.execute("PRAGMA foreign_key_check").fetchone():
        raise ValueError("Database has invalid foreign keys; repair explicitly before updating.")


def readiness(db):
    issues = []
    required = {"LEVEL_UP": 80, "K_OBJECTPOS": 1, "HISTORICAL_WARP": 1,
                "QUEST_HELPER": 1, "ITEM_UPGRADE": 1, "MONSTER_TEMPLATE": 1}
    for category, minimum in required.items():
        count = db.execute("SELECT count(*) FROM world_reference_rules WHERE table_name=?", (category,)).fetchone()[0]
        if count < minimum:
            issues.append(f"{category}: need at least {minimum} rules; found {count}")
    for name in ("world_spawns", "world_monster_ai", "world_monster_loot", "world_town_starts"):
        if not db.execute(f'SELECT EXISTS(SELECT 1 FROM "{name}")').fetchone()[0]:
            issues.append(f"{name}: missing world data")
    routes = {r[0] % 100000 for r in db.execute("SELECT rule_id FROM world_reference_rules WHERE table_name='HISTORICAL_WARP' AND rule_id/100000=30")}
    if not DELOS_IDS <= routes:
        issues.append("Delos routes are incomplete.")
    return issues


def delos(db, historical, template=None):
    present = {int(i) % 100000: json.loads(data) for i, data in db.execute(
        "SELECT rule_id,data FROM world_reference_rules WHERE table_name='HISTORICAL_WARP' AND rule_id/100000=30")}
    path = historical / "siege_0722.smd"
    if path.is_file():
        spec = importlib.util.spec_from_file_location("historical_travel", ROOT / "scripts/Import-1453TravelQuests.py")
        travel = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(travel)
        rows = travel.smd_warps(path)
        if {r["id"] for r in rows} != DELOS_IDS:
            raise ValueError("Unexpected historical Delos route set; refusing guessed destinations.")
    elif DELOS_IDS <= present.keys():
        rows = [present[i] for i in sorted(DELOS_IDS)]
    elif template and template.is_file():
        data = template_data(template)
        table = data["tables"]["world_reference_rules"]
        if table["columns"] != ["table_name", "rule_id", "data"]:
            raise ValueError("Unexpected world-rule template columns.")
        rows = [json.loads(row) for category, key, row in table["rows"]
                if category == "HISTORICAL_WARP" and key // 100000 == 30 and key % 100000 in DELOS_IDS]
        if {r["id"] for r in rows} != DELOS_IDS:
            raise ValueError("Local template is missing the five Delos routes; refresh the world template.")
    else:
        raise ValueError(f"Delos migration needs {path} or an updated local world template.")
    for row in rows:
        old = present.get(row["id"])
        # Provenance path separators do not affect gameplay. Preserve customized routes.
        if old and {k: v for k, v in old.items() if k != "source"} != {k: v for k, v in row.items() if k != "source"}:
            raise ValueError(f"Delos route {row['id']} differs from reference; review the local customization.")
        gate = db.execute("SELECT data FROM world_reference_rules WHERE table_name='K_OBJECTPOS' AND rule_id/100000=30 AND CAST(json_extract(data,'$.Type') AS INTEGER)=5 AND CAST(json_extract(data,'$.ControlNpcID') AS INTEGER)=?", (row["id"] // 10,)).fetchone()
        if not gate or int(json.loads(gate[0])["Belong"]) != row["nation"]:
            raise ValueError(f"Delos gate group {row['id']//10} is absent or has a different nation.")
        if not db.execute("SELECT 1 FROM world_town_starts WHERE zone=?", (row["zone"],)).fetchone():
            raise ValueError(f"Delos destination zone {row['zone']} needs its original town data.")
        db.execute("INSERT OR IGNORE INTO world_reference_rules VALUES('HISTORICAL_WARP',?,?)",
                   (3000000 + row["id"], json.dumps(row, sort_keys=True)))


def module(name):
    spec = importlib.util.spec_from_file_location(name.replace('-', '_'), ROOT / 'scripts' / (name + '.py'))
    result = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(result)
    return result


def from_template(db, template, selectors, minimum=1):
    if not template or not template.is_file():
        raise ValueError('Pending content migration needs its original inputs or an updated world template; see docs/setup.md.')
    data = template_data(template)
    total = 0
    for name, select in selectors.items():
        table = data['tables'][name]
        names = columns(db, name)
        if table['columns'] != names:
            raise ValueError(f'Template schema mismatch: {name}.')
        keys = [r[1] for r in db.execute(f'PRAGMA table_info("{name}")') if r[5]]
        where = ' AND '.join(f'"{key}"=?' for key in keys)
        for row in table['rows']:
            record = dict(zip(names, row))
            if not select(record):
                continue
            total += 1
            values = [record[key] for key in keys]
            old = db.execute(f'SELECT * FROM "{name}" WHERE {where}', values).fetchone()
            if old is not None and tuple(row) != old:
                allowed = False
                if name == 'world_reference_rules':
                    before, after = json.loads(old[names.index('data')]), json.loads(record['data'])
                    if before == after:
                        continue
                    if record['table_name'] == 'HISTORICAL_WARP':
                        before.pop('source', None); after.pop('source', None)
                        if before == after:
                            continue
                        if before.get('zone') == 201 and after.get('zone') == 71:
                            before['zone'] = 71
                            before.pop('compatibility', None); after.pop('compatibility', None)
                            allowed = before == after
                elif name == 'world_drop_rules':
                    allowed = old[names.index('item')] == 0
                if not allowed:
                    raise ValueError(f'Local customization conflicts with template migration: {name}/{values}.')
                db.execute(f'DELETE FROM "{name}" WHERE {where}', values)
            db.execute(f'INSERT OR IGNORE INTO "{name}" VALUES ({",".join("?" for _ in row)})', row)
    if total < minimum:
        raise ValueError('World template predates this migration; obtain a current template.')


def mage_travel(db, historical, template):
    reference = ROOT / 'logs/reference1453'
    supplementary = Path(os.environ.get('REKO_SUPPLEMENTARY_MAPS', ROOT.parent / 'kn_2086_patch_private_server/lCAFILES/Map'))
    game = Path(os.environ.get('REKO_GAME_DIR', ROOT.parent / 'KnightOnline1453'))
    if (reference / 'MAGIC_TYPE8.json').is_file() and (historical / 'freezone_1217.smd').is_file() and (supplementary / 'freezone_a.smd').is_file():
        repair = module('Repair-1453MageTravel')
        repair.apply(db, *repair.plan(db, reference, historical, supplementary, game))
    else:
        from_template(db, template, {
            'world_reference_rules': lambda r: r['table_name'] in ('MAGIC_TYPE8', 'MAP_BOUNDS') or
                r['table_name'] == 'HISTORICAL_WARP' and
                (r['rule_id']//100000 in (30,71,72) or json.loads(r['data']).get('zone') in (71,72)),
            'world_town_starts': lambda r: r['zone'] == 72,
        }, minimum=15)
    if db.execute("SELECT count(*) FROM world_reference_rules WHERE table_name='MAGIC_TYPE8'").fetchone()[0] < 14:
        raise ValueError('Mage travel rules are incomplete.')
    if any(json.loads(r[0]).get('zone') == 201 for r in db.execute("SELECT data FROM world_reference_rules WHERE table_name='HISTORICAL_WARP'")):
        raise ValueError('Legacy Colony Zone201 routes still need migration to71.')


def combat_zones(db, historical, template):
    reference = ROOT / 'logs/reference-rules'
    if (reference / 'K_NPCPOS.csv').is_file() and (reference / 'K_NPC.csv').is_file():
        repair = module('Repair-1453CombatZones')
        repair.apply(db, *repair.plan(db, reference, Path(os.environ.get('REKO_GAME_DIR', ROOT.parent / 'KnightOnline1453'))))
    else:
        data = template_data(template)
        spawns = data['tables']['world_spawns']
        ids = {dict(zip(spawns['columns'], row))['id'] for row in spawns['rows'] if dict(zip(spawns['columns'], row))['zone'] == 72}
        from_template(db, template, {
            'world_spawns': lambda r: r['zone'] == 72,
            'world_spawn_conditions': lambda r: r['entity_id'] in ids,
            'world_boss_pools': lambda r: r['zone'] == 71,
            'world_boss_members': lambda r: r['pool_id'].startswith('71:'),
            'world_reference_rules': lambda r: r['table_name'] == 'BIFROST_ACCESS',
        }, minimum=442)
    if db.execute('SELECT count(*) FROM world_spawns WHERE zone=72').fetchone()[0] != 442:
        raise ValueError('Ardream population must contain442 source-adapted entities.')


def transformations(db, historical, template):
    reference = ROOT / 'logs/reference1453/MAGIC_TYPE6.json'
    if reference.is_file():
        repair = module('Import-1453Transformations')
        repair.apply(db, repair.plan(db, reference, Path(os.environ.get('REKO_GAME_DIR', ROOT.parent / 'KnightOnline1453'))))
    else:
        from_template(db, template, {'world_reference_rules': lambda r: r['table_name'] == 'MAGIC_TYPE6'}, minimum=46)


def suppression_loot(db, historical, template):
    game = Path(os.environ.get('REKO_GAME_DIR', ROOT.parent / 'KnightOnline1453'))
    if (ROOT / 'target/debug/reko.exe').is_file() and (game / 'Data/item_org_us.tbl').is_file():
        repair = module('Repair-1453SuppressionLoot')
        repair.apply(db, *repair.plan(db, game, 150, 100))
    else:
        from_template(db, template, {
            'world_drop_rules': lambda r: r['drop_id'] in (7005,7006,7007,7009,7040,7010,7011,7012,7014) and r['slot'] in (0,1,2) or
                r['drop_id'] in (7008,7013) and r['slot'] == 2,
            'world_drop_groups': lambda r: r['group_id'] in (82001,82002,83001,83002),
        }, minimum=29)


def zone_names(db, historical, template):
    reference = ROOT / 'logs/reference1453/ZONE_INFO.json'
    if reference.is_file():
        rows = json.loads(reference.read_text(encoding='utf-8-sig'))
        for row in rows:
            key = int(row['ZoneNo'])
            old = db.execute("SELECT data FROM world_reference_rules WHERE table_name='ZONE_INFO' AND rule_id=?", (key,)).fetchone()
            if old and json.loads(old[0]) != row:
                raise ValueError(f'Zone metadata customization conflicts: {key}.')
            db.execute("INSERT OR IGNORE INTO world_reference_rules VALUES('ZONE_INFO',?,?)", (key,json.dumps(row,sort_keys=True)))
    else:
        from_template(db, template, {'world_reference_rules': lambda r: r['table_name'] == 'ZONE_INFO'})


def potion_balance(db, historical, template):
    from item_balance import definitions
    for item in definitions():
        value = json.dumps(dict(price=item['price'], weight=item['weight']), sort_keys=True)
        existing = db.execute("SELECT data FROM world_reference_rules WHERE table_name='ITEM_ECONOMY' AND rule_id=?", (item['id'],)).fetchone()
        if existing and json.loads(existing[0]) != json.loads(value):
            raise ValueError(f"Item {item['id']} has a different local economy rule; refusing overwrite.")
        db.execute("INSERT OR IGNORE INTO world_reference_rules VALUES('ITEM_ECONOMY',?,?)", (item['id'], value))


# Append new revisions in order; never rename/replay an already shipped revision.
MIGRATIONS = ((REVISION, delos), ('002-mage-combat-travel', mage_travel),
              ('003-combat-zone-population', combat_zones), ('004-transformations', transformations),
              ('005-suppression-loot', suppression_loot), ('006-zone-metadata', zone_names),
              ('007-potion-balance', potion_balance))


def migrate_content(db, historical, template=None):
    db.execute("CREATE TABLE IF NOT EXISTS reko_content_migrations (revision TEXT PRIMARY KEY, applied_at TEXT NOT NULL)")
    revisions = {r[0] for r in db.execute("SELECT revision FROM reko_content_migrations")}
    if revisions - {revision for revision, _ in MIGRATIONS}:
        raise ValueError("Save has newer/unknown content migrations; update the checkout first.")
    for revision, apply in MIGRATIONS:
        if revision not in revisions:
            apply(db, historical, template)
            db.execute("INSERT INTO reko_content_migrations VALUES (?,?)", (revision, datetime.now().isoformat()))


def status(path):
    if not path.exists():
        return ["No save: a new database and local world template are required."]
    with connect(path, True) as db:
        verify(db)
        version = db.execute("PRAGMA user_version").fetchone()[0]
        if version > SCHEMA:
            raise ValueError(f"Schema {version} is newer than supported {SCHEMA}; update the checkout.")
        names = {r[0] for r in db.execute("SELECT name FROM sqlite_master WHERE type='table'")}
        issues = [] if version == SCHEMA else [f"Schema upgrade needed: {version} -> {SCHEMA}"]
        revisions = {r[0] for r in db.execute("SELECT revision FROM reko_content_migrations")} if "reko_content_migrations" in names else set()
        if revisions - {revision for revision, _ in MIGRATIONS}:
            raise ValueError("Unknown content migrations; update the checkout.")
        for revision, _ in MIGRATIONS:
            if revision not in revisions:
                issues.append(f"Content migration needed: {revision}")
        if set(TABLES) <= names:
            issues.extend(readiness(db))
        else:
            issues.append("Schema/world tables are incomplete.")
        if not (ROOT / "logs" / INPUTS[0]).is_file():
            issues.append("Local combat coefficients are missing; provide the world template or reference CSV.")
        return issues


def export_template(source, output):
    if source.resolve() == output.resolve() or output.exists():
        raise ValueError("Template output must be a new file, separate from the save.")
    with connect(source, True) as db:
        db.execute("BEGIN")
        verify(db)
        issues = readiness(db)
        if issues:
            raise ValueError("Cannot export incomplete world: " + "; ".join(issues))
        tables = {name: {"columns": columns(db, name), "rows": list(db.execute(f'SELECT * FROM "{name}"'))} for name in TABLES}
        # Never select account, character, clan, grant, password or migration tables.
        inputs = {}
        for name in INPUTS:
            path = ROOT / "logs" / name
            if path.is_file():
                inputs[name] = path.read_text(encoding="utf-8-sig")
        if INPUTS[0] not in inputs:
            raise ValueError("Combat coefficients are required for a usable world template.")
        payload = dict(format=1, schema=SCHEMA, tables=tables, inputs=inputs)
        output.parent.mkdir(parents=True, exist_ok=True)
        output.write_text(json.dumps(dict(payload=payload, sha256=digest(payload))), encoding="utf-8")


def template_data(path):
    bundle = json.loads(path.read_text(encoding="utf-8"))
    data = bundle["payload"]
    if digest(data) != bundle["sha256"] or data["format"] != 1 or data["schema"] != SCHEMA:
        raise ValueError("Unsupported or damaged world template.")
    if set(data["tables"]) != set(TABLES) or set(data["inputs"]) - set(INPUTS):
        raise ValueError("Template contains unexpected tables/files; refusing personal or executable data.")
    return data


def load_template(path, db):
    data = template_data(path)
    for name in TABLES:
        table = data["tables"][name]
        expected = columns(db, name)
        if table["columns"] != expected:
            raise ValueError(f"Template schema mismatch in {name}.")
        marks = ",".join("?" for _ in expected)
        db.executemany(f'INSERT INTO "{name}" VALUES ({marks})', table["rows"])
    if INPUTS[0] not in data["inputs"] or not list(csv.DictReader(io.StringIO(data["inputs"][INPUTS[0]]))):
        raise ValueError("Template has no combat coefficients.")
    return data["inputs"]


def ensure_stopped():
    if os.name == "nt":
        running = subprocess.check_output(["powershell", "-NoProfile", "-Command",
            "Get-Process | Where-Object ProcessName -eq 'reko-serverd' | Select-Object -ExpandProperty Id"], text=True)
        if running.strip():
            raise ValueError("Log out normally, then stop RE-KO before database maintenance.")


def prepare(path, template, server, historical, reset=False):
    """Stage all changes on a copy; publish only after schema/content/integrity checks."""
    path = path.resolve()
    path.parent.mkdir(parents=True, exist_ok=True)
    existed = path.exists()
    if (not existed or reset) and not template.is_file():
        raise ValueError(f"Fresh creation needs local world data: {template}. Export a world-only template from a provisioned installation; see docs/setup.md.")
    if existed:
        status(path)  # Includes future-version and integrity guards even on explicit reset.
    handle, name = tempfile.mkstemp(prefix=".reko-update-", suffix=".db", dir=path.parent)
    os.close(handle)
    stage = Path(name)
    backup = None
    try:
        if existed and not reset:
            with connect(path, True) as source, closing(sqlite3.connect(stage)) as target:
                source.backup(target)
        subprocess.run([str(server), "--prepare-db", str(stage)], check=True, timeout=30)
        inputs = {}
        with connect(stage) as db:
            db.execute("PRAGMA foreign_keys=ON")
            with db:
                if not existed or reset:
                    inputs = load_template(template, db)
                migrate_content(db, historical, template)
                issues = readiness(db)
                if issues:
                    raise ValueError("World data incomplete: " + "; ".join(issues))
                verify(db)
            db.execute("PRAGMA wal_checkpoint(TRUNCATE)")
        missing_inputs = [name for name in INPUTS if not (ROOT / "logs" / name).exists()]
        if missing_inputs and template.is_file():
            available = template_data(template)["inputs"]
            inputs.update({name: available[name] for name in missing_inputs if name in available})
        if INPUTS[0] in missing_inputs and INPUTS[0] not in inputs:
            raise ValueError("Combat coefficients are missing; provide the world template or reference CSV before startup.")
        # Closing/checkpointing the source prevents stale WAL replay after replacement.
        if existed:
            stamp = datetime.now().strftime("%Y%m%d-%H%M%S-%f")
            backup = path.parent / "logs" / f"before-database-update-{stamp}.db"
            backup.parent.mkdir(parents=True, exist_ok=True)
            with connect(path) as source, closing(sqlite3.connect(backup)) as target:
                source.backup(target)
                if source.execute("PRAGMA wal_checkpoint(TRUNCATE)").fetchone()[0]:
                    raise ValueError("Database WAL is busy; another reader/writer must close before maintenance.")
        for relative, content in inputs.items():
            dest = ROOT / "logs" / relative
            if dest.exists() and dest.read_text(encoding="utf-8-sig") != content:
                raise ValueError(f"Existing {dest.name} differs from template; review it before creating this world.")
        for relative, content in inputs.items():
            dest = ROOT / "logs" / relative
            dest.parent.mkdir(parents=True, exist_ok=True)
            if not dest.exists():
                dest.write_text(content, encoding="utf-8")
        os.replace(stage, path)
        return backup
    finally:
        for candidate in (stage, Path(str(stage) + "-wal"), Path(str(stage) + "-shm")):
            candidate.unlink(missing_ok=True)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("command", choices=("check", "update", "export-template", "reset"))
    parser.add_argument("--db", type=Path, default=ROOT / "reko.db")
    parser.add_argument("--template", type=Path, default=ROOT / "local-data/world-template.json")
    parser.add_argument("--server", type=Path, default=ROOT / "target/debug/reko-serverd.exe")
    parser.add_argument("--historical", type=Path, default=ROOT.parent / "KnightOnline/assets/Server/MAP")
    parser.add_argument("--confirm-reset", help="For reset only: exact database filename; all players will be removed.")
    args = parser.parse_args()
    if args.command == "check":
        issues = status(args.db)
        print("\n".join(issues) if issues else "Database schema/content are current.")
        return 2 if issues else 0
    if args.command == "export-template":
        export_template(args.db, args.template)
        print("World-only template exported; keep it outside Git:", args.template)
        return 0
    ensure_stopped()
    reset = args.command == "reset"
    if reset and args.confirm_reset != args.db.name:
        raise ValueError("Reset requires --confirm-reset with the exact database filename. It removes all accounts/characters/clans.")
    if not reset and args.db.exists() and not status(args.db):
        print("Database schema/content are current; no changes.")
        return 0
    backup = prepare(args.db, args.template, args.server.resolve(), args.historical.resolve(), reset)
    print("Database ready. Previous save backup:", backup or "new save")
    return 0


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except (ValueError, sqlite3.Error, OSError, subprocess.SubprocessError) as error:
        raise SystemExit(str(error)) from error
