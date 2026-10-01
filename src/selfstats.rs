/*
 * Copyright (c) Kia Shakiba
 *
 * This source code is licensed under the GNU AGPLv3 license found in the
 * LICENSE file in the root directory of this source tree.
 */

//! The server's own view of what the cache costs, excluding the socket.
//!
//! A client can only ever time a round trip: two syscalls, the protocol frames,
//! a copy each way and the TCP stack, wrapped around an operation the
//! in-process benchmark resolves at a few hundred nanoseconds. Those numbers
//! answer "what does a deployed cache cost", which is a fair question but not
//! the same question, and they cannot be compared against an in-process result.
//!
//! So the server times the cache call and nothing else -- `Instant::now()`
//! immediately before `cache.get(..)` and `elapsed()` immediately after, with
//! the frame already parsed and the response not yet written. What is left out
//! is deliberate: reading the request, parsing the key, writing the reply and
//! flushing it are all outside the span.
//!
//! The counters are per-command and lock-free, so instrumenting costs two
//! `Instant::now()` calls and three relaxed atomic adds on the hot path. That
//! is not nothing at this timescale (order 20-40 ns on x86), and it is charged
//! to the server, not to the cache: it sits outside the timed span except for
//! the clock reads themselves, which bracket it. Treat the reported mean as the
//! cache call plus one clock read, and compare like with like.
//!
//! # Reading the SET figure
//!
//! A cache that is still FILLING charges every set a first-touch page fault per
//! value page: `fast_alloc` hands back memory the process has never touched,
//! while `get`'s `to_vec` recycles a hot tcache block. Measured at 4 KiB
//! values -- 3935 ns/set while filling, 1087 ns once keys are overwritten and
//! memory recycles, against 1154 ns for a get. So set is not slower than get;
//! filling is, by 3.6x. Warm to steady state before believing a set number.
//!
//! # The report
//!
//! `render` is the text command byte 200 returns and `--stats-interval` prints
//! to stderr. The protocol's STATUS frame has no room for the tier fields, so a
//! stock client cannot see promotions, demotions or the fast/slow split at all;
//! this is where they are. `run_mem.py` (the benchmark's memory run) reads it
//! with line-anchored regular expressions, so a section is only ever APPENDED
//! and no line of it may start with a word one of them anchors on -- see
//! `tests::READER_KEYWORDS`.

use std::{
	fmt::Write as _,
	io,
	sync::atomic::{AtomicU64, Ordering},
	time::Instant,
};

use paper_cache::CacheError;
use paper_utils::command::CommandByte;

use crate::server::Cache;

/// Command bytes run 0..=13, so one slot each covers every command.
pub const SLOTS: usize = 16;

/// Log-ish buckets: exact below 16 ns, then four sub-buckets per octave,
/// i.e. ~25% resolution. Enough for percentiles without a lock or a
/// reservoir, and the MEAN is exact regardless (count and total are).
pub const BUCKETS: usize = 256;

/// Slot for GET misses, outside the command range so it needs no command byte.
///
/// A miss does no copy and no allocation, so folding it into the GET average
/// deflates that average in proportion to the miss ratio. The in-process
/// benchmark times only hits (`handle_read_through` calls `store_get_time`
/// solely on `Ok`), and these figures exist to be compared against those.
pub const SLOT_GET_MISS: u8 = 14;

/// Slot for SETs the cache refused to admit (cache errors 8, 9 and the size
/// checks): the time from the request to the refusal, which for a set that
/// waited at the byte gate is the wait. A refused set is not a set -- nothing
/// was allocated or inserted -- so it must not share the `set` average either.
pub const SLOT_SET_REFUSED: u8 = 15;

pub struct SelfStats {
	count: [AtomicU64; SLOTS],
	total_ns: [AtomicU64; SLOTS],
	hist: Vec<AtomicU64>,

	/// How the SETs that went through a permit ended (S9), reported under SET
	/// ADMISSION. Plain counters, bumped on paths that are not the hit path.
	set_committed: AtomicU64,
	set_commit_refused: AtomicU64,
	refused_stalled: AtomicU64,
	refused_metadata: AtomicU64,
	refused_other: AtomicU64,
	body_timeouts: AtomicU64,
	body_aborts: AtomicU64,
	skip_failures: AtomicU64,
	bytes_skipped: AtomicU64,
}

