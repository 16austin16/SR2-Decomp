"""Turn sr2-callers.log (the caller-survey build) into per-method caller context.

Usage: python tools/caller_report.py bin/SR2_pc.unpacked.exe <sr2-callers.log> [min_calls]

For every logged vtable slot it prints the method (functions.csv name), how often
it ran, and each distinct caller: the calling function's name (if any), its
strings, and the named functions it calls. That is the evidence for naming the
slot in docs/vtable_slots.csv. Needs capstone (pip install capstone), so run it
without -I.
"""
import bisect
import csv
import re
import sys
from collections import defaultdict
from pathlib import Path

from capstone import CS_ARCH_X86, CS_MODE_32, Cs

BASE = 0x400000
ROOT = Path(__file__).resolve().parent.parent
sys.path.insert(0, str(ROOT / "tools"))
from harvest_names import Exe  # noqa: E402
from progress import function_starts  # noqa: E402


def main(exe_path, log_path, min_calls=1):
    x = Exe(exe_path)
    d = x.d
    md = Cs(CS_ARCH_X86, CS_MODE_32)
    names = {int(r["address"], 16): r["name"]
             for r in csv.DictReader(open(ROOT / "docs/functions.csv", encoding="utf-8"))}
    starts = sorted(set(function_starts(d, x.lo, x.hi - x.lo)) | set(names))

    def containing(a):
        return starts[bisect.bisect_right(starts, a) - 1]

    def string_at(v):
        if 0xDB8000 <= v < 0xE80000:
            o = v - BASE
            e = d.find(b"\0", o, o + 80)
            if e - o >= 3 and all(32 <= c < 127 for c in d[o:e]):
                return d[o:e].decode()

    def context(f):
        strs, calls = [], []
        j = bisect.bisect_right(starts, f)
        end = min(starts[j] if j < len(starts) else x.hi, f + 0x1000)
        for i in md.disasm(d[f - BASE:end - BASE], f):
            for m in re.finditer(r"0x[0-9a-f]{6,8}", i.op_str):
                v = int(m.group(), 16)
                s = string_at(v)
                if s:
                    strs.append(s)
                elif i.mnemonic == "call" and v in names and "::vf" not in names[v]:
                    calls.append(names[v])
            if i.mnemonic == "int3":  # padding: the function ended
                break
        return list(dict.fromkeys(strs))[:4], list(dict.fromkeys(calls))[:4]

    per_fn = defaultdict(lambda: [0, defaultdict(int)])
    for line in open(log_path, encoding="utf-8"):
        if line.startswith("#") or "|" not in line:
            continue
        head, tail = line.split("|", 1)
        _, _, slot, fn, total, other = head.split()
        f = int(fn, 16)
        per_fn[f][0] += int(total)
        for pair in tail.split():
            ret, n = pair.split(":")
            per_fn[f][1][int(ret, 16)] += int(n)
    for f, (total, rets) in sorted(per_fn.items(), key=lambda kv: -kv[1][0]):
        if total < min_calls:
            continue
        print(f"{names.get(f, hex(f))} ({f:#x}) calls={total}")
        for ret, n in sorted(rets.items(), key=lambda kv: -kv[1]):
            c = containing(ret)
            strs, calls = context(c)
            print(f"    {n:>8} from {c:#x} {names.get(c, '')} strs={strs} calls={calls}")


if __name__ == "__main__":
    main(sys.argv[1], sys.argv[2], int(sys.argv[3]) if len(sys.argv) > 3 else 1)
