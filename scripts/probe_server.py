#!/usr/bin/env python3
"""Independent functional probe for paper-server (this fork).

Builds every frame by hand from the protocol, sharing nothing with the server's
own code, so a symmetric bug in both cannot pass. It took over from the cache
repo's scripts/probe_server.py when the server moved here; unlike that one it
checks PING against the protocol Kia's server speaks (`[!][len]pong`, what the
stock client's `ping()` reads) and covers the SET admission path.

Wire format: little-endian fixed-width integers; b'!' (33) true and b'?' (63)
false; a u32 length prefix on every buffer and string; a request is [command: u8]
and its arguments, a response [ok: bool] and then the payload or [code: u8] with
code 0 meaning a cache error code follows as a second byte. A SET is
[4][key buf][value buf][ttl: u32], and the SERVER speaks first with a bare
handshake byte.

Start a server, then point the probe at its port. `basic` takes any server
(`--flat` for an all_dram build, `--measured` for one built with
measured_accounting, `--policy` if it is not lru-compact-hybrid, which is the
all_dram build's lru-compact):

    paper-server --bind 127.0.0.1:3145 --max-size 256MiB --fast-tier-size 32MiB
    scripts/probe_server.py --port 3145 basic

`errors` is the SET admission path (S9) and each scenario needs a server started
for it; the tiered build only, except `skip`:

    stall     PAPER_GATE_STALL_WINDOW_MS=100 paper-server --max-size 256MiB \
                  --fast-tier-size 16MiB --set-timeout 1500
    deadline  PAPER_GATE_STALL_WINDOW_MS=60000 paper-server --max-size 256MiB \
                  --fast-tier-size 16MiB --set-timeout 600
    metadata  PAPER_GATE_METADATA_FLOOR_BYTES=1048576 paper-server \
                  --max-size 64MiB --fast-tier-size 1MiB --set-timeout 1000
    skip      paper-server --max-size 4MiB --fast-tier-size 1MiB \
                  --set-timeout 3000 --auth tok      (also on an all_dram build)

    scripts/probe_server.py --port 3145 errors --scenario stall

The exit status is 0 only if every check passed. No load: a few connections, a
few thousand small frames and a few MiB of values.
"""
import argparse
import random
import re
import socket
import struct
import sys
import threading
import time

TRUE, FALSE = 33, 63
PING, VERSION, AUTH, GET, SET, DEL, HAS, PEEK, TTL, SIZE, WIPE, RESIZE, POLICY, STATUS = range(14)
SELF_STATS = 200

# Server error codes (first byte after a false), and cache error codes (second byte after
# a zero). Kia's codes, then this fork's additions.
CACHE_KEY_NOT_FOUND, CACHE_ZERO_VALUE, CACHE_EXCEEDING, CACHE_ZERO_CACHE = 1, 2, 3, 4
CACHE_UNCONFIGURED, CACHE_INVALID_POLICY = 5, 6
CACHE_POLICY_NOT_IMPLEMENTED, CACHE_FAST_TIER_STALLED, CACHE_METADATA_OVERFLOW = 7, 8, 9


class Closed(Exception):
    """The server closed the connection."""


class Reply(Exception):
    """A false reply: server_code is the first code byte, cache_code the second (when the first is 0)."""

    def __init__(self, server_code, cache_code=None):
        self.server_code = server_code
        self.cache_code = cache_code
        super().__init__(f"reply server_code={server_code} cache_code={cache_code}")

    @property
    def code(self):
        return self.cache_code if self.server_code == 0 else None


