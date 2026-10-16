"""Read-only exit observer for the original client; no debugger or memory writes."""
import ctypes as c
from ctypes import wintypes as w
import datetime
from pathlib import Path
import sys
import time

kernel = c.WinDLL("kernel32", use_last_error=True)
class ProcessEntry(c.Structure):
    _fields_ = [("size", w.DWORD), ("usage", w.DWORD), ("pid", w.DWORD),
                ("heap", c.c_size_t), ("module", w.DWORD), ("threads", w.DWORD),
                ("parent", w.DWORD), ("priority", w.LONG), ("flags", w.DWORD),
                ("name", w.WCHAR * 260)]
kernel.CreateToolhelp32Snapshot.argtypes = [w.DWORD, w.DWORD]
kernel.CreateToolhelp32Snapshot.restype = w.HANDLE
kernel.Process32FirstW.argtypes = [w.HANDLE, c.POINTER(ProcessEntry)]
kernel.Process32NextW.argtypes = [w.HANDLE, c.POINTER(ProcessEntry)]
kernel.OpenProcess.argtypes = [w.DWORD, w.BOOL, w.DWORD]
kernel.OpenProcess.restype = w.HANDLE
kernel.QueryFullProcessImageNameW.argtypes = [w.HANDLE, w.DWORD, w.LPWSTR, c.POINTER(w.DWORD)]
kernel.WaitForSingleObject.argtypes = [w.HANDLE, w.DWORD]
kernel.GetExitCodeProcess.argtypes = [w.HANDLE, c.POINTER(w.DWORD)]
kernel.CloseHandle.argtypes = [w.HANDLE]
expected = str(Path(sys.argv[1]).resolve()).casefold()
log = Path(sys.argv[2])
def record(message):
    with log.open("a", encoding="utf-8") as out:
        out.write(datetime.datetime.now(datetime.timezone.utc).isoformat() + " " + message + "\n")
record("read-only client exit observer started")
deadline = time.monotonic() + 3600
seen = set()
while time.monotonic() < deadline:
    snapshot = kernel.CreateToolhelp32Snapshot(2, 0)
    if snapshot == c.c_void_p(-1).value:
        time.sleep(1)
        continue
    entry = ProcessEntry()
    entry.size = c.sizeof(entry)
    found = kernel.Process32FirstW(snapshot, c.byref(entry))
    target = None
    while found:
        if entry.name.casefold() == "knightonline.exe" and entry.pid not in seen:
            handle = kernel.OpenProcess(0x101000, False, entry.pid)
            if handle:
                path = c.create_unicode_buffer(32768)
                length = w.DWORD(len(path))
                if kernel.QueryFullProcessImageNameW(handle, 0, path, c.byref(length)) and path.value.casefold() == expected:
                    target = (entry.pid, handle)
                    break
                kernel.CloseHandle(handle)
        found = kernel.Process32NextW(snapshot, c.byref(entry))
    kernel.CloseHandle(snapshot)
    if target:
        pid, handle = target
        seen.add(pid)
        started = time.monotonic()
        record(f"client observed pid={pid}")
        while time.monotonic() < deadline:
            result = kernel.WaitForSingleObject(handle, 1000)
            if result == 0:
                code = w.DWORD()
                if kernel.GetExitCodeProcess(handle, c.byref(code)):
                    record(f"client exit pid={pid} code=0x{code.value:08X} observed_seconds={time.monotonic()-started:.3f}")
                break
            if result == 0xFFFFFFFF:
                record(f"client wait failed pid={pid} error={c.get_last_error()}")
                break
        kernel.CloseHandle(handle)
    else:
        time.sleep(1)
record("client exit observer finished")
