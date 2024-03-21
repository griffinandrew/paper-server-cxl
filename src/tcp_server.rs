use std::{
	sync::{
		Arc,
		atomic::{AtomicUsize, Ordering},
	},
	hash::{DefaultHasher, Hash, Hasher, BuildHasherDefault},
	net::{TcpListener, Shutdown},
};

use log::{info, warn, error};
use nohash_hasher::NoHashHasher;
use kwik::ThreadPool;
use paper_cache::{PaperCache, Policy};

use paper_utils::{
	stream::Buffer,
	sheet::builder::SheetBuilder,
	policy::PolicyByte,
};

use crate::{
	server_error::ServerError,
	server_object::ServerObject,
	command::Command,
	tcp_connection::TcpConnection,
	config::Config,
};

pub type NoHasher = BuildHasherDefault<NoHashHasher<u64>>;
type Cache = PaperCache<u64, ServerObject, NoHasher>;

pub struct TcpServer {
	listener: TcpListener,
	cache: Arc<Cache>,

	pool: ThreadPool,

	max_connections: usize,
	num_connections: Arc<AtomicUsize>,
	auth: Option<String>,
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
			auth: config.auth().map(|auth| auth.to_owned()),
		};

		Ok(server)
	}

	pub fn listen(&mut self) -> Result<(), ServerError> {
		for stream in self.listener.incoming() {
			match stream {
				Ok(stream) => {
					if self.num_connections.load(Ordering::Relaxed) == self.max_connections {
						warn!("Maximum number of connections exceeded.");

						let _ = stream.shutdown(Shutdown::Both);
						return Err(ServerError::MaxConnectionsExceeded);
					}

					let address = stream
						.peer_addr()
						.map(|address| address.to_string())
						.unwrap_or("-1".into());

					info!("Connected: {}", address);

					let connection = TcpConnection::new(stream, self.auth.clone());
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

			let sheet = match (connection.is_authorized(), command) {
				(_, Command::Ping) => {
					SheetBuilder::new()
						.write_bool(true)
						.write_buf(b"pong")
						.to_sheet()
				},

				(_, Command::Version) => {
					SheetBuilder::new()
						.write_bool(true)
						.write_str(&cache.version())
						.to_sheet()
				},

				(_, Command::Auth(auth)) => {
					let is_authorized = String::from_utf8(auth.to_vec())
						.is_ok_and(|token| connection.authorize(&token));

					match is_authorized {
						true => SheetBuilder::new()
							.write_bool(true)
							.write_buf(b"done")
							.to_sheet(),

						false => SheetBuilder::new()
							.write_bool(false)
							.write_buf(b"unauthorized")
							.to_sheet(),
					}
				},

				(true, Command::Get(key)) => {
					let key = hash(key);

					match cache.get(key) {
						Ok(object) => SheetBuilder::new()
							.write_bool(true)
							.write_buf(object.as_buf())
							.to_sheet(),

						Err(err) => SheetBuilder::new()
							.write_bool(false)
							.write_buf(err.to_string().as_bytes())
							.to_sheet(),
					}
				},

				(true, Command::Set(key, value, ttl)) => {
					let key = hash(key);

					match cache.set(key, value, ttl) {
						Ok(_) => SheetBuilder::new()
							.write_bool(true)
							.write_buf(b"done")
							.to_sheet(),

						Err(err) => SheetBuilder::new()
							.write_bool(false)
							.write_buf(err.to_string().as_bytes())
							.to_sheet(),
					}
				},

				(true, Command::Del(key)) => {
					let key = hash(key);

					match cache.del(key) {
						Ok(_) => SheetBuilder::new()
							.write_bool(true)
							.write_buf(b"done")
							.to_sheet(),

						Err(err) => SheetBuilder::new()
							.write_bool(false)
							.write_buf(err.to_string().as_bytes())
							.to_sheet(),
					}
				},

				(true, Command::Has(key)) => {
					let key = hash(key);

					SheetBuilder::new()
						.write_bool(true)
						.write_bool(cache.has(key))
						.to_sheet()
				},

				(true, Command::Peek(key)) => {
					let key = hash(key);

					match cache.peek(key) {
						Ok(object) => SheetBuilder::new()
							.write_bool(true)
							.write_buf(object.as_buf())
							.to_sheet(),

						Err(err) => SheetBuilder::new()
							.write_bool(false)
							.write_buf(err.to_string().as_bytes())
							.to_sheet(),
					}
				},

				(true, Command::Ttl(key, ttl)) => {
					let key = hash(key);

					match cache.ttl(key, ttl) {
						Ok(_) => SheetBuilder::new()
							.write_bool(true)
							.write_buf(b"done")
							.to_sheet(),

						Err(err) => SheetBuilder::new()
							.write_bool(false)
							.write_buf(err.to_string().as_bytes())
							.to_sheet(),
					}
				},

				(true, Command::Size(key)) => {
					let key = hash(key);

					match cache.size(key) {
						Ok(size) => SheetBuilder::new()
							.write_bool(true)
							.write_u64(size)
							.to_sheet(),

						Err(err) => SheetBuilder::new()
							.write_bool(false)
							.write_buf(err.to_string().as_bytes())
							.to_sheet(),
					}
				},

				(true, Command::Wipe) => {
					match cache.wipe() {
						Ok(_) => SheetBuilder::new()
							.write_bool(true)
							.write_buf(b"done")
							.to_sheet(),

						Err(err) => SheetBuilder::new()
							.write_bool(false)
							.write_buf(err.to_string().as_bytes())
							.to_sheet(),
					}
				},

				(true, Command::Resize(size)) => {
					match cache.resize(size) {
						Ok(_) => SheetBuilder::new()
							.write_bool(true)
							.write_buf(b"done")
							.to_sheet(),

						Err(err) => SheetBuilder::new()
							.write_bool(false)
							.write_buf(err.to_string().as_bytes())
							.to_sheet(),
					}
				},

				(true, Command::Policy(policy)) => {
					match cache.policy(policy) {
						Ok(_) => SheetBuilder::new()
							.write_bool(true)
							.write_buf(b"done")
							.to_sheet(),

						Err(err) => SheetBuilder::new()
							.write_bool(false)
							.write_buf(err.to_string().as_bytes())
							.to_sheet(),
					}
				},

				(true, Command::Stats) => {
					let stats = cache.stats();

					let policy_byte = match stats.get_policy() {
						Policy::Lfu => PolicyByte::LFU,
						Policy::Fifo => PolicyByte::FIFO,
						Policy::Lru => PolicyByte::LRU,
						Policy::Mru => PolicyByte::MRU,
					};

					SheetBuilder::new()
						.write_bool(true)
						.write_u64(stats.get_max_size())
						.write_u64(stats.get_used_size())
						.write_u64(stats.get_total_gets())
						.write_u64(stats.get_total_sets())
						.write_u64(stats.get_total_dels())
						.write_f64(stats.get_miss_ratio())
						.write_u8(policy_byte)
						.write_u64(stats.get_uptime())
						.to_sheet()
				},

				_ => {
					SheetBuilder::new()
						.write_bool(false)
						.write_buf(b"unauthorized")
						.to_sheet()
				},
			};

			if (connection.send_response(sheet.serialize())).is_err() {
				error!("Could not send response to command.");
			}
		}
	}
}

fn hash(key: Buffer) -> u64 {
	let mut s = DefaultHasher::new();
	key.hash(&mut s);
	s.finish()
}
