"""Import primary1453 EVENT actions and historical terrain trigger tiles offline."""
import argparse
from datetime import datetime
import importlib.util
import json
from pathlib import Path
import sqlite3
import subprocess

ROOT = Path(__file__).resolve().parents[1]
spec=importlib.util.spec_from_file_location('travel',ROOT/'scripts/Import-1453TravelQuests.py')
travel=importlib.util.module_from_spec(spec)
spec.loader.exec_module(travel)


def collect():
    query="""$ErrorActionPreference='Stop'
$connection=New-Object System.Data.SqlClient.SqlConnection 'Server=.\\TEW_SQLEXPRESS;Database=pv1453_test;Integrated Security=True'
$connection.Open()
try {
 $command=$connection.CreateCommand()
 $command.CommandText='SELECT ZoneNum,EventNum,Type,Cond1,Cond2,Cond3,Cond4,Cond5,Exec1,Exec2,Exec3,Exec4,Exec5 FROM dbo.[EVENT] ORDER BY ZoneNum,EventNum'
 $table=New-Object System.Data.DataTable
 $table.Load($command.ExecuteReader())
 $records=@(foreach($row in $table.Rows) { $record=@{}; foreach($col in $table.Columns) {$record[$col.ColumnName]=([string]$row[$col.ColumnName]).Trim()}; $record })
 ConvertTo-Json -InputObject $records -Compress
} finally {$connection.Close()}
"""
    events=json.loads(subprocess.check_output(['powershell','-NoProfile','-Command',query],text=True,encoding='utf-8'))
    maps={1:'karus_0730.smd',2:'elmo_0730.smd',11:'k_eslant.smd',12:'e_eslant.smd',21:'moradon_0826.smd'}
    records=[]
    for zone,name in maps.items():
        path=ROOT.parent/'KnightOnline/assets/Server/MAP'/name
        if zone==12:
            # Open-KO's e_eslant has Karus1131 copied into it; the supplied
            # e_eslant counterpart has the1231 action required by primary1453.
            path=ROOT.parent/'kn_2086_patch_private_server/lCAFILES/Map'/name
        _,tiles=travel.smd_warps(path,include_tiles=True)
        for x,z,event in tiles:
            row=next((r for r in events if int(r['ZoneNum'])==zone and int(r['EventNum'])==event),None)
            if row is not None:
                records.append((zone*1000000+x*1024+z,dict(zone=zone,x=x,z=z,event=event,source=name)))
        print('Map trigger tiles:',zone,len(tiles),'matched:',sum(1 for _,r in records if r['zone']==zone))
    return events,records


def main():
    p=argparse.ArgumentParser(description=__doc__);p.add_argument('--apply',action='store_true');args=p.parse_args()
    events,tiles=collect()
    (ROOT/'logs/map-trigger-audit.json').write_text(json.dumps(dict(events=events,tiles=[r for _,r in tiles]),indent=2))
    if not args.apply:return
    running=subprocess.check_output(['powershell','-NoProfile','-Command',"Get-Process | Where-Object ProcessName -eq 'reko-serverd' | Select-Object -ExpandProperty Id"],text=True)
    if running.strip():raise SystemExit('Stop RE-KO before importing rules.')
    db=sqlite3.connect(ROOT/'reko.db')
    backup=ROOT/'logs'/('before-map-triggers-'+datetime.now().strftime('%Y%m%d-%H%M%S')+'.db')
    with sqlite3.connect(backup) as target:db.backup(target)
    with db:
        for r in events:
            db.execute('INSERT OR REPLACE INTO world_reference_rules VALUES(?,?,?)',('MAP_EVENT',int(r['ZoneNum'])*100000+int(r['EventNum']),json.dumps(r)))
        for key,r in tiles:
            db.execute('INSERT OR REPLACE INTO world_reference_rules VALUES(?,?,?)',('MAP_EVENT_TILE',key,json.dumps(r)))
    print('Backup:',backup)
    print('Integrity:',db.execute('PRAGMA integrity_check').fetchone()[0])


if __name__=='__main__':main()
