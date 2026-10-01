/*
 * Copyright (c) Griffin Andrew
 *
 * This source code is licensed under the GNU AGPLv3 license found in the
 * LICENSE file in the root directory of this source tree.
 */

//! The SET arm: a set is admitted BEFORE its value is read.
//!
//! The wire's order is key, value, TTL. `PaperCache::set` takes the value as a
//! slice, so a server that calls it has already read the whole request body into
//! a buffer of its own -- one no budget covers -- and copies it again into the
//! cache's allocation. The tiered cache instead splits a set into steps, and
//! this arm runs them around the socket:
//!
//! ```text
//!   read the key and the value's length        (the Set command: nothing else)
//!   register_setter()                          this SET is in flight, if that can
//!        |                                     widen anything (`SetterCount`)
//!   reserve_set(key, len, None, deadline)      admission: the size checks, the
//!        |                                     metadata cap, the tier and the byte
//!        |                                     gate, which WAITS for demotions to
//!        |                                     free room, at most until `deadline`
//!        +-- Err: skip the value and the TTL, answer the code
//!        v
//!   permit.fill()                              the value's allocation, in the tier
//!        |                                     the permit decided, UNINITIALIZED
//!        v
//!   read_exact_from(socket)                    the value, straight into it
//!   read the TTL, set_ttl, commit()            published as `set` publishes
//! ```
//!
//! While the gate holds a set the value is still in the kernel's socket buffer,
//! so TCP flow control slows that one client and no cache DRAM is held for it.
//! The receive timeout (`SO_RCVTIMEO`) is armed for the value and the TTL and
//! disarmed after, because between commands a connection waits without limit.
//!
//! # Counting setters
//!
//! `register_setter()` counts a SET in flight into the byte gate's near band,
//! which the cache widens to `(concurrency_hint + live setters) x value_hint`
//! (`bands_for`, the cache's gate.rs). Two things leave a setter nothing to
//! widen:
//!
//! * a `value_hint` of 0, the default: a product with 0, whatever the count;
//! * a byte gate that is off (`PAPER_GATE_MODE=off`): the policy worker
//!   publishes bands only while the gate is enabled (the cache's
//!   worker/policy/mod.rs), so there is no near band at all.
//!
//! Each registration and each release unparks the cache's policy worker all the
//! same (`kick_policy_worker`, the cache's status.rs): two wake-ups per SET for
//! nothing. So a SET is registered only when the cache runs the gate in `Block`
//! mode with a `value_hint` above 0 (`SetterCount`), which is read once, at
//! start-up, from the configuration the cache actually runs
//! (`PaperCache::gate_config`, `PAPER_GATE_MODE` and
//! `PAPER_GATE_VALUE_HINT_BYTES` included): there is no second setting here to
//! keep in step with it.
//!
//! The other ways the gate can be disabled are not checked: the cache's
//! `Ungated` state (the faithful fast-admission designs) and `Bands` (a drain
//! target the near band cannot clear) are decided by the policy worker, and
//! are not known when the server starts: with a value hint, such a cache has
//! its SETs registered for nothing.
//!
//! # What a failure does
//!
//! * The cache refuses the set (`reserve_set` errs): nothing is allocated. The
//!   value and its TTL are read and discarded through a small scratch buffer, so
//!   the connection stays in step, and the refusal is the reply -- cache error
//!   8 for `FastTierStalled`, 9 for `MetadataOverflow`, the usual codes for the
//!   rest (`error.rs`). If skipping fails the connection is closed instead.
//! * The value or TTL does not arrive whole (timeout, hang-up, reset): the
//!   `PendingSet` is dropped, which frees the allocation and refunds what it
//!   was charged, exactly once, and the connection is closed. Its stream
//!   position is lost, so nothing more can be read from it.
//!
//! # What it does not bound
//!
//! `SO_RCVTIMEO` bounds each READ, not the whole value. A client that has sent
//! its key and length and goes quiet, or stops partway, pins at most `len` fast
//! bytes for one `set_timeout`: `fill` charges the allocation before the first
//! byte of the value arrives. One that delivers a byte just inside every timeout
//! keeps them charged for as long as it keeps doing so; bounding that takes a
//! deadline over the whole value, which this does not impose.

use std::{
	sync::Arc,
	time::{Duration, Instant},
};

use log::warn;
#[cfg(feature = "tiered")]
use paper_cache::{GateConfig, GateMode};
use paper_utils::stream::Buffer;

use crate::{
	config::Config,
	connection::Connection,
	error::ServerError,
	selfstats::SelfStats,
	server::{Cache, SheetResult},
};

/// What the SET arm needs beside the cache and the connection, fixed when the
/// server starts.
#[derive(Clone, Copy)]
pub struct SetSettings {
	/// How long a SET may wait for the cache to take it, and then for each read of
	/// its value and its TTL (`set_timeout`).
	pub timeout: Duration,

