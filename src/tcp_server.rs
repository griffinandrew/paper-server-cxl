use std::{
	sync::{
		Arc,
		Mutex,
		atomic::{AtomicUsize, Ordering},
	},
	net::{TcpListener, Shutdown},
};

use kwik::ThreadPool;

use paper_utils::{
	stream::Buffer,
	error::PaperError,
	sheet::builder::SheetBuilder,
};

use paper_cache::PaperCache;

use crate::{
	server_error::{ServerError, ErrorKind},
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
			return Err(ServerError::new(
				ErrorKind::InvalidAddress,
				"Could not establish a connection."
			));
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
						let _ = stream.shutdown(Shutdown::Both);

						return Err(ServerError::new(
							ErrorKind::MaxConnectionsExceeded,
							"The maximum number of connections was exceeded."
						));
					}

					let connection = TcpConnection::new(stream);
					let cache = Arc::clone(&self.cache);
					let num_connections = Arc::clone(&self.num_connections);

					self.pool.execute(move || {
						num_connections.fetch_add(1, Ordering::Relaxed);
						TcpServer::handle_connection(connection, cache);
						num_connections.fetch_sub(1, Ordering::Relaxed);
					});
				},

				Err(_) => {
					return Err(ServerError::new(
						ErrorKind::InvalidConnection,
						"Could not establish a connection."
					));
				}
			}
		}

		Ok(())
	}

	fn handle_connection(mut connection: TcpConnection, cache: Arc<Mutex<Cache>>) {
		loop {
			let command = match connection.get_command() {
				Ok(command) => command,

				Err(ref err) if err.kind() == &ErrorKind::Disconnected => {
					return;
				},

				Err(err) => {
					println!("\x1B[31mErr\x1B[0m: {}", err.message());
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

					let (is_ok, response) = match cache.get(&key) {
						Ok(response) => (true, response.into_buf()),
						Err(err) => (false, err.message().as_bytes().to_vec()),
					};

					SheetBuilder::new()
						.write_bool(is_ok)
						.write_buf(&response)
						.to_sheet()
				},

				Command::Set(key, value, ttl) => {
					let mut cache = cache.lock().unwrap();

					let (is_ok, response) = match cache.set(key, value, ttl) {
						Ok(_) => (true, b"done".to_vec()),
						Err(err) => (false, err.message().as_bytes().to_vec()),
					};

					SheetBuilder::new()
						.write_bool(is_ok)
						.write_buf(&response)
						.to_sheet()
				},

				Command::Del(key) => {
					let mut cache = cache.lock().unwrap();

					let (is_ok, response) = match cache.del(&key) {
						Ok(_) => (true, b"done".to_vec()),
						Err(err) => (false, err.message().as_bytes().to_vec()),
					};

					SheetBuilder::new()
						.write_bool(is_ok)
						.write_buf(&response)
						.to_sheet()
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

					let (is_ok, response) = match cache.peek(&key) {
						Ok(response) => (true, response.into_buf()),
						Err(err) => (false, err.message().as_bytes().to_vec()),
					};

					SheetBuilder::new()
						.write_bool(is_ok)
						.write_buf(&response)
						.to_sheet()
				},

				Command::Wipe => {
					let mut cache = cache.lock().unwrap();

					let (is_ok, response) = match cache.wipe() {
						Ok(_) => (true, b"done".to_vec()),
						Err(err) => (false, err.message().as_bytes().to_vec()),
					};

					SheetBuilder::new()
						.write_bool(is_ok)
						.write_buf(&response)
						.to_sheet()
				},

				Command::Resize(size) => {
					let mut cache = cache.lock().unwrap();

					let (is_ok, response) = match cache.resize(size) {
						Ok(_) => (true, b"done".to_vec()),
						Err(err) => (false, err.message().as_bytes().to_vec()),
					};

					SheetBuilder::new()
						.write_bool(is_ok)
						.write_buf(&response)
						.to_sheet()
				},

				Command::Policy(policy) => {
					let mut cache = cache.lock().unwrap();

					let (is_ok, response) = match cache.policy(policy) {
						Ok(_) => (true, b"done".to_vec()),
						Err(err) => (false, err.message().as_bytes().to_vec()),
					};

					SheetBuilder::new()
						.write_bool(is_ok)
						.write_buf(&response)
						.to_sheet()
				},

				Command::Stats => {
					let cache = cache.lock().unwrap();
					let stats = cache.stats();

					SheetBuilder::new()
						.write_bool(true)
						.write_u64(stats.get_max_size())
						.write_u64(stats.get_used_size())
						.write_u64(stats.get_total_gets())
						.write_u64(stats.get_total_sets())
						.write_u64(stats.get_total_dels())
						.write_f64(stats.get_miss_ratio())
						.write_u8(stats.get_policy().index() as u8)
						.write_u64(stats.get_uptime())
						.to_sheet()
				},
			};

			if (connection.send_response(sheet.serialize())).is_err() {
				println!("Error sending response to command.");
			}
		}
	}
}