impl SelfStats {
	pub fn new() -> Self {
		SelfStats {
			count: std::array::from_fn(|_| AtomicU64::new(0)),
			total_ns: std::array::from_fn(|_| AtomicU64::new(0)),
			hist: (0..SLOTS * BUCKETS).map(|_| AtomicU64::new(0)).collect(),

			set_committed: AtomicU64::new(0),
			set_commit_refused: AtomicU64::new(0),
			refused_stalled: AtomicU64::new(0),
			refused_metadata: AtomicU64::new(0),
			refused_other: AtomicU64::new(0),
			body_timeouts: AtomicU64::new(0),
			body_aborts: AtomicU64::new(0),
			skip_failures: AtomicU64::new(0),
			bytes_skipped: AtomicU64::new(0),
		}
	}

	pub fn record(&self, slot: u8, nanos: u64) {
		let slot = slot as usize;

		if slot >= SLOTS {
			return;
		}

		self.count[slot].fetch_add(1, Ordering::Relaxed);
		self.total_ns[slot].fetch_add(nanos, Ordering::Relaxed);
		self.hist[slot * BUCKETS + bucket(nanos)].fetch_add(1, Ordering::Relaxed);
	}

	pub fn count(&self, slot: u8) -> u64 {
		self.count[slot as usize].load(Ordering::Relaxed)
	}

	pub fn mean_ns(&self, slot: u8) -> f64 {
		match self.count(slot) {
			0 => 0.0,
			n => self.total_ns[slot as usize].load(Ordering::Relaxed) as f64 / n as f64,
		}
	}

	/// A SET that was admitted, read and committed.
	#[cfg_attr(feature = "all_dram", allow(dead_code))]
	pub fn set_committed(&self) {
		self.set_committed.fetch_add(1, Ordering::Relaxed);
	}

	/// A SET whose value arrived whole but whose commit was refused: its TTL
	/// put it over the eviction threshold. The value was dropped, and refunded.
	#[cfg_attr(feature = "all_dram", allow(dead_code))]
	pub fn set_commit_refused(&self) {
		self.set_commit_refused.fetch_add(1, Ordering::Relaxed);
	}

	/// A SET the cache refused to admit, counted by the code it is answered with.
	#[cfg_attr(feature = "all_dram", allow(dead_code))]
	pub fn set_refused(&self, error: &CacheError) {
		let counter = match error {
			CacheError::FastTierStalled => &self.refused_stalled,
			CacheError::MetadataOverflow => &self.refused_metadata,
			_ => &self.refused_other,
		};

		counter.fetch_add(1, Ordering::Relaxed);
	}

	/// A SET abandoned while its value or TTL was being read: the receive
	/// timeout fired (a stalled client), or the client hung up or was reset.
	#[cfg_attr(feature = "all_dram", allow(dead_code))]
	pub fn body_failed(&self, error: &io::Error) {
		let counter = match error.kind() {
			io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut => &self.body_timeouts,
			_ => &self.body_aborts,
		};

		counter.fetch_add(1, Ordering::Relaxed);
	}

	/// The value of a refused set, read and discarded to keep the connection in
	/// step.
	pub fn skipped(&self, bytes: u64) {
		self.bytes_skipped.fetch_add(bytes, Ordering::Relaxed);
	}

	/// A refused set whose value could not be skipped (the client stalled or
	/// hung up): the connection was closed.
	pub fn skip_failed(&self) {
		self.skip_failures.fetch_add(1, Ordering::Relaxed);
	}

	/// Lower bound of the bucket the requested quantile falls in. Reported
	/// as a bound rather than an interpolated value because the buckets are
	/// ~25% wide and interpolating would imply precision the histogram does
	/// not have.
	pub fn quantile_ns(&self, slot: u8, q: f64) -> u64 {
		let total = self.count(slot);

		if total == 0 {
			return 0;
		}

		let target = (total as f64 * q).ceil() as u64;
		let base = slot as usize * BUCKETS;
		let mut seen = 0u64;

		for b in 0..BUCKETS {
			seen += self.hist[base + b].load(Ordering::Relaxed);

			if seen >= target {
				return bucket_low(b);
			}
		}

		bucket_low(BUCKETS - 1)
	}
}

pub fn bucket(ns: u64) -> usize {
	if ns < 16 {
		return ns as usize;
	}

	let e = 63 - ns.leading_zeros() as usize;
	let sub = ((ns >> (e - 2)) & 0b11) as usize;

	(16 + (e - 4) * 4 + sub).min(BUCKETS - 1)
}

