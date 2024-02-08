use std::{
	sync::{
		Arc,
		Mutex,
		atomic::{AtomicUsize, Ordering},
	},
	net::{TcpListener, Shutdown},
};

use log::{info, warn, error};
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

type Cache = PaperCache<Buffer, ServerObject>;

pub struct TcpServer {
	listener: TcpListener,
	cache: Arc<Mutex<Cache>>,

	pool: ThreadPool,

	max_connections: usize,
	num_connections: Arc<AtomicUsize>,
}

impl TcpServer {
	pub fn new(
		config: &Config,
		cache: Arc<Mutex<Cache>>,
	) -> Result<Self, ServerError> {
		let addr = format!("{}:{}", config.host(), config.port());

		let Ok(listener) = TcpListener::bind(addr) else {
			return Err(ServerError::InvalidAddress);
		};

		let server = TcpServer {
			listener,
			cache,

			pool: ThreadPool::new(config.max_connections()),

			max_connections: config.max_connections(),
			num_connections: Arc::new(AtomicUsize::new(0)),
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

					let connection = TcpConnection::new(stream);
					let cache = Arc::clone(&self.cache);
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

	fn handle_connection(mut connection: TcpConnection, cache: Arc<Mutex<Cache>>) {
		loop {
			let command = match connection.get_command() {
				Ok(command) => command,
				Err(ServerError::Disconnected) => return,

				Err(err) => {
					error!("{err}");
					continue;
				},
			};

			let sheet = match command {
				Command::Ping => {
					SheetBuilder::new()
						.write_bool(true)
						.write_buf(b"pong")
						.to_sheet()
				},

				Command::Version => {
					let cache = cache.lock().unwrap();

					SheetBuilder::new()
						.write_bool(true)
						.write_str(&cache.version())
						.to_sheet()
				},

				Command::Get(key) => {
					let mut cache = cache.lock().unwrap();

					match cache.get(&key) {
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

				Command::Set(key, value, ttl) => {
					let mut cache = cache.lock().unwrap();

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

				Command::Del(key) => {
					let mut cache = cache.lock().unwrap();

					match cache.del(&key) {
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

				Command::Has(key) => {
					let cache = cache.lock().unwrap();

					SheetBuilder::new()
						.write_bool(true)
						.write_bool(cache.has(&key))
						.to_sheet()
				},

				Command::Peek(key) => {
					let cache = cache.lock().unwrap();

					match cache.peek(&key) {
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

				Command::Wipe => {
					let mut cache = cache.lock().unwrap();

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

				Command::Resize(size) => {
					let mut cache = cache.lock().unwrap();

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

				Command::Policy(policy) => {
					let mut cache = cache.lock().unwrap();

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

				Command::Stats => {
					let cache = cache.lock().unwrap();
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
			};

			if (connection.send_response(sheet.serialize())).is_err() {
				error!("Could not send response to command.");
			}
		}
	}
}
