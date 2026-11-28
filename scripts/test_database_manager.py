"""Save migration/bootstrap safety without proprietary fixtures or player data."""
from contextlib import closing
import json
from pathlib import Path
import sqlite3
import tempfile
import unittest
from unittest.mock import patch

import database_manager as manager


def schema(path):
    with closing(sqlite3.connect(path)) as db, db:
        for name in manager.TABLES:
            if name == "world_reference_rules":
                db.execute("CREATE TABLE world_reference_rules(table_name TEXT,rule_id INTEGER,data TEXT,PRIMARY KEY(table_name,rule_id))")
            elif name == "world_town_starts":
                db.execute("CREATE TABLE world_town_starts(zone INTEGER PRIMARY KEY)")
            else:
                db.execute(f'CREATE TABLE "{name}"(id INTEGER PRIMARY KEY)')
        db.execute("CREATE TABLE lobby_accounts(name TEXT PRIMARY KEY,password TEXT)")
        db.execute("PRAGMA user_version=1")


def world(path):
    schema(path)
    with manager.connect(path) as db:
        for category in ("LEVEL_UP", "K_OBJECTPOS", "HISTORICAL_WARP", "QUEST_HELPER", "ITEM_UPGRADE", "MONSTER_TEMPLATE"):
            for key in range(80 if category == "LEVEL_UP" else 1):
                db.execute("INSERT INTO world_reference_rules VALUES(?,?,?)", (category, key, "{}"))
        for name in ("world_spawns", "world_monster_ai", "world_monster_loot"):
            db.execute(f'INSERT INTO "{name}" VALUES(1)')
        db.execute("INSERT INTO world_town_starts VALUES(21)")
        db.execute("INSERT INTO world_town_starts VALUES(32)")
        for key in manager.DELOS_IDS:
            nation = 1 if key // 10 in (301, 303) else 2 if key // 10 in (302, 304) else 0
            row = dict(id=key, nation=nation, zone=32 if key == 3055 else 21)
            db.execute("INSERT INTO world_reference_rules VALUES('HISTORICAL_WARP',?,?)", (3000000 + key, json.dumps(row)))
            db.execute("INSERT INTO world_reference_rules VALUES('K_OBJECTPOS',?,?)", (3000000 + key, json.dumps(dict(Type=5, ControlNpcID=key // 10, Belong=nation))))


class DatabaseTests(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        self.root = Path(self.tmp.name)
        self.save = self.root / "reko.db"
        world(self.save)
        (self.root / "logs").mkdir()
        (self.root / "logs/reference-coefficients.csv").write_text("class,hp\n101,1\n", encoding="utf-8")
        self.root_patch = patch.object(manager, "ROOT", self.root)
        migration_patch = patch.object(manager, 'MIGRATIONS', ((manager.REVISION, manager.delos),))
        migration_patch.start()
        self.addCleanup(migration_patch.stop)
        self.root_patch.start()
        self.addCleanup(self.root_patch.stop)
        self.template = self.root / "local-data/world-template.json"
        self.historical = self.root / "missing-maps"

    def test_update_preserves_players_and_is_idempotent(self):
        with manager.connect(self.save) as db:
            db.execute("INSERT INTO lobby_accounts VALUES('private','secret')")
        backup = manager.prepare(self.save, self.template, Path("server"), self.historical)
        with manager.connect(self.save, True) as db:
            self.assertEqual(db.execute("SELECT * FROM lobby_accounts").fetchall(), [("private", "secret")])
            self.assertEqual(db.execute("SELECT count(*) FROM reko_content_migrations").fetchone()[0], 1)
            manager.migrate_content(db, self.historical)
        self.assertTrue(backup.is_file())
        self.assertEqual(manager.status(self.save), [])

    def test_missing_world_rolls_back_without_replacing_save(self):
        with manager.connect(self.save) as db:
            db.execute("DELETE FROM world_reference_rules WHERE table_name='HISTORICAL_WARP' AND rule_id=3003014")
        before = self.save.read_bytes()
        with self.assertRaisesRegex(ValueError, "Delos migration needs"):
            manager.prepare(self.save, self.template, Path("server"), self.historical)
        self.assertEqual(self.save.read_bytes(), before)
        self.assertFalse(list(self.root.glob(".reko-update-*")))

    def test_existing_save_can_take_only_missing_routes_from_shared_template(self):
        manager.export_template(self.save, self.template)
        coefficients = self.root / "logs/reference-coefficients.csv"
        coefficients.unlink()
        with manager.connect(self.save) as db:
            db.execute("INSERT INTO lobby_accounts VALUES('private','secret')")
            db.execute("DELETE FROM world_reference_rules WHERE table_name='HISTORICAL_WARP' AND rule_id=3003014")
        manager.prepare(self.save, self.template, Path("server"), self.historical)
        self.assertEqual(coefficients.read_text(encoding="utf-8"), "class,hp\n101,1\n")
        with manager.connect(self.save, True) as db:
            self.assertEqual(db.execute("SELECT password FROM lobby_accounts").fetchone()[0], "secret")
            self.assertEqual(db.execute("SELECT count(*) FROM world_reference_rules WHERE table_name='HISTORICAL_WARP' AND rule_id=3003014").fetchone()[0], 1)

    def test_template_creation_and_explicit_reset_have_no_player_records(self):
        with manager.connect(self.save) as db:
            db.execute("INSERT INTO lobby_accounts VALUES('private','secret')")
        manager.export_template(self.save, self.template)
        self.assertNotIn("secret", self.template.read_text())
        self.assertNotIn("lobby_accounts", self.template.read_text())
        target = self.root / "friend.db"
        manager.prepare(target, self.template, Path("server"), self.historical)
        with manager.connect(target, True) as db:
            self.assertEqual(db.execute("SELECT count(*) FROM lobby_accounts").fetchone()[0], 0)
        backup = manager.prepare(self.save, self.template, Path("server"), self.historical, reset=True)
        with manager.connect(backup, True) as db:
            self.assertEqual(db.execute("SELECT password FROM lobby_accounts").fetchone()[0], "secret")
        with manager.connect(self.save, True) as db:
            self.assertEqual(db.execute("SELECT count(*) FROM lobby_accounts").fetchone()[0], 0)

    def test_future_schema_and_tampered_template_are_rejected(self):
        with manager.connect(self.save) as db:
            db.execute("PRAGMA user_version=2")
        with self.assertRaisesRegex(ValueError, "newer"):
            manager.prepare(self.save, self.template, Path("server"), self.historical)
        with manager.connect(self.save) as db:
            db.execute("PRAGMA user_version=1")
        manager.export_template(self.save, self.template)
        bundle = json.loads(self.template.read_text())
        bundle["payload"]["tables"]["account_passwords"] = {}
        bundle["sha256"] = manager.digest(bundle["payload"])
        self.template.write_text(json.dumps(bundle))
        target = self.root / "friend.db"
        with self.assertRaisesRegex(ValueError, "unexpected tables"):
            manager.prepare(target, self.template, Path("server"), self.historical)
        self.assertFalse(target.exists())

    def test_later_migration_failure_never_publishes_earlier_staged_changes(self):
        before = self.save.read_bytes()
        def fail(db, historical, template):
            db.execute("DELETE FROM lobby_accounts")
            raise ValueError("missing later input")
        with patch.object(manager, 'MIGRATIONS', ((manager.REVISION, manager.delos), ('002-failure', fail))):
            with self.assertRaisesRegex(ValueError, "missing later input"):
                manager.prepare(self.save, self.template, Path("server"), self.historical)
        self.assertEqual(self.save.read_bytes(), before)

    def test_unknown_content_revision_is_rejected_before_updates(self):
        with manager.connect(self.save) as db:
            db.execute("CREATE TABLE reko_content_migrations(revision TEXT PRIMARY KEY,applied_at TEXT)")
            db.execute("INSERT INTO reko_content_migrations VALUES('999-future','test')")
        with self.assertRaisesRegex(ValueError, "Unknown content migrations"):
            manager.status(self.save)

    def test_potion_defaults_are_absolute_idempotent_and_preserve_personal_data(self):
        with manager.connect(self.save) as db:
            db.execute("INSERT INTO lobby_accounts VALUES('private','secret')")
            manager.potion_balance(db,self.historical,None)
            first=db.execute("SELECT rule_id,data FROM world_reference_rules WHERE table_name='ITEM_ECONOMY' ORDER BY rule_id").fetchall()
            manager.potion_balance(db,self.historical,None)
            self.assertEqual(first,db.execute("SELECT rule_id,data FROM world_reference_rules WHERE table_name='ITEM_ECONOMY' ORDER BY rule_id").fetchall())
            self.assertEqual(len(first),11)
            by_id={key:json.loads(value) for key,value in first}
            self.assertEqual(by_id[389020000],dict(price=11250,weight=15))
            self.assertEqual(by_id[800015000],dict(price=1000,weight=1))
            self.assertEqual(db.execute("SELECT password FROM lobby_accounts").fetchone()[0],'secret')
        with manager.connect(self.save) as db:
            db.execute("UPDATE world_reference_rules SET data='{\"price\":1,\"weight\":1}' WHERE table_name='ITEM_ECONOMY' AND rule_id=389020000")
        with self.assertRaisesRegex(ValueError,'different local economy'):
            with manager.connect(self.save) as db:
                manager.potion_balance(db,self.historical,None)

    def test_skill_type1_rules_are_idempotent_and_refuse_conflicts(self):
        rows = [dict(iNum=100000 + key, AddDamage=key, Name="synthetic") for key in range(200)]
        (self.root / "logs/reference1453").mkdir(parents=True)
        (self.root / "logs/reference1453/MAGIC_TYPE1.json").write_text(json.dumps(rows), encoding="utf-8")
        with manager.connect(self.save) as db:
            db.execute("INSERT INTO lobby_accounts VALUES('private','secret')")
            manager.skill_type1_damage(db, self.historical, None)
            first = db.execute("SELECT rule_id,data FROM world_reference_rules WHERE table_name='MAGIC_TYPE1' ORDER BY rule_id").fetchall()
            manager.skill_type1_damage(db, self.historical, None)
            self.assertEqual(first, db.execute("SELECT rule_id,data FROM world_reference_rules WHERE table_name='MAGIC_TYPE1' ORDER BY rule_id").fetchall())
            self.assertEqual(len(first), 200)
            self.assertEqual(json.loads(first[42][1])["AddDamage"], 42)
            self.assertEqual(db.execute("SELECT password FROM lobby_accounts").fetchone()[0], 'secret')
            db.execute("UPDATE world_reference_rules SET data='{\"AddDamage\":9}' WHERE table_name='MAGIC_TYPE1' AND rule_id=100001")
        with self.assertRaisesRegex(ValueError, 'Skill Type1 customization'):
            with manager.connect(self.save) as db:
                manager.skill_type1_damage(db, self.historical, None)
        (self.root / "logs/reference1453/MAGIC_TYPE1.json").unlink()
        with self.assertRaisesRegex(ValueError, 'original inputs or an updated world template'):
            with manager.connect(self.save) as db:
                db.execute("DELETE FROM world_reference_rules WHERE table_name='MAGIC_TYPE1'")
                manager.skill_type1_damage(db, self.historical, None)

    def test_missing_optional_accessory_recipes_do_not_block_later_migrations(self):
        with manager.connect(self.save) as db:
            manager.accessory_compound(db, self.historical, None)
            self.assertEqual(db.execute("SELECT count(*) FROM world_reference_rules WHERE table_name='ACCESSORY_UPGRADE'").fetchone()[0], 0)

    def test_monster_magic_and_accessory_rules_are_idempotent_and_refuse_conflicts(self):
        for folder, name, rows, migrate, category, field in (
            ("reference1453", "MONSTER_MAGIC.json", [dict(sSid=100 + key, byMagicAttack=key % 6) for key in range(700)],
             manager.monster_magic, "MONSTER_MAGIC", "byMagicAttack"),
            ("reference2086", "ACCESSORY_UPGRADE.json",
             [dict(nIndex=300000 + key, strNote="(+0 -> +1)", nOriginType=18, nOriginItem=10000 + key,
                   nReqItem1=379159000, nReqNoah=500000, nGiveItem=100, nNPCNum=5001) for key in range(300)],
             manager.accessory_compound, "ACCESSORY_UPGRADE", "nReqNoah"),
        ):
            (self.root / "logs" / folder).mkdir(parents=True, exist_ok=True)
            (self.root / "logs" / folder / name).write_text(json.dumps(rows), encoding="utf-8")
            with manager.connect(self.save) as db:
                migrate(db, self.historical, None)
                first = db.execute("SELECT rule_id,data FROM world_reference_rules WHERE table_name=? ORDER BY rule_id", (category,)).fetchall()
                migrate(db, self.historical, None)
                self.assertEqual(first, db.execute("SELECT rule_id,data FROM world_reference_rules WHERE table_name=? ORDER BY rule_id", (category,)).fetchall())
                self.assertEqual(len(first), len(rows))
                db.execute("UPDATE world_reference_rules SET data=? WHERE table_name=? AND rule_id=?", (json.dumps({field: 1}), category, first[0][0]))
            with self.assertRaisesRegex(ValueError, 'conflicts'):
                with manager.connect(self.save) as db:
                    migrate(db, self.historical, None)

    def test_world_events_import_waves_keep_owner_schedule_edits_and_use_template_fallback(self):
        rows = [dict(sIndex=key, bLevel=2 if key < 56 else 3, bStage=key % 56 + 1, bStageLevel=1, sTime=310,
                     sSid=1000 + key, sCount=5, sPosX=126, sPosZ=126, bRange=45) for key in range(224)]
        (self.root / "logs/reference1453").mkdir(parents=True)
        (self.root / "logs/reference1453/MONSTER_CHALLENGE_SUMMON_LIST.json").write_text(json.dumps(rows), encoding="utf-8")
        with manager.connect(self.save) as db:
            db.execute("INSERT INTO lobby_accounts VALUES('private','secret')")
            manager.world_events(db, self.historical, None)
            db.execute("UPDATE world_reference_rules SET data=? WHERE table_name='EVENT_SCHEDULE' AND rule_id=1",
                       (json.dumps(dict(event="temple", time="21:00")),))
            manager.world_events(db, self.historical, None)
            self.assertEqual(db.execute("SELECT count(*) FROM world_reference_rules WHERE table_name='MONSTER_CHALLENGE_SUMMON_LIST'").fetchone()[0], 224)
            self.assertEqual(json.loads(db.execute("SELECT data FROM world_reference_rules WHERE table_name='EVENT_SCHEDULE' AND rule_id=1").fetchone()[0])["time"], "21:00")
            schedule = {key: json.loads(data) for key, data in db.execute("SELECT rule_id,data FROM world_reference_rules WHERE table_name='EVENT_SCHEDULE'")}
            self.assertEqual(schedule[2]["level_min"], 45)
            self.assertEqual(schedule[3]["zones"], [61, 62, 64])
            self.assertEqual(db.execute("SELECT password FROM lobby_accounts").fetchone()[0], 'secret')
        manager.export_template(self.save, self.template)
        with manager.connect(self.save) as db:
            db.execute("UPDATE world_reference_rules SET data=? WHERE table_name='MONSTER_CHALLENGE_SUMMON_LIST' AND rule_id=3",
                       (json.dumps(dict(sSid=1)),))
        with self.assertRaisesRegex(ValueError, 'wave customization'):
            with manager.connect(self.save) as db:
                manager.world_events(db, self.historical, None)
        (self.root / "logs/reference1453/MONSTER_CHALLENGE_SUMMON_LIST.json").unlink()
        target = self.root / "friend.db"
        world(target)
        with manager.connect(target) as db:
            manager.world_events(db, self.historical, self.template)
            self.assertEqual(db.execute("SELECT count(*) FROM world_reference_rules WHERE table_name='MONSTER_CHALLENGE_SUMMON_LIST'").fetchone()[0], 224)
        partial = self.root / "partial.db"
        world(partial)
        (self.root / "logs/reference1453/MONSTER_CHALLENGE_SUMMON_LIST.json").write_text(json.dumps(rows[:56]), encoding="utf-8")
        with self.assertRaisesRegex(ValueError, 'incomplete'):
            with manager.connect(partial) as db:
                manager.world_events(db, self.historical, None)

    def run(self, result=None):
        def initialize(arguments, **kwargs):
            path = Path(arguments[-1])
            with closing(sqlite3.connect(path)) as db:
                exists = db.execute("SELECT count(*) FROM sqlite_master").fetchone()[0]
            if not exists:
                schema(path)
        with patch.object(manager.subprocess, "run", side_effect=initialize):
            return super().run(result)


if __name__ == "__main__":
    unittest.main()
