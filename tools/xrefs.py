"""Find instructions in .text whose operands mention any of the given addresses."""
import sys, struct
from capstone import Cs, CS_ARCH_X86, CS_MODE_32

exe = open(r"F:\Saints Row 2 Decomp\bin\SR2_pc.unpacked.exe", "rb").read()
targets = [int(a, 16) for a in sys.argv[1:]]
pats = [struct.pack("<I", t) for t in targets]
text_lo, text_hi = 0x401000, 0xDB0000
md = Cs(CS_ARCH_X86, CS_MODE_32)
seen = set()
for p in pats:
    i = exe.find(p)
    while i != -1:
        va = i + 0x400000
        if text_lo <= va < text_hi:
            # back up a few bytes and decode until we cover the match
            for back in range(1, 8):
                start = va - back
                ins = next(md.disasm(exe[start - 0x400000:start - 0x400000 + 16], start), None)
                if ins and ins.address + ins.size > va and ins.address <= va - 1 and ins.address not in seen:
                    seen.add(ins.address)
                    print(f"{ins.address:08x}  {ins.mnemonic} {ins.op_str}")
                    break
        i = exe.find(p, i + 1)
