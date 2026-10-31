"""Compare two sr2-fileopen.log runs: for every distinct call (caller, path,
mode, flag), the set of results must be the same; counts may differ with timing."""
import sys
from collections import Counter, defaultdict

def load(p):
    calls = defaultdict(Counter)
    for line in open(p, encoding="utf-8", errors="replace"):
        key, _, result = line.rstrip("\n").partition(" -> ")
        calls[key][result] += 1
    return calls

a, b = load(sys.argv[1]), load(sys.argv[2])
same = differ = 0
only_a = [k for k in a if k not in b]
only_b = [k for k in b if k not in a]
for k in a.keys() & b.keys():
    if set(a[k]) == set(b[k]):
        same += 1
    else:
        differ += 1
        if differ <= 15:
            print("DIFF", k)
            print("   original:", dict(a[k]))
            print("   rust    :", dict(b[k]))
print(f"distinct calls: {len(a)} vs {len(b)}; same results {same}, different {differ}")
print(f"only in original run: {len(only_a)}, only in rust run: {len(only_b)}")
for k in only_a[:8]:
    print("  only original:", k, dict(a[k]))
for k in only_b[:8]:
    print("  only rust    :", k, dict(b[k]))
null_a = sum(c for k in a for r, c in a[k].items() if r == "NULL")
null_b = sum(c for k in b for r, c in b[k].items() if r == "NULL")
print(f"NULL results: {null_a} vs {null_b}")
