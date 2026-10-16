"""Export imported upgrade recipes without changing their rates or behavior."""
import csv
import json
from pathlib import Path
import sqlite3

ROOT=Path(__file__).resolve().parents[1]
db=sqlite3.connect(f'file:{(ROOT / "reko.db").as_posix()}?mode=ro',uri=True)
output=ROOT/'docs/re/upgrade-rates.csv'
columns=['recipe','name','upgrade','scroll','other_requirements','origin_suffix','item_type','rate_class','table_rate_percent','effective_percent','with_booster_percent','coins']

def chance(rate):
    return sum(min(rate+1,upper+1)/(upper+1) for upper in range(9000,10001))/1001*100

with output.open('w',encoding='utf-8-sig',newline='') as f:
    writer=csv.DictWriter(f,fieldnames=columns)
    writer.writeheader()
    for text, in db.execute("SELECT data FROM world_reference_rules WHERE table_name='ITEM_UPGRADE' ORDER BY rule_id"):
        v=json.loads(text)
        rate=int(v['nGenRate'])
        writer.writerow(dict(recipe=v['nIndex'],name=v['strName'].strip(),upgrade=v['strNote'].strip(),scroll=v['nReqItem1'],other_requirements=';'.join(v['nReqItem'+str(n)] for n in range(2,9) if int(v['nReqItem'+str(n)])),origin_suffix=v['nOriginItem'],item_type=v['nOriginType'],rate_class=v['bRateType'],table_rate_percent=rate/100,effective_percent=f'{chance(rate):.4f}',with_booster_percent=f'{chance(min(10000,rate+1000)):.4f}',coins=v['nReqNoah']))
print(output)
