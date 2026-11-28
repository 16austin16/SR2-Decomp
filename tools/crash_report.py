"""Turn the last CRASH block of sr2-hook.log into names.

Usage: python tools/crash_report.py [path/to/sr2-hook.log]
       (default: E:/SteamGames/steamapps/common/Saints Row 2/sr2-hook.log)

Exe addresses are named from docs/functions.csv (nearest known start at or below). DLL addresses are
named from the linker map of the current build (target/i686-pc-windows-msvc/release/deps/dinput8.map),
so run it against the DLL that crashed: rebuild nothing between the crash and this report.
"""
import bisect, csv, os, re, sys

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
LOG = sys.argv[1] if len(sys.argv) > 1 else r"E:/SteamGames/steamapps/common/Saints Row 2/sr2-hook.log"
MAP = os.path.join(ROOT, os.environ.get("SR2_TARGET","target"), "i686-pc-windows-msvc", "release", "deps", "dinput8.map")


def exe_names():
    rows = []
    for r in csv.reader(open(os.path.join(ROOT, "docs", "functions.csv"), encoding="utf-8")):
        if r and r[0].startswith("0x"):
            rows.append((int(r[0], 16), r[1], r[3] if len(r) > 3 else ""))
    rows.sort()
    return rows


def demangle(sym):
    # Rust v0 (_RNv...) and legacy (_ZN...) symbols: keep the readable identifier pieces.
    parts = re.findall(r"\d+([A-Za-z_][A-Za-z0-9_]*)", sym)
    parts = [p for p in parts if not re.fullmatch(r"[a-z0-9]{0,2}", p)]
    return "::".join(parts[-3:]) if parts else sym


def map_names():
    if not os.path.exists(MAP):
        return None, []
    pref = 0x10000000
    syms = []
    for line in open(MAP, encoding="latin-1"):
        m = re.search(r"Preferred load address is ([0-9a-fA-F]+)", line)
        if m:
            pref = int(m.group(1), 16)
        m = re.match(r"\s*0001:[0-9a-f]+\s+(\S+)\s+([0-9a-f]{8})", line)
        if m:
            syms.append((int(m.group(2), 16), m.group(1)))
    syms.sort()
    return pref, syms


def lookup(table, a):
    keys = [t[0] for t in table]
    i = bisect.bisect_right(keys, a) - 1
    return table[i] if i >= 0 else None


def main():
    lines = open(LOG, encoding="utf-8", errors="replace").read().splitlines()
    idx = [i for i, l in enumerate(lines) if l.startswith("CRASH: access violation")]
    if not idx:
        print("no CRASH line in", LOG)
        return
    crash = lines[idx[-1]]
    trace = [l for l in lines[idx[-1]:idx[-1] + 4] if l.startswith("CRASH: last ports")]
    exe = exe_names()
    pref, dsyms = map_names()
    dll = int(re.search(r"dll (0x[0-9a-f]+)", crash).group(1), 16)

    def name(a):
        if 0x401000 <= a < 0xDB0000:
            r = lookup(exe, a)
            return f"exe {r[1]}+{a - r[0]:#x} [{r[2]}]" if r else "exe ?"
        if dsyms and dll <= a < dll + 0x2000000:
            r = lookup(dsyms, a - dll + pref)
            return f"dll {demangle(r[1])}+{a - dll + pref - r[0]:#x}" if r else "dll ?"
        return "not code"

    print(crash.split(" | ")[0])
    eip = int(re.search(r"eip (0x[0-9a-f]+)", crash).group(1), 16)
    print(f"  eip {eip:#x}: {name(eip)}")
    if trace:
        print("\n" + trace[0])
    refs = re.findall(r"\+(0x[0-9a-f]+):(0x[0-9a-f]+)", crash)
    print("\nstack (offset from esp: address -> function), innermost first:")
    for off, v in refs[:60]:
        print(f"  +{off:>6}: {v:>10}  {name(int(v, 16))}")
    if not dsyms:
        print("\n(no linker map found; rebuild once so", MAP, "exists)")


main()
