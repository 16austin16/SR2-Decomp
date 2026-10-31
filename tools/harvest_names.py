"""Add automatically recoverable function names to docs/functions.csv.

Usage: python -I tools/harvest_names.py bin/SR2_pc.unpacked.exe [ghidra_names.json] [--write]

Without --write it only prints what it would add. Rows already in the CSV are never
changed; new rows get status `named` and a note saying where the name came from:

- script: the Lua binding tables. The game registers its script functions by
  building {name, fn} arrays on the stack (`mov [esp+X], "name"` then
  `mov [esp+X+4], fn`). Named `luafn_<name>`.
- ghidra: Ghidra's function-ID matches (VS2005 CRT, STL, the C++ undecorator,
  import thunks), from a {address: name} JSON exported from REA
  (search_procedures, regex `^(?!FUN_|thunk_FUN_|Catch|Unwind)`).
- vtable: every slot of every class in docs/rtti_classes.csv. A function shared by
  several classes is named after the class with the fewest bases (where it was
  introduced). Scalar deleting destructors are recognised by their
  `test byte [esp+4], 1` prologue.
- string: an error/log string that starts with a function name ("foo_bar: ...",
  "foo_bar()", "foo::bar: ...") and is referenced from exactly one function.
"""
import csv
import json
import re
import struct
import sys
from collections import defaultdict
from pathlib import Path

BASE = 0x400000
ROOT = Path(__file__).resolve().parent.parent
CSV = ROOT / "docs/functions.csv"
sys.path.insert(0, str(ROOT / "tools"))
from progress import function_starts, text_section  # noqa: E402


class Exe:
    def __init__(self, path):
        self.d = open(path, "rb").read()
        self.lo, size = text_section(self.d)
        self.hi = self.lo + size

    def u32(self, va):
        return struct.unpack_from("<I", self.d, va - BASE)[0]

    def ident(self, va, pattern=rb"[A-Za-z_][A-Za-z0-9_]{2,63}"):
        o = va - BASE
        if not 0 <= o < len(self.d):
            return None
        e = self.d.find(b"\0", o, o + 256)
        if e <= o:
            return None
        s = self.d[o:e]
        return s.decode() if re.fullmatch(pattern, s) else None

    def plausible(self, f):
        d = self.d
        return self.lo <= f < self.hi and (d[f - BASE - 1] in (0xCC, 0xC3, 0x90) or d[f - BASE - 3] == 0xC2)


def script_bindings(x):
    stores = []
    t = x.d[x.lo - BASE:x.hi - BASE]
    for m in re.finditer(rb"\xC7(\x44\x24|\x84\x24|\x45|\x85)", t):
        i, k = m.start(), m.group(1)
        p = i + 1 + len(k)
        if k in (b"\x44\x24", b"\x45"):
            disp = struct.unpack_from("<b", t, p)[0]
            p += 1
        else:
            disp = struct.unpack_from("<i", t, p)[0]
            p += 4
        stores.append((x.lo + i, k[:1], disp, struct.unpack_from("<I", t, p)[0]))
    found = defaultdict(set)
    for n, (a, base, disp, imm) in enumerate(stores):
        s = x.ident(imm)
        if not s:
            continue
        for a2, b2, d2, i2 in stores[n + 1:n + 3]:
            if b2 == base and d2 == disp + 4 and a2 - a < 16 and x.plausible(i2):
                found[i2].add(s)
    return {f: ("luafn_" + sorted(n)[0], "script binding " + "/".join(sorted(n))) for f, n in found.items()}


def nice_class(name):
    if name.startswith("?$"):
        head = name[2:].split("@", 1)[0]
        return head + "<>"
    return name


def vtable_slots(x):
    owners = defaultdict(list)  # fn -> [(nbases, class, slot)]
    for r in csv.DictReader(open(ROOT / "docs/rtti_classes.csv", encoding="utf-8")):
        nb = len([b for b in r["bases"].split(" ; ") if b])
        for vt in filter(None, r["vtables"].split(" ")):
            va, off = vt.split("@")
            va = int(va, 16)
            for k in range(int(r["n_virtuals"] or 0)):
                f = x.u32(va + 4 * k)
                if x.lo <= f < x.hi:
                    owners[f].append((nb, int(off, 16), nice_class(r["name"]), k))
    out = {}
    for f, os_ in owners.items():
        nb, off, cls, k = min(os_)
        body = x.d[f - BASE:f - BASE + 24]
        if b"\xF6\x44\x24\x04\x01" in body or b"\xF6\x44\x24\x08\x01" in body:
            name = f"{cls}::scalar_deleting_dtor"
        else:
            name = f"{cls}::vf{k}" + (f"_at{off:x}" if off else "")
        n = len({c for _, _, c, _ in os_})
        out[f] = (name, f"vtable slot {k} of {cls}" + (f" (shared by {n} classes)" if n > 1 else ""))
    return out


