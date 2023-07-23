use std::{
    io::Write,
    net::TcpStream,
};

use paper_utils::{
    error::PaperError,
    stream::ErrorKind as StreamErrorKind,
};

use crate::{
    server_error::{ServerError, ErrorKind},
    command::Command,
};

pub struct TcpConnection {
	stream: TcpStream,
}

impl TcpConnection {
	pub fn new(stream: TcpStream) -> Self {
		TcpConnection {
			stream,
		}
	}

	pub fn get_command(&mut self) -> Result<Command, ServerError> {
		match Command::from_stream(&mut self.stream) {
			Ok(command) => Ok(command),

			Err(err) if err.kind() == &StreamErrorKind::InvalidStream => Err(ServerError::new(
				ErrorKind::Disconnected,
				"Disconnected from client."
			)),

			Err(err) => Err(ServerError::new(
				ErrorKind::InvalidCommand,
				err.message(),
			)),
		}
	}

	pub fn send_response(&mut self, buf: &[u8]) -> Result<(), ServerError> {
		self.stream.write_all(buf).map_err(|_| {
			ServerError::new(
				ErrorKind::InvalidResponse,
				"Invalid response."
			)
		})
	}
}
