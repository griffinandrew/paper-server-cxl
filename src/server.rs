/*
 * Copyright (c) Kia Shakiba
 *
 * This source code is licensed under the GNU AGPLv3 license found in the
 * LICENSE file in the root directory of this source tree.
 */

use std::{
	io::Write,
	net::{Shutdown, TcpListener, TcpStream},
	os::fd::AsRawFd,
	str::FromStr,
	sync::{
		Arc,
		Mutex,
		atomic::{AtomicBool, Ordering},
	},
	thread,
	time::{Duration, Instant},
};

use kwik::thread_pool::ThreadPool;
use log::{error, info, warn};
use paper_cache::{CacheError, PaperCache, PaperPolicy};
#[cfg(feature = "all_dram")]
use paper_cache::BufferDRAM;
#[cfg(feature = "tiered")]
use paper_cache::{CacheTierSize, TieredBuffer};
use paper_utils::{
	command::CommandByte,
	sheet::{Sheet, SheetBuilder},
	stream::Buffer,
};

use crate::{
	command::Command,
	config::Config,
	connection::Connection,
	error::ServerError,
	selfstats::{self, SLOT_GET_MISS, SelfStats, timed},
	set::{self, SetSettings},
};

/// Keys stay `Buffer` (the protocol's byte strings); values become
/// `TieredBuffer`, which is what makes this the tiered cache rather than the
/// flat one -- each value lives in either the fast (DRAM) or slow (PMEM/CXL)
/// tier and migrates between them under the policy's control.
#[cfg(feature = "tiered")]
pub type Cache = PaperCache<Buffer, TieredBuffer>;

/// The flat all-DRAM baseline: one tier, nothing on the slow node.
#[cfg(feature = "all_dram")]
pub type Cache = PaperCache<Buffer, BufferDRAM>;

pub type SheetResult = Result<Sheet, ServerError>;

/// The tiered constructor: overall budget, the fast tier's share of it, and
/// the design. It takes no policy LIST -- `auto` policy switching is gone from
/// the cache, and a tiered stack's fast/slow split is not transferable between
/// designs, so `policies[]` from the config is unused here.
#[cfg(feature = "tiered")]
pub fn new_cache(config: &Config) -> Result<Cache, CacheError> {
	Cache::new(
		config.max_size(),
		CacheTierSize::Bytes(config.fast_tier_size()),
		config.policy(),
	)
}

/// The flat constructor takes the set of CONFIGURED policies rather than a tier
/// size -- there is no tier to size. Only the running policy is configured.
#[cfg(feature = "all_dram")]
pub fn new_cache(config: &Config) -> Result<Cache, CacheError> {
	Cache::new(config.max_size(), &[config.policy()], config.policy())
}

pub struct Server {
	listener: TcpListener,
	cache:    Arc<Cache>,

	/// The server's own latency figures (see `selfstats`): what command byte 200
	/// and `--stats-interval` report.
	stats: Arc<SelfStats>,

	pool: ThreadPool,

	max_connections: usize,
	streams:         Arc<Mutex<Vec<Option<TcpStream>>>>,
	auth_token:      Option<u64>,

	/// What the SET arm needs: its timeout and whether it counts setters (see
	/// `set`).
	set_settings: SetSettings,

	shutdown: Arc<AtomicBool>,
}

impl Server {
	pub fn new(config: &Config, cache: Cache) -> Result<Self, ServerError> {
		let addr = format!("{}:{}", config.host(), config.port());

		let listener = match TcpListener::bind(&addr) {
			Ok(listener) => listener,

			Err(err) => {
				error!("could not bind {addr}: {err}");
				return Err(ServerError::InvalidAddress);
			},
		};

		let set_settings = SetSettings::new(config, &cache);

		let mut streams = Vec::with_capacity(config.max_connections());
		for _ in 0..config.max_connections() {
			streams.push(None);
		}

		let server = Server {
			listener,
			cache: Arc::new(cache),
			stats: Arc::new(SelfStats::new()),

			pool: ThreadPool::new(config.max_connections()),

			max_connections: config.max_connections(),
			streams: Arc::new(Mutex::new(streams)),
			auth_token: config.auth_token(),

			set_settings,

			shutdown: Arc::new(AtomicBool::new(false)),
		};

		Ok(server)
	}