pub fn bucket_low(b: usize) -> u64 {
	if b < 16 {
		return b as u64;
	}

	let e = (b - 16) / 4 + 4;
	let sub = ((b - 16) % 4) as u64;

	(1u64 << e) | (sub << (e - 2))
}

/// Times ONE cache call and nothing around it. The request is already parsed
/// and the response is not yet written when this runs.
pub fn timed<T>(stats: &SelfStats, slot: u8, call: impl FnOnce() -> T) -> T {
	let started = Instant::now();
	let out = call();

	stats.record(slot, started.elapsed().as_nanos() as u64);

	out
}

/// The text report: the latency table, then the cache's own figures and the
/// tier sections. Sections are only appended, never reordered.
pub fn render(stats: &SelfStats, cache: &Cache) -> String {
	let mut out = String::new();

	out.push_str("*** SERVER-SIDE CACHE LATENCY (socket excluded) ***\n\n");
	out.push_str("op      count            mean       p50       p90       p99      p999\n");

	for (name, slot) in [
		("get(hit)", CommandByte::GET),
		("get(miss)", SLOT_GET_MISS),
		("set", CommandByte::SET),
		("set(refused)", SLOT_SET_REFUSED),
		("del", CommandByte::DEL),
		("has", CommandByte::HAS),
		("peek", CommandByte::PEEK),
		("ttl", CommandByte::TTL),
		("size", CommandByte::SIZE),
	] {
		let n = stats.count(slot);

		if n == 0 {
			continue;
		}

		let _ = writeln!(
			out,
			"{name:<6} {n:>10} {:>13.1}ns {:>7}ns {:>7}ns {:>7}ns {:>7}ns",
			stats.mean_ns(slot),
			stats.quantile_ns(slot, 0.50),
			stats.quantile_ns(slot, 0.90),
			stats.quantile_ns(slot, 0.99),
			stats.quantile_ns(slot, 0.999),
		);
	}

	out.push_str("\npercentiles are bucket LOWER BOUNDS (~25% wide); the mean is exact.\n");
	out.push_str(
		"get(hit) is the figure comparable with the in-process benchmark, which times\n\
		 only hits. A miss neither copies nor allocates, so averaging the two together\n\
		 would understate the cost of a get by the miss ratio.\n\
		 A set on a FILLING cache pays a first-touch page fault per value page (3.6x at\n\
		 4 KiB); warm to steady state before comparing set against get.\n",
	);

	// The tier figures are the whole point of reporting here rather than over
	// the wire: the protocol's STATUS frame has no room for them.
	if let Ok(status) = cache.status() {
		let _ = writeln!(out, "\n*** CACHE ***\n");
		let _ = writeln!(out, "objects        {}", status.num_objects());
		let _ = writeln!(out, "used size      {} B", status.used_size());
		let _ = writeln!(out, "max size       {} B", status.max_size());
		let _ = writeln!(out, "miss ratio     {:.4}", status.miss_ratio());
		let _ = writeln!(
			out,
			"gets/sets/dels {}/{}/{}",
			status.total_gets(),
			status.total_sets(),
			status.total_dels(),
		);
		let _ = writeln!(out, "rss            {} B (hwm {} B)", status.rss(), status.hwm());
	}

	#[cfg(feature = "tiered")]
	{
		let tier = cache.hybrid_stats();

		let _ = writeln!(out, "\n*** TIERS ***\n");
		let _ = writeln!(out, "fast tier size {} B", cache.fast_tier_size());
		let _ = writeln!(
			out,
			"fast           {} objects, {} B",
			tier.fast_objects, tier.fast_bytes_used,
		);
		let _ = writeln!(
			out,
			"               + {} B reserved for per-object metadata",
			tier.fast_metadata_bytes,
		);
		let _ = writeln!(
			out,
			"slow           {} objects, {} B",
			tier.slow_objects, tier.slow_bytes_used,
		);
		let _ = writeln!(out, "promotions     {}", tier.promotions);
		let _ = writeln!(out, "demotions      {}", tier.demotions);
		let _ = writeln!(out, "evictions      {}", tier.evictions);
	}

	// A flat build has no tiers to report, and says so rather than printing
	// a table of zeroes that reads like a tiered run that never migrated.
	#[cfg(feature = "all_dram")]
	{
		let _ = writeln!(out, "\n*** TIERS ***\n");
		let _ = writeln!(
			out,
			"flat all-DRAM build: no tiers, no migrations, nothing on the slow node.",
		);
	}

	// SHADOW. What the fitted constants predict, beside what the allocator
	// actually handed out. Nothing reads these to make a decision -- the whole
	// point is to characterise the drift before anything depends on it.
	//
	// Expect measured > modelled on DRAM, and by a knowable amount: the counter
	// sees every node-0 allocation in the process, which is the object map's
	// bucket arrays, the eviction-stack arenas, the value headers, ghost
	// entries, the expiry index AND this server's own per-connection buffers.
	// The modelled figure is `fast_bytes_used + fast_metadata_bytes`, where the
	// second term is a per-object constant times the tracked count. The gap
	// between them IS the question.
	#[cfg(feature = "measured_accounting")]
	{
		use paper_cache::numa_alloc::measured;

		let measured_dram = measured::dram_allocated();
		let measured_slow = measured::slow_allocated();

		// The modelled side is shape-dependent; the measured side is not,
		// which is exactly why the two arms are comparable at all. For a flat
		// build the model IS `used_size`, and the slow pool must read zero --
		// if it does not, something placed bytes off-node and the build is not
		// the all-DRAM baseline it claims to be.
		#[cfg(feature = "tiered")]
		let (modelled_dram, modelled_slow, tracked, object_bytes, metadata_bytes) = {
			let tier = cache.hybrid_stats();

			(
				tier.fast_bytes_used + tier.fast_metadata_bytes,
				tier.slow_bytes_used,
				tier.fast_objects + tier.slow_objects,
				tier.fast_bytes_used,
				tier.fast_metadata_bytes,
			)
		};

		#[cfg(feature = "all_dram")]
		let (modelled_dram, modelled_slow, tracked, object_bytes, metadata_bytes) = {
			let (objects, used) = cache
				.status()
				.map(|status| (status.num_objects(), status.used_size()))
				.unwrap_or((0, 0));

			(used, 0u64, objects, used, 0u64)
		};

		let _ = writeln!(out, "\n*** MEASURED vs MODELLED (nothing acts on this) ***\n");
		let _ = writeln!(
			out,
			"{:<10} {:>18} {:>18} {:>14} {:>8}",
			"pool", "modelled B", "measured B", "drift B", "ratio",
		);

		for (name, modelled, measured_bytes) in [
			("dram", modelled_dram, measured_dram),
			("slow", modelled_slow, measured_slow),
		] {
			let drift = measured_bytes as i64 - modelled as i64;
			let ratio = match modelled {
				0 => 0.0,
				m => measured_bytes as f64 / m as f64,
			};

			let _ = writeln!(
				out,
				"{name:<10} {modelled:>18} {measured_bytes:>18} {drift:>+14} {ratio:>8.3}",
			);
		}

		// Per-object, which is the form the fitted constants are written in and
		// therefore the only form in which the two are directly comparable.
		if tracked > 0 {
			let _ = writeln!(
				out,
				"\nper tracked object ({tracked}): modelled metadata {:.1} B, \
				 measured DRAM less object bytes {:.1} B",
				metadata_bytes as f64 / tracked as f64,
				(measured_dram as f64 - object_bytes as f64) / tracked as f64,
			);
		}

		let _ = writeln!(
			out,
			"\nmeasured counts EVERY node-0 allocation in this process, including \
			 this\nserver's own connection buffers -- it is an upper bound on the \
			 cache's DRAM,\nnot an attribution of it.",
		);
	}

	// PHYS_FAST (`paper_cache::phys`) and the budget it is measured against,
	// APPENDED after every existing line so no reader's anchor or position
	// moves. P and its peak are process-global; the server runs one tiered
	// cache and no flat one, which `tiered caches` and `flat caches` confirm.
	// Reporting only.
	#[cfg(feature = "tiered")]
	render_physical_fast_tier(&mut out, &cache.hybrid_stats());

	// The migration statistics, which are this cache's own since the cache
	// was built (they were process-global statics until the cache's S8), and
	// the capacity passes the eviction watermark armed. Appended after the
	// physical section for the same reason that one was appended.
	#[cfg(feature = "tiered")]
	render_migrations(&mut out, &cache.hybrid_stats());

	// How SETs fared at the byte gate, and the server's own counts of the ones
	// it refused or abandoned. Last: it is the newest section.
	#[cfg(feature = "tiered")]
	render_set_admission(&mut out, stats, &cache.hybrid_stats(), cache.live_setters());

	out
}

