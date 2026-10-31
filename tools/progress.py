"""Measure how much of SR2_pc.exe has been ported and render docs/img/progress.svg.

Usage: python -I tools/progress.py bin/SR2_pc.unpacked.exe docs/img/progress.svg

Ported/identified functions come from docs/functions.csv (status `rust` /
anything else). The total is an estimate taken from the exe: every distinct
`call rel32` target inside .text, plus every vtable slot in
docs/rtti_classes.csv. A function's size runs to the next known start (minus
int3 padding). Inlined code, jump-table-only and pointer-only functions are
missed, so the real total is somewhat higher; Havok and the CRT are counted.
"""
import csv
import re
import struct
import sys
from collections import Counter, defaultdict
from pathlib import Path

BASE = 0x400000
ROOT = Path(__file__).resolve().parent.parent


def text_section(data):
    pe = struct.unpack_from("<I", data, 0x3C)[0]
    nsec = struct.unpack_from("<H", data, pe + 6)[0]
    opt = struct.unpack_from("<H", data, pe + 20)[0]
    for i in range(nsec):
        o = pe + 24 + opt + i * 40
        if data[o:o + 8].rstrip(b"\0") == b".text":
            vsize, va = struct.unpack_from("<II", data, o + 8)
            return BASE + va, vsize
    raise SystemExit("no .text")


def function_starts(data, lo, size):
    hi = lo + size
    starts = set()
    text = data[lo - BASE:hi - BASE]
    i = text.find(b"\xE8")
    while i != -1 and i + 5 <= len(text):
        target = lo + i + 5 + struct.unpack_from("<i", text, i + 1)[0]
        if lo <= target < hi:
            starts.add(target)
        i = text.find(b"\xE8", i + 1)
    # Random E8 bytes inside other instructions also land in .text; keep only
    # targets that look like a function entry (previous byte is padding or ret).
    def plausible(va):
        prev = data[va - BASE - 1]
        return prev in (0xCC, 0xC3, 0x90) or data[va - BASE - 3] == 0xC2
    starts = {s for s in starts if plausible(s)}
    rows = csv.DictReader(open(ROOT / "docs/rtti_classes.csv", encoding="utf-8"))
    for r in rows:
        for vt in filter(None, r["vtables"].split(" ")):
            va = int(vt.split("@")[0], 16)
            for k in range(int(r["n_virtuals"] or 0)):
                f = struct.unpack_from("<I", data, va - BASE + 4 * k)[0]
                if lo <= f < hi:
                    starts.add(f)
    return sorted(starts)


def sizes(data, starts, hi):
    out = {}
    for a, b in zip(starts, starts[1:] + [hi]):
        end = b
        while end > a and data[end - 1 - BASE] == 0xCC:
            end -= 1
        out[a] = end - a
    return out


def load_functions():
    rows = list(csv.DictReader(open(ROOT / "docs/functions.csv", encoding="utf-8")))
    return [(int(r["address"], 16), r["name"], r["subsystem"], r["status"], r["notes"]) for r in rows]


def fill(x, y, width, height, frac, cls):
    """The filled part of a bar: nothing at 0, at least a visible sliver above 0."""
    if frac <= 0:
        return ""
    wpx = max(min(frac, 1.0) * width, height / 2)
    return f'<rect x="{x}" y="{y}" width="{wpx:.1f}" height="{height}" rx="{height / 2}" class="{cls}"/>'


def bar(x, y, w, frac, label, value, cls):
    return (
        f'<text x="{x}" y="{y - 8}" class="label">{label}</text>'
        f'<text x="{x + w}" y="{y - 8}" class="value" text-anchor="end">{value}</text>'
        f'<rect x="{x}" y="{y}" width="{w}" height="14" rx="7" class="track"/>'
        + fill(x, y, w, 14, frac, cls)
    )


