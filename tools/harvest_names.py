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
  Slots worked out by hand go in docs/vtable_slots.csv (class, slot, method);
  every override of a named slot in a subclass gets that method name, and
  rows already named `vfN` are renamed.
- ctor/dtor: constructors (write a vtable, return this) and destructors (write a
  vtable first, or called first by a scalar deleting destructor).
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


def slot_names():
    """docs/vtable_slots.csv: (class, slot) -> method, worked out by hand."""
    out = {}
    for r in csv.DictReader(open(ROOT / "docs/vtable_slots.csv", encoding="utf-8")):
        out[(r["class"], int(r["slot"]))] = r["method"]
    return out


def vtable_slots(x):
    owners = defaultdict(list)  # fn -> [(nbases, class, slot)]
    lineage = {}  # class -> (itself and its bases, most derived first; virtual counts)
    nvirt = {}
    for r in csv.DictReader(open(ROOT / "docs/rtti_classes.csv", encoding="utf-8")):
        nb = len([b for b in r["bases"].split(" ; ") if b])
        cls = nice_class(r["name"])
        nvirt[cls] = int(r["n_virtuals"] or 0)
        lineage[cls] = [cls] + [nice_class(b) for b in r["bases"].split(" ; ") if b]
        for vt in filter(None, r["vtables"].split(" ")):
            va, off = vt.split("@")
            va = int(va, 16)
            for k in range(int(r["n_virtuals"] or 0)):
                f = x.u32(va + 4 * k)
                if x.lo <= f < x.hi:
                    owners[f].append((nb, int(off, 16), nice_class(r["name"]), k))
    named = slot_names()
    out = {}
    for f, os_ in owners.items():
        nb, off, cls, k = min(os_)
        body = x.d[f - BASE:f - BASE + 24]
        # A slot named in docs/vtable_slots.csv for this class or one of its bases
        # (primary vtable only, and only where that base has the slot).
        method = None
        if off == 0:
            for a in lineage.get(cls, [cls]):
                if (a, k) in named and k < nvirt.get(a, 0):
                    method = named[(a, k)]
                    break
        if method:
            name = f"{cls}::{method}"
        elif b"\xF6\x44\x24\x04\x01" in body or b"\xF6\x44\x24\x08\x01" in body:
            name = f"{cls}::scalar_deleting_dtor"
        else:
            name = f"{cls}::vf{k}" + (f"_at{off:x}" if off else "")
        n = len({c for _, _, c, _ in os_})
        out[f] = (name, f"vtable slot {k} of {cls}" + (f" (shared by {n} classes)" if n > 1 else ""))
    return out


MOV_EAX_THIS = {b"\x8B\xC6", b"\x89\xF0", b"\x8B\xC1", b"\x89\xC8", b"\x8B\xC7", b"\x89\xF8", b"\x8B\xC3", b"\x89\xD8"}
POPS = {0x5B, 0x5D, 0x5E, 0x5F}


def returns_this(x, f, end):
    """The function ends `mov eax, <esi|ecx|edi|ebx>; pop*; ret[ n]` (MSVC constructors return this)."""
    d = x.d
    e = end
    while e > f and d[e - 1 - BASE] == 0xCC:
        e -= 1
    if e - f >= 1 and d[e - 1 - BASE] == 0xC3:
        e -= 1
    elif e - f >= 3 and d[e - 3 - BASE] == 0xC2:
        e -= 3
    else:
        return False
    for _ in range(4):
        if d[e - 2 - BASE:e - BASE] in MOV_EAX_THIS:
            return True
        if d[e - 1 - BASE] not in POPS:
            return False
        e -= 1
    return False