	pub fn shutdown(&self) -> Result<(), ServerError> {
		self.shutdown.store(true, Ordering::Relaxed);

		let streams = self
			.streams
			.lock()
			.map_err(|_| ServerError::Internal)?;

		for stream in streams.iter().flatten() {
			stream
				.shutdown(Shutdown::Both)
				.map_err(|_| ServerError::Disconnected)?;
		}

		let fd = self.listener.as_raw_fd();
		unsafe { libc::shutdown(fd, libc::SHUT_RD) };

		Ok(())
	}

	/// Whether a SET in flight is registered with the cache as a setter, and if
	/// not, why not: it is only when a setter widens the near band (see
	/// `set::SetterCount`).
	#[cfg(feature = "tiered")]
	pub fn setters(&self) -> set::SetterCount {
		self.set_settings.setters
	}

	/// Prints the self-stats report to stderr every `every`, so a long run
	/// leaves a trace without anyone having to ask for it. A thread of its own
	/// rather than a signal handler, and it holds the cache only weakly: the
	/// cache's workers are joined when it drops, and a reporter keeping it alive
	/// would leave them running at process exit.
	pub fn spawn_stats_reporter(&self, every: Duration) {
		let cache = Arc::downgrade(&self.cache);
		let stats = self.stats.clone();

		thread::spawn(move || loop {
			thread::sleep(every);

			let Some(cache) = cache.upgrade() else {
				return;
			};

			eprint!("{}", selfstats::render(&stats, &cache));
		});
	}

	pub fn listen(&self) -> Result<(), ServerError> {
		for stream in self.listener.incoming() {
			if self.shutdown.load(Ordering::Relaxed) {
				if let Ok(stream) = stream {
					let _ = stream.shutdown(Shutdown::Both);
				}

				return Ok(());
			}

			match stream {
				Ok(mut stream) => {
					let Ok(stream_handle) = stream.try_clone() else {
						return Err(ServerError::InvalidConnection);
					};

					if count_active_streams(self.streams.clone())? == self.max_connections {
						warn!("Maximum number of connections exceeded");

						max_connections_reject_handshake(&mut stream)?;

						let _ = stream.shutdown(Shutdown::Both);
						return Err(ServerError::MaxConnectionsExceeded);
					}

					// Latency is the point of this server, and Nagle would batch
					// small responses into 40ms stalls that have nothing to do
					// with the cache.
					if let Err(err) = stream.set_nodelay(true) {
						warn!("Could not set TCP_NODELAY: {err}");
					}

					let address = stream
						.peer_addr()
						.map(|address| address.to_string())
						.unwrap_or("-1".into());

					info!("Connected: {address}");

					success_handshake(&mut stream)?;

					let connection = Connection::new(stream, self.auth_token);
					let cache = self.cache.clone();
					let stats = self.stats.clone();
					let set_settings = self.set_settings;
					let streams = self.streams.clone();

					self.pool.execute(move || {
						let Some(index) = insert_stream(streams.clone(), stream_handle) else {
							let _ = connection.close();
							info!("Disconnected: {address}");

							return;
						};

						Server::handle_connection(connection, cache, stats, set_settings);
						info!("Disconnected: {address}");

						remove_stream(streams, index);
					});
				},

				Err(_) => return Err(ServerError::InvalidConnection),
			}
		}

		Ok(())
	}