class Client:
    def __init__(self, port, host="127.0.0.1", timeout=15.0, connect_retries=0):
        last = None
        for _ in range(connect_retries + 1):
            try:
                self.s = socket.create_connection((host, port), timeout=timeout)
                break
            except OSError as err:
                last = err
                time.sleep(0.1)
        else:
            raise last
        self.s.setsockopt(socket.IPPROTO_TCP, socket.TCP_NODELAY, 1)
        self.handshake = self.read(1)[0]

    # ---- raw reads ----
    def read(self, n):
        b = bytearray()
        while len(b) < n:
            chunk = self.s.recv(n - len(b))
            if not chunk:
                raise Closed(f"closed after {len(b)}/{n} bytes")
            b += chunk
        return bytes(b)

    def u8(self):
        return self.read(1)[0]

    def u32(self):
        return struct.unpack("<I", self.read(4))[0]

    def u64(self):
        return struct.unpack("<Q", self.read(8))[0]

    def f64(self):
        return struct.unpack("<d", self.read(8))[0]

    def buf(self):
        return self.read(self.u32())

    def boolean(self):
        v = self.u8()
        if v == TRUE:
            return True
        if v == FALSE:
            return False
        raise ValueError(f"not a boolean indicator: {v}")

    def ok(self):
        """True on a true reply; raises Reply on a false one."""
        if self.boolean():
            return True
        code = self.u8()
        if code == 0:
            raise Reply(0, self.u8())
        raise Reply(code)

    def send(self, data):
        self.s.sendall(data)

    # ---- frames ----
    @staticmethod
    def pbuf(b):
        return struct.pack("<I", len(b)) + b

    @staticmethod
    def frame_set(key, value, ttl=0):
        return bytes([SET]) + Client.pbuf(key) + Client.pbuf(value) + struct.pack("<I", ttl)

    # ---- commands ----
    def ping(self):
        self.send(bytes([PING]))
        self.ok()
        return self.buf()

    def version(self):
        self.send(bytes([VERSION]))
        self.ok()
        return self.buf().decode()

    def auth(self, token):
        self.send(bytes([AUTH]) + self.pbuf(token))
        return self.ok()

    def set(self, key, value, ttl=0):
        self.send(self.frame_set(key, value, ttl))
        return self.ok()

    def get(self, key):
        self.send(bytes([GET]) + self.pbuf(key))
        self.ok()
        return self.buf()

    def peek(self, key):
        self.send(bytes([PEEK]) + self.pbuf(key))
        self.ok()
        return self.buf()

    def has(self, key):
        self.send(bytes([HAS]) + self.pbuf(key))
        self.ok()
        return self.boolean()

    def delete(self, key):
        self.send(bytes([DEL]) + self.pbuf(key))
        return self.ok()

    def size(self, key):
        self.send(bytes([SIZE]) + self.pbuf(key))
        self.ok()
        return self.u32()

    def ttl(self, key, ttl):
        self.send(bytes([TTL]) + self.pbuf(key) + struct.pack("<I", ttl))
        return self.ok()

    def wipe(self):
        self.send(bytes([WIPE]))
        return self.ok()

    def resize(self, size):
        self.send(bytes([RESIZE]) + struct.pack("<Q", size))
        return self.ok()

    def policy(self, name):
        self.send(bytes([POLICY]) + self.pbuf(name.encode()))
        return self.ok()

    def status(self):
        self.send(bytes([STATUS]))
        self.ok()
        st = {}
        st["pid"] = self.u32()
        st["max_size"] = self.u64()
        st["used_size"] = self.u64()
        st["num_objects"] = self.u64()
        st["rss"] = self.u64()
        st["hwm"] = self.u64()
        st["total_gets"] = self.u64()
        st["total_sets"] = self.u64()
        st["total_dels"] = self.u64()
        st["miss_ratio"] = self.f64()
        st["policies"] = [self.buf().decode() for _ in range(self.u32())]
        st["policy"] = self.buf().decode()
        st["is_auto"] = self.boolean()
        st["uptime"] = self.u64()
        return st

    def self_stats(self):
        """Command 200: outside the protocol's 0..=13, the server's own report as a text buffer."""
        self.send(bytes([SELF_STATS]))
        self.ok()
        return self.buf().decode()

    def close(self):
        try:
            self.s.close()
        except OSError:
            pass


# The patterns the benchmark's run_mem.py reads the report with (experiments/cxl-node-memory/
# run_mem.py:85-105 on bench-realkeys), copied rather than imported so the probe stands alone.
def parse_report(t):
    def g(pat, idx=1):
        m = re.search(pat, t, re.M)
        return int(m.group(idx).replace(",", "")) if m else None

    return {
        "objects": g(r"^objects\s+(\d+)"),
        "used": g(r"^used size\s+(\d+)"),
        "max": g(r"^max size\s+(\d+)"),
        "fast_obj": g(r"^fast\s+(\d+) objects"),
        "fast_b": g(r"^fast\s+\d+ objects, (\d+) B"),
        "fast_meta_b": g(r"^\s+\+ (\d+) B reserved"),
        "slow_obj": g(r"^slow\s+(\d+) objects"),
        "slow_b": g(r"^slow\s+\d+ objects, (\d+) B"),
        "promo": g(r"^promotions\s+(\d+)"),
        "demo": g(r"^demotions\s+(\d+)"),
        "evict": g(r"^evictions\s+(\d+)"),
        "rss": g(r"^rss\s+(\d+) B"),
        "meas_dram": g(r"^dram\s+-?\d+\s+(\d+)"),
        "meas_slow": g(r"^slow\s+-?\d+\s+(\d+)\s+[-+]"),
        # Fields of this fork's appended sections (not read by run_mem.py).
        "phys_fast": g(r"^phys fast\s+(\d+) B"),
        "eff_fast_cap": g(r"^eff fast cap\s+(\d+) B"),
    }