def ctors_dtors(x, starts, names):
    """Constructors and destructors, from the vtable stores in their bodies.

    - A function that writes a vtable and returns `this` is a constructor of the
      last (most derived) class it writes at offset 0.
    - One that writes a vtable within its first 12 bytes (no call before it) and
      doesn't return `this` is a destructor of the first class it writes.
    - A scalar deleting destructor that calls a function before its
      `test byte [esp+n], 1` names that function the destructor.
    """
    import bisect
    vts = {}
    for r in csv.DictReader(open(ROOT / "docs/rtti_classes.csv", encoding="utf-8")):
        for v in filter(None, r["vtables"].split(" ")):
            va, off = v.split("@")
            vts[int(va, 16)] = (nice_class(r["name"]), int(off, 16))
    t = x.d[x.lo - BASE:x.hi - BASE]
    by_fn = defaultdict(list)
    # mov dword [reg (+disp8/32)], imm32, reg != esp/ebp
    for m in re.finditer(rb"\xC7[\x00-\x03\x06\x07\x40-\x43\x46\x47\x80-\x83\x86\x87]", t):
        i = m.start()
        mod = t[i + 1] >> 6
        imm = struct.unpack_from("<I", t, i + {0: 2, 1: 3, 2: 6}[mod])[0]
        disp = 0 if mod == 0 else (t[i + 2] if mod == 1 else 1)
        if imm in vts and disp == 0 and vts[imm][1] == 0:
            site = x.lo + i
            j = bisect.bisect_right(starts, site) - 1
            if j >= 0:
                by_fn[starts[j]].append((site, vts[imm][0]))
    # `test eax, eax; je short; call X`: X runs on a fresh allocation, so a vtable
    # writer called like this is a constructor (often with this in EAX, which it
    # returns without a `mov eax, this`).
    after_alloc = set()
    for m in re.finditer(rb"\x85\xC0\x74.\xE8", t, re.S):
        i = m.start() + 4
        after_alloc.add(x.lo + i + 5 + struct.unpack_from("<i", t, i + 1)[0])
    out = {}
    for f, stores in by_fn.items():
        j = bisect.bisect_right(starts, f)
        end = starts[j] if j < len(starts) else x.hi
        # Many functions are only reached through pointers, so the next known start
        # can be far away; MSVC pads only between functions, so a ret followed by
        # int3 ends this one.
        pad = re.search(rb"(\xC3|\xC2..)\xCC", x.d[f - BASE:end - BASE], re.S)
        if pad:
            end = f + pad.start() + len(pad.group(1))
        stores = [s for s in stores if s[0] < end]
        if not stores:
            continue
        if returns_this(x, f, end):
            cls = stores[-1][1]
            out[f] = (f"{cls}::ctor", f"writes the {cls} vtable and returns this")
        elif f in after_alloc:
            cls = stores[-1][1]
            out[f] = (f"{cls}::ctor", f"writes the {cls} vtable and is called on a fresh allocation")
        elif stores[0][0] - f <= 12 and b"\xE8" not in x.d[f - BASE:stores[0][0] - BASE]:
            cls = stores[0][1]
            out[f] = (f"{cls}::dtor", f"writes the {cls} vtable first and doesn't return this")
    for f, name in names.items():
        if not name.endswith("::scalar_deleting_dtor"):
            continue
        cls = name[:-len("::scalar_deleting_dtor")]
        body = x.d[f - BASE:f - BASE + 32]
        test = re.search(rb"\xF6\x44\x24[\x04\x08]\x01", body)
        call = body.find(b"\xE8")
        if test and 0 <= call < test.start():
            tgt = f + call + 5 + struct.unpack_from("<i", body, call + 1)[0]
            if x.lo <= tgt < x.hi and tgt not in after_alloc:
                out[tgt] = (f"{cls}::dtor", f"called first by {cls}::scalar_deleting_dtor")
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
    slots = vtable_slots(x)
    add("vtable", slots, class_area)
    # Auto vtable rows are renamed when docs/vtable_slots.csv names their slot.
    renamed = {}
    for r in rows:
        f = int(r["address"], 16)
        if r["notes"].startswith("auto (vtable)") and f in slots:
            want = slots[f][0]
            if r["name"] != want and re.sub(r"_\d+$", "", r["name"]) != want:
                used.discard(r["name"])
                name, i = want, 2
                while name in used:
                    name = f"{want}_{i}"
                    i += 1
                used.add(name)
                renamed[r["address"]] = name
    print(f"vtable: {len(renamed)} renamed from docs/vtable_slots.csv")
    starts = sorted(set(function_starts(x.d, x.lo, x.hi - x.lo)) | known | set(new))
    names = {int(r["address"], 16): r["name"] for r in rows} | {f: r["name"] for f, r in new.items()}
    cd = ctors_dtors(x, starts, names)
    # Earlier ctor/dtor rows follow rule fixes (name and note).
    fixed = 0
    for r in rows:
        f = int(r["address"], 16)
        if r["notes"].startswith("auto (ctor/dtor)") and f in cd:
            want, note = cd[f]
            if re.sub(r"_\d+$", "", r["name"]) != want:
                used.discard(r["name"])
                name, i = want, 2
                while name in used:
                    name = f"{want}_{i}"
                    i += 1
                used.add(name)
                renamed[r["address"]] = (name, f"auto (ctor/dtor): {note}")
                fixed += 1
    print(f"ctor/dtor: {fixed} existing rows corrected")
    add("ctor/dtor", cd, class_area)
    add("string", string_names(x, starts), lambda f, n: "named")

    print(f"total new rows: {len(new)} (csv had {len(rows)})")
    if write:
        # Keep existing lines byte-for-byte and in their order (LF, notes always quoted);
        # the new ones go after them in address order.
        lines = CSV.read_text(encoding="utf-8").splitlines()
        for n, ln in enumerate(lines):
            addr, name, rest = ln.split(",", 2)
            if addr in renamed:
                new_name = renamed[addr]
                if isinstance(new_name, tuple):
                    new_name, note = new_name
                    sub, status, _ = rest.split(",", 2)
                    rest = f'{sub},{status},"{note}"'
                lines[n] = f"{addr},{new_name},{rest}"
        for f in sorted(new):
            r = new[f]
            notes = r["notes"].replace('"', '""')
            lines.append(f'{r["address"]},{r["name"]},{r["subsystem"]},{r["status"]},"{notes}"')
        CSV.write_text("\n".join(lines) + "\n", encoding="utf-8", newline="\n")
        print(f"wrote {CSV}")


if __name__ == "__main__":
    main(sys.argv[1:])