/// The PHYSICAL FAST TIER section, from one `HybridStats` snapshot: a
/// function of its own so it is unit-tested without a cache or a socket.
///
/// None of its lines starts with a word the existing readers anchor on
/// (run_mem.py's `^fast\s+\d+ objects`, `^slow\s+`, `^dram\s+`,
/// `^promotions`, ...; the tests below check all fourteen). `flat caches`
/// counts the flat caches whose values are fast -- the only flat caches whose
/// values P counts. `metadata` (S5a, appended last) is M, the bytes the
/// cache's own DRAM metadata structures hold, measured; the modelled
/// reservation stays where it was, under TIERS.
#[cfg(feature = "tiered")]
fn render_physical_fast_tier(out: &mut String, tier: &paper_cache::HybridStats) {
	let _ = writeln!(out, "\n*** PHYSICAL FAST TIER (reporting only) ***\n");
	let _ = writeln!(
		out,
		"phys fast      {} B (peak >= {} B)",
		tier.phys_fast_bytes, tier.phys_fast_bytes_max,
	);
	let _ = writeln!(out, "eff fast cap   {} B", tier.effective_fast_capacity);
	let _ = writeln!(out, "over budget    {} B*s", tier.over_budget_byte_seconds);
	let _ = writeln!(out, "hits fast/slow {}/{}", tier.fast_hits, tier.slow_hits);
	let _ = writeln!(out, "tiered caches  {}", tier.live_tiered_caches);
	let _ = writeln!(out, "flat caches    {} (fast values)", tier.live_flat_fast_caches);
	let _ = writeln!(
		out,
		"metadata       {} B measured (eff {} B; slow node {} B)",
		tier.dram_metadata_bytes, tier.effective_fast_capacity_measured, tier.slow_metadata_bytes,
	);
}

