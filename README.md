# paper-server (tiered fork)

PaperCache is an in-memory cache which supports dynamic eviction policy switching at runtime.

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

| Feature | Effect |
|---|---|
| `tiered` (default) | Serves `PaperCache<Buffer, TieredBuffer>`: values live in the fast (DRAM) or slow (CXL) tier. Enables the cache's `hybrid_cache_common` and `key_value_pmem`. |
| `all_dram` | Serves the flat `PaperCache<Buffer, BufferDRAM>` instead, the all-DRAM baseline: one tier, no admission gate, nothing on the slow node. Needs `--no-default-features`; building both, or neither, is a compile error. |
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
`auth_token`; `policies[]` is accepted and ignored, with a warning. A file given
with `--config` is not layered over the defaults (as upstream): what it leaves
out is zero or empty, so it must set `host`, `port`, `max_size`,
`fast_tier_size` and `max_connections`, and `policy` unless that is
`lru-compact-hybrid`. `fast_tier_size` is the fork's key: the fast (DRAM) tier's
byte budget, and the rest of `max_size` is the slow tier.

### Command line

Every flag is the config line it stands for, parsed as the file's line is, and
wins over the file, which wins over `default.pconf`.

| Flag | Overrides | Notes |
|---|---|---|
| `--bind <ADDR:PORT>` | `host`, `port` | `0.0.0.0:3145` accepts connections from other machines |
| `--max-size <BYTES>` | `max_size` | a byte count, or with a suffix (`2GiB`) |
| `--fast-tier-size <BYTES>` | `fast_tier_size` | tiered build only; at most `max_size` |
| `--policy <POLICY>` | `policy` | e.g. `lru-compact-hybrid`, `s3-fifo-faithful-compact-hybrid-0.1` |
| `--auth <TOKEN>` | `auth_token` | clients must send it with AUTH before any other command |
| `--stats-interval <S>` | | print the self-stats report to stderr every S seconds |
| `--config <FILE>`, `--log-config <FILE>` | | as upstream |

`--bind`, `--max-size`, `--fast-tier-size` and `--policy` are the flags the
benchmark's `run_mem.py` launches a server with, so pointing it at this binary
needs a different `--server` path and nothing else on the server side.

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

`run_mem.py` reads the report with line-anchored regular expressions
(`^fast\s+\d+ objects`, `^slow\s+`, `^dram\s+`, `^promotions`, ...), so no line
of a later section starts with one of those words.

PING answers `[!][len]pong`, as upstream's server does and as the stock client's
`ping()` reads it. A client that reads only the boolean (the benchmark's
`bench-client` does) leaves the buffer in the stream and loses frame sync at its
next command.
