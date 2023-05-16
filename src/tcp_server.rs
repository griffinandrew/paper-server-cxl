use std::sync::Arc;
use tokio::sync::Mutex;
use tokio::net::TcpListener;
use paper_core::sheet::Sheet;
use paper_cache::PaperCache;
use crate::server_error::{PaperError, ServerError, ErrorKind};
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

				Err(ref err) if err.kind() == &ErrorKind::ConnectionLost => {
					println!("{}", err.message());
					return;
				},

				Err(err) => {
					println!("\x1B[31mErr\x1B[0m: {}", err.message());
					continue;
				},
			};

			match command {
				Command::Ping => {
					let sheet = Sheet::new(true, 4, "PONG".to_owned());

					if let Err(_) = connection.send_response(&sheet.serialize()).await {
						println!("Error sending response to command.");
					}
				},

				Command::Get(key) => {
					let mut guard = cache.lock().await;

					let (is_ok, response) = match guard.get(&key) {
						Ok(response) => (true, response.to_owned()),
						Err(err) => (false, err.message().to_owned()),
					};

					let sheet = Sheet::new(is_ok, response.len() as u32, response);

					if let Err(_) = connection.send_response(&sheet.serialize()).await {
						println!("Error sending response to command.");
					}
				},

				/*Command::Set(key, value) => {
					let mut guard = cache.write().await;

					let (is_ok, response) = match guard.set(key, value, None) {
						Ok(_) => (true, "456".to_owned()),
						Err(err) => (false, err.message().to_owned()),
					};

					let sheet = Sheet::new(is_ok, response.len() as u32, response);

					if let Err(_) = connection.send_response(&sheet.serialize()).await {
						println!("Error sending response to command.");
					}
				},

				Command::Del(key) => {
					let mut guard = cache.write().await;

					let (is_ok, response) = match guard.del(&key) {
						Ok(_) => (true, "blank".to_owned()),
						Err(err) => (false, err.message().to_owned()),
					};

					let sheet = Sheet::new(is_ok, response.len() as u32, response);

					if let Err(_) = connection.send_response(&sheet.serialize()).await {
						println!("Error sending response to command.");
					}
				},*/

				_ => {
					let sheet = Sheet::new(true, 16, "Sample text here".to_owned());

					if let Err(_) = connection.send_response(&sheet.serialize()).await {
						println!("Error sending response to command.");
					}
				},
			}
		}
	}
}
