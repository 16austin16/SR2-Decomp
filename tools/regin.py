import sys, struct
from capstone import Cs, CS_ARCH_X86, CS_MODE_32
from capstone.x86 import X86_OP_REG
exe=open(r"F:\Saints Row 2 Decomp\bin\SR2_pc.unpacked.exe","rb").read()
pe=struct.unpack_from("<I",exe,0x3C)[0];n=struct.unpack_from("<H",exe,pe+6)[0];opt=struct.unpack_from("<H",exe,pe+20)[0]
def off(va):
    for i in range(n):
        vsz,v,rsz,raw=struct.unpack_from("<IIII",exe,pe+24+opt+i*40+8)
        if v+0x400000<=va<v+0x400000+max(vsz,rsz): return raw+va-v-0x400000
md=Cs(CS_ARCH_X86,CS_MODE_32); md.detail=True
GP={"eax","ecx","edx","ebx","esi","edi","ebp"}
def full(r):
    m={"al":"eax","ah":"eax","ax":"eax","cl":"ecx","ch":"ecx","cx":"ecx","dl":"edx","dh":"edx","dx":"edx","bl":"ebx","bh":"ebx","bx":"ebx","si":"esi","di":"edi","bp":"ebp"}
    return m.get(r,r)
def scan(a, depth=0, seen=None):
    """regs read before written along the first path (follows only fallthrough, stops at ret/jmp/call)"""
    written=set(); reads=set()
    for ins in md.disasm(exe[off(a):off(a)+0x200], a):
        r,w = ins.regs_access()
        rr={full(ins.reg_name(x)) for x in r}&GP; ww={full(ins.reg_name(x)) for x in w}&GP
        if ins.mnemonic=="push" : rr=rr  # push reg = save, not a real read if callee-saved
        if ins.mnemonic=="push" and ins.operands[0].type==X86_OP_REG: rr=set()
        if ins.mnemonic=="xor" and len(ins.operands)==2 and ins.op_str.split(",")[0].strip()==ins.op_str.split(",")[1].strip(): rr=set()
        reads |= (rr - written); written |= ww
        if ins.mnemonic.startswith("ret") or ins.mnemonic in("jmp","call") or ins.mnemonic.startswith("j"): break
    return reads
for x in sys.argv[1:]:
    print(x, sorted(scan(int(x,16))))