	fn handle_connection(
		mut connection: Connection,
		cache: Arc<Cache>,
		stats: Arc<SelfStats>,
		set_settings: SetSettings,
	) {
		loop {
			let command = match connection.get_command() {
				Ok(command) => command,

				Err(ServerError::Disconnected) => {
					let _ = connection.close();
					return;
				},

				Err(err) => {
					error!("{err}");
					continue;
				},
			};

			let sheet_result = match (connection.is_authorized(), command) {
				(_, Command::Ping) => handle_ping(),
				(_, Command::Version) => handle_version(&cache),

				(_, Command::Auth(token)) => handle_auth(&mut connection, &token),

				(true, Command::Get(key)) => handle_get(&cache, &stats, key),
				(true, Command::Set(key, len)) => {
					set::handle_set(&cache, &stats, &set_settings, &mut connection, key, len)
				},
				(true, Command::Del(key)) => handle_del(&cache, &stats, key),

				(true, Command::Has(key)) => handle_has(&cache, &stats, key),
				(true, Command::Peek(key)) => handle_peek(&cache, &stats, key),
				(true, Command::Ttl(key, ttl)) => handle_ttl(&cache, &stats, key, ttl),
				(true, Command::Size(key)) => handle_size(&cache, &stats, key),

				(true, Command::Wipe) => handle_wipe(&cache),

				(true, Command::Resize(size)) => handle_resize(&cache, size),
				(true, Command::Policy(policy_str)) => handle_policy(&cache, policy_str),

				(true, Command::Status) => handle_status(&cache),
				(true, Command::SelfStats) => handle_self_stats(&cache, &stats),

				// The value and the TTL of a SET are still on the socket, and
				// are read past so the connection stays in step.
				(false, Command::Set(_, len)) => {
					set::handle_unauthorized_set(&mut connection, &stats, &set_settings, len)
				},

				_ => Err(ServerError::Unauthorized),
			};

			// A SET whose value or TTL did not arrive, or could not be skipped,
			// has lost its place in the stream: nothing more can be read from
			// the connection, and no reply is owed to a client that is gone.
			if matches!(sheet_result, Err(ServerError::Disconnected)) {
				let _ = connection.close();
				return;
			}

			let sheet = sheet_result.unwrap_or_else(|err| err.to_sheet());

			if (connection.send_response(sheet.serialize())).is_err() {
				error!("Could not send response to command");
			}
		}
	}
}

fn success_handshake(stream: &mut TcpStream) -> Result<(), ServerError> {
	let sheet = SheetBuilder::new().write_bool(true).into_sheet();

	stream
		.write_all(sheet.serialize())
		.map_err(|_| ServerError::InvalidResponse)
}

fn max_connections_reject_handshake(stream: &mut TcpStream) -> Result<(), ServerError> {
	let sheet = ServerError::MaxConnectionsExceeded.to_sheet();

	stream
		.write_all(sheet.serialize())
		.map_err(|_| ServerError::InvalidResponse)
}

fn count_active_streams(streams: Arc<Mutex<Vec<Option<TcpStream>>>>) -> Result<usize, ServerError> {
	let streams = streams
		.lock()
		.map_err(|_| ServerError::Internal)?;

	let num_active_streams = streams
		.iter()
		.filter(|maybe_stream| maybe_stream.is_some())
		.count();

	Ok(num_active_streams)
}

fn insert_stream(streams: Arc<Mutex<Vec<Option<TcpStream>>>>, stream: TcpStream) -> Option<usize> {
	let mut streams = streams.lock().ok()?;

	for (index, maybe_stream) in streams.iter_mut().enumerate() {
		if maybe_stream.is_none() {
			maybe_stream.replace(stream);
			return Some(index);
		}
	}

	None
}

fn remove_stream(streams: Arc<Mutex<Vec<Option<TcpStream>>>>, index: usize) {
	let Ok(mut streams) = streams.lock() else {
		return;
	};

	if index >= streams.len() {
		return;
	}

	if let Some(stream) = streams[index].take() {
		let _ = stream.shutdown(Shutdown::Both);
	}
}

fn handle_ping() -> SheetResult {
	let sheet = SheetBuilder::new()
		.write_bool(true)
		.write_buf(b"pong")
		.into_sheet();

	Ok(sheet)
}

fn handle_version(cache: &Arc<Cache>) -> SheetResult {
	let sheet = SheetBuilder::new()
		.write_bool(true)
		.write_str(cache.version())
		.into_sheet();

	Ok(sheet)
}

fn handle_auth(connection: &mut Connection, token: &Buffer) -> SheetResult {
	let is_authorized =
		String::from_utf8(token.to_vec()).is_ok_and(|token| connection.authorize(&token));

	if !is_authorized {
		return Err(ServerError::Unauthorized);
	}

	let sheet = SheetBuilder::new().write_bool(true).into_sheet();

	Ok(sheet)
}

fn handle_get(cache: &Arc<Cache>, stats: &SelfStats, key: Buffer) -> SheetResult {
	// Timed inline rather than through `timed` so the outcome can pick the
	// slot: hits and misses are different operations and must not share an
	// average.
	let started = Instant::now();
	let outcome = cache.get(&key);
	let nanos = started.elapsed().as_nanos() as u64;

	stats.record(
		if outcome.is_ok() { CommandByte::GET } else { SLOT_GET_MISS },
		nanos,
	);

	outcome
		.map(|object| {
			SheetBuilder::new()
				.write_bool(true)
				.write_buf(&object)
				.into_sheet()
		})
		.map_err(ServerError::CacheError)
}