class Checks:
    def __init__(self):
        self.passed = 0
        self.failed = []

    def check(self, name, cond, detail=""):
        if cond:
            self.passed += 1
            print(f"  PASS  {name}")
        else:
            self.failed.append(name)
            print(f"  FAIL  {name}  {detail}")
        sys.stdout.flush()
        return cond

    def raises(self, name, fn, server_code=0, cache_code=None):
        """fn must fail with exactly this reply."""
        try:
            fn()
        except Reply as r:
            return self.check(
                name,
                r.server_code == server_code and (cache_code is None or r.cache_code == cache_code),
                f"got {r}",
            )
        except Exception as e:  # noqa: BLE001
            return self.check(name, False, f"got {type(e).__name__}: {e}")
        return self.check(name, False, "no error")

    def summary(self):
        print(f"\n{self.passed} passed, {len(self.failed)} failed")
        if self.failed:
            print("FAILED: " + ", ".join(self.failed))
        return not self.failed


def eventually(fn, timeout, every=0.1):
    """fn() true within timeout seconds."""
    end = time.time() + timeout
    while True:
        if fn():
            return True
        if time.time() >= end:
            return False
        time.sleep(every)


def payload(n, seed):
    """Deterministic pseudo-random bytes, so a readback mismatch is not a lucky pattern."""
    return random.Random(seed).randbytes(n) if n else b""


