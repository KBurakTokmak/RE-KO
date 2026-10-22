"""English transformation display names in the isolated copy; original tables stay intact."""
import hashlib
import json
from pathlib import Path
import struct

ROOT=Path(__file__).resolve().parents[1]
LOCAL=ROOT / '.local/client-1453'


def cipher(data, encrypt=False):
    key=0x0816
    result=bytearray()
    for value in data:
        encoded=value ^ (key >> 8)
        result.append(encoded)
        key=((key + (encoded if encrypt else value))*0x6081+0x1608)&0xffff
    return bytes(result)


def rows(data):
    count=struct.unpack_from('<I',data)[0]
    if count>100: raise ValueError('Invalid table width.')
    types=struct.unpack_from('<'+'I'*count,data,4)
    cursor=4+4*count
    size=struct.unpack_from('<I',data,cursor)[0];cursor+=4
    if size>100000:raise ValueError('Invalid table length.')
    header=data[:cursor]
    parsed=[]
    for _ in range(size):
        row=[]
        for kind in types:
            start=cursor
            if kind==7:
                length=struct.unpack_from('<I',data,cursor)[0];cursor+=4+length
            else:
                cursor+={1:1,2:1,3:2,4:2,5:4,6:4,8:4,9:8,10:8,11:8}[kind]
            if cursor>len(data):raise ValueError('Truncated table.')
            row.append(data[start:cursor])
        parsed.append(row)
    if cursor!=len(data):raise ValueError('Unexpected table trailer.')
    return header,parsed


def english_transformations(encrypted):
    plain=cipher(encrypted)
    header,original=rows(plain)
    changed=0
    result=[]
    for before in original:
        row=list(before)
        if struct.unpack('<I',row[0])[0]//1000==472:
            name=row[1][4:]
            if not name or any(b<32 or b>126 for b in name):raise ValueError('Original English form name is invalid.')
            row[2]=row[1]
            changed+=1
        result.append(row)
    if changed!=46:raise ValueError('Expected46 original scroll forms.')
    patched=header+b''.join(cell for row in result for cell in row)
    parsed_header,verified=rows(patched)
    assert parsed_header==header
    for before,after in zip(original,verified):
        for column,(old,new) in enumerate(zip(before,after)):
            if old!=new:
                assert column==2 and struct.unpack('<I',before[0])[0]//1000==472
                assert new==before[1]
    output=cipher(patched,encrypt=True)
    assert cipher(output)==patched
    return output


def prepare():
    destination=LOCAL / 'Data/Skill_Magic_6.tbl'
    if not (LOCAL / '.reko-local-client.json').is_file():raise ValueError('Prepare the isolated client first.')
    source=ROOT.parent / 'KnightOnline1453/Data/Skill_Magic_6.tbl'
    original=source.read_bytes()
    expected=english_transformations(original)
    current=destination.read_bytes()
    if current not in (original,expected):raise ValueError('Local transformation table was customized; refusing overwrite.')
    if current!=expected:
        backup=ROOT / 'logs/client-transformation-names-original.tbl'
        if backup.exists() and backup.read_bytes()!=original:raise ValueError('Unexpected existing table backup.')
        backup.write_bytes(original)
        temporary=destination.with_suffix('.reko-tmp')
        if temporary.exists():raise ValueError('Pending local table update exists; review it first.')
        temporary.write_bytes(expected)
        temporary.replace(destination)
    assert source.read_bytes()==original
    assert destination.read_bytes()==expected
    receipt={'source_sha256':hashlib.sha256(original).hexdigest(),'local_sha256':hashlib.sha256(expected).hexdigest(),'english_forms':46,'changed_column':2}
    (ROOT / 'logs/client-transformation-names.json').write_text(json.dumps(receipt,indent=2))
    return receipt


if __name__=='__main__':
    print(json.dumps(prepare()))
