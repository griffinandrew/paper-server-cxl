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
`auth_token`; `policies[]` is accepted and ignored. `fast_tier_size` is the
fork's key: the fast (DRAM) tier's byte budget, and the rest of `max_size` is
the slow tier.
