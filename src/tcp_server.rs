use std::{
	sync::{
		Arc,
		atomic::{AtomicUsize, Ordering},
	},
	io::Write,
	hash::{DefaultHasher, Hash, Hasher, BuildHasherDefault},
	net::{TcpListener, TcpStream, Shutdown},
};

use log::{info, warn, error};
use nohash_hasher::NoHashHasher;
use kwik::thread_pool::ThreadPool;
use paper_cache::{PaperCache, PaperPolicy};

use paper_utils::{
	stream::Buffer,
	sheet::{Sheet, SheetBuilder},
	policy::PolicyByte,
};

use crate::{
	error::ServerError,
	command::Command,
	tcp_connection::TcpConnection,
	config::Config,
};

pub type NoHasher = BuildHasherDefault<NoHashHasher<u64>>;

type Cache = PaperCache<u64, Buffer, NoHasher>;
type SheetResult = Result<Sheet, ServerError>;

pub struct TcpServer {
	listener: TcpListener,
	cache: Arc<Cache>,

	pool: ThreadPool,

	max_connections: usize,
	num_connections: Arc<AtomicUsize>,
	auth_token: Option<u64>,
}

impl TcpServer {
	pub fn new(
		config: &Config,
		cache: Cache,
	) -> Result<Self, ServerError> {
		let addr = format!("{}:{}", config.host(), config.port());

		let Ok(listener) = TcpListener::bind(addr) else {
			return Err(ServerError::InvalidAddress);
		};

		let server = TcpServer {
			listener,
			cache: Arc::new(cache),

			pool: ThreadPool::new(config.max_connections()),

			max_connections: config.max_connections(),
			num_connections: Arc::new(AtomicUsize::new(0)),
			auth_token: config.auth_token(),
		};

		Ok(server)
	}

	pub fn listen(&mut self) -> Result<(), ServerError> {
		for stream in self.listener.incoming() {
			match stream {
				Ok(mut stream) => {
					if self.num_connections.load(Ordering::Relaxed) == self.max_connections {
						warn!("Maximum number of connections exceeded.");

						max_connections_reject_handshake(&mut stream)?;

						let _ = stream.shutdown(Shutdown::Both);
						return Err(ServerError::MaxConnectionsExceeded);
					}

					let address = stream
						.peer_addr()
						.map(|address| address.to_string())
						.unwrap_or("-1".into());

					info!("Connected: {}", address);

					success_handshake(&mut stream)?;

					let connection = TcpConnection::new(stream, self.auth_token);
					let cache = self.cache.clone();
					let num_connections = Arc::clone(&self.num_connections);

					self.pool.execute(move || {
						num_connections.fetch_add(1, Ordering::Relaxed);
						TcpServer::handle_connection(connection, cache);

						info!("Disconnected: {}", address);
						num_connections.fetch_sub(1, Ordering::Relaxed);
					});
				},

				Err(_) => return Err(ServerError::InvalidConnection),
			}
		}

		Ok(())
	}

	fn handle_connection(mut connection: TcpConnection, cache: Arc<Cache>) {
		loop {
			let command = match connection.get_command() {
				Ok(command) => command,
				Err(ServerError::Disconnected) => return,

				Err(err) => {
					error!("{err}");
					continue;
				},
			};

			let sheet_result = match (connection.is_authorized(), command) {
				(_, Command::Ping) => handle_ping(),
				(_, Command::Version) => handle_version(&cache),

				(_, Command::Auth(token)) => handle_auth(&mut connection, &token),

				(true, Command::Get(key)) => handle_get(&cache, key),
				(true, Command::Set(key, value, ttl)) => handle_set(&cache, key, value, ttl),
				(true, Command::Del(key)) => handle_del(&cache, key),

				(true, Command::Has(key)) => handle_has(&cache, key),
				(true, Command::Peek(key)) => handle_peek(&cache, key),
				(true, Command::Ttl(key, ttl)) => handle_ttl(&cache, key, ttl),
				(true, Command::Size(key)) => handle_size(&cache, key),

				(true, Command::Wipe) => handle_wipe(&cache),

				(true, Command::Resize(size)) => handle_resize(&cache, size),
				(true, Command::Policy(policy)) => handle_policy(&cache, policy),

				(true, Command::Stats) => handle_stats(&cache),

				_ => Err(ServerError::Unauthorized),
			};

			let sheet = sheet_result.unwrap_or_else(|err| err.to_sheet());

			if (connection.send_response(sheet.serialize())).is_err() {
				error!("Could not send response to command.");
			}
		}
	}
}

