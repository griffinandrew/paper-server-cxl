use std::io;
use tokio::net::{TcpListener, TcpStream};
use crate::error::{ServerError, ErrorKind};
use crate::command::Command;

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
		let (socket, _) = match self.listener.accept().await {
			Ok(connection) => {
				println!("\x1B[33mConnection:\x1B[0m New connection established.");
				connection
			},

			Err(_) => {
				return Err(ServerError::new(
					ErrorKind::InvalidConnection,
					"Could not establish a connection."
				));
			}
		};

		if let Err(err) = self.process_socket(&socket).await {
			return Err(err);
		}
	}

	async fn process_socket(&mut self, socket: &TcpStream) -> Result<(), ServerError> {
		loop {
			let command = match self.read_command(socket).await {
				Ok(command) => command,

				Err(err) => {
					return Err(err);
				},
			};
		}

		Ok(())
	}

	async fn read_command(&mut self, socket: &TcpStream) -> Result<Command, ServerError> {
		loop {
			let mut buf = [0; 1];

			if let Err(_) = socket.readable().await {
				return Err(ServerError::new(
					ErrorKind::InvalidStream,
					"An error occured while communicating with the client."
				));
			}

			match socket.try_read(&mut buf) {
				Ok(0) => {
					return Err(ServerError::new(
						ErrorKind::InvalidCommand,
						"Invalid command."
					));
				},

				Ok(_) => {
					return Command::deserialize(&buf);
				},

				Err(ref err) if err.kind() == io::ErrorKind::WouldBlock => {
					continue;
				},

				Err(_) => {
					return Err(ServerError::new(
						ErrorKind::InvalidCommand,
						"Invalid command."
					));
				}
			}
		}
	}
}