	/// Whether a SET in flight is registered with the cache as a setter, and if
	/// not, why not. See `SetterCount`.
	#[cfg(feature = "tiered")]
	pub setters: SetterCount,
}

impl SetSettings {
	#[cfg(feature = "tiered")]
	pub fn new(config: &Config, cache: &Cache) -> SetSettings {
		SetSettings {
			timeout: config.set_timeout(),
			setters: SetterCount::of(&cache.gate_config()),
		}
	}

	#[cfg(feature = "all_dram")]
	pub fn new(config: &Config, _cache: &Cache) -> SetSettings {
		SetSettings {
			timeout: config.set_timeout(),
		}
	}
}

/// Whether a SET in flight is registered as a setter: only if that can widen
/// the near band, which takes the byte gate running (`PAPER_GATE_MODE` not
/// `off`) and a value hint above 0. If not, why not.
///
/// A pure function of the cache's `GateConfig` -- the one it runs, as built,
/// with `PAPER_GATE_MODE` and `PAPER_GATE_VALUE_HINT_BYTES` applied. That does
/// not change while the server runs: nothing here calls `set_gate_config`.
#[cfg(feature = "tiered")]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SetterCount {
	/// The gate runs and the value hint is above 0: each SET in flight is
	/// registered, and widens the near band by the hint.
	Counted,

	/// The byte gate is off, so the policy worker publishes no near band for a
	/// setter to widen, whatever the value hint.
	GateOff,

	/// The value hint is 0: a setter widens the near band by 0 B.
	NoValueHint,
}

#[cfg(feature = "tiered")]
impl SetterCount {
	pub fn of(gate: &GateConfig) -> SetterCount {
		// The gate first: with it off there is no band, hint or no hint.
		if gate.mode != GateMode::Block {
			return SetterCount::GateOff;
		}

		match gate.value_hint {
			0 => SetterCount::NoValueHint,
			_ => SetterCount::Counted,
		}
	}

	/// Whether a SET in flight is registered with the cache.
	pub fn counted(self) -> bool {
		self == SetterCount::Counted
	}
}

/// `Instant::now() + timeout`, or a time far enough ahead not to matter if the
/// sum does not fit the clock (the config caps a timeout at a day, so it does).
fn deadline_after(start: Instant, timeout: Duration) -> Instant {
	start
		.checked_add(timeout)
		.unwrap_or_else(|| start + Duration::from_secs(365 * 24 * 3600))
}

/// Reads and discards the rest of a SET frame: `len` bytes of value and the
/// four of TTL. Gives up at one `timeout` from now, and each read also waits at
/// most that long. On success the connection is in step and the receive timeout
/// is disarmed again; on failure the caller closes the connection.
fn skip_rest(
	connection: &mut Connection,
	stats: &SelfStats,
	len: u32,
	timeout: Duration,
) -> Result<(), ServerError> {
	let deadline = deadline_after(Instant::now(), timeout);

	let skipped = connection
		.set_read_timeout(Some(timeout))
		.and_then(|_| connection.skip(u64::from(len) + 4, deadline))
		.and_then(|_| connection.set_read_timeout(None));

	match skipped {
		Ok(()) => {
			stats.skipped(u64::from(len));

			Ok(())
		},

		Err(err) => {
			stats.skip_failed();
			warn!("Could not skip the {len} B value of a refused SET ({err}): closing the connection");

			Err(ServerError::Disconnected)
		},
	}
}

/// A SET from a client that has not authorized: its frame is consumed, so the
/// connection stays in step, and the reply is the refusal.
pub fn handle_unauthorized_set(
	connection: &mut Connection,
	stats: &SelfStats,
	settings: &SetSettings,
	len: u32,
) -> SheetResult {
	skip_rest(connection, stats, len, settings.timeout)?;

	Err(ServerError::Unauthorized)
}

