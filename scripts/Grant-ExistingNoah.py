"""One-time offline grant to existing characters; starting gold is never changed."""
import argparse
from datetime import datetime
import hashlib
import json
from pathlib import Path
import sqlite3
import subprocess

ROOT = Path(__file__).resolve().parents[1]
AMOUNT = 200_000_000
CAP = 2_100_000_000

def table_hashes(db):
    tables = [r[0] for r in db.execute("SELECT name FROM sqlite_master WHERE type='table' AND name!='sqlite_sequence'")]
    return {t: hashlib.sha256(repr(db.execute('SELECT * FROM "' + t.replace('"', '""') + '" ORDER BY rowid').fetchall()).encode()).hexdigest() for t in tables if t != 'character_wallets'}

def grant(db):
    before = table_hashes(db)
    characters = db.execute("SELECT c.account,c.slot,c.name,w.coins FROM lobby_characters c LEFT JOIN character_wallets w ON w.account=c.account AND w.slot=c.slot ORDER BY c.account,c.slot").fetchall()
    if any(coins is None for _, _, _, coins in characters):
        raise ValueError('An existing character has no initialized wallet; no grant was applied.')
    if any(coins + AMOUNT > CAP for _, _, _, coins in characters):
        raise ValueError('The grant would exceed the native coin cap; no grant was applied.')
    with db:
        for account, slot, _, coins in characters:
            changed = db.execute('UPDATE character_wallets SET coins=? WHERE account=? AND slot=? AND coins=?', (coins + AMOUNT, account, slot, coins)).rowcount
            if changed != 1:
                raise RuntimeError('Wallet changed during grant; rolling back.')
        if table_hashes(db) != before:
            raise RuntimeError('Non-wallet data changed; rolling back.')
        expected = [(a, s, n, c + AMOUNT) for a, s, n, c in characters]
        actual = db.execute("SELECT c.account,c.slot,c.name,w.coins FROM lobby_characters c JOIN character_wallets w ON w.account=c.account AND w.slot=c.slot ORDER BY c.account,c.slot").fetchall()
        if actual != expected:
            raise RuntimeError('Grant verification failed; rolling back.')
    return [dict(account=a, slot=s, character=n, before=c, after=c+AMOUNT) for a,s,n,c in characters]

def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--apply', action='store_true')
    args = parser.parse_args()
    path = ROOT / 'reko.db'
    manifest = ROOT / 'logs/existing-noah-grant-200m.json'
    if args.apply:
        if manifest.exists():
            raise SystemExit('This one-time grant already has a manifest; refusing to apply twice.')
        running = subprocess.check_output(['powershell.exe','-NoProfile','-Command','@(Get-Process reko-serverd -ErrorAction SilentlyContinue).Count'],text=True)
        if int(running.strip()):
            raise SystemExit('Stop RE-KO before applying; live sessions cache wallets.')
    with sqlite3.connect(f'file:{path}?mode={"rw" if args.apply else "ro"}',uri=True) as db:
        rows = db.execute('SELECT c.name,w.coins FROM lobby_characters c JOIN character_wallets w ON w.account=c.account AND w.slot=c.slot ORDER BY c.account,c.slot').fetchall()
        print(f'Existing characters: {len(rows)}; add {AMOUNT:,} Noah each.')
        if not args.apply:
            for name, coins in rows: print(f'{name}: {coins:,} -> {coins+AMOUNT:,}')
            print('Dry run. New-character starting gold remains unchanged.'); return
        backup = ROOT / ('logs/before-existing-noah-grant-' + datetime.now().strftime('%Y%m%d-%H%M%S') + '.db')
        with sqlite3.connect(backup) as out: db.backup(out)
        report = grant(db)
        manifest.write_text(json.dumps(dict(amount=AMOUNT, backup=str(backup), characters=report),indent=2),encoding='utf-8')
        print('Grant committed; only existing wallet balances changed. Backup:',backup)
        print('Manifest:',manifest)

if __name__ == '__main__':
    main()
