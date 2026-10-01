# paper-server (tiered fork)

PaperCache is an in-memory cache with a choice of eviction policies. In this fork
the policy is fixed when the server starts (`policy=`, `--policy`): there is no
switching at runtime, and the POLICY command is refused (see "Known limits").

Visit [PaperCache](https://papercache.io) for more details.

This is a fork of [paper-server](https://github.com/PaperCache/paper-server) that
serves the DRAM/CXL tiered cache of
[paper-cache-cxl](https://github.com/griffinandrew/paper-cache-cxl) instead of the
flat published `paper-cache`, behind the same socket protocol.

## Building

The cache is a Git dependency pinned by revision (`Cargo.toml`), and it needs a
nightly compiler (`allocator_api`, `btreemap_alloc`, `read_buf`,
`core_io_borrowed_buf`). `rust-toolchain.toml` pins the one the benchmark box
builds the cache with, `nightly-2026-09-14`. `Cargo.lock` is committed, so the
build resolves to the versions it was validated with.

```bash
cargo build --release                       # tiered, DashMap, default layout
cargo build --release --features thin_header
cargo build --release --features merged_object_store
cargo build --release --features merged_object_store,thin_header
cargo build --release --no-default-features --features all_dram
```

On the benchmark box rustup knows that nightly as `nightly`, not by its dated
name, so build there with `cargo +nightly ...`: a bare `cargo` would try to
install `nightly-2026-09-14` as a second toolchain.

Release builds abort on a panic (`panic = "abort"` in `[profile.release]`).
Upstream's connection pool (`kwik::thread_pool`) does not catch a panic in a
connection's thread: the worker dies and is never replaced, the connection's
slot is never released, and its socket stays open through the clone held in that
slot, so the client hangs and the slot is lost. A server under benchmark should
fail fast and visibly instead. Debug builds still unwind.

| Feature | Effect |
|---|---|
| `tiered` (default) | Serves `PaperCache<Buffer, TieredBuffer>`: values live in the fast (DRAM) or slow (CXL) tier. Enables the cache's `key_value_pmem`, which with `hybrid_cache_common` (always on) puts the TTL expiry index in the slow tier too. |
| `all_dram` | Serves the flat `PaperCache<Buffer, BufferDRAM>` instead, the all-DRAM baseline: one tier, no admission gate, nothing on the slow node (no `key_value_pmem`). Needs `--no-default-features`; building both is a compile error, and neither does not build. |
| `merged_object_store` | The merged store, where the object map is the eviction order. Serves only the lru, lfu, fifo and clock policies. |
| `thin_header` | The thin value layout: a 16-byte DRAM header in front of one tiered item holding the length, the expiry, the key and the bytes. |
| `measured_accounting` | Per-pool allocator counters, reported beside the cache's own accounting. |

The cache hosts all 23 of its tiered designs in every tiered build and picks one
at start-up from `policy=` (`--policy`), so there is one build per store and
layout, not one per design. The cache's own `*_hybrid_cache` features only gate
its tests; nothing here enables them.

The cache installs its own `#[global_allocator]` (a NUMA-bound jemalloc), so
this crate declares none.

## Configuration

`default.pconf` is the default configuration and documents every key. Keys:
`host`, `port`, `max_size`, `fast_tier_size`, `policy`, `max_connections`,
`set_timeout`, `auth_token`; `policies[]` is accepted and ignored, with a
warning. A file given with `--config` is not layered over the defaults (as
upstream): what it leaves out is zero or empty, so it must set `host`, `port`,
`max_size`, `fast_tier_size` and `max_connections`, and `policy` unless that is
`lru-compact-hybrid` (`set_timeout` defaults to 5000). `fast_tier_size` is the
fork's key: the fast (DRAM) tier's byte budget, and the rest of `max_size` is
the slow tier.

### Command line

Every flag is the config line it stands for, parsed as the file's line is, and
wins over the file, which wins over `default.pconf`.

| Flag | Overrides | Notes |
|---|---|---|
| `--bind <ADDR:PORT>` | `host`, `port` | `0.0.0.0:3145` accepts connections from other machines |
| `--max-size <BYTES>` | `max_size` | a byte count, or with a suffix (`2GiB`) |
| `--fast-tier-size <BYTES>` | `fast_tier_size` | tiered build only; at most `max_size` |
| `--policy <POLICY>` | `policy` | e.g. `lru-compact-hybrid`, `s3-fifo-faithful-compact-hybrid-0.1` |
| `--set-timeout <MS>` | `set_timeout` | how long a SET may wait for the cache to take it and for each read of its value (see below); tiered build only |
| `--auth <TOKEN>` | `auth_token` | clients must send it with AUTH before any other command; give it as `'$VAR'` (see below), not as the token |
| `--stats-interval <S>` | | print the self-stats report to stderr every S seconds |
| `--config <FILE>`, `--log-config <FILE>` | | as upstream |

`--bind`, `--max-size`, `--fast-tier-size` and `--policy` are the flags the
benchmark's `run_mem.py` launches a server with, so pointing it at this binary
needs a different `--server` path and nothing else on the server side.

`--auth TOKEN` puts the token in the process's command line, where any user of
the machine can read it with `ps`. On a shared box pass it as `--auth '$VAR'`,
single-quoted so the shell leaves it alone: the server then reads the token from
the environment variable `VAR`, and `ps` shows `$VAR`. A config file takes the
same form, `auth_token=$VAR` (`Config::set`, `try_parse_env` in `src/config.rs`).
A `$VAR` that is not set in the server's environment is not an error: the text
`$VAR` itself becomes the token.

## Self-reported latency

A client can only time a round trip. The server also times the cache call
itself, per command, and reports it with the cache's own figures: command byte
**200** (outside the protocol's 0..=13) answers `[!][len][text]`, and
`--stats-interval` prints the same report to stderr. The report is text, and
only ever grows by appending a section:

| Section | Content |
|---|---|
| `SERVER-SIDE CACHE LATENCY` | count, mean and p50/p90/p99/p99.9 per command, socket excluded. Percentiles are bucket lower bounds (about 25% wide); the mean is exact. GET hits and misses are separate rows. |
| `CACHE` | objects, used and max size, miss ratio, counters, RSS |
| `TIERS` | the fast/slow split (objects and bytes per tier), the fast tier's metadata reservation, promotions, demotions, evictions. The flat build says it has none. |
| `MEASURED vs MODELLED` | the allocator's per-pool totals beside the model; only with `measured_accounting` |
| `PHYSICAL FAST TIER` | the bytes physically in the fast tier's value pool and its peak, the effective budget, hits by tier, the cache's measured DRAM metadata |
| `MIGRATIONS AND CAPACITY PASSES` | the migration queue's depth, backlog and dispositions (applied, gone, declined, superseded), the correctives the reconcile queued and applied, and the capacity passes the eviction watermark armed: this cache's own counts since it was built |
| `SET ADMISSION` | SETs committed, SETs refused by code (8, 9, other), body timeouts and aborts, bytes skipped, the live setters and the cache's value hint, with whether SETs in flight are counted and why not (see "SET" below), and the byte gate's waits, stalls and levels (settle, near, close). Refused SETs have a row of their own in the latency table too. |

`run_mem.py` reads the report with line-anchored regular expressions
(`^fast\s+\d+ objects`, `^slow\s+`, `^dram\s+`, `^promotions`, ...), so no line
of a later section starts with one of those words.

Client-side round trips are not comparable with the cache repo's former server's:
it buffered its reads and wrote a reply with `writev`, where this server, like
upstream, reads field by field and writes a reply whole. The self-reported
figures time the same span in both and are.

PING answers `[!][len]pong`, as upstream's server does and as the stock client's
`ping()` reads it. A client that reads only the boolean (the benchmark's
`bench-client` does) leaves the buffer in the stream and loses frame sync at its
next command.

## SET: admitted before its value is read

The wire's order is key, value, TTL. A server that calls `PaperCache::set` has
to read the whole value into a buffer of its own first, one no budget covers,
and copy it again into the cache. The tiered build splits the set instead
(`src/set.rs`, on the cache's `reserve_set` / `SetPermit` / `PendingSet`):

1. read the key and the value's length, and nothing more;
2. `register_setter()` for this SET, counting it into the byte gate's near band
   while it is in flight (a SET, not a connection: an idle connection counts
   for nothing) -- but only when that can widen anything. The band is widened by
   `(concurrency_hint + live setters) x value_hint`, so with the cache's
   `value_hint` at its default of 0 a setter widens nothing, and with the byte
   gate off (`PAPER_GATE_MODE=off`) the cache publishes no band to widen at all.
   Each registration and release would wake the cache's policy worker for no
   effect. The server reads the cache's gate configuration once at start-up
   (`PaperCache::gate_config`, so the `PAPER_GATE_*` variables apply) and
   registers SETs only when the gate runs and `value_hint` is above 0
   (`SetterCount` in `src/set.rs`); the report's `value hint` line says which,
   and `live setters` stays 0 otherwise;
3. `reserve_set(key, len, None, now + set_timeout)`: the size checks, the
   metadata cap, the tier and the byte gate, which **waits** for demotions to
   free room when the fast tier is full, at most until the deadline. The value
   is still in the kernel's socket buffer meanwhile, so TCP flow control slows
   that one client and no cache DRAM is held for it;
4. `fill()` allocates the value in the tier the permit chose, uninitialized, and
   the value is read off the socket straight into it (`read_exact_from`, with
   `SO_RCVTIMEO` armed), so a slow-tier value is never zero-filled first;
5. the TTL is read, `set_ttl`, `commit()`.

`set_timeout` (config key, `--set-timeout`; milliseconds, 1 to 86400000;
default 5000) is the deadline in step 3 and the receive timeout in step 4 and
for the TTL. The byte gate's own behaviour is the cache's, configured by its
`PAPER_GATE_*` environment variables (`PAPER_GATE_MODE`, `_STALL_WINDOW_MS`,
`_ON_STALL`, `_ON_METADATA_OVERFLOW`, `_VALUE_HINT_BYTES`, ...; see the
cache's README, "Environment variables"); the server passes it no `GateConfig`,
so the environment applies. The `all_dram` build has no gate and no permit: it
reads the value into a buffer and calls `set`, as upstream does, and has no
timeout.

What a failure does:

* **The cache refuses the set** (`reserve_set` errs): nothing is allocated. The
  value and the TTL are read and discarded through a 16 KiB scratch buffer, so
  the connection stays in step, and then the refusal is the reply. If the skip
  itself fails (the client stalls, or hangs up) the connection is closed
  instead.
* **The value or the TTL does not arrive whole** (timeout, hang-up, reset): the
  pending set is dropped, which frees its allocation and refunds what it was
  charged, exactly once, and the connection is closed (see "Known limits" for
  what the timeout does not bound).
* **A client that has not authorized** has its SET consumed the same way and is
  answered with server error 3.

## Wire additions

The protocol is upstream's. This fork adds:

| What | Code | Meaning |
|---|---|---|
| cache error | **8** `FastTierStalled` | The fast tier stayed full for as long as the set would wait: the gate's stall window passed with nothing freed, or `set_timeout` ran out first. Nothing was allocated or stored; retrying later may succeed. |
| cache error | **9** `MetadataOverflow` | A new key's metadata would not fit the fast tier (`PAPER_GATE_METADATA_FLOOR_BYTES`, or the tier is smaller than the cache's own structures: the merged store's fill a fast tier of a few MiB, and under the default measured metadata model it refuses new keys there after a few dozen; `PAPER_GATE_METADATA_MODEL=per_object` is the setting for toy scales). Overwrites, gets and deletes continue. |
| cache error | 7 `PolicyNotImplemented` | The merged store does not implement the policy. Only ever reported at start-up, never on the wire; kept for parity with the cache repo's old server. |
| command | **200** | The self-stats report as one text buffer (see above). |

A cache error is `[?][0][code]`: false, then 0 meaning "a cache error code
follows", then the code. 8 and 9 have the same shape as upstream's 1 to 6, so a
stock client stays in step; **it does not know them**: its `from_code` ends in
`_ => Internal`, and it reports 7, 8 and 9 as an internal error. A client that
wants to tell a refused SET from a bug has to know the codes. Every error the
cache can return is given a code explicitly (`src/error.rs`), so no variant it
adds can pass as "no code".

A refused SET is a complete exchange: the whole request, value and TTL, is
consumed before the reply is written, so the next command on the connection
parses. A SET that is *abandoned* gets no reply: its connection is closed.

## Known limits

* `SO_RCVTIMEO` bounds each read of a SET's value, not the whole value. A client
  that delivers a byte just inside every timeout keeps its bytes charged for as
  long as it keeps dribbling: `set_timeout` as a deadline covers only the wait
  for admission, before the value is read.
* `fill()` charges the allocation before the first byte of the value arrives, so
  a client that has sent its key and length and gone quiet pins the value's
  length in fast bytes for one `set_timeout`.
* A merged-store build with a toy fast tier refuses new keys (cache error 9): the
  store's own structures fill a tier of a few MiB under the default measured
  metadata model, after a few dozen keys. Give it a fast tier of at least about
  8 MiB, or set `PAPER_GATE_METADATA_MODEL=per_object`.
* The Dockerfile is upstream's and is not maintained for the tiered build, which
  needs a nightly toolchain and the cache's Git dependency.
* Lengths on the wire are trusted before the bytes arrive: a key, an AUTH token
  or a policy string is read into a `vec![0; declared_len]`, as upstream does
  (paper-utils), so a client can declare a multi-GiB one. The flat build reads a
  value the same way.
* STATUS is upstream's frame, carrying this cache's own policy names
  (`lru-compact-hybrid`, `2q-compact-hybrid-0.2`, ...: `handle_status` in
  `src/server.rs`). The stock `paper-client` 1.11.0 parses each one as a name of
  its own (`lru`, `2q-<in>-<out>`, `s3-fifo-<ratio>`, ...: `PaperPolicy::from_str`
  in its `src/policy.rs`) and rejects any other, which it takes for a broken
  connection: `status()` reconnects and asks again, up to three times
  (`RECONNECT_MAX_ATTEMPTS`), and then fails with `Disconnected`
  (`process_status` in its `src/client.rs`). A client that needs STATUS has to
  parse those names itself; the self-stats report (command 200) is the
  tier-aware status.
* POLICY is not supported. It is answered with cache error 6 (`InvalidPolicy`),
  whatever policy it names (`handle_policy` in `src/server.rs`): a tiered
  stack's fast/slow split cannot be carried over to another design, so the
  design is chosen once, at start-up, and changing it means restarting the
  server. `policies[]` in a config file is ignored, with a warning.
* A zero-length value is stored. The cache raises `ZeroValueSize` (cache error 2)
  only for an object whose accounted size is 0, and no object's is: its key and
  its bookkeeping count. So a SET of 0 bytes succeeds and reads back as 0 bytes,
  on every build (`scripts/probe_server.py basic` sets and gets one), and this
  server never sends error 2.

## Testing

`cargo test --release` (with the features the build was made with) runs the unit
tests, and, for the tiered build, `tests/set_path.rs`: it starts the binary on a
port of its own and drives the SET path over a socket in hand-built frames -- a
value read into the cache byte-exact, a value too big for the cache skipped, a
client that stalls mid-value closed and its fast bytes refunded, a SET refused
with cache error 8 and another with 9 and the connection still in step, an
unauthorized SET consumed, and SETs in flight counted as setters only when the
cache's byte gate runs and its value hint is above 0. `SET_PATH_TEST_LOGS=<dir>`
keeps each server's log.

The wire tests start the binary Cargo built for the run: with `--release` that
is the release binary, `panic = "abort"` included, and without it a dev-profile
binary, which unwinds. The unit tests are built to unwind either way (Cargo
ignores the setting for tests).

`scripts/probe_server.py` is the same kind of probe for a server you started
yourself, with the wire's every command, binary keys up to 250 bytes, values over
1 MiB and the self-stats report; see its header for the servers each scenario
needs.