/// The tiered SET: see the module documentation.
#[cfg(feature = "tiered")]
pub fn handle_set(
	cache: &Arc<Cache>,
	stats: &SelfStats,
	settings: &SetSettings,
	connection: &mut Connection,
	key: Buffer,
	len: u32,
) -> SheetResult {
	use paper_utils::{command::CommandByte, sheet::SheetBuilder};

	use crate::selfstats::SLOT_SET_REFUSED;

	let timeout = settings.timeout;

	// One live setter per SET in flight, not per connection: the byte gate widens
	// the band it holds a fast tier to by the setters that could be mid-flight
	// when it fills, and an idle connection is none of them. Only when that widens
	// anything (see `SetterCount`): the registration and the release each wake
	// the policy worker. The guard lives to the end of this function, which is the
	// end of the SET.
	let _setter = settings.setters.counted().then(|| cache.register_setter());

	let started = Instant::now();
	let deadline = deadline_after(started, timeout);

	let permit = match cache.reserve_set(key, len as usize, None, deadline) {
		Ok(permit) => permit,

		Err(err) => {
			stats.record(SLOT_SET_REFUSED, started.elapsed().as_nanos() as u64);
			stats.set_refused(&err);

			// Nothing was allocated for a value the cache would not take, and
			// none is: its bytes are skipped, not buffered.
			skip_rest(connection, stats, len, timeout)?;

			return Err(ServerError::CacheError(err));
		},
	};

	let admitted = started.elapsed();

	// Armed before the value's allocation is charged, so that no moment holds
	// fast bytes for a client the timeout does not cover.
	if connection.set_read_timeout(Some(timeout)).is_err() {
		return Err(ServerError::Disconnected);
	}

	let filling = Instant::now();
	let mut pending = permit.fill();
	let filled = filling.elapsed();

	// The value, straight off the socket into the allocation. `TcpStream`
	// implements `read_buf` itself, so nothing is staged and no zero-fill is
	// written to the slow tier.
	if let Err(err) = pending.read_exact_from(connection.stream_mut()) {
		// Dropped, and so refunded, before anything else is said about it.
		drop(pending);
		stats.body_failed(&err);
		warn!("A SET's {len} B value did not arrive ({err}): closing the connection");

		return Err(ServerError::Disconnected);
	}

	let ttl = match connection.read_u32() {
		Ok(0) => None,
		Ok(ttl) => Some(ttl),

		Err(err) => {
			drop(pending);
			stats.body_failed(&err);
			warn!("A SET's TTL did not arrive ({err}): closing the connection");

			return Err(ServerError::Disconnected);
		},
	};

	pending.set_ttl(ttl);

	let committing = Instant::now();
	let result = pending.commit();
	let committed = committing.elapsed();

	// What the cache charged this set: its admission (the wait at the gate
	// included), the allocation and the commit. The reads of the value and of
	// the TTL are the socket's, and left out, as `set`'s were when the body was
	// read before the call.
	stats.record(CommandByte::SET, (admitted + filled + committed).as_nanos() as u64);

	if connection.set_read_timeout(None).is_err() {
		return Err(ServerError::Disconnected);
	}

	match result {
		Ok(()) => {
			stats.set_committed();

			Ok(SheetBuilder::new().write_bool(true).into_sheet())
		},

		Err(err) => {
			stats.set_commit_refused();

			Err(ServerError::CacheError(err))
		},
	}
}

/// The flat SET: the all-DRAM cache admits nothing before it has the bytes, so
/// the value is read into a buffer of its own, then the TTL, and then it is set,
/// as upstream does. There is no gate to wait at and no allocation to charge,
/// so no timeout either.
#[cfg(feature = "all_dram")]
pub fn handle_set(
	cache: &Arc<Cache>,
	stats: &SelfStats,
	_settings: &SetSettings,
	connection: &mut Connection,
	key: Buffer,
	len: u32,
) -> SheetResult {
	use paper_utils::{command::CommandByte, sheet::SheetBuilder, stream::read_buf};

	use crate::selfstats::timed;

	let Ok(value) = read_buf(connection.stream_mut(), len as usize) else {
		return Err(ServerError::Disconnected);
	};

	let ttl = match connection.read_u32() {
		Ok(0) => None,
		Ok(ttl) => Some(ttl),

		Err(_) => return Err(ServerError::Disconnected),
	};

	timed(stats, CommandByte::SET, || cache.set(key, &value, ttl))
		.map(|_| SheetBuilder::new().write_bool(true).into_sheet())
		.map_err(ServerError::CacheError)
}

#[cfg(all(test, feature = "tiered"))]
mod tests {
	use super::*;

	fn gate(mode: GateMode, value_hint: u64) -> GateConfig {
		let mut gate = GateConfig::default();

		gate.mode = mode;
		gate.value_hint = value_hint;

		gate
	}

	#[test]
	fn a_set_is_counted_only_when_the_gate_runs_and_the_value_hint_is_above_0() {
		let counted = SetterCount::of(&gate(GateMode::Block, 4096));

		assert_eq!(counted, SetterCount::Counted);
		assert!(counted.counted());

		// A hint of 0 widens nothing, and neither does a gate that is off.
		assert_eq!(SetterCount::of(&gate(GateMode::Block, 0)), SetterCount::NoValueHint);
		assert_eq!(SetterCount::of(&gate(GateMode::Off, 4096)), SetterCount::GateOff);

		// With both, the gate is the reason: with it off there is no band at all.
		assert_eq!(SetterCount::of(&gate(GateMode::Off, 0)), SetterCount::GateOff);

		for not_counted in [
			SetterCount::of(&gate(GateMode::Block, 0)),
			SetterCount::of(&gate(GateMode::Off, 4096)),
			SetterCount::of(&gate(GateMode::Off, 0)),
		] {
			assert!(!not_counted.counted(), "{not_counted:?}");
		}
	}
}