/// The MIGRATIONS AND CAPACITY PASSES section, from one `HybridStats` snapshot.
///
/// The counts are the policy stack's DECISIONS and the migration queue's
/// dispositions, not byte copies (the cache's README, "Migration counters vs
/// physical copies"): `migrations` splits every queue entry into the four ways
/// one ends. A capacity pass is an eviction pass the cache-wide threshold
/// armed (`EVICTION_HIGH_WATERMARK` of `max_size`), so under the default one
/// threshold `capacity passes` is nearly the number of sets that found the
/// cache full. No line starts with a word `run_mem.py` anchors on.
#[cfg(feature = "tiered")]
fn render_migrations(out: &mut String, tier: &paper_cache::HybridStats) {
	let _ = writeln!(out, "\n*** MIGRATIONS AND CAPACITY PASSES (reporting only) ***\n");
	let _ = writeln!(
		out,
		"capacity passes {} armed, evicting {} objects, {} B",
		tier.capacity_passes, tier.capacity_pass_evictions, tier.capacity_pass_bytes,
	);
	let _ = writeln!(
		out,
		"migration queue deepest {}, largest batch {}",
		tier.queue_depth_max, tier.burst_max,
	);
	let _ = writeln!(
		out,
		"pending demote  {} now (max {}), pending promote {} now (max {}), net max {}",
		tier.pending_demote,
		tier.pending_demote_max,
		tier.pending_promote,
		tier.pending_promote_max,
		tier.pending_net_max,
	);
	let _ = writeln!(
		out,
		"migrations      {} applied, {} gone, {} declined, {} superseded",
		tier.mig_applied, tier.mig_gone, tier.mig_declined, tier.mig_superseded,
	);
	let _ = writeln!(
		out,
		"reconcile       queued {} to fast and {} to slow, applied {} and {}",
		tier.reconcile_queued_to_fast,
		tier.reconcile_queued_to_slow,
		tier.reconcile_applied_to_fast,
		tier.reconcile_applied_to_slow,
	);
	let _ = writeln!(out, "erase fallbacks {}", tier.erase_fallbacks);
}