def render(stats, areas, out):
    w, pad = 720, 28
    inner = w - 2 * pad
    parts = [
        "<style>"
        ":root{--bg:#ffffff;--fg:#1f2328;--muted:#59636e;--track:#e6e9ee;--rust:#d9531e;--known:#6f9fd8;--line:#d1d9e0}"
        "@media (prefers-color-scheme: dark){:root{--bg:#0d1117;--fg:#e6edf3;--muted:#9198a1;--track:#262c36;--rust:#f0883e;--known:#4c8ed9;--line:#3d444d}}"
        ".bg{fill:var(--bg);stroke:var(--line)}.title{fill:var(--fg);font-size:18px;font-weight:600}"
        ".label{fill:var(--fg);font-size:13px}.value{fill:var(--muted);font-size:13px}"
        ".note{fill:var(--muted);font-size:11px}.track{fill:var(--track)}.rust{fill:var(--rust)}.known{fill:var(--known)}"
        "</style>",
        f'<text x="{pad}" y="36" class="title">Saints Row 2 → Rust: progress</text>',
    ]
    y = 78
    s = stats
    parts.append(bar(pad, y, inner, s["rust_n"] / s["total_n"], "Functions running in Rust",
                     f'{s["rust_n"]} of ~{s["total_n"]:,} ({100 * s["rust_n"] / s["total_n"]:.2f}%)', "rust"))
    y += 52
    parts.append(bar(pad, y, inner, s["rust_b"] / s["total_b"], "Code size running in Rust",
                     f'{s["rust_b"] / 1024:.1f} KB of ~{s["total_b"] / 1048576:.1f} MB ({100 * s["rust_b"] / s["total_b"]:.2f}%)', "rust"))
    y += 52
    parts.append(bar(pad, y, inner, s["known_n"] / s["total_n"], "Functions identified (incl. auto-named: script bindings, library matches, vtable slots)",
                     f'{s["known_n"]:,} of ~{s["total_n"]:,} ({100 * s["known_n"] / s["total_n"]:.1f}%)', "known"))
    y += 52
    parts.append(bar(pad, y, inner, s["named_n"] / s["total_n"], "Functions with descriptive names (identified minus Class::vfN placeholders)",
                     f'{s["named_n"]:,} of ~{s["total_n"]:,} ({100 * s["named_n"] / s["total_n"]:.1f}%)', "known"))
    y += 52
    parts.append(bar(pad, y, inner, s["hand_n"] / s["total_n"], "Functions analysed by hand (named and documented, incl. Rust)",
                     f'{s["hand_n"]} of ~{s["total_n"]:,} ({100 * s["hand_n"] / s["total_n"]:.2f}%)', "known"))
    y += 52
    parts.append(bar(pad, y, inner, s["vnamed_n"] / s["virt_n"], "Virtual methods of game classes with real names (excl. Havok)",
                     f'{s["vnamed_n"]:,} of {s["virt_n"]:,} ({100 * s["vnamed_n"] / s["virt_n"]:.1f}%)', "known"))
    y += 40
    parts.append(f'<text x="{pad}" y="{y}" class="label" font-weight="600">By area (functions in Rust / identified)</text>')
    y += 10
    col = 210
    for name, (rust, known) in areas:
        y += 26
        frac = rust / known if known else 0
        parts.append(f'<text x="{pad}" y="{y + 11}" class="label">{name}</text>')
        parts.append(f'<rect x="{pad + col}" y="{y}" width="{inner - col - 70}" height="12" rx="6" class="track"/>')
        parts.append(fill(pad + col, y, inner - col - 70, 12, frac, "rust"))
        parts.append(f'<text x="{w - pad}" y="{y + 11}" class="value" text-anchor="end">{rust} / {known}</text>')
    y += 44
    parts.append(f'<text x="{pad}" y="{y}" class="note">The total is an estimate from the exe (call targets + vtable slots) and includes Havok and the CRT.</text>')
    y += 16
    parts.append(f'<text x="{pad}" y="{y}" class="note">Generated by tools/progress.py from docs/functions.csv.</text>')
    h = y + 20
    head = (
        f'<svg xmlns="http://www.w3.org/2000/svg" width="{w}" height="{h}" viewBox="0 0 {w} {h}" '
        'font-family="-apple-system,Segoe UI,Helvetica,Arial,sans-serif">'
    )
    frame = f'<rect x="0.5" y="0.5" width="{w - 1}" height="{h - 1}" rx="10" class="bg"/>'
    # The frame goes right after the <style> element so it is drawn first.
    body = [head, parts[0], frame, *parts[1:], "</svg>"]
    out.parent.mkdir(parents=True, exist_ok=True)
    out.write_text("".join(body), encoding="utf-8")


def game_virtuals(data, lo, size):
    """Functions in the vtables of game classes (Havok's hk* classes left out)."""
    out = set()
    for r in csv.DictReader(open(ROOT / "docs/rtti_classes.csv", encoding="utf-8")):
        if r["name"].startswith(("hk", "Hk", "?$hk")):
            continue
        for vt in filter(None, r["vtables"].split(" ")):
            va = int(vt.split("@")[0], 16)
            for k in range(int(r["n_virtuals"] or 0)):
                f = struct.unpack_from("<I", data, va - BASE + 4 * k)[0]
                # Slots past a vtable's real end hold data; real targets follow padding.
                if lo <= f < lo + size and (data[f - BASE - 1] in (0xCC, 0xC3, 0x90) or data[f - BASE - 3] == 0xC2):
                    out.add(f)
    return out


def main(exe, out):
    data = open(exe, "rb").read()
    lo, size = text_section(data)
    starts = function_starts(data, lo, size)
    funcs = load_functions()
    virt = game_virtuals(data, lo, size)
    name_of = {a: n for a, n, *_ in funcs}
    starts = sorted(set(starts) | {a for a, *_ in funcs if lo <= a < lo + size})
    sz = sizes(data, starts, lo + size)
    rust = [f for f in funcs if f[3] == "rust"]
    stats = {
        "total_n": len(starts),
        "total_b": sum(sz.values()),
        "rust_n": len(rust),
        "rust_b": sum(sz.get(a, 0) for a, *_ in rust),
        "known_n": len(funcs),
        "hand_n": sum(not f[4].startswith("auto") for f in funcs),
        "named_n": sum(not re.search(r"::vf\d", f[1]) for f in funcs),
        "virt_n": len(virt),
        "vnamed_n": sum(a in name_of and not re.search(r"::vf\d", name_of[a]) for a in virt),
    }
    by_area = defaultdict(lambda: [0, 0])
    for _, _, area, status, _ in funcs:
        by_area[area][1] += 1
        by_area[area][0] += status == "rust"
    areas = sorted(((k, tuple(v)) for k, v in by_area.items()), key=lambda kv: (-kv[1][0], -kv[1][1], kv[0]))
    render(stats, areas, Path(out))
    print(stats, dict(Counter(f[3] for f in funcs)))
    print(f"wrote {out}")


if __name__ == "__main__":
    main(sys.argv[1], sys.argv[2])
