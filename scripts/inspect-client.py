"""Read-only disassembly of the unpacked v1453 memory image (requires capstone)."""
import argparse
import struct
from pathlib import Path

from capstone import Cs, CS_ARCH_X86, CS_MODE_32

parser = argparse.ArgumentParser()
parser.add_argument("address", type=lambda value: int(value, 0))
parser.add_argument("--size", type=lambda value: int(value, 0), default=0x400)
parser.add_argument("--pointers", action="store_true")
parser.add_argument("--xrefs", action="store_true")
parser.add_argument("--image", type=Path, default=Path(__file__).resolve().parents[2] / "unpacked_knightonline.exe")
args = parser.parse_args()
data = args.image.read_bytes()
base = 0x400000
if args.xrefs:
    needle = struct.pack("<I", args.address)
    position = data.find(needle)
    while position >= 0:
        print(f"pointer {position + base:08X}")
        position = data.find(needle, position + 1)
    for position, byte in enumerate(data[:-5]):
        if byte in (0xE8, 0xE9) and base + position + 5 + struct.unpack_from("<i", data, position + 1)[0] == args.address:
            print(f"branch {position + base:08X}")
elif args.pointers:
    for address in range(args.address, args.address + args.size, 4):
        print(f"{address:08X}: {struct.unpack_from('<I', data, address - base)[0]:08X}")
else:
    decoder = Cs(CS_ARCH_X86, CS_MODE_32)
    for instruction in decoder.disasm(data[args.address - base:args.address - base + args.size], args.address):
        print(f"{instruction.address:08X} {instruction.mnemonic} {instruction.op_str}")