fn handle_del(cache: &Arc<Cache>, stats: &SelfStats, key: Buffer) -> SheetResult {
	timed(stats, CommandByte::DEL, || cache.del(&key))
		.map(|_| SheetBuilder::new().write_bool(true).into_sheet())
		.map_err(ServerError::CacheError)
}

fn handle_has(cache: &Arc<Cache>, stats: &SelfStats, key: Buffer) -> SheetResult {
	let has = timed(stats, CommandByte::HAS, || cache.has(&key));
	let sheet = SheetBuilder::new().write_bool(true).write_bool(has).into_sheet();

	Ok(sheet)
}

fn handle_peek(cache: &Arc<Cache>, stats: &SelfStats, key: Buffer) -> SheetResult {
	timed(stats, CommandByte::PEEK, || cache.peek(&key))
		.map(|object| {
			SheetBuilder::new()
				.write_bool(true)
				.write_buf(&object)
				.into_sheet()
		})
		.map_err(ServerError::CacheError)
}

fn handle_ttl(cache: &Arc<Cache>, stats: &SelfStats, key: Buffer, ttl: Option<u32>) -> SheetResult {
	timed(stats, CommandByte::TTL, || cache.ttl(&key, ttl))
		.map(|_| SheetBuilder::new().write_bool(true).into_sheet())
		.map_err(ServerError::CacheError)
}

fn handle_size(cache: &Arc<Cache>, stats: &SelfStats, key: Buffer) -> SheetResult {
	timed(stats, CommandByte::SIZE, || cache.size(&key))
		.map(|size| {
			SheetBuilder::new()
				.write_bool(true)
				.write_u32(size)
				.into_sheet()
		})
		.map_err(ServerError::CacheError)
}

fn handle_wipe(cache: &Arc<Cache>) -> SheetResult {
	cache
		.wipe()
		.map(|_| SheetBuilder::new().write_bool(true).into_sheet())
		.map_err(ServerError::CacheError)
}

fn handle_resize(cache: &Arc<Cache>, size: u64) -> SheetResult {
	cache
		.resize(size)
		.map(|_| SheetBuilder::new().write_bool(true).into_sheet())
		.map_err(ServerError::CacheError)
}

/// Refused rather than silently ignored.
///
/// No cache has a runtime policy setter any more: a cache's policy is fixed
/// when it is built, and `PaperCache::policy` is gone. Switching design means
/// restarting with a different `policy=` -- which is also the honest thing for
/// a tiered cache, since the fast/slow split a running stack has built up is
/// not transferable to another design.
fn handle_policy(_cache: &Arc<Cache>, policy_str: String) -> SheetResult {
	let Ok(_policy) = PaperPolicy::from_str(&policy_str) else {
		return Err(ServerError::CacheError(CacheError::InvalidPolicy));
	};

	Err(ServerError::CacheError(CacheError::InvalidPolicy))
}

fn handle_status(cache: &Arc<Cache>) -> SheetResult {
	let status = cache.status().map_err(ServerError::CacheError)?;

	let mut sheet_builder = SheetBuilder::new()
		.write_bool(true)
		.write_u32(status.pid())
		.write_u64(status.max_size())
		.write_u64(status.used_size())
		.write_u64(status.num_objects())
		.write_u64(status.rss())
		.write_u64(status.hwm())
		.write_u64(status.total_gets())
		.write_u64(status.total_sets())
		.write_u64(status.total_dels())
		.write_f64(status.miss_ratio())
		.write_u32(status.policies().len() as u32);

	for policy in status.policies() {
		sheet_builder = sheet_builder.write_str(policy.to_string());
	}

	// The auto flag is always false: a cache's policy is fixed when it is
	// built, and `auto` no longer exists.
	let sheet = sheet_builder
		.write_str(status.policy().to_string())
		.write_bool(false)
		.write_u64(status.uptime())
		.into_sheet();

	Ok(sheet)
}

/// The server's own latency and tier report, as one text buffer: command byte
/// 200, outside the protocol's range. See `selfstats`.
fn handle_self_stats(cache: &Arc<Cache>, stats: &SelfStats) -> SheetResult {
	let sheet = SheetBuilder::new()
		.write_bool(true)
		.write_buf(selfstats::render(stats, cache).as_bytes())
		.into_sheet();

	Ok(sheet)
}
