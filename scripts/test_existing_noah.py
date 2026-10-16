import importlib.util
from pathlib import Path
import sqlite3
import unittest

spec = importlib.util.spec_from_file_location('noah',Path(__file__).with_name('Grant-ExistingNoah.py'))
noah = importlib.util.module_from_spec(spec)
spec.loader.exec_module(noah)

class ExistingNoahTests(unittest.TestCase):
    def setup_db(self):
        db=sqlite3.connect(':memory:')
        db.executescript("""
        CREATE TABLE lobby_characters(account TEXT,slot INTEGER,name TEXT,level INTEGER);
        CREATE TABLE character_wallets(account TEXT,slot INTEGER,coins INTEGER);
        CREATE TABLE character_equipment(account TEXT,item INTEGER);
        CREATE TABLE starting_rules(gold INTEGER);
        INSERT INTO lobby_characters VALUES ('a',0,'Existing1',80),('b',0,'Existing2',1);
        INSERT INTO character_wallets VALUES ('a',0,123),('b',0,20000);
        INSERT INTO character_equipment VALUES ('a',110110001);
        INSERT INTO starting_rules VALUES (20000);
        """)
        return db

    def test_existing_wallets_only_and_later_character_keeps_starting_gold(self):
        db=self.setup_db()
        before=noah.table_hashes(db)
        report=noah.grant(db)
        self.assertEqual(len(report),2)
        self.assertEqual(db.execute('SELECT coins FROM character_wallets ORDER BY account').fetchall(),[(200000123,),(200020000,)])
        self.assertEqual(noah.table_hashes(db),before)
        db.execute("INSERT INTO lobby_characters VALUES ('later',0,'New',1)")
        db.execute("INSERT INTO character_wallets VALUES ('later',0,(SELECT gold FROM starting_rules))")
        self.assertEqual(db.execute("SELECT coins FROM character_wallets WHERE account='later'").fetchone(),(20000,))

    def test_cap_or_missing_wallet_rejects_entire_grant(self):
        for mode in ['cap','missing']:
            db=self.setup_db()
            if mode=='cap':db.execute("UPDATE character_wallets SET coins=2000000000 WHERE account='b'")
            else:db.execute("DELETE FROM character_wallets WHERE account='b'")
            before=db.execute('SELECT * FROM character_wallets ORDER BY account').fetchall()
            with self.assertRaises(ValueError):noah.grant(db)
            self.assertEqual(db.execute('SELECT * FROM character_wallets ORDER BY account').fetchall(),before)

if __name__=='__main__': unittest.main()
