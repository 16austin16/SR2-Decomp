"""Print a Rust `const NAME: [u8; N] = [...]` holding the bytes of a function in the unpacked exe,
plus the offsets of its `call rel32` instructions (for detour::relocate).

Usage: python tools/body_bytes.py NAME 0x4fe760 [0x4ff3ea]

Without an end address the body runs to the first `int3` after the start. Needs capstone, so
run without -I.
"""
import struct
import sys

from capstone import Cs, CS_ARCH_X86, CS_MODE_32

exe = open(r"F:\Saints Row 2 Decomp\bin\SR2_pc.unpacked.exe", "rb").read()
pe = struct.unpack_from("<I", exe, 0x3C)[0]
nsec = struct.unpack_from("<H", exe, pe + 6)[0]
opt = struct.unpack_from("<H", exe, pe + 20)[0]


def off(va):
    for i in range(nsec):
        vsz, va0, rsz, raw = struct.unpack_from("<IIII", exe, pe + 24 + opt + i * 40 + 8)
        if va0 + 0x400000 <= va < va0 + 0x400000 + max(vsz, rsz):
            return raw + va - va0 - 0x400000
    raise SystemExit(f"{va:#x} is not in a section")


name, start = sys.argv[1], int(sys.argv[2], 16)
md = Cs(CS_ARCH_X86, CS_MODE_32)
if len(sys.argv) > 3:
    end = int(sys.argv[3], 16)
else:
    end = None
    for ins in md.disasm(exe[off(start):off(start) + 0x1000], start):
        if ins.mnemonic == "int3":
            end = ins.address
            break
    if end is None:
        raise SystemExit("no int3 within 4 KB; pass an end address")
body = exe[off(start):off(end)]
calls = [ins.address - start for ins in md.disasm(body, start) if ins.bytes[0] == 0xE8]
print(f"const {name}: [u8; {len(body)}] = [")
for i in range(0, len(body), 16):
    print("    " + ", ".join(f"0x{b:02X}" for b in body[i:i + 16]) + ",")
print("];")
print(f"const {name}_CALLS: [usize; {len(calls)}] = [{', '.join(hex(c) for c in calls)}];")