fn success_handshake(stream: &mut TcpStream) -> Result<(), ServerError> {
	let sheet = SheetBuilder::new()
		.write_bool(true)
		.into_sheet();

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
		.write_str(&cache.version())
		.into_sheet();

	Ok(sheet)
}

fn handle_auth(connection: &mut TcpConnection, token: &Buffer) -> SheetResult {
	let is_authorized = String::from_utf8(token.to_vec())
		.is_ok_and(|token| connection.authorize(&token));

	if !is_authorized {
		return Err(ServerError::Unauthorized);
	}

	let sheet = SheetBuilder::new()
		.write_bool(true)
		.into_sheet();

	Ok(sheet)
}

fn handle_get(cache: &Arc<Cache>, key: Buffer) -> SheetResult {
	let key = hash(key);

	cache.get(key)
		.map(|object|
			SheetBuilder::new()
				.write_bool(true)
				.write_buf(&object)
				.into_sheet(),
		)
		.map_err(ServerError::CacheError)
}

fn handle_set(
	cache: &Arc<Cache>,
	key: Buffer,
	value: Buffer,
	ttl: Option<u32>,
) -> SheetResult {
	let key = hash(key);

	cache.set(key, value, ttl)
		.map(|_|
			SheetBuilder::new()
				.write_bool(true)
				.into_sheet()
		)
		.map_err(ServerError::CacheError)
}

fn handle_del(cache: &Arc<Cache>, key: Buffer) -> SheetResult {
	let key = hash(key);

	cache.del(key)
		.map(|_|
			SheetBuilder::new()
				.write_bool(true)
				.into_sheet()
		)
		.map_err(ServerError::CacheError)
}

fn handle_has(cache: &Arc<Cache>, key: Buffer) -> SheetResult {
	let key = hash(key);

	let sheet = SheetBuilder::new()
		.write_bool(true)
		.write_bool(cache.has(key))
		.into_sheet();

	Ok(sheet)
}

fn handle_peek(cache: &Arc<Cache>, key: Buffer) -> SheetResult {
	let key = hash(key);

	cache.peek(key)
		.map(|object|
			SheetBuilder::new()
				.write_bool(true)
				.write_buf(&object)
				.into_sheet()
		)
		.map_err(ServerError::CacheError)
}

fn handle_ttl(cache: &Arc<Cache>, key: Buffer, ttl: Option<u32>) -> SheetResult {
	let key = hash(key);

	cache.ttl(key, ttl)
		.map(|_|
			SheetBuilder::new()
				.write_bool(true)
				.into_sheet()
		)
		.map_err(ServerError::CacheError)
}

fn handle_size(cache: &Arc<Cache>, key: Buffer) -> SheetResult {
	let key = hash(key);

	cache.size(key)
		.map(|size|
			SheetBuilder::new()
				.write_bool(true)
				.write_u64(size)
				.into_sheet()
		)
		.map_err(ServerError::CacheError)
}

fn handle_wipe(cache: &Arc<Cache>) -> SheetResult {
	cache.wipe()
		.map(|_|
			SheetBuilder::new()
				.write_bool(true)
				.into_sheet()
		)
		.map_err(ServerError::CacheError)
}

fn handle_resize(cache: &Arc<Cache>, size: u64) -> SheetResult {
	cache.resize(size)
		.map(|_|
			SheetBuilder::new()
				.write_bool(true)
				.into_sheet()
		)
		.map_err(ServerError::CacheError)
}

fn handle_policy(cache: &Arc<Cache>, policy: PaperPolicy) -> SheetResult {
	cache.policy(policy)
		.map(|_|
			SheetBuilder::new()
				.write_bool(true)
				.into_sheet()
		)
		.map_err(ServerError::CacheError)
}

fn handle_stats(cache: &Arc<Cache>) -> SheetResult {
	let stats = cache.stats();

	let policy_byte = match stats.get_policy() {
		PaperPolicy::Lfu => PolicyByte::LFU,
		PaperPolicy::Fifo => PolicyByte::FIFO,
		PaperPolicy::Lru => PolicyByte::LRU,
		PaperPolicy::Mru => PolicyByte::MRU,
	};

	let sheet = SheetBuilder::new()
		.write_bool(true)
		.write_u64(stats.get_max_size())
		.write_u64(stats.get_used_size())
		.write_u64(stats.get_total_gets())
		.write_u64(stats.get_total_sets())
		.write_u64(stats.get_total_dels())
		.write_f64(stats.get_miss_ratio())
		.write_u8(policy_byte)
		.write_u64(stats.get_uptime())
		.into_sheet();

	Ok(sheet)
}

fn hash(key: Buffer) -> u64 {
	let mut s = DefaultHasher::new();
	key.hash(&mut s);
	s.finish()
}
