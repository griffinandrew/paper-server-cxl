use std::sync::Arc;
use tokio::sync::Mutex;
use tokio::net::TcpListener;
use paper_core::error::PaperError;
use paper_core::sheet::builder::SheetBuilder;
use paper_cache::PaperCache;
use crate::server_error::{ServerError, ErrorKind};
use crate::server_object::ServerObject;
use crate::command::Command;
use crate::tcp_connection::TcpConnection;

type Cache = PaperCache<u32, ServerObject>;

pub struct TcpServer {
	listener: TcpListener,
	cache: Arc<Mutex<Cache>>,
}

impl TcpServer {
	pub async fn new(
		host: &str,
		port: &u32,
		cache: Arc<Mutex<Cache>>,
	) -> Result<Self, ServerError> {
		let addr = format!("{}:{}", host, port);

		let listener = match TcpListener::bind(addr).await {
			Ok(listener) => listener,

			Err(_) => {
				return Err(ServerError::new(
					ErrorKind::InvalidAddress,
					"Could not establish a connection."
				));
			}
		};

		let server = TcpServer {
			listener,
			cache,
		};

		Ok(server)
	}

	pub async fn listen(&mut self) -> Result<(), ServerError> {
		let connection = match self.listener.accept().await {
			Ok((stream, socket)) => {
				let connection = TcpConnection::new(stream, socket);

				println!("\x1B[32mConnected\x1B[0m:\t<{}>", connection.ip());

				connection
			},

			Err(_) => {
				return Err(ServerError::new(
					ErrorKind::InvalidConnection,
					"Could not establish a connection."
				));
			}
		};

		let cache = Arc::clone(&self.cache);

		tokio::spawn(async move {
			TcpServer::handle_connection(connection, cache).await;
		});

		Ok(())
	}

	async fn handle_connection(mut connection: TcpConnection, cache: Arc<Mutex<Cache>>) {
		loop {
			let command = match connection.get_command().await {
				Ok(command) => command,

				Err(ref err) if err.kind() == &ErrorKind::Disconnected => {
					println!("\x1B[33mDisconnected\x1B[0m:\t<{}>", connection.ip());
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
						.write_bool(&true)
						.write_buf(b"pong")
						.to_sheet()
				},

				Command::Version => {
					let cache = cache.lock().await;

					SheetBuilder::new()
						.write_bool(&true)
						.write_str(&cache.version())
						.to_sheet()
				},

				Command::Get(key) => {
					let mut cache = cache.lock().await;

					let (is_ok, response) = match cache.get(&key) {
						Ok(response) => (true, response.into_buf()),
						Err(err) => (false, err.message().as_bytes().to_vec()),
					};

					SheetBuilder::new()
						.write_bool(&is_ok)
						.write_buf(&response)
						.to_sheet()
				},

				Command::Set(key, value, ttl) => {
					let mut cache = cache.lock().await;

					let (is_ok, response) = match cache.set(key, value, ttl) {
						Ok(_) => (true, b"done".to_vec()),
						Err(err) => (false, err.message().as_bytes().to_vec()),
					};

					SheetBuilder::new()
						.write_bool(&is_ok)
						.write_buf(&response)
						.to_sheet()
				},

				Command::Del(key) => {
					let mut cache = cache.lock().await;

					let (is_ok, response) = match cache.del(&key) {
						Ok(_) => (true, b"done".to_vec()),
						Err(err) => (false, err.message().as_bytes().to_vec()),
					};

					SheetBuilder::new()
						.write_bool(&is_ok)
						.write_buf(&response)
						.to_sheet()
				},

				Command::Wipe => {
					let mut cache = cache.lock().await;

					let (is_ok, response) = match cache.wipe() {
						Ok(_) => (true, b"done".to_vec()),
						Err(err) => (false, err.message().as_bytes().to_vec()),
					};

					SheetBuilder::new()
						.write_bool(&is_ok)
						.write_buf(&response)
						.to_sheet()
				},

				Command::Resize(size) => {
					let mut cache = cache.lock().await;

					let (is_ok, response) = match cache.resize(&size) {
						Ok(_) => (true, b"done".to_vec()),
						Err(err) => (false, err.message().as_bytes().to_vec()),
					};

					SheetBuilder::new()
						.write_bool(&is_ok)
						.write_buf(&response)
						.to_sheet()
				},

				Command::Policy(policy) => {
					let mut cache = cache.lock().await;

					let (is_ok, response) = match cache.policy(policy) {
						Ok(_) => (true, b"done".to_vec()),
						Err(err) => (false, err.message().as_bytes().to_vec()),
					};

					SheetBuilder::new()
						.write_bool(&is_ok)
						.write_buf(&response)
						.to_sheet()
				},

				Command::Stats => {
					let cache = cache.lock().await;
					let stats = cache.stats();

					SheetBuilder::new()
						.write_bool(&true)
						.write_u64(stats.get_max_size())
						.write_u64(stats.get_used_size())
						.write_u64(stats.get_total_gets())
						.write_f64(&stats.get_miss_ratio())
						.write_u8(&(stats.get_policy().index() as u8))
						.to_sheet()
				},
			};

			if (connection.send_response(sheet.serialize()).await).is_err() {
				println!("Error sending response to command.");
			}
		}
	}
}
