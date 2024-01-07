use std::{
	io::Write,
	net::TcpStream,
};

use paper_utils::stream::StreamError;

use crate::{
	server_error::ServerError,
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
		Command::from_stream(&mut self.stream).map_err(|err| match err {
			StreamError::InvalidStream | StreamError::ClosedStream
				=> ServerError::Disconnected,

			_ => ServerError::InvalidCommand(err.to_string()),
		})
	}

	pub fn send_response(&mut self, buf: &[u8]) -> Result<(), ServerError> {
		self.stream.write_all(buf).map_err(|_| ServerError::InvalidResponse)
	}
}