/// The SET ADMISSION section: the server's own counts of how SETs through a
/// permit ended, beside the byte gate's figures from `HybridStats`.
///
/// `live setters` is the number of SETs in flight right now (a setter is
/// registered per SET, not per connection, so an idle connection counts for
/// nothing). `refused` is what the server answered with codes 8 and 9 and the
/// size checks; `gate stalls` is the gate's own count of times its watchdog
/// found nothing freed, which is not the same quantity (a SET can also be
/// refused when its deadline passes while the gate still sees progress). No
/// line starts with a word `run_mem.py` anchors on.
#[cfg(feature = "tiered")]
fn render_set_admission(out: &mut String, stats: &SelfStats, tier: &paper_cache::HybridStats, live_setters: u32) {
	let load = |counter: &AtomicU64| counter.load(Ordering::Relaxed);

	let stalled = load(&stats.refused_stalled);
	let metadata = load(&stats.refused_metadata);
	let other = load(&stats.refused_other);

	let _ = writeln!(out, "\n*** SET ADMISSION (reporting only) ***\n");
	let _ = writeln!(
		out,
		"permit sets    {} committed, {} refused at commit",
		load(&stats.set_committed),
		load(&stats.set_commit_refused),
	);
	let _ = writeln!(
		out,
		"refused        {} ({stalled} fast tier stalled [8], {metadata} metadata overflow [9], {other} other)",
		stalled + metadata + other,
	);
	let _ = writeln!(out, "body timeouts  {}", load(&stats.body_timeouts));
	let _ = writeln!(out, "body aborts    {}", load(&stats.body_aborts));
	let _ = writeln!(out, "skip failures  {}", load(&stats.skip_failures));
	let _ = writeln!(out, "bytes skipped  {} B", load(&stats.bytes_skipped));
	let _ = writeln!(out, "live setters   {live_setters}");
	let _ = writeln!(
		out,
		"gate           {:?}, {} waits ({:.1} ms waited, {:.1} ms longest)",
		tier.gate_state,
		tier.gate_waits,
		tier.gate_wait_ns_total as f64 / 1e6,
		tier.gate_wait_ns_max as f64 / 1e6,
	);
	let _ = writeln!(
		out,
		"gate stalls    {} watchdog stalls, {} refusals; metadata overflows {}",
		tier.gate_stalls, tier.gate_stall_errors, tier.metadata_overflows,
	);
	let _ = writeln!(
		out,
		"waiters        {} waiting now (at most {}), {} B reserved",
		tier.waiters, tier.max_waiters, tier.reserved_bytes,
	);
	let _ = writeln!(
		out,
		"levels         settle {} B, near {} B, close {} B",
		tier.band_s, tier.band_n, tier.band_b,
	);
}

#[cfg(all(test, feature = "tiered"))]
mod tests {
	use paper_cache::{CacheTierSize, GateConfig, MetadataModel, PaperPolicy};

	use super::*;

	/// The fourteen patterns run_mem.py reads the self-stats with, reduced to
	/// what a line must START with to match one: thirteen are a keyword and
	/// then whitespace (`^fast\s+`, `^slow\s+`, `^used size\s+`, ...; `fast`
	/// and `slow` open two and three of them), and the fourteenth is leading
	/// whitespace and a `+`. A section line starting like any of them could be
	/// read as that figure.
	const READER_KEYWORDS: [&str; 10] = [
		"objects", "used size", "max size", "fast", "slow", "promotions",
		"demotions", "evictions", "rss", "dram",
	];

	fn read_by_run_mem(line: &str) -> bool {
		let keyword = READER_KEYWORDS.iter().any(|keyword| {
			line.strip_prefix(keyword)
				.is_some_and(|rest| rest.starts_with(char::is_whitespace))
		});

		keyword || line.trim_start().starts_with('+')
	}

	#[test]
	fn the_physical_fast_tier_section_prints_each_reading_under_its_own_label() {
		let tier = paper_cache::HybridStats {
			phys_fast_bytes: 697_344,
			phys_fast_bytes_max: 3_072_000,
			effective_fast_capacity: 815_104,
			over_budget_byte_seconds: 2_369_712,
			fast_hits: 429,
			slow_hits: 7,
			live_tiered_caches: 1,
			live_flat_fast_caches: 2,
			dram_metadata_bytes: 131_072,
			effective_fast_capacity_measured: 684_032,
			slow_metadata_bytes: 4_096,
			// Everything the section must NOT print: distinct values, so a
			// line reading the wrong field shows up in the comparison.
			fast_bytes_used: 11,
			slow_bytes_used: 13,
			fast_objects: 17,
			slow_objects: 19,
			fast_metadata_bytes: 23,
			..paper_cache::HybridStats::default()
		};

		let mut out = String::new();
		render_physical_fast_tier(&mut out, &tier);

		assert_eq!(
			out,
			"\n*** PHYSICAL FAST TIER (reporting only) ***\n\n\
			 phys fast      697344 B (peak >= 3072000 B)\n\
			 eff fast cap   815104 B\n\
			 over budget    2369712 B*s\n\
			 hits fast/slow 429/7\n\
			 tiered caches  1\n\
			 flat caches    2 (fast values)\n\
			 metadata       131072 B measured (eff 684032 B; slow node 4096 B)\n",
		);

		for line in out.lines() {
			assert!(
				!read_by_run_mem(line),
				"run_mem.py would read {line:?} as one of its figures",
			);
		}

		// The matcher itself, on the lines it exists to find.
		for anchored in [
			"fast           3 objects, 4 B",
			"slow           0 objects, 0 B",
			"               + 5 B reserved for per-object metadata",
			"promotions     1",
			"dram                        1                  2",
			"rss            7 B (hwm 8 B)",
		] {
			assert!(read_by_run_mem(anchored), "{anchored:?} is a line run_mem.py reads");
		}
	}

