"""Check every (name, addr, prologue) in mempool.rs PORTS against the exe, and
that the prologue ends on an instruction boundary with no relative branch."""
import re
from capstone import Cs, CS_ARCH_X86, CS_MODE_32

exe = open(r"F:\Saints Row 2 Decomp\bin\SR2_pc.unpacked.exe", "rb").read()
src = open(r"F:\Saints Row 2 Decomp\crates\sr2-hook\src\mempool.rs", encoding="utf-8").read()
block = src[src.index("const PORTS"):src.index("];", src.index("const PORTS"))]
md = Cs(CS_ARCH_X86, CS_MODE_32)
bad = 0
for name, addr, body in re.findall(r'\("([^"]+)", (0x[0-9A-Fa-f_]+), &\[([^\]]*)\]\)', block):
    a = int(addr.replace("_", ""), 16)
    want = bytes(int(x.strip(), 16) if x.strip().startswith("0x") else int(x.strip()) for x in body.split(",") if x.strip())
    have = exe[a - 0x400000:a - 0x400000 + len(want)]
    ok = have == want
    # instruction boundary + no relative branch inside the stolen bytes
    off, rel = 0, False
    for ins in md.disasm(exe[a - 0x400000:a - 0x400000 + 32], a):
        if off >= len(want):
            break
        if ins.mnemonic.startswith("j") or ins.mnemonic == "call":
            rel = True
        off += ins.size
    boundary = off == len(want)
    if not (ok and boundary and not rel):
        bad += 1
    print(f"{'OK ' if ok and boundary and not rel else 'BAD'} {name:32} {a:#x} len {len(want)} bytes_match={ok} boundary={boundary} relative={rel}")
print("bad:", bad)
