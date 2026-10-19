"""Standalone startup through the legacy watchdog's own shutdown event."""
import ctypes as c
from ctypes import wintypes as w
import hashlib
from pathlib import Path
import time
import argparse
import json

ROOT = Path(__file__).resolve().parents[1]
ORIGINAL_GAME = ROOT.parent / "KnightOnline1453"
LOCAL_GAME = ROOT / ".local" / "client-1453"
parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument('--attach-new', type=int)
parser.add_argument('--game-dir', type=Path)
args = parser.parse_args()
GAME = (args.game_dir or (LOCAL_GAME if (LOCAL_GAME / '.reko-local-client.json').is_file() else ORIGINAL_GAME)).resolve()
assert GAME in (ORIGINAL_GAME.resolve(), LOCAL_GAME.resolve()), 'Unexpected client directory'
EXE = GAME / "KnightOnline.exe"
EXPECTED_HASH = "9513ecdf29e99bfee307fa8dc93ef64bf616d77d475d57fde906107f32169bb0"
WATCHDOG_ADDRESS = 0x78D8C0
WATCHDOG_CODE = bytes.fromhex("a1f4029e0081ecf40100005668c0d4010050")
EVENT_ADDRESS = 0x9E02F4
assert hashlib.sha256(EXE.read_bytes()).hexdigest() == EXPECTED_HASH, "Unknown client build"
if GAME == LOCAL_GAME.resolve():
    profile = json.loads((GAME / '.reko-local-client.json').read_text(encoding='utf-8-sig'))
    assert profile['profile'] == 'local-1453-inert-guard-v1', 'Unknown local client profile'
    assert profile['client_sha256'] == EXPECTED_HASH, 'Unexpected profile client build'
    for relative, key in [('XTrap/XTrap.exe', 'guard_sha256'), ('XPatch.exe', 'patch_sha256')]:
        assert hashlib.sha256((GAME / relative).read_bytes()).hexdigest() == profile[key], 'Local helper changed; prepare a fresh copy'

class Startup(c.Structure):
    _fields_ = [("cb", w.DWORD), ("reserved", w.LPWSTR), ("desktop", w.LPWSTR),
                ("title", w.LPWSTR), ("x", w.DWORD), ("y", w.DWORD),
                ("width", w.DWORD), ("height", w.DWORD), ("xchars", w.DWORD),
                ("ychars", w.DWORD), ("fill", w.DWORD), ("flags", w.DWORD),
                ("show", w.WORD), ("reservedsize", w.WORD), ("reservedptr", c.c_void_p),
                ("stdin", w.HANDLE), ("stdout", w.HANDLE), ("stderr", w.HANDLE)]

class Process(c.Structure):
    _fields_ = [("process", w.HANDLE), ("thread", w.HANDLE), ("pid", w.DWORD), ("tid", w.DWORD)]

k = c.WinDLL("kernel32", use_last_error=True)
k.CreateProcessW.argtypes = [w.LPCWSTR, w.LPWSTR, c.c_void_p, c.c_void_p, w.BOOL,
                            w.DWORD, c.c_void_p, w.LPCWSTR, c.POINTER(Startup), c.POINTER(Process)]
k.ReadProcessMemory.argtypes = [w.HANDLE, c.c_void_p, c.c_void_p, c.c_size_t, c.POINTER(c.c_size_t)]
k.DuplicateHandle.argtypes = [w.HANDLE, w.HANDLE, w.HANDLE, c.POINTER(w.HANDLE), w.DWORD, w.BOOL, w.DWORD]
k.GetCurrentProcess.restype = w.HANDLE
k.SetEvent.argtypes = [w.HANDLE]
k.WaitForSingleObject.argtypes = [w.HANDLE, w.DWORD]
k.GetExitCodeProcess.argtypes = [w.HANDLE, c.POINTER(w.DWORD)]
k.CloseHandle.argtypes = [w.HANDLE]
startup = Startup()
startup.cb = c.sizeof(startup)
startup.flags = 1
startup.show = 5
process = Process()
if args.attach_new is not None:
    k.OpenProcess.argtypes = [w.DWORD,w.BOOL,w.DWORD]
    k.OpenProcess.restype = w.HANDLE
    k.QueryFullProcessImageNameW.argtypes = [w.HANDLE,w.DWORD,w.LPWSTR,c.POINTER(w.DWORD)]
    k.GetProcessTimes.argtypes = [w.HANDLE,c.POINTER(w.FILETIME),c.POINTER(w.FILETIME),c.POINTER(w.FILETIME),c.POINTER(w.FILETIME)]
    process.pid = args.attach_new
    process.process = k.OpenProcess(0x100000 | 0x1000 | 0x40 | 0x10,False,process.pid)
    assert process.process, c.get_last_error()
    path = c.create_unicode_buffer(32768); length=w.DWORD(32768)
    assert k.QueryFullProcessImageNameW(process.process,0,path,c.byref(length))
    assert Path(path.value).resolve() == EXE.resolve(), 'Unexpected process image'
    created,ended,kernel,user = (w.FILETIME() for _ in range(4))
    assert k.GetProcessTimes(process.process,c.byref(created),c.byref(ended),c.byref(kernel),c.byref(user))
    age = time.time() - ((created.dwHighDateTime << 32 | created.dwLowDateTime)/10000000 - 11644473600)
    assert 0 <= age <= 30, 'Startup profile refuses late attachment'
    print(f'NEW_DIRECT_LAUNCH_AGE={age:.3f}',flush=True)
