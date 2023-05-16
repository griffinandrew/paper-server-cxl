use tokio::net::TcpListener;
use paper_core::sheet::Sheet;
use crate::server_error::{PaperError, ServerError, ErrorKind};
use crate::command::Command;
use crate::tcp_connection::TcpConnection;

pub struct TcpServer {
	listener: TcpListener,
}

impl TcpServer {
	pub async fn new(host: &str, port: &u32) -> Result<Self, ServerError> {
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

		tokio::spawn(TcpServer::handle_connection(connection));

		Ok(())
	}

	async fn handle_connection(mut connection: TcpConnection) {
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