def string_names(x, starts):
    pat = re.compile(rb"([A-Za-z_][A-Za-z0-9_]*(?:::~?[A-Za-z_][A-Za-z0-9_]*)?)(?:\(\))?(?::\s|\(\))")
    refs = defaultdict(set)
    data = x.d
    for m in re.finditer(rb"[ -~]{6,}\x00", data):
        s = m.group()[:-1]
        mm = pat.match(s)
        if not mm:
            continue
        name = mm.group(1).decode()
        if "_" not in name and "::" not in name or len(name) < 6:
            continue
        va = m.start() + BASE
        k = struct.pack("<I", va)
        i = data.find(k, x.lo - BASE)
        while i != -1 and i < x.hi - BASE:
            refs[va].add((name, i + BASE))
            i = data.find(k, i + 1)
    import bisect
    by_fn = defaultdict(set)
    fn_refs = defaultdict(set)
    for va, rs in refs.items():
        fns = set()
        for name, r in rs:
            j = bisect.bisect_right(starts, r) - 1
            if j >= 0:
                fns.add((starts[j], name))
        if len({f for f, _ in fns}) == 1:
            f, name = next(iter(fns))
            by_fn[f].add(name)
    for f, names in by_fn.items():
        fn_refs[f] = names
    return {f: (next(iter(n)).replace("::~", "::dtor_"), "named in its own error string") for f, n in fn_refs.items() if len(n) == 1}


def main(argv):
    write = "--write" in argv
    args = [a for a in argv if a != "--write"]
    x = Exe(args[0])
    rows = list(csv.DictReader(open(CSV, encoding="utf-8")))
    known = {int(r["address"], 16) for r in rows}
    used = {r["name"] for r in rows}
    new = {}

    def add(src, found, subsystem):
        n = 0
        for f, (name, note) in sorted(found.items()):
            if f in known or f in new:
                continue
            base, i = name, 2
            while name in used:
                name = f"{base}_{i}"
                i += 1
            used.add(name)
            new[f] = {"address": f"0x{f:08x}", "name": name, "subsystem": subsystem(f, name),
                      "status": "named", "notes": f"auto ({src}): {note}"}
            n += 1
        print(f"{src}: {len(found)} found, {n} new")

    def lib_area(f, name):
        if "::" in name and name.split("::")[0].endswith(".DLL"):
            return "import"
        return "crt"

    if len(args) > 1:
        g = json.load(open(args[1], encoding="utf-8"))
        found = {int(a, 16): (n.replace("FID_conflict:", "").replace(".DLL::", "_"), "Ghidra function ID " + n)
                 for a, n in g.items() if a.startswith("0x")}
        add("ghidra", found, lib_area)
    add("script", script_bindings(x), lambda f, n: "script")

    def class_area(f, name):
        return "havok" if name.startswith(("hk", "Hk", "?$hk")) else "class"
    add("vtable", vtable_slots(x), class_area)
    starts = sorted(set(function_starts(x.d, x.lo, x.hi - x.lo)) | known | set(new))
    add("string", string_names(x, starts), lambda f, n: "named")

    print(f"total new rows: {len(new)} (csv had {len(rows)})")
    if write:
        # Keep existing lines byte-for-byte and in their order (LF, notes always quoted);
        # the new ones go after them in address order.
        lines = CSV.read_text(encoding="utf-8").splitlines()
        for f in sorted(new):
            r = new[f]
            notes = r["notes"].replace('"', '""')
            lines.append(f'{r["address"]},{r["name"]},{r["subsystem"]},{r["status"]},"{notes}"')
        CSV.write_text("\n".join(lines) + "\n", encoding="utf-8", newline="\n")
        print(f"wrote {CSV}")


if __name__ == "__main__":
    main(sys.argv[1:])
