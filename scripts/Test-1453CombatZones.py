"""Exercise the offline repair on a disposable save, never on live characters."""
import importlib.util
import sqlite3
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
spec = importlib.util.spec_from_file_location('repair', ROOT / 'scripts/Repair-1453CombatZones.py')
repair = importlib.util.module_from_spec(spec)
spec.loader.exec_module(repair)


def main():
    target = ROOT / 'logs/combat-zones-test.db'
    with sqlite3.connect(f'file:{(ROOT / "reko.db").as_posix()}?mode=ro', uri=True) as source:
        with sqlite3.connect(target) as fixture:
            source.backup(fixture)
    with sqlite3.connect(target) as db:
        before = repair.travel.snapshot(db)
        other_spawns = db.execute('SELECT * FROM world_spawns WHERE zone!=72 ORDER BY id').fetchall()
        changes = repair.plan(db, ROOT / 'logs/reference-rules', ROOT.parent / 'KnightOnline1453')
        with db:
            repair.apply(db, *changes)
        after = repair.travel.snapshot(db)
        allowed = {'world_spawns', 'world_spawn_conditions', 'world_boss_pools', 'world_boss_members', 'world_reference_rules'}
        assert all(before[t] == after[t] for t in before if t not in allowed)
        assert other_spawns == db.execute('SELECT * FROM world_spawns WHERE zone!=72 ORDER BY id').fetchall()
        assert db.execute('SELECT count(*) FROM world_spawns WHERE zone=72 AND monster=1').fetchone()[0] == 379
        assert db.execute('SELECT count(*) FROM world_spawns WHERE zone=72 AND monster=0').fetchone()[0] == 63
        assert db.execute('SELECT count(*) FROM world_boss_pools WHERE zone=71').fetchone()[0] == 14
        assert db.execute("SELECT count(*) FROM world_boss_members WHERE pool_id LIKE '71:%'").fetchone()[0] == 63
        assert db.execute("SELECT count(*) FROM world_boss_members b JOIN world_spawns s ON s.id=b.entity_id WHERE s.zone=71 AND s.name IN('Riote','Atross','undying','Blood Don','Orc bandit officer')").fetchone()[0] == 0
        with db:
            repair.apply(db, *changes)
        assert after == repair.travel.snapshot(db)
        for mutate in [
            "UPDATE world_spawns SET hp=hp+1 WHERE id=21000",
            "UPDATE world_spawn_conditions SET enabled=0 WHERE entity_id=21000",
            "UPDATE world_boss_pools SET respawn_ms=1000 WHERE pool_id='71:lobo'",
            "UPDATE world_reference_rules SET data='{\"mode\":\"event\"}' WHERE table_name='BIFROST_ACCESS'",
        ]:
            try:
                with db:
                    db.execute(mutate)
                    repair.apply(db, *changes)
                    raise AssertionError('Conflicting customization was not refused.')
            except ValueError:
                pass
            assert repair.travel.snapshot(db) == after
        assert db.execute('PRAGMA integrity_check').fetchone()[0] == 'ok'
    print('Combat-zone repair passed: original templates, bounded population, one boss pool per name, ordinary mobs preserved, all unrelated tables preserved, idempotence and four conflict rollbacks.')
    print('Disposable Rust integration fixture:', target)


if __name__ == '__main__':
    main()
