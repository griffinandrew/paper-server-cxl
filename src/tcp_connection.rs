use std::io;
use std::net::SocketAddr;
use tokio::net::TcpStream;
use paper_core::error::PaperError;
use paper_core::stream::ErrorKind as StreamErrorKind;
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
		if (self.stream.readable().await).is_err() {
			return Err(ServerError::new(
				ErrorKind::InvalidStream,
				"An error occured while communicating with the client."
			));
		}

		match Command::from_stream(&self.stream).await {
			Ok(command) => Ok(command),

			Err(err) if err.kind() == &StreamErrorKind::Disconnected => Err(ServerError::new(
				ErrorKind::Disconnected,
				"Disconnected from client."
			)),

			Err(err) => Err(ServerError::new(
				ErrorKind::InvalidCommand,
				err.message(),
			)),
		}
	}

	pub async fn send_response(&mut self, buf: &[u8]) -> Result<(), ServerError> {
		loop {
			if (self.stream.writable().await).is_err() {
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
