"""Dump MSVC RTTI from SR2_pc.exe (unpacked): every class with a type descriptor,
its vtables (via complete object locators), virtual-function counts and base classes.

Usage: python -I tools/rtti_dump.py bin/SR2_pc.unpacked.exe docs/rtti_classes.csv

MSVC x86 layout:
  TypeDescriptor        { type_info* vftable; void* spare; char name[] }   name ".?AV<mangled>@@" (class) / ".?AU" (struct)
  CompleteObjectLocator { sig=0; offset; cdOffset; TypeDescriptor*; ClassHierarchyDescriptor* }
  vtable[-1]            = CompleteObjectLocator*
  ClassHierarchyDescriptor { sig=0; attributes; numBaseClasses; BaseClassDescriptor** }
  BaseClassDescriptor   { TypeDescriptor*; numContainedBases; PMD{mdisp, pdisp, vdisp}; attributes }
"""
import csv
import struct
import sys
from collections import defaultdict

BASE = 0x400000


def load(path):
    data = open(path, "rb").read()
    pe = struct.unpack_from("<I", data, 0x3C)[0]
    nsec = struct.unpack_from("<H", data, pe + 6)[0]
    opt = struct.unpack_from("<H", data, pe + 20)[0]
    secs = {}
    for i in range(nsec):
        o = pe + 24 + opt + i * 40
        name = data[o:o + 8].rstrip(b"\0").decode()
        vsize, va, rsize, raw = struct.unpack_from("<IIII", data, o + 8)
        secs[name] = (BASE + va, max(vsize, rsize), raw)
    return data, secs


def demangle(raw):
    """Plain names only: '.?AVfoo@bar@@' -> 'bar::foo'. Templates and other
    special names are returned mangled (minus the '.?AV' prefix)."""
    body = raw[4:]
    if body.endswith("@@"):
        body = body[:-2]
    if "?" in body or "$" in body:
        return raw[4:]
    return "::".join(reversed([p for p in body.split("@") if p]))


def main(exe, out_csv):
    data, secs = load(exe)

    def in_sec(va, name):
        lo, size, _ = secs[name]
        return lo <= va < lo + size

    def u32(va):
        return struct.unpack_from("<I", data, va - BASE)[0]

    def cstr(va):
        o = va - BASE
        return data[o:data.index(b"\0", o)].decode("latin-1")

    # 1. Type descriptors: ".?AV"/".?AU" names in .data, 8 bytes after the header.
    tds = {}
    for tag in (b".?AV", b".?AU"):
        i = data.find(tag)
        while i != -1:
            va = i + BASE
            if in_sec(va, ".data"):
                tds[va - 8] = cstr(va)
            i = data.find(tag, i + 1)

    # 2. Complete object locators in .rdata: sig 0, then a known type descriptor.
    rlo, rsize, _ = secs[".rdata"]
    cols = {}
    for va in range(rlo, rlo + rsize - 20, 4):
        if u32(va) == 0 and u32(va + 12) in tds and in_sec(u32(va + 16), ".rdata"):
            cols[va] = (u32(va + 4), u32(va + 12), u32(va + 16))

    # 3. Vtables: a dword pointing at a COL, followed by code pointers.
    vtables = defaultdict(list)  # td -> [(vtable, this_offset, n_virtuals)]
    col_set = set(cols)
    for va in range(rlo, rlo + rsize - 8, 4):
        col = u32(va)
        if col in col_set and in_sec(u32(va + 4), ".text"):
            vt = va + 4
            n = 0  # consecutive code pointers; the next vtable's COL slot (in .rdata) ends it
            while in_sec(u32(vt + 4 * n), ".text"):
                n += 1
            offset, td, _ = cols[col]
            vtables[td].append((vt, offset, n))

    # 4. Base classes from each class's hierarchy descriptor.
    bases = {}
    for col, (offset, td, chd) in cols.items():
        if td in bases:
            continue
        nbases = u32(chd + 8)
        arr = u32(chd + 12)
        names = []
        for k in range(1, nbases):  # entry 0 is the class itself
            bcd = u32(arr + 4 * k)
            btd = u32(bcd)
            if btd in tds:
                names.append(demangle(tds[btd]))
        bases[td] = (u32(chd + 4), names)

    rows = []
    for td, raw in sorted(tds.items(), key=lambda kv: demangle(kv[1]).lower()):
        vts = sorted(vtables.get(td, []))
        attrs, base_names = bases.get(td, (None, []))
        rows.append({
            "name": demangle(raw),
            "mangled": raw,
            "type_descriptor": f"{td:#010x}",
            "vtables": " ".join(f"{vt:#010x}@{off:#x}" for vt, off, _ in vts),
            "n_virtuals": max((n for _, _, n in vts), default=0),
            "bases": " ; ".join(base_names),
            "multiple_inheritance": "" if attrs is None else int(bool(attrs & 1)),
        })
    with open(out_csv, "w", newline="", encoding="utf-8") as f:
        w = csv.DictWriter(f, fieldnames=list(rows[0]))
        w.writeheader()
        w.writerows(rows)
    with_vt = sum(1 for r in rows if r["vtables"])
    print(f"type descriptors {len(tds)}, object locators {len(cols)}, classes with vtables {with_vt}")
    print(f"wrote {out_csv}")


if __name__ == "__main__":
    main(sys.argv[1], sys.argv[2])