def basic(args):
    c = Checks()
    port = args.port
    cl = Client(port, connect_retries=50)
    print(f"== basic probe against 127.0.0.1:{port} ({'flat all_dram' if args.flat else 'tiered'} build)")

    c.check("handshake byte is '!'", cl.handshake == TRUE, f"got {cl.handshake}")
    c.check("ping answers [!][len]pong", cl.ping() == b"pong")
    version = cl.version()
    c.check(f"version is {args.cache_version}", version == args.cache_version, version)
    cl.wipe()

    # Set/get with byte-exact readback, 0 B to over 1 MiB.
    sizes = [0, 1, 2, 7, 255, 256, 1000, 4096, 65535, 65536, 100_000, 524_288, 1_048_576, 1_048_593, 1_572_864]
    for n in sizes:
        key = b"size-%d" % n
        value = payload(n, n)
        cl.set(key, value)
        got = cl.get(key)
        c.check(f"set/get {n} B reads back byte-exact", got == value, f"got {len(got)} B")
        c.check(f"peek {n} B agrees with get", cl.peek(key) == value)

    # Overwrite changing the length, both ways.
    cl.set(b"ow", payload(100, 1))
    cl.set(b"ow", payload(5000, 2))
    c.check("overwrite grows the value", cl.get(b"ow") == payload(5000, 2))
    cl.set(b"ow", payload(3, 3))
    c.check("overwrite shrinks the value", cl.get(b"ow") == payload(3, 3))

    # has/peek/del/size.
    c.check("has a present key", cl.has(b"ow") is True)
    c.check("has an absent key", cl.has(b"nothing-here") is False)
    sz = cl.size(b"size-1000")
    c.check("size is at least the value's length", sz >= 1000, f"size {sz}")
    c.check("size is a small fixed overhead over the value (< 1 KiB)", sz < 1000 + 1024, f"size {sz}")
    c.raises("get of an absent key is cache error 1", lambda: cl.get(b"nothing-here"), 0, CACHE_KEY_NOT_FOUND)
    c.raises("peek of an absent key is cache error 1", lambda: cl.peek(b"nothing-here"), 0, CACHE_KEY_NOT_FOUND)
    c.raises("size of an absent key is cache error 1", lambda: cl.size(b"nothing-here"), 0, CACHE_KEY_NOT_FOUND)
    c.raises("del of an absent key is cache error 1", lambda: cl.delete(b"nothing-here"), 0, CACHE_KEY_NOT_FOUND)
    c.raises("ttl of an absent key is cache error 1", lambda: cl.ttl(b"nothing-here", 5), 0, CACHE_KEY_NOT_FOUND)
    cl.set(b"del-me", b"x" * 10)
    c.check("del of a present key", cl.delete(b"del-me") is True)
    c.check("has after del", cl.has(b"del-me") is False)

    # Binary (non-UTF-8) keys, and keys up to 250 bytes: every length.
    binary_keys = [
        b"\xff\xfe\x00\x80", b"\x00", b"a\x00b", b"\x80", b"\xc3\x28", b"\xf0\x9f\x98", b"line\nbreak\r\n",
        bytes(range(256))[:250], b"\x80" * 250, payload(250, 99),
    ]
    for i, key in enumerate(binary_keys):
        value = payload(300 + i, 1000 + i)
        cl.set(key, value)
        c.check(f"binary key #{i} ({len(key)} B) reads back", cl.get(key) == value)
    ok_all = True
    for n in range(1, 251):
        key = bytes((n * 7 + j) % 256 for j in range(n))
        value = payload(64, n)
        cl.set(key, value)
        if cl.get(key) != value:
            ok_all = False
            print(f"    key length {n} did not read back")
            break
    c.check("every key length 1..=250 sets and reads back (distinct non-UTF-8 keys)", ok_all)
    cl.set(b"", b"empty-key-value")
    c.check("the empty key works", cl.get(b"") == b"empty-key-value")
    c.check("keys are bytes, not text: 'a' and 'a\\x00' differ", cl.set(b"k\x00", b"one") and cl.set(b"k", b"two") and cl.get(b"k\x00") == b"one" and cl.get(b"k") == b"two")

    # TTL: set with one, the ttl command, and clearing.
    cl.set(b"ttl-set", b"v", ttl=1)
    cl.set(b"ttl-cmd", b"v")
    cl.ttl(b"ttl-cmd", 1)
    cl.set(b"ttl-clear", b"v", ttl=1)
    cl.ttl(b"ttl-clear", 0)
    c.check("a key with a TTL is present at once", cl.has(b"ttl-set") and cl.has(b"ttl-cmd"))
    time.sleep(2.6)
    c.check("a key set with a TTL of 1 s has expired", cl.has(b"ttl-set") is False)
    c.check("a key given a TTL by the ttl command has expired", cl.has(b"ttl-cmd") is False)
    c.check("a TTL cleared with ttl 0 does not expire", cl.has(b"ttl-clear") is True)
    c.raises("get of an expired key is cache error 1", lambda: cl.get(b"ttl-set"), 0, CACHE_KEY_NOT_FOUND)

    # Framing: pipelined commands, and a SET sent in awkward pieces.
    cl.send(Client.frame_set(b"pipe-1", b"A" * 100) + Client.frame_set(b"pipe-2", b"B" * 200) + bytes([PING]))
    c.check("two pipelined SETs and a PING are answered in order", cl.ok() and cl.ok() and cl.ok() and cl.buf() == b"pong")
    c.check("both pipelined values are there", cl.get(b"pipe-1") == b"A" * 100 and cl.get(b"pipe-2") == b"B" * 200)
    frame = Client.frame_set(b"pieces", payload(300_000, 7), ttl=0)
    for cut in (1, 2, 3, 7, 1 + 4 + 6 + 2, 1 + 4 + 6 + 4 + 1, 150_000, len(frame) - 5, len(frame) - 1):
        cl.send(frame[:cut])
        time.sleep(0.02)
        cl.send(frame[cut:])
        cl.ok()
    c.check("a SET split at nine different points always lands whole", cl.get(b"pieces") == payload(300_000, 7))
    for i, byte in enumerate(Client.frame_set(b"drip", b"0123456789", ttl=0)):
        cl.send(bytes([byte]))
    cl.ok()
    c.check("a SET sent one byte at a time lands whole", cl.get(b"drip") == b"0123456789")

    # Refused commands keep the frame in step.
    c.raises("policy is refused with cache error 6", lambda: cl.policy("lru-compact-hybrid"), 0, CACHE_INVALID_POLICY)
    c.raises("a bogus policy name is refused with cache error 6", lambda: cl.policy("not-a-policy"), 0, CACHE_INVALID_POLICY)
    c.check("the connection is still in step after the refusals", cl.ping() == b"pong")
    c.raises("resize to zero is cache error 4", lambda: cl.resize(0), 0, CACHE_ZERO_CACHE)

    # STATUS (13), Kia's frame: pid, sizes, counters, miss ratio, policies, policy, auto flag, uptime.
    st = cl.status()
    c.check("status: one configured policy, the running one", st["policies"] == [st["policy"]] and st["policy"] == args.policy, str(st["policies"]))
    c.check("status: the auto flag is false", st["is_auto"] is False)
    c.check("status: counters are live", st["total_sets"] > 20 and st["total_gets"] > 20 and st["num_objects"] > 0, str(st))
    c.check("status: pid is the server's", args.pid is None or st["pid"] == args.pid, f"{st['pid']} vs {args.pid}")
    c.check("status: sizes are in step", 0 < st["used_size"] <= st["max_size"], str(st))

    # The self-reported latency and tier report (command byte 200).
    text = cl.self_stats()
    rep = parse_report(text)
    c.check("self-stats: the latency table is there", "*** SERVER-SIDE CACHE LATENCY (socket excluded) ***" in text and "get(hit)" in text and "set " in text)
    wanted = ["objects", "used", "max", "rss"] + ([] if args.flat else ["fast_obj", "fast_b", "fast_meta_b", "slow_obj", "slow_b", "promo", "demo", "evict", "phys_fast", "eff_fast_cap"])
    missing = [k for k in wanted if rep[k] is None]
    c.check("self-stats: every anchor run_mem.py reads parses", not missing, f"missing {missing}")
    if args.flat:
        c.check("self-stats: a flat build says it has no tiers", "flat all-DRAM build: no tiers" in text and rep["fast_obj"] is None)
    if args.measured:
        c.check("self-stats: the measured table parses (dram and slow rows)", rep["meas_dram"] is not None and rep["meas_slow"] is not None, str((rep["meas_dram"], rep["meas_slow"])))
    else:
        c.check("self-stats: no measured table without measured_accounting", "MEASURED vs MODELLED" not in text)
    c.check("self-stats: the object count agrees with status", rep["objects"] == cl.status()["num_objects"], f"{rep['objects']} vs {cl.status()['num_objects']}")
    if not args.flat:
        # The tier gauges are republished once per policy-worker pass, so they trail the live count.
        def tiers_add_up():
            r = parse_report(cl.self_stats())
            return r["fast_obj"] + r["slow_obj"] == cl.status()["num_objects"]
        c.check("self-stats: the fast+slow object counts add up to the cache's", eventually(tiers_add_up, 5.0))

    # Wipe leaves an empty cache, still serving.
    c.check("wipe is ok", cl.wipe() is True)
    c.check("wipe emptied the cache", cl.has(b"size-1000") is False and cl.status()["num_objects"] == 0)
    cl.set(b"after-wipe", b"alive")
    c.check("the cache serves after a wipe", cl.get(b"after-wipe") == b"alive")

    cl.close()
    # Hang-up mid-frame must not take the server down.
    for cut in (1, 3, 8, 20):
        h = Client(port)
        h.send(Client.frame_set(b"hang-up", b"z" * 64)[:cut])
        h.close()
    cl2 = Client(port)
    c.check("the server survives clients that hang up mid-frame", cl2.ping() == b"pong")
    cl2.close()
    return c.summary()


