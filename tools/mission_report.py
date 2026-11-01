"""Summarise sr2-mission.log (crates/sr2-hook/src/missionlog.rs) per section.

Each sampled address is credited to the nearest named function at or below it in
docs/functions.csv. Addresses far past that function (more than --gap bytes) are
listed as "unnamed near <name>+offset": candidates for naming.

Usage: python tools/mission_report.py <sr2-mission.log> [--top N] [--gap BYTES]
"""
import argparse
import bisect
import csv
import re
from collections import Counter

ap = argparse.ArgumentParser()
ap.add_argument("log")
ap.add_argument("--top", type=int, default=25)
ap.add_argument("--gap", type=int, default=0x800)
a = ap.parse_args()

names = {}
with open("docs/functions.csv", encoding="utf-8") as f:
    for row in csv.DictReader(f):
        names[int(row["address"], 16)] = row["name"]
starts = sorted(names)


def owner(addr):
    i = bisect.bisect_right(starts, addr) - 1
    if i < 0:
        return f"unnamed {addr:08x}"
    s = starts[i]
    if addr - s > a.gap:
        return f"unnamed near {names[s]}+{addr - s:#x}"
    return names[s]


def report(head, counts):
    if not head:
        return
    print(head)
    for tag, label in (("g", "game thread"), ("r", "render thread")):
        c = Counter()
        for (t, addr), n in counts.items():
            if t == tag:
                c[owner(addr)] += n
        total = sum(c.values())
        if not total:
            continue
        print(f"  {label}: {total} samples, {len(c)} functions")
        for name, n in c.most_common(a.top):
            print(f"    {n * 100 / total:5.1f}%  {name}")
    print()


head, counts = None, Counter()
for line in open(a.log, encoding="utf-8"):
    if line.startswith("== section"):
        report(head, counts)
        head, counts = line.strip(), Counter()
    elif line.startswith("=="):
        report(head, counts)
        head, counts = None, Counter()
        print(line.strip())
    elif m := re.match(r"([gr]) ([0-9a-f]{8}) (\d+)", line):
        counts[(m[1], int(m[2], 16))] += int(m[3])
report(head, counts)
