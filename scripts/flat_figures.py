#!/usr/bin/env python3
"""The per-object charges of a flat (all_dram) build, against the cache's own figures.

Start a flat server, then say which build it is (the layout, the store, and for a DashMap
build the policy's stack term):

    paper-server --bind 127.0.0.1:3145 --max-size 64MiB --policy lru-compact   # an all_dram build
    scripts/flat_figures.py --port 3145 --layout thin --store dashmap --stack 56

For each of eight shapes (key length, value length) it sets 50 distinct keys, asks SIZE of
one, and reads the self-stats report before and after, so `used size` per object is measured
beside it. Every figure is predicted from the formulas of the cache's src/object/overhead.rs
(450a03f), written out here independently of the server and of the cache:

  total_size = base_size + get_policy_overhead(policy)
  base_size  = key + nallocx(item_prefix + value_len) + 4          (an expiry is a u32; no TTL here)
    split value (--layout thick): key = 16 + key_len (a Box<[u8]>, as `TypeSize` counts it),
                                  item_prefix = 0, and nallocx(0) = 0
    thin_header (--layout thin):  key = 8 (the 8-byte hash: the key is in the item),
                                  item_prefix = (12 + key_len + 7) & ~7
  DashMap (--store dashmap):      get_policy_overhead = stack + OBJECT_MAP_ROW_OVERHEAD
    OBJECT_MAP_ROW_OVERHEAD = 40 + VALUE_ALLOCATION_OVERHEAD - 12    = 60 split, 44 thin
    stack = 56 (--stack 56) for lru, lfu, fifo, clock, sieve and mru-compact,
            72 (--stack 72) for 2q-compact, arc and s3-fifo-compact
  merged store (--store merged):  get_policy_overhead = 46 + VALUE_ALLOCATION_OVERHEAD - 12
    VALUE_ALLOCATION_OVERHEAD = 32 split, 16 thin_header           = 66 split, 50 thin
So a flat thin DashMap charges 100 B (56-stack) or 116 B (72-stack) of policy overhead per
object, the split layout 116 B or 132 B, the merged store 66 B split and 50 B thin.

Hand-built frames, one connection, 400 small SETs: no load. The exit status is 0 only if every
figure matched.
"""
import argparse
import os
import re
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from probe_server import Checks, Client, payload  # noqa: E402


def nallocx(n):
    """jemalloc's size class for a request of n bytes: a 16-byte quantum up to 128, four classes per doubling above."""
    if n == 0:
        return 0
    if n <= 8:
        return 8
    if n <= 128:
        return (n + 15) & ~15
    d = 1 << ((n - 1).bit_length() - 1 - 2)
    return (n + d - 1) // d * d


def predicted(layout, store, stack, key_len, value_len):
    """(base_size, policy overhead, total_size) of one object with no TTL."""
    expire = 4
    if layout == "thick":
        key, item, header = 16 + key_len, nallocx(value_len), 32
    else:
        key, item, header = 8, nallocx(((12 + key_len + 7) & ~7) + value_len), 16
    base = key + item + expire
    policy = stack + (40 + header - 12) if store == "dashmap" else 46 + header - 12
    return base, policy, base + policy


def used_and_objects(text):
    used = int(re.search(r"^used size\s+(\d+) B", text, re.M).group(1))
    objects = int(re.search(r"^objects\s+(\d+)", text, re.M).group(1))
    return used, objects


def main():
    ap = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    ap.add_argument("--port", type=int, required=True)
    ap.add_argument("--layout", choices=["thick", "thin"], required=True, help="thin: built with thin_header")
    ap.add_argument("--store", choices=["dashmap", "merged"], required=True, help="merged: built with merged_object_store")
    ap.add_argument("--stack", default="-", help="56 or 72, the policy's eviction-stack term (DashMap only)")
    ap.add_argument("--measured", action="store_true", help="built with measured_accounting: print the report's dram rows too")
    args = ap.parse_args()
    if args.store == "dashmap" and args.stack not in ("56", "72"):
        ap.error("--stack 56 or 72 is needed for a DashMap build")
    stack = 0 if args.store == "merged" else int(args.stack)

    c = Checks()
    cl = Client(args.port, connect_retries=50)
    ctl = Client(args.port)
    c.check("the server is a flat build", "flat all-DRAM build: no tiers" in ctl.self_stats())
    n_keys = 50
    shapes = [(16, 0), (16, 1), (16, 64), (16, 100), (5, 64), (40, 1000), (16, 4096), (250, 8192)]
    print(f"== flat figures: {args.layout} value layout, {args.store} store, stack {args.stack}")
    print(f"   {'key':>4} {'value':>6} | {'base':>5} {'policy':>6} {'total':>6} predicted | SIZE   used/object")
    for shape, (key_len, value_len) in enumerate(shapes):
        base, policy, total = predicted(args.layout, args.store, stack, key_len, value_len)
        used0, objects0 = used_and_objects(ctl.self_stats())
        # Exactly key_len bytes, distinct across shapes and indices: filler, then a 5-byte big-endian counter.
        keys = [b"K" * (key_len - 5) + (shape * 1000 + i).to_bytes(5, "big") for i in range(n_keys)]
        assert len(set(keys)) == n_keys and all(len(k) == key_len for k in keys)
        for i, key in enumerate(keys):
            cl.set(key, payload(value_len, i))
        size = cl.size(keys[0])
        used1, objects1 = used_and_objects(ctl.self_stats())
        per_object = (used1 - used0) / (objects1 - objects0) if objects1 > objects0 else float("nan")
        print(f"   {key_len:>4} {value_len:>6} | {base:>5} {policy:>6} {total:>6}           | {size:<6} {per_object:.1f}")
        c.check(f"key {key_len} B, value {value_len} B: SIZE is {total} ({base} base + {policy} policy)", size == total, f"SIZE {size}")
        c.check(
            f"key {key_len} B, value {value_len} B: the report's used size per object is {total}",
            objects1 - objects0 == n_keys and per_object == total,
            f"{per_object} over {objects1 - objects0} objects",
        )
    if args.measured:
        for line in ctl.self_stats().splitlines():
            if line.startswith(("dram ", "slow ", "per tracked object")):
                print("   " + line)
    return c.summary()


if __name__ == "__main__":
    ok = main()
    print("FIGURES PASSED" if ok else "FIGURES FAILED")
    sys.exit(0 if ok else 1)