# ---- S9 error-path scenarios ----------------------------------------
# S9 error-path scenarios for probe.py: a SET the cache will not take, a client that stalls mid-value.
#
# Each scenario runs against a server the driver (probe_all.sh errors) started with the settings the scenario
# names; nothing here generates load -- a handful of connections and a few MiB of bytes.
#
#   stall     a tiny fast tier, PAPER_GATE_STALL_WINDOW_MS=100 and --set-timeout 1500; four clients stall at four
#             points of a SET (before the value, mid-value, one byte short of it, before the TTL) and so pin the tier;
#             a fifth SET is then refused with cache error 8, answered on a connection that stays in step; the
#             stalled clients are timed out and closed, and the fast-tier byte count returns to its prior value.
#   deadline  the gate's stall window is a minute and the set timeout 600 ms: clients that dribble a byte inside
#             every receive timeout keep the tier pinned past it, a SET is refused with cache error 8 when the set
#             timeout runs out (not the watchdog), and the dribblers are timed out once they go quiet.
#   metadata  PAPER_GATE_METADATA_FLOOR_BYTES = the whole fast tier: no new key's metadata fits; every SET is
#             refused with cache error 9 and the connection stays in step.
#   skip      a SET larger than the cache can hold (cache error 3): its value is skipped, not buffered; a client
#             that has not authorized sends a large SET and the frame is consumed too.


def s9(text):
    """The SET ADMISSION figures of the report."""
    def g(pat):
        m = re.search(pat, text, re.M)
        return int(m.group(1)) if m else None

    return {
        "committed": g(r"^permit sets\s+(\d+) committed"),
        "commit_refused": g(r"^permit sets\s+\d+ committed, (\d+) refused at commit"),
        "refused": g(r"^refused\s+(\d+) \("),
        "stalled": g(r"^refused\s+\d+ \((\d+) fast tier stalled"),
        "metadata": g(r"^refused\s+\d+ \(\d+ fast tier stalled \[8\], (\d+) metadata overflow"),
        "other": g(r"^refused\s+\d+ \(\d+ fast tier stalled \[8\], \d+ metadata overflow \[9\], (\d+) other"),
        "timeouts": g(r"^body timeouts\s+(\d+)"),
        "aborts": g(r"^body aborts\s+(\d+)"),
        "skip_failures": g(r"^skip failures\s+(\d+)"),
        "skipped": g(r"^bytes skipped\s+(\d+) B"),
        "setters": g(r"^live setters\s+(\d+)"),
        "overflows": g(r"^gate stalls\s+.*metadata overflows (\d+)"),
        "stall_errors": g(r"^gate stalls\s+\d+ watchdog stalls, (\d+) refusals"),
    }


def fetch(ctl):
    text = ctl.self_stats()
    r = parse_report(text)
    r.update(s9(text))
    r["text"] = text
    return r


