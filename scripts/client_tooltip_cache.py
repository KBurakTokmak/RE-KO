"""Reversible UI, command and melee-position fixes for the isolated 1453 client.

Only process memory changes. The original installation and both EXEs stay intact.
"""
import ctypes as c
from ctypes import wintypes as w
import hashlib
import json
from pathlib import Path
import struct

ROOT = Path(__file__).resolve().parents[1]
SITE = 0x4BCA69
ORIGINAL = bytes.fromhex('e862a20400')
CLIENT_HASH = '9513ecdf29e99bfee307fa8dc93ef64bf616d77d475d57fde906107f32169bb0'
COMMAND_SITE = 0x5762F5
COMMAND_ORIGINAL = bytes.fromhex('80382f7511')
MELEE_SITE = 0x479CA0
MELEE_ORIGINAL = bytes.fromhex('e8ab482200')


def build_melee_stub(address, attack=0x69E550, move=0x69E5F0,
                     player_pointer=0x9DD3AC, stopped_flag=0x9DC960):
    """Synchronize changed native positions before R, preserving its three arguments."""
    code, labels, fixups = bytearray(), {}, []

    def emit(value):
        code.extend(bytes.fromhex(value))

    def branch(opcode, label):
        emit(opcode); fixups.append((len(code), label)); code.extend(bytes(4))

    def call(target):
        emit('e8'); code.extend(struct.pack('<i', target - address - len(code) - 4))

    emit('55 8bec 53 56 57 8bf1 a1')
    code.extend(struct.pack('<I', player_pointer))
    emit('85c0'); branch('0f84', 'attack')
    for source, saved in [(0x98, 0x28c), (0xa0, 0x294)]:
        emit('8b90'); code.extend(struct.pack('<I', source))
        emit('3b96'); code.extend(struct.pack('<I', saved))
        branch('0f85', 'sync')
    branch('e9', 'attack')
    labels['sync'] = len(code)
    emit('83b81c03000000'); branch('0f84', 'stop')
    emit('6a01 6a01 8bce'); call(move)
    branch('e9', 'attack')
    labels['stop'] = len(code)
    # Native MsgSend_Move suppresses duplicate stops, even if the final position changed.
    emit('c605'); code.extend(struct.pack('<I', stopped_flag)); emit('00')
    emit('6a00 6a00 8bce'); call(move)
    labels['attack'] = len(code)
    emit('ff7510 ff750c ff7508 8bce'); call(attack)
    emit('5f 5e 5b 5d c20c00')
    for offset, label in fixups:
        struct.pack_into('<i', code, offset, labels[label] - offset - 4)
    return bytes(code)


def build_command_stub(address, slash=0x5762FA, ordinary=0x57630B):
    """Route only /autoloot through ordinary chat; retain every other slash command."""
    code, labels, fixups = bytearray(), {}, []

    def emit(value):
        code.extend(bytes.fromhex(value))

    def branch(opcode, label):
        emit(opcode); fixups.append((len(code), label)); code.extend(bytes(4))

    emit('9c 60 837e0809')  # save flags/registers; native string length
    branch('0f82', 'native')
    emit('81382f617574')    # /aut
    branch('0f85', 'native')
    emit('8178046f6c6f6f')  # oloo
    branch('0f85', 'native')
    emit('80780874')        # t
    branch('0f85', 'native')
    emit('837e0809')
    branch('0f84', 'ordinary')
    emit('80780920')
    branch('0f84', 'ordinary')
    labels['native'] = len(code)
    emit('61 9d 80382f')
    branch('0f85', 'ordinary_restored')
    emit('e9'); code.extend(struct.pack('<i', slash - address - len(code) - 4))
    labels['ordinary'] = len(code)
    emit('61 9d')
    labels['ordinary_restored'] = len(code)
    emit('e9'); code.extend(struct.pack('<i', ordinary - address - len(code) - 4))
    for offset, label in fixups:
        struct.pack_into('<i', code, offset, labels[label] - offset - 4)
    return bytes(code)


