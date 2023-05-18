use std::sync::Arc;
use tokio::sync::Mutex;
use tokio::net::TcpListener;
use paper_core::sheet::{Sheet, SheetBuilder};
use paper_cache::{PaperCache, PaperError};
use crate::server_error::{PaperError as ServerPaperError, ServerError, ErrorKind};
use crate::command::Command;
use crate::tcp_connection::TcpConnection;

type Cache = PaperCache<u32, String>;

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

	async fn handle_connection(
		mut connection: TcpConnection,
		cache: Arc<Mutex<Cache>>
	) {
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

			match command {
				Command::Ping => {
					let sheet = Sheet::new(true, b"pong".to_vec());

					if let Err(_) = connection.send_response(&sheet.serialize()).await {
						println!("Error sending response to command.");
					}
				},

				Command::Get(key) => {
					let mut cache = cache.lock().await;

					let (is_ok, response) = match cache.get(&key) {
						Ok(response) => (true, response.as_bytes().to_vec()),
						Err(err) => (false, err.message().as_bytes().to_vec()),
					};

					let sheet = Sheet::new(is_ok, response);

					if let Err(_) = connection.send_response(&sheet.serialize()).await {
						println!("Error sending response to command.");
					}
				},

				Command::Set(key, value, ttl) => {
					let mut cache = cache.lock().await;

					let (is_ok, response) = match cache.set(key, value, ttl) {
						Ok(_) => (true, b"blank".to_vec()),
						Err(err) => (false, err.message().as_bytes().to_vec()),
					};

					let sheet = Sheet::new(is_ok, response);

					if let Err(_) = connection.send_response(&sheet.serialize()).await {
						println!("Error sending response to command.");
					}
				},

				Command::Del(key) => {
					let mut cache = cache.lock().await;

					let (is_ok, response) = match cache.del(&key) {
						Ok(_) => (true, b"blank".to_vec()),
						Err(err) => (false, err.message().as_bytes().to_vec()),
					};

					let sheet = Sheet::new(is_ok, response);

					if let Err(_) = connection.send_response(&sheet.serialize()).await {
						println!("Error sending response to command.");
					}
				},

				Command::Resize(size) => {
					let mut cache = cache.lock().await;

					let (is_ok, response) = match cache.resize(&size) {
						Ok(_) => (true, b"blank".to_vec()),
						Err(err) => (false, err.message().as_bytes().to_vec()),
					};

					let sheet = Sheet::new(is_ok, response);

					if let Err(_) = connection.send_response(&sheet.serialize()).await {
						println!("Error sending response to command.");
					}
				},

				Command::Policy(policy) => {
					let mut cache = cache.lock().await;

					let (is_ok, response) = match cache.policy(policy) {
						Ok(_) => (true, b"blank".to_vec()),
						Err(err) => (false, err.message().as_bytes().to_vec()),
					};

					let sheet = Sheet::new(is_ok, response);

					if let Err(_) = connection.send_response(&sheet.serialize()).await {
						println!("Error sending response to command.");
					}
				},

				Command::Stats => {
					let cache = cache.lock().await;

					let stats = cache.stats();

					let sheet = SheetBuilder::new(true)
						.add_u64(stats.get_max_size())
						.add_u64(stats.get_used_size())
						.add_u64(stats.get_total_gets())
						.add_f64(&stats.get_miss_ratio())
						.to_sheet();

					if let Err(_) = connection.send_response(&sheet.serialize()).await {
						println!("Error sending response to command.");
					}
				},
			}
		}
	}
}