class Staller:
    """A client that sends part of a SET and then goes quiet, on its own connection."""

    def __init__(self, port, key, length, send, label):
        self.label = label
        self.cl = Client(port)
        frame = bytes([4]) + Client.pbuf(key) + struct.pack("<I", length)
        body = payload(length, sum(key))
        # `send` says how much of the value goes out: 0, half, all but one byte, or everything (TTL withheld).
        self.cl.send(frame + body[:send(length)])

    def closed_within(self, seconds):
        """True if the server closes this connection (EOF or reset) within `seconds`."""
        self.cl.s.settimeout(seconds)
        try:
            data = self.cl.s.recv(1)
        except socket.timeout:
            return False
        except ConnectionResetError:
            return True
        return data == b""


def stall(args):
    c = Checks()
    port = args.port
    ctl = Client(port, connect_retries=50)
    base = fetch(ctl)
    eff, p0 = base["eff_fast_cap"], base["phys_fast"]
    print(f"== stall scenario: eff {eff} B, phys fast {p0} B, setters {base['setters']}")
    c.check("the byte gate is on (a stall is possible at all)", re.search(r"^gate\s+Enabled", base["text"], re.M) is not None)
    c.check("no setter is live at rest", base["setters"] == 0)
    if eff < 4 << 20:
        c.check("the fast tier is big enough for the scenario", False, f"eff {eff}")
        return c.summary()

    # Idle connections are not setters: a setter is registered per SET in flight, not per connection.
    idle = [Client(port) for _ in range(5)]
    c.check("five idle connections are no live setters", fetch(ctl)["setters"] == 0)

    # Each is larger than the gate's B - S (5% of eff), so each is admitted on a SETTLED tier (P <= S, nothing
    # reserved) and may overshoot S: four of eff/4 leave P at eff, above S and at the close level B, which no
    # further set is admitted over until something is freed. (A tier only ever pinned below S would still admit
    # a value larger than B - S: the oversize path bounds P by S + v_max, not by B.)
    hold = eff // 4
    cut = {
        "header only": lambda n: 0,
        "half the value": lambda n: n // 2,
        "one byte short of the value": lambda n: n - 1,
        "the whole value, no TTL": lambda n: n,
    }
    t_start = time.time()
    stallers = [Staller(port, b"hold-%d" % i, hold, fn, name) for i, (name, fn) in enumerate(cut.items())]

    def pinned():
        return fetch(ctl)["phys_fast"] >= p0 + 4 * hold
    c.check(f"four stalled SETs pin {4 * hold} B of the fast tier", eventually(pinned, 5.0), f"phys {fetch(ctl)['phys_fast']}")
    mid = fetch(ctl)
    c.check("four live setters while four SETs are in flight", mid["setters"] == 4, f"setters {mid['setters']}")

    # The refusal: a SET that does not fit, answered with code 8, on a connection that stays in step.
    probe = Client(port)
    big = payload(eff // 2, 5)
    t0 = time.time()
    c.raises("a SET the full tier cannot take is cache error 8", lambda: probe.set(b"probe", big), 0, CACHE_FAST_TIER_STALLED)
    waited = time.time() - t0
    c.check("the refusal came after the gate's stall window and well before the set timeout", 0.05 < waited < 1.45, f"{waited:.3f} s")
    c.check("the next command on the same connection works (framing kept)", probe.ping() == b"pong")
    c.check("the refused SET stored nothing", probe.has(b"probe") is False)
    # And once more with the next command already queued behind the refused SET's bytes.
    probe.send(Client.frame_set(b"probe", big) + bytes([0]))
    c.raises("a second refusal", probe.ok, 0, CACHE_FAST_TIER_STALLED)
    c.check("a PING pipelined behind the refused SET is answered", probe.ok() and probe.buf() == b"pong")
    after = fetch(ctl)
    c.check("the server counts the refusals as code 8", after["stalled"] == 2 and after["metadata"] == 0, f"{after['stalled']}/{after['metadata']}")
    c.check("the refused values were skipped, not stored", after["skipped"] >= 2 * len(big), f"{after['skipped']} B")
    c.check("the refused SETs held no fast bytes", after["phys_fast"] < p0 + 4 * hold + len(big) // 2, f"phys {after['phys_fast']}")

    # The stalled clients are timed out and closed, whichever point they stalled at.
    for st in stallers:
        timeout_left = max(0.1, 1.5 + 2.5 - (time.time() - t_start))
        c.check(f"the server closes the client that stalled at: {st.label}", st.closed_within(timeout_left))
    c.check("the fast-tier byte count returns to its prior value", eventually(lambda: fetch(ctl)["phys_fast"] == p0, 3.0), f"phys {fetch(ctl)['phys_fast']} vs {p0}")
    done = fetch(ctl)
    c.check("four body timeouts are counted", done["timeouts"] == 4, f"{done['timeouts']} (aborts {done['aborts']})")
    c.check("no setter is live again", eventually(lambda: fetch(ctl)["setters"] == 0, 3.0))
    c.check("the same SET is now admitted and reads back", probe.set(b"probe", big) and probe.get(b"probe") == big)

    # A client that hangs up mid-value is an abort, refunded at once.
    p1 = fetch(ctl)["phys_fast"]
    quitter = Staller(port, b"quitter", hold, lambda n: n // 2, "hangs up")
    c.check("a client that has sent half a value pins its bytes", eventually(lambda: fetch(ctl)["phys_fast"] >= p1 + hold, 3.0))
    quitter.cl.close()
    c.check("and they are refunded when it hangs up, well inside the timeout", eventually(lambda: fetch(ctl)["phys_fast"] == p1, 1.0), f"phys {fetch(ctl)['phys_fast']} vs {p1}")
    c.check("a hang-up is an abort, not a timeout", fetch(ctl)["aborts"] >= 1 and fetch(ctl)["timeouts"] == 4)

    # The connection a refused SET was answered on is still a working connection.
    c.check("the refused client's connection serves a normal SET and GET", probe.set(b"after", b"fine") and probe.get(b"after") == b"fine")
    for i in idle:
        i.close()
    return c.summary()


class Dribbler(Staller):
    """A Staller that then keeps its reads alive: a byte of the value every `every` seconds until stopped.

    The server's receive timeout bounds each READ, so a client that delivers a byte inside every timeout is never
    timed out, however long it takes: the limit the README states, shown rather than asserted.
    """

    def __init__(self, port, key, length, send, label, every=0.15):
        super().__init__(port, key, length, send, label)
        self.sent = send(length)
        self.body = payload(length, sum(key))
        self.every = every
        self.running = True
        self.thread = threading.Thread(target=self._dribble, daemon=True)
        self.thread.start()

    def _dribble(self):
        while self.running and self.sent < len(self.body) - 1:
            try:
                self.cl.send(self.body[self.sent:self.sent + 1])
            except OSError:
                return
            self.sent += 1
            time.sleep(self.every)

    def stop(self):
        self.running = False
        self.thread.join(1.0)


def deadline(args):
    c = Checks()
    port = args.port
    ctl = Client(port, connect_retries=50)
    base = fetch(ctl)
    eff, p0 = base["eff_fast_cap"], base["phys_fast"]
    print(f"== deadline scenario: eff {eff} B; the gate's stall window is 60 s, the set timeout 600 ms")
    if eff < 4 << 20:
        c.check("the fast tier is big enough for the scenario", False, f"eff {eff}")
        return c.summary()

    # Four clients that keep delivering a byte of their values every 150 ms, inside the 600 ms receive timeout.
    hold = eff // 4
    dribblers = [Dribbler(port, b"hold-%d" % i, hold, lambda n: n // 2, "dribbles") for i in range(4)]
    c.check(f"four dribbling SETs pin {4 * hold} B of the fast tier", eventually(lambda: fetch(ctl)["phys_fast"] >= p0 + 4 * hold, 5.0))
    time.sleep(1.6)
    c.check("they still pin it after almost three set timeouts: a read that gets a byte is not timed out", fetch(ctl)["phys_fast"] >= p0 + 4 * hold)
    c.check("and none has been timed out or closed", fetch(ctl)["timeouts"] == 0 and fetch(ctl)["aborts"] == 0)

    probe = Client(port)
    t0 = time.time()
    c.raises("a SET the pinned tier cannot take is cache error 8 when the set timeout runs out", lambda: probe.set(b"probe", payload(eff // 2, 5)), 0, CACHE_FAST_TIER_STALLED)
    waited = time.time() - t0
    c.check("it waited out the 600 ms set timeout, not the gate's 60 s stall window", 0.5 < waited < 1.5, f"{waited:.3f} s")
    c.check("the connection is in step afterwards", probe.ping() == b"pong")
    after = fetch(ctl)
    c.check("the refusal is counted as code 8, and the gate's own watchdog never fired", after["stalled"] == 1 and after["stall_errors"] == 0, f"{after['stalled']}/{after['stall_errors']}")

    # Once they go quiet the receive timeout does its work.
    for d in dribblers:
        d.stop()
    c.check("quiet, they are timed out and the tier is whole again", eventually(lambda: fetch(ctl)["phys_fast"] == p0, 4.0), f"phys {fetch(ctl)['phys_fast']} vs {p0}")
    done = fetch(ctl)
    c.check("four body timeouts are counted", done["timeouts"] == 4, f"{done['timeouts']} (aborts {done['aborts']})")
    c.check("the refused SET now goes through", probe.set(b"probe", payload(eff // 2, 5)) and probe.get(b"probe") == payload(eff // 2, 5))
    return c.summary()


def metadata(args):
    c = Checks()
    port = args.port
    ctl = Client(port, connect_retries=50)
    cl = Client(port)
    print("== metadata scenario: the whole fast tier is the metadata floor, so no new key fits")
    for i in range(3):
        c.raises(f"SET #{i} of a new key is cache error 9", lambda: cl.set(b"new-%d" % i, payload(1000, i)), 0, CACHE_METADATA_OVERFLOW)
        c.check(f"the connection is in step after refusal #{i}", cl.ping() == b"pong")
    c.check("nothing was stored", cl.has(b"new-0") is False)
    cl.send(Client.frame_set(b"piped", payload(70_000, 1)) + bytes([0]))
    c.raises("a large refused SET with a PING pipelined behind it: error 9", cl.ok, 0, CACHE_METADATA_OVERFLOW)
    c.check("the PING behind it is answered", cl.ok() and cl.buf() == b"pong")
    r = fetch(ctl)
    c.check("the server counts four refusals as code 9", r["metadata"] == 4 and r["stalled"] == 0, f"{r['metadata']}/{r['stalled']}")
    c.check("the gate's own count of metadata overflows agrees", r["overflows"] == 4, f"{r['overflows']}")
    c.check("the skipped values are counted", r["skipped"] == 3 * 1000 + 70_000, f"{r['skipped']}")
    c.check("a refusal that waited for nothing holds no setter", r["setters"] == 0)
    return c.summary()


def skip(args):
    c = Checks()
    port = args.port
    ctl = Client(port, connect_retries=50)
    ctl.auth(b"tok")
    cl = Client(port)
    cl.auth(b"tok")
    before = fetch(ctl)
    print(f"== skip scenario: max size {before['max']} B; rss {before['rss']} B")
    huge = payload(8 << 20, 3)
    c.raises("a value the cache cannot hold is cache error 3", lambda: cl.set(b"huge", huge), 0, CACHE_EXCEEDING)
    c.check("the connection is in step afterwards", cl.ping() == b"pong")
    after = fetch(ctl)
    if not args.flat:
        c.check("its 8 MiB were skipped through a scratch buffer, not stored", after["skipped"] >= len(huge), f"{after['skipped']} B")
    c.check("the server's resident size did not grow by the value's size", after["rss"] < before["rss"] + (4 << 20), f"{before['rss']} -> {after['rss']}")
    c.check("nothing was stored", cl.has(b"huge") is False and after["objects"] == 0)

    # A client that has not authorized: a large SET's frame is consumed and answered with server error 3.
    anon = Client(port)
    anon.send(Client.frame_set(b"anon", payload(2 << 20, 4)) + bytes([0]))
    c.raises("an unauthorized SET is server error 3", anon.ok, 3)
    c.check("and the PING behind its value is answered: the frame was consumed", anon.ok() and anon.buf() == b"pong")
    c.check("the unauthorized value was not stored", cl.has(b"anon") is False)

    # A refused SET whose client then stalls: the skip itself is bounded.
    # (The flat build has no timeout: it reads a value into a buffer of its own, as upstream does.)
    if not args.flat:
        stuck = Client(port)
        stuck.auth(b"tok")
        stuck.send(bytes([4]) + struct.pack("<I", 5) + b"stuck" + struct.pack("<I", 8 << 20) + huge[: 1 << 20])
        t0 = time.time()
        stuck.s.settimeout(8.0)
        try:
            data = stuck.s.recv(1)
            closed = data == b""
        except ConnectionResetError:
            closed = True
        except socket.timeout:
            closed = False
        c.check("a refused SET whose client stalls mid-value is closed within the set timeout", closed and time.time() - t0 < 6.0, f"{time.time() - t0:.2f} s")
        c.check("the failed skip is counted", eventually(lambda: fetch(ctl)["skip_failures"] == 1, 3.0), f"{fetch(ctl)['skip_failures']}")
    c.check("the server serves on", cl.ping() == b"pong")
    return c.summary()


def errors(args):
    scenario = args.scenario
    if scenario == "stall":
        return stall(args)
    if scenario == "deadline":
        return deadline(args)
    if scenario == "metadata":
        return metadata(args)
    if scenario == "skip":
        return skip(args)
    raise SystemExit(f"unknown scenario {scenario}")


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--port", type=int, required=True)
    ap.add_argument("--flat", action="store_true", help="an all_dram build: no tier sections")
    ap.add_argument("--measured", action="store_true", help="built with measured_accounting")
    ap.add_argument("--cache-version", default="1.11.12")
    ap.add_argument("--policy", default="lru-compact-hybrid")
    ap.add_argument("--pid", type=int, default=None)
    ap.add_argument("--scenario", choices=["stall", "deadline", "metadata", "skip"], default="stall", help="errors mode")
    ap.add_argument("mode", choices=["basic", "errors"])
    args = ap.parse_args()
    if args.mode == "basic":
        ok = basic(args)
    else:
        ok = errors(args)
    print("ALL PROBES PASSED" if ok else "PROBE FAILED")
    sys.exit(0 if ok else 1)


if __name__ == "__main__":
    main()