	#[test]
	fn the_migrations_section_prints_each_reading_under_its_own_label() {
		let tier = paper_cache::HybridStats {
			capacity_passes: 101,
			capacity_pass_evictions: 103,
			capacity_pass_bytes: 107,
			queue_depth_max: 109,
			burst_max: 113,
			pending_demote: 127,
			pending_demote_max: 131,
			pending_promote: 137,
			pending_promote_max: 139,
			pending_net_max: 149,
			mig_applied: 151,
			mig_gone: 157,
			mig_declined: 163,
			mig_superseded: 167,
			reconcile_queued_to_fast: 173,
			reconcile_queued_to_slow: 179,
			reconcile_applied_to_fast: 181,
			reconcile_applied_to_slow: 191,
			erase_fallbacks: 193,
			// Figures the section must NOT print.
			promotions: 3,
			demotions: 5,
			evictions: 7,
			..paper_cache::HybridStats::default()
		};

		let mut out = String::new();
		render_migrations(&mut out, &tier);

		assert_eq!(
			out,
			"\n*** MIGRATIONS AND CAPACITY PASSES (reporting only) ***\n\n\
			 capacity passes 101 armed, evicting 103 objects, 107 B\n\
			 migration queue deepest 109, largest batch 113\n\
			 pending demote  127 now (max 131), pending promote 137 now (max 139), net max 149\n\
			 migrations      151 applied, 157 gone, 163 declined, 167 superseded\n\
			 reconcile       queued 173 to fast and 179 to slow, applied 181 and 191\n\
			 erase fallbacks 193\n",
		);

		for line in out.lines() {
			assert!(!read_by_run_mem(line), "run_mem.py would read {line:?} as one of its figures");
		}
	}

	#[test]
	fn the_set_admission_section_counts_what_the_server_did_with_each_set() {
		let stats = SelfStats::new();

		stats.set_committed();
		stats.set_committed();
		stats.set_commit_refused();
		stats.set_refused(&CacheError::FastTierStalled);
		stats.set_refused(&CacheError::FastTierStalled);
		stats.set_refused(&CacheError::MetadataOverflow);
		stats.set_refused(&CacheError::ExceedingValueSize);
		stats.body_failed(&io::ErrorKind::WouldBlock.into());
		stats.body_failed(&io::ErrorKind::TimedOut.into());
		stats.body_failed(&io::ErrorKind::UnexpectedEof.into());
		stats.body_failed(&io::ErrorKind::ConnectionReset.into());
		stats.body_failed(&io::ErrorKind::BrokenPipe.into());
		stats.skip_failed();
		stats.skipped(1_000);
		stats.skipped(24);

		let tier = paper_cache::HybridStats {
			gate_waits: 7,
			gate_wait_ns_total: 12_500_000,
			gate_wait_ns_max: 3_240_000,
			gate_stalls: 11,
			gate_stall_errors: 13,
			metadata_overflows: 17,
			waiters: 19,
			max_waiters: 23,
			reserved_bytes: 29,
			band_s: 31,
			band_n: 37,
			band_b: 41,
			..paper_cache::HybridStats::default()
		};

		let mut out = String::new();
		render_set_admission(&mut out, &stats, &tier, 3);

		assert_eq!(
			out,
			"\n*** SET ADMISSION (reporting only) ***\n\n\
			 permit sets    2 committed, 1 refused at commit\n\
			 refused        4 (2 fast tier stalled [8], 1 metadata overflow [9], 1 other)\n\
			 body timeouts  2\n\
			 body aborts    3\n\
			 skip failures  1\n\
			 bytes skipped  1024 B\n\
			 live setters   3\n\
			 gate           Off, 7 waits (12.5 ms waited, 3.2 ms longest)\n\
			 gate stalls    11 watchdog stalls, 13 refusals; metadata overflows 17\n\
			 waiters        19 waiting now (at most 23), 29 B reserved\n\
			 levels         settle 31 B, near 37 B, close 41 B\n",
		);

		for line in out.lines() {
			assert!(!read_by_run_mem(line), "run_mem.py would read {line:?} as one of its figures");
		}
	}

