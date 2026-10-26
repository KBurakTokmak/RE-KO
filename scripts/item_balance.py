"""Owner-defined shop recovery-potion prices/weights for database migrations."""
import json
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]


def definitions():
    rules = json.loads((ROOT / 'config/item-balance.json').read_text(encoding='utf-8'))['items']
    ids = set()
    for rule in rules:
        if rule['id'] in ids or rule['id'] % 1000 or not rule['name']:
            raise ValueError('Duplicate/invalid item-balance ID.')
        ids.add(rule['id'])
        for name, maximum in (('price', 2147483647), ('original_price', 2147483647),
                              ('weight', 32767), ('original_weight', 32767)):
            if type(rule[name]) is not int or not 0 <= rule[name] <= maximum:
                raise ValueError('Invalid price/weight value.')
    if len(ids) != 11:
        raise ValueError('Expected ten shop recovery potions and Speed+ Potion.')
    return rules