else:
    command = c.create_unicode_buffer(f'"{EXE}" USAKnightOnline')
    assert k.CreateProcessW(str(EXE), command, None, None, False, 0, None, str(GAME),
                            c.byref(startup), c.byref(process)), c.get_last_error()
print(f"CLIENT_PID={process.pid}", flush=True)
k.CreateMutexW.argtypes = [c.c_void_p,w.BOOL,w.LPCWSTR]
k.CreateMutexW.restype = w.HANDLE
profile_mutex=k.CreateMutexW(None,False,f'Local\\REKO_ClientProfile_{process.pid}')
assert profile_mutex,c.get_last_error()
already_managed=c.get_last_error()==183

def read(address, size):
    buffer = c.create_string_buffer(size)
    count = c.c_size_t()
    if not k.ReadProcessMemory(process.process, address, buffer, size, c.byref(count)) or count.value != size:
        return None
    return buffer.raw

try:
    if already_managed:
        print('LOCAL_PROFILE_ALREADY_MANAGED: startup launcher owns this client',flush=True)
        while k.WaitForSingleObject(process.process,1000)!=0:
            pass
        raise SystemExit(0)
    deadline = time.monotonic() + 45
    signalled = False
    while time.monotonic() < deadline:
        if k.WaitForSingleObject(process.process, 0) == 0:
            raise RuntimeError("Client exited during initialization")
        if read(WATCHDOG_ADDRESS, len(WATCHDOG_CODE)) != WATCHDOG_CODE:
            time.sleep(0.01)
            continue
        raw = read(EVENT_ADDRESS, 4)
        event = int.from_bytes(raw, "little") if raw else 0
        if not event:
            time.sleep(0.01)
            continue
        duplicate = w.HANDLE()
        if not k.DuplicateHandle(process.process, event, k.GetCurrentProcess(), c.byref(duplicate), 2, False, 0):
            time.sleep(0.01)
            continue
        try:
            assert k.SetEvent(duplicate), c.get_last_error()
        finally:
            k.CloseHandle(duplicate)
        signalled = True
        break
    assert signalled, "Recognized watchdog shutdown event unavailable"
    deadline = time.monotonic() + 5
    while read(EVENT_ADDRESS, 4) != bytes(4) and time.monotonic() < deadline:
        time.sleep(0.01)
    assert read(EVENT_ADDRESS, 4) == bytes(4), "Watchdog did not complete its own cleanup"
    assert read(WATCHDOG_ADDRESS, len(WATCHDOG_CODE)) == WATCHDOG_CODE
    assert hashlib.sha256(EXE.read_bytes()).hexdigest() == EXPECTED_HASH
    print("LOCAL_PROFILE_READY: legacy heartbeat watchdog exited through its shutdown event; no code or file changes", flush=True)
    while k.WaitForSingleObject(process.process, 1000) != 0:
        pass
    exit_code = w.DWORD()
    assert k.GetExitCodeProcess(process.process, c.byref(exit_code))
    print(f"CLIENT_EXIT_CODE=0x{exit_code.value:08X}", flush=True)
finally:
    k.CloseHandle(profile_mutex)
    if process.thread:
        k.CloseHandle(process.thread)
    k.CloseHandle(process.process)