def build_stub(address, data, original=0x506CD0, player_pointer=0x9DD3AC, viewport=0x8A4944):
    """x86 thiscall, six stack arguments, preserving all nonvolatile registers."""
    # Data: 0..47 item keys, 48/52 draw/build counts, 56..79 cycle metrics,
    # 80 player pointer, 84..107 class keys, 128..351 extended stat snapshot.
    code = bytearray()
    labels, fixups = {}, []

    def emit(value):
        code.extend(bytes.fromhex(value))

    def branch(opcode, label):
        emit(opcode)
        fixups.append((len(code), label))
        code.extend(bytes(4))

    emit('55 8bec 53 56 57 8bf1 bf')
    code.extend(struct.pack('<I', data))
    emit('ff4730'); emit('3b37')                         # this
    branch('0f85', 'rebuild')
    for index in range(2, 6):
        emit(f'8b45{8 + 4 * index:02x} 3b47{4 + 4 * index:02x}')
        branch('0f85', 'rebuild')
    emit('8b4510 85c0')                  # item
    branch('0f84', 'rebuild')
    for index in range(4):              # base/ext pointers, quantity, durability
        emit(f'8b48{0x38 + index * 4:02x} 3b4f{28 + index * 4:02x}')
        branch('0f85', 'rebuild')
    emit('a1'); code.extend(struct.pack('<I', player_pointer))
    emit('85c0'); branch('0f84', 'rebuild')
    emit('3b4750'); branch('0f85', 'rebuild')
    for index in range(6):
        emit('8b88'); code.extend(struct.pack('<I', 0x4e0 + index * 4))
        emit(f'3b4f{84 + index * 4:02x}'); branch('0f85', 'rebuild')
    emit('56 57 8db048080000 8dbf80000000 b938000000 f3a7 5f 5e')
    branch('0f85', 'rebuild')
    # Inventory hides its tooltip every frame. Restore visibility without
    # invalidating the cached text just because of that normal render cycle.
    emit('80bea800000000')
    branch('0f85', 'reposition')
    emit('8b16 6a01 8bce ff5244')
    labels['reposition'] = len(code)
    # Match native layout anchoring, but move existing children without rebuilding fonts.
    emit('8b8690000000 2b8688000000 8b5d08 83c302 8bcb 03c8 3b0d')
    code.extend(struct.pack('<I', viewport)); branch('0f8d', 'left')
    emit('83c31a'); branch('e9', 'vertical')
    labels['left'] = len(code); emit('2bd8')
    labels['vertical'] = len(code)
    emit('8b8694000000 2b868c000000 8b550c 83c202 8bca 2bc8 85c9')
    branch('0f8f', 'above')
    emit('8bca 03c8 3b0d'); code.extend(struct.pack('<I', viewport + 4))
    branch('0f8c', 'position')
    emit('8b15'); code.extend(struct.pack('<I', viewport + 4)); emit('2bd0')
    branch('e9', 'position')
    labels['above'] = len(code); emit('8bd1')
    labels['position'] = len(code)
    emit('8b06 52 53 8bce ff5024')
    labels['render'] = len(code)
    emit('0f31 894738 89573c 8b16 8bce ff526c')
    emit('0f31 2b4738 1b573c 014748 11574c')
    branch('e9', 'done')
    labels['rebuild'] = len(code)
    emit('8937')
    for index in range(6):
        emit(f'8b45{8 + 4 * index:02x} 8947{4 + 4 * index:02x}')
    emit('8b4510 85c0')
    branch('0f84', 'invoke')
    for index in range(4):
        emit(f'8b48{0x38 + index * 4:02x} 894f{28 + index * 4:02x}')
    emit('a1'); code.extend(struct.pack('<I', player_pointer))
    emit('894750 85c0'); branch('0f84', 'invoke')
    for index in range(6):
        emit('8b88'); code.extend(struct.pack('<I', 0x4e0 + index * 4))
        emit(f'894f{84 + index * 4:02x}')
    emit('56 57 8db048080000 8dbf80000000 b938000000 f3a5 5f 5e')
    labels['invoke'] = len(code)
    emit('c7472c00000000')
    for offset in range(28, 4, -4):
        emit(f'ff75{offset:02x}')
    emit('ff4734 0f31 894738 89573c'); emit('8bce e8')
    code.extend(struct.pack('<i', original - (address + len(code) + 4)))
    emit('0f31 2b4738 1b573c 014740 115744')
    labels['done'] = len(code)
    emit('5f 5e 5b 5d c21800')
    for offset, label in fixups:
        struct.pack_into('<i', code, offset, labels[label] - offset - 4)
    return bytes(code)


