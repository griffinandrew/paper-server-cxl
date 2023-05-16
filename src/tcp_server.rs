use std::sync::{Arc, Mutex};
use tokio::net::TcpListener;
use tokio::task;
use paper_core::sheet::Sheet;
use paper_cache::PaperCache;
use crate::server_error::{PaperError, ServerError, ErrorKind};
use crate::command::Command;
use crate::tcp_connection::TcpConnection;

pub struct TcpServer {
	listener: TcpListener,
	cache: Arc<Mutex<PaperCache<'static, u64, String>>>,
}

impl TcpServer {
	pub async fn new(
		host: &str,
		port: &u32,
		cache: PaperCache<'static, u64, String>
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
			cache: Arc::new(Mutex::new(cache)),
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

		let local_set = task::LocalSet::new();
		let cache = self.cache.clone();

		local_set.spawn_local(TcpServer::handle_connection(connection, cache));

		local_set.await;

		Ok(())
	}

	async fn handle_connection(
		mut connection: TcpConnection,
		cache: Arc<Mutex<PaperCache<'static, u64, String>>>
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
					let mut guard = cache.lock().unwrap();

					let (is_ok, response) = match guard.get(&key) {
						Ok(response) => (true, response.to_owned()),
						Err(err) => (false, err.message().to_owned()),
					};

					let sheet = Sheet::new(is_ok, response.len() as u32, response);

					if let Err(_) = connection.send_response(&sheet.serialize()).await {
						println!("Error sending response to command.");
					}
				},

				Command::Set(key, value) => {
					let mut guard = cache.lock().unwrap();

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
					let mut guard = cache.lock().unwrap();

					let (is_ok, response) = match guard.del(&key) {
						Ok(_) => (true, "blank".to_owned()),
						Err(err) => (false, err.message().to_owned()),
					};

					let sheet = Sheet::new(is_ok, response.len() as u32, response);

					if let Err(_) = connection.send_response(&sheet.serialize()).await {
						println!("Error sending response to command.");
					}
				},

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
