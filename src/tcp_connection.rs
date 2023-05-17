use std::io;
use std::net::SocketAddr;
use tokio::net::TcpStream;
use crate::server_error::{ServerError, ErrorKind};
use crate::command::Command;

pub struct TcpConnection {
	stream: TcpStream,
	socket: SocketAddr,
}

impl TcpConnection {
	pub fn new(stream: TcpStream, socket: SocketAddr) -> Self {
		TcpConnection {
			stream,
			socket,
		}
	}

	pub async fn get_command(&mut self) -> Result<Command, ServerError> {
		if let Err(_) = self.stream.readable().await {
			return Err(ServerError::new(
				ErrorKind::InvalidStream,
				"An error occured while communicating with the client."
			));
		}

		Command::from_stream(&self.stream).await
	}

	pub async fn send_response(&mut self, buf: &[u8]) -> Result<(), ServerError> {
		loop {
			if let Err(_) = self.stream.writable().await {
				return Err(ServerError::new(
					ErrorKind::InvalidStream,
					"An error occured while communicating with the client."
				));
			}

			match self.stream.try_write(buf) {
				Ok(_) => {
					return Ok(());
				},

				Err(ref e) if e.kind() == io::ErrorKind::WouldBlock => {
					continue;
				},

				Err(_) => {
					return Err(ServerError::new(
						ErrorKind::InvalidResponse,
						"Invalid response."
					));
				},
			}
		}
	}

	pub fn ip(&self) -> String {
		match &self.socket {
			SocketAddr::V4(addr) => {
				addr.ip()
					.octets()
					.iter()
					.map(|&value| value.to_string())
					.collect::<Vec<String>>()
					.join(".")
			},

			SocketAddr::V6(_) => {
				"IPv6".to_owned()
			},
		}
	}
}
