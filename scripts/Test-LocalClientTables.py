"""Verify the isolated English-name repair without changing any game table."""
from pathlib import Path
from local_client_tables import cipher,rows,english_transformations

root=Path(__file__).resolve().parents[1]
original=(root.parent / 'KnightOnline1453/Data/Skill_Magic_6.tbl').read_bytes()
patched=english_transformations(original)
assert english_transformations(patched)==patched
header,before=rows(cipher(original))
after_header,after=rows(cipher(patched))
assert header==after_header and len(before)==len(after)
changed=0
for old,new in zip(before,after):
    for index,(a,b) in enumerate(zip(old,new)):
        if a!=b:
            assert index==2 and b==old[1]
            changed+=1
assert changed==46
try:
    english_transformations(original[:-1])
    raise AssertionError('Truncated table accepted.')
except (ValueError,KeyError):
    pass
assert (root.parent / 'KnightOnline1453/Data/Skill_Magic_6.tbl').read_bytes()==original
print('English transformation tables passed:46 display names, every other raw cell/header unchanged, idempotent cipher round-trip, truncated input rejected, original file unchanged.')