def install(pid, remove=False, command=False, melee=False):
    if command and melee:
        raise ValueError('Select one local client hook per installation.')
    site = MELEE_SITE if melee else COMMAND_SITE if command else SITE
    expected_code = MELEE_ORIGINAL if melee else COMMAND_ORIGINAL if command else ORIGINAL
    label = 'Basic melee position synchronization' if melee else 'Autoloot command forwarding' if command else 'Inventory tooltip cache'
    receipt_name = 'melee-position' if melee else 'autoloot-command' if command else 'tooltip-cache'
    version = 1 if melee else 3
    if c.sizeof(c.c_void_p) != 8:
        raise RuntimeError('Use the 64-bit Windows Python runtime.')
    k = c.WinDLL('kernel32', use_last_error=True)
    k.OpenProcess.argtypes = [w.DWORD, w.BOOL, w.DWORD]
    k.OpenProcess.restype = w.HANDLE
    k.CloseHandle.argtypes = [w.HANDLE]
    k.QueryFullProcessImageNameW.argtypes = [w.HANDLE, w.DWORD, w.LPWSTR, c.POINTER(w.DWORD)]
    k.ReadProcessMemory.argtypes = [w.HANDLE, c.c_void_p, c.c_void_p, c.c_size_t, c.POINTER(c.c_size_t)]
    k.WriteProcessMemory.argtypes = k.ReadProcessMemory.argtypes
    k.VirtualAllocEx.argtypes = [w.HANDLE, c.c_void_p, c.c_size_t, w.DWORD, w.DWORD]
    k.VirtualAllocEx.restype = c.c_void_p
    k.VirtualProtectEx.argtypes = [w.HANDLE, c.c_void_p, c.c_size_t, w.DWORD, c.POINTER(w.DWORD)]
    k.FlushInstructionCache.argtypes = [w.HANDLE, c.c_void_p, c.c_size_t]
    k.CreateToolhelp32Snapshot.argtypes = [w.DWORD, w.DWORD]
    k.CreateToolhelp32Snapshot.restype = w.HANDLE
    k.OpenThread.argtypes = [w.DWORD, w.BOOL, w.DWORD]
    k.OpenThread.restype = w.HANDLE
    k.SuspendThread.argtypes = k.ResumeThread.argtypes = [w.HANDLE]
    k.SuspendThread.restype = k.ResumeThread.restype = w.DWORD

    class Thread(c.Structure):
        _fields_ = [(name, w.DWORD) for name in
                    ('size', 'usage', 'id', 'owner', 'priority', 'delta', 'flags')]

    k.Thread32First.argtypes = k.Thread32Next.argtypes = [w.HANDLE, c.POINTER(Thread)]
    process = k.OpenProcess(0x1000 | 0x8 | 0x10 | 0x20, False, pid)
    if not process:
        raise c.WinError(c.get_last_error())
    suspended = []

    def read(address, length):
        buffer, count = c.create_string_buffer(length), c.c_size_t()
        if not k.ReadProcessMemory(process, address, buffer, length, c.byref(count)) or count.value != length:
            raise c.WinError(c.get_last_error())
        return buffer.raw

    def write(address, value):
        count = c.c_size_t()
        if not k.WriteProcessMemory(process, address, value, len(value), c.byref(count)) or count.value != len(value):
            raise c.WinError(c.get_last_error())

    try:
        path, length = c.create_unicode_buffer(32768), w.DWORD(32768)
        if not k.QueryFullProcessImageNameW(process, 0, path, c.byref(length)):
            raise c.WinError(c.get_last_error())
        expected = ROOT / '.local/client-1453/KnightOnline.exe'
        if Path(path.value).resolve() != expected.resolve():
            raise RuntimeError('Tooltip cache is restricted to the isolated local client.')
        if hashlib.sha256(expected.read_bytes()).hexdigest() != CLIENT_HASH:
            raise RuntimeError('Unknown client build.')
        manifest = ROOT / 'logs' / f'{receipt_name}-{pid}.json'
        current = read(site, 5)
        if remove:
            receipt = json.loads(manifest.read_text())
            if current != bytes.fromhex(receipt['replacement']):
                raise RuntimeError('Tooltip hook differs from its receipt.')
            replacement = expected_code
        else:
            if current != expected_code:
                if manifest.exists() and current.hex() == json.loads(manifest.read_text())['replacement']:
                    if json.loads(manifest.read_text()).get('version') == version:
                        return label + ' already installed.'
                    install(pid, remove=True, command=command, melee=melee)
                    current = read(site, 5)
                if current != expected_code:
                    raise RuntimeError('Unknown inventory tooltip call site.')
            allocation = k.VirtualAllocEx(process, None, 8192, 0x3000, 0x04)
            if not allocation or allocation + 8192 > 0xFFFFFFFF:
                raise RuntimeError('Cannot allocate x86 tooltip cache.')
            entry = allocation + 4096
            code = build_melee_stub(entry) if melee else build_command_stub(entry) if command else build_stub(entry, allocation)
            if len(code) > 4096:
                raise RuntimeError('Local client hook exceeds its executable page.')
            write(entry, code)
            protection = w.DWORD()
            if not k.VirtualProtectEx(process, entry, 4096, 0x20, c.byref(protection)):
                raise c.WinError(c.get_last_error())
            if not k.FlushInstructionCache(process, entry, len(code)):
                raise c.WinError(c.get_last_error())
            replacement = (b'\xe9' if command else b'\xe8') + struct.pack('<i', entry - site - 5)
            manifest.write_text(json.dumps({'version': version, 'pid': pid, 'site': site, 'allocation': allocation,
                                           'original': expected_code.hex(), 'replacement': replacement.hex()}, indent=2))
        # Pause target threads only while replacing the five-byte call. A removed
        # hook's tiny allocation remains until process exit for in-flight callers.
        snapshot = k.CreateToolhelp32Snapshot(4, 0)
        if snapshot == c.c_void_p(-1).value:
            raise c.WinError(c.get_last_error())
        try:
            thread = Thread(); thread.size = c.sizeof(thread)
            found = k.Thread32First(snapshot, c.byref(thread))
            while found:
                if thread.owner == pid:
                    handle = k.OpenThread(2, False, thread.id)
                    if not handle:
                        error = c.get_last_error()
                        if error in (87, 1168):  # Thread exited after the snapshot.
                            found = k.Thread32Next(snapshot, c.byref(thread))
                            continue
                        raise c.WinError(error)
                    if k.SuspendThread(handle) == 0xFFFFFFFF:
                        error = c.get_last_error()
                        k.CloseHandle(handle)
                        if error in (6, 87, 1168):
                            found = k.Thread32Next(snapshot, c.byref(thread))
                            continue
                        raise c.WinError(error)
                    suspended.append(handle)
                found = k.Thread32Next(snapshot, c.byref(thread))
            if not suspended:
                raise RuntimeError('No client threads found.')
            old = w.DWORD()
            if not k.VirtualProtectEx(process, site, 5, 0x40, c.byref(old)):
                raise c.WinError(c.get_last_error())
            try:
                write(site, replacement)
                assert read(site, 5) == replacement
                if not k.FlushInstructionCache(process, site, 5):
                    raise c.WinError(c.get_last_error())
            except BaseException:
                write(site, current)
                k.FlushInstructionCache(process, site, 5)
                raise
            finally:
                restored = w.DWORD()
                k.VirtualProtectEx(process, site, 5, old.value, c.byref(restored))
        finally:
            k.CloseHandle(snapshot)
        return label + (' removed.' if remove else ' installed in memory.')
    finally:
        for thread in reversed(suspended):
            k.ResumeThread(thread)
            k.CloseHandle(thread)
        k.CloseHandle(process)


if __name__ == '__main__':
    import argparse
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--pid', required=True, type=int)
    parser.add_argument('--remove', action='store_true')
    parser.add_argument('--command', action='store_true')
    parser.add_argument('--melee-sync', action='store_true')
    args = parser.parse_args()
    print(install(args.pid, args.remove, args.command, args.melee_sync))
