import sys, struct
from capstone import Cs, CS_ARCH_X86, CS_MODE_32

exe = open(r"F:\Saints Row 2 Decomp\bin\SR2_pc.unpacked.exe", "rb").read()
pe = struct.unpack_from("<I", exe, 0x3C)[0]
nsec = struct.unpack_from("<H", exe, pe + 6)[0]
opt = struct.unpack_from("<H", exe, pe + 20)[0]
secs = []
for i in range(nsec):
    o = pe + 24 + opt + i * 40
    vsz, va, rsz, raw = struct.unpack_from("<IIII", exe, o + 8)
    secs.append((va + 0x400000, vsz, raw, rsz))

def off(va):
    for sva, vsz, raw, rsz in secs:
        if sva <= va < sva + max(vsz, rsz):
            return raw + va - sva
    raise ValueError(hex(va))

md = Cs(CS_ARCH_X86, CS_MODE_32)
start = int(sys.argv[1], 16)
n = int(sys.argv[2], 16) if len(sys.argv) > 2 else 0x200
code = exe[off(start):off(start) + n]
for ins in md.disasm(code, start):
    print(f"{ins.address:08x}  {ins.bytes.hex():<16} {ins.mnemonic} {ins.op_str}")
    if ins.mnemonic == "int3":
        break