	/// The section is wired to the cache: appended after every other section,
	/// and reading this process's one tiered cache.
	///
	/// The only test in this binary that builds a cache: the fast-byte counter
	/// it reads is process-wide, and a second cache alive at once would make
	/// `tiered caches` read 2.
	#[test]
	fn self_stats_carry_the_physical_fast_tier_of_the_one_cache() {
		// The per-object metadata model (S5): this test is not about the
		// model, and a 64 KiB tier is smaller than the cache's own empty
		// structures -- under the measured model's key ceiling it would refuse
		// every key.
		let mut gate = GateConfig::default();
		gate.metadata_model = MetadataModel::PerObject;

		let cache = Cache::new_with_gate(
			1 << 20,
			CacheTierSize::Bytes(64 << 10),
			PaperPolicy::LruCompactHybrid,
			gate,
		)
		.expect("a tiered cache");

		let key: Box<[u8]> = Box::from(&b"key"[..]);

		cache.set(key.clone(), &[1u8; 1_000], None).expect("set");
		cache.get(&key).expect("a hit");

		let full = render(&SelfStats::new(), &cache);

		let (before, rest) = full
			.rsplit_once("\n*** PHYSICAL FAST TIER (reporting only) ***\n\n")
			.expect("the section is present");

		assert!(before.contains("*** TIERS ***"), "it comes after the tier section");

		// The section runs to the next header, and the migrations section is
		// appended after it.
		let (section, later) = rest.split_once("\n*** ").expect("a section follows it");

		assert!(later.starts_with("MIGRATIONS AND CAPACITY PASSES"), "{later}");
		assert!(later.contains("\n*** SET ADMISSION (reporting only) ***\n"), "{later}");

		let labels: Vec<&str> = section.lines().map(|line| &line[..15]).collect();

		assert_eq!(
			labels,
			[
				"phys fast      ", "eff fast cap   ", "over budget    ",
				"hits fast/slow ", "tiered caches  ", "flat caches    ",
				"metadata       ",
			],
		);

		assert!(section.contains("hits fast/slow 1/0\n"), "the hit was served from DRAM: {section}");
		assert!(section.contains("tiered caches  1\n"), "{section}");
		assert!(section.contains("flat caches    0 (fast values)\n"), "{section}");

		// Every figure run_mem.py reads is on one line, once. `fast` and `slow`
		// also open a line of the MEASURED table, which it tells apart by what
		// follows the keyword, so those two are counted by their object count.
		for (anchor, tail) in [
			("objects ", ""), ("used size ", ""), ("max size ", ""), ("fast ", " objects, "),
			("slow ", " objects, "), ("promotions ", ""), ("demotions ", ""),
			("evictions ", ""), ("rss ", " B"),
		] {
			let n = full
				.lines()
				.filter(|line| line.starts_with(anchor) && line.contains(tail))
				.count();

			assert_eq!(n, 1, "run_mem.py reads {anchor:?}, which must open exactly one line");
		}
	}

	#[test]
	fn the_histogram_keeps_the_mean_exact_and_the_percentiles_as_bounds() {
		let stats = SelfStats::new();

		for nanos in [100, 200, 300, 400] {
			stats.record(CommandByte::GET, nanos);
		}

		assert_eq!(stats.count(CommandByte::GET), 4);
		assert_eq!(stats.mean_ns(CommandByte::GET), 250.0);

		// 400 ns is in the [384, 448) bucket: reported as its lower bound.
		assert_eq!(stats.quantile_ns(CommandByte::GET, 1.0), 384);
		assert_eq!(stats.count(CommandByte::SET), 0);

		// A slot outside the table is ignored, not a panic.
		stats.record(200, 1);
	}
}
